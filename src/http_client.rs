use std::{
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use anyhow::Result;
use futures::StreamExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::runtime::Runtime;

use crate::types::{AppSettings, BodyType, FormDataValue, HttpMethod};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static NEXT_DOWNLOAD_ID: AtomicU64 = AtomicU64::new(1);

/// A completed HTTP response. Normal sends collect a bounded body on the Tokio
/// runtime; download sends retain only metadata because their body is streamed
/// to a destination file.
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// A download uses the exact same request path, but streams decoded chunks
    /// to this destination instead of retaining a second complete copy in RAM.
    pub downloaded_to: Option<PathBuf>,
    pub downloaded_bytes: Option<u64>,
}

/// Marker error: the in-flight request was aborted by the user.
/// Callers detect it with `err.downcast_ref::<RequestCanceled>()`.
#[derive(Debug)]
pub struct RequestCanceled;

impl std::fmt::Display for RequestCanceled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request canceled")
    }
}

impl std::error::Error for RequestCanceled {}

/// A reqwest timeout expressed in language that is safe and useful to show in
/// the response pane. Raw transport errors can include a URL with credentials.
#[derive(Debug)]
pub struct RequestTimedOut;

impl std::fmt::Display for RequestTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Request timed out. Adjust the connection, read idle, or total timeout in Settings → General."
        )
    }
}

impl std::error::Error for RequestTimedOut {}

/// Raised only after counting chunks yielded from reqwest's decoded response
/// stream. `Content-Length` is deliberately not trusted for this safeguard.
#[derive(Debug)]
pub struct ResponseLimitExceeded {
    pub limit_bytes: u64,
}

impl std::fmt::Display for ResponseLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Response exceeds the {} viewer limit. Increase it in Settings → General, or use Download to stream the response directly to a file.",
            format_byte_limit(self.limit_bytes),
        )
    }
}

impl std::error::Error for ResponseLimitExceeded {}

/// Return a user-facing error only for errors that we explicitly made safe to
/// expose. Other transport failures intentionally remain the generic
/// "Request failed" so resolved URLs and their secrets never leak into UI.
pub fn actionable_error_message(error: &anyhow::Error) -> Option<String> {
    error
        .downcast_ref::<RequestTimedOut>()
        .map(ToString::to_string)
        .or_else(|| {
            error
                .downcast_ref::<ResponseLimitExceeded>()
                .map(ToString::to_string)
        })
}

/// A request already running on the tokio runtime. `abort_handle()` lets the
/// UI abort the underlying task — the transfer really stops, the result isn't
/// merely ignored. Await `wait()` for the outcome.
pub struct InFlightRequest {
    handle: tokio::task::JoinHandle<Result<HttpResponse>>,
}

impl InFlightRequest {
    pub fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.handle.abort_handle()
    }

    pub async fn wait(self) -> Result<HttpResponse> {
        match self.handle.await {
            Ok(result) => result,
            Err(e) if e.is_cancelled() => Err(anyhow::Error::new(RequestCanceled)),
            Err(e) => Err(e.into()),
        }
    }
}

/// HTTP client that builds reqwest requests natively and manages its own
/// tokio runtime.
pub struct HttpClient {
    client: reqwest::Client,
    settings: AppSettings,
}

impl HttpClient {
    /// Build a client for one request. Reqwest's timeout configuration belongs
    /// to a `Client`, so creating it at this boundary means a General-settings
    /// edit affects the next request immediately rather than only after restart.
    pub fn new(settings: AppSettings) -> Self {
        let settings = settings.normalized();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(settings.connect_timeout_ms))
            .read_timeout(Duration::from_millis(settings.read_timeout_ms))
            .timeout(Duration::from_millis(settings.total_timeout_ms))
            .build()
            .expect("Failed to initialize HTTP client");

        Self { client, settings }
    }

    /// Spawn a request built from our own model onto the shared tokio runtime
    /// and return immediately with a cancellable [`InFlightRequest`].
    ///
    /// - `BodyType::Raw` is sent as a raw byte body.
    /// - `BodyType::FormData` is sent as real `multipart/form-data` via
    ///   reqwest's `multipart::Form` (it generates the boundary and the
    ///   `Content-Type` header; file parts are read from disk with their MIME
    ///   guessed from the extension).
    pub fn start_send(
        &self,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: BodyType,
    ) -> InFlightRequest {
        let client = self.client.clone();
        let max_response_size_bytes = self.settings.max_response_size_bytes;

        let handle = runtime().spawn(async move {
            let response = send_request(client, method, url, headers, body).await?;
            collect_response(response, max_response_size_bytes).await
        });

        InFlightRequest { handle }
    }

    /// Start a request whose decoded body is streamed to `destination`. The
    /// response status and headers still reach the UI, while the body stays out
    /// of the response viewer and never needs a complete in-memory allocation.
    pub fn start_download(
        &self,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: BodyType,
        destination: PathBuf,
    ) -> InFlightRequest {
        let client = self.client.clone();
        let handle = runtime().spawn(async move {
            let response = send_request(client, method, url, headers, body).await?;
            download_response(response, destination).await
        });
        InFlightRequest { handle }
    }
}

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to initialize tokio runtime")
    })
}

async fn send_request(
    client: reqwest::Client,
    method: HttpMethod,
    url: String,
    headers: Vec<(String, String)>,
    body: BodyType,
) -> Result<reqwest::Response> {
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())?;
    let mut req = client.request(reqwest_method, &url);

    let is_form = matches!(body, BodyType::FormData(_));
    for (key, value) in &headers {
        // Never send a manual Content-Length — reqwest computes the correct one
        // from the actual body. A stale predefined value would truncate it.
        if key.eq_ignore_ascii_case("content-length") {
            continue;
        }
        // For multipart, reqwest owns Content-Type so it includes its boundary.
        if is_form && key.eq_ignore_ascii_case("content-type") {
            continue;
        }
        req = req.header(key.as_str(), value.as_str());
    }

    match body {
        BodyType::None => {}
        BodyType::Raw { content, .. } => {
            req = req.body(content.into_bytes());
        }
        BodyType::FormData(rows) => {
            let mut form = reqwest::multipart::Form::new();
            for row in rows {
                if !row.enabled || row.key.is_empty() {
                    continue;
                }
                match row.value {
                    FormDataValue::Text(text) => {
                        form = form.text(row.key, text);
                    }
                    FormDataValue::File { path } => {
                        if path.is_empty() {
                            continue;
                        }
                        form = form.file(row.key, &path).await.map_err(|e| {
                            anyhow::anyhow!("Failed to read file '{}': {}", path, e)
                        })?;
                    }
                }
            }
            req = req.multipart(form);
        }
    }

    req.send().await.map_err(classify_reqwest_error)
}

fn response_metadata(response: &reqwest::Response) -> (u16, Vec<(String, String)>) {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
        .collect();
    (status, headers)
}

async fn collect_response(response: reqwest::Response, limit_bytes: u64) -> Result<HttpResponse> {
    let (status, headers) = response_metadata(&response);
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_reqwest_error)?;
        let next_len = body.len().saturating_add(chunk.len()) as u64;
        if next_len > limit_bytes {
            return Err(anyhow::Error::new(ResponseLimitExceeded { limit_bytes }));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(HttpResponse {
        status,
        headers,
        body,
        downloaded_to: None,
        downloaded_bytes: None,
    })
}

/// Drop-cleaned partial file. Aborting the tokio task drops this guard, so a
/// canceled/erroring transfer never leaves a misleading completed destination.
struct PartialDownload {
    path: PathBuf,
    keep: bool,
}

impl PartialDownload {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for PartialDownload {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn partial_download_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("response.bin");
    let id = NEXT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed);
    destination.with_file_name(format!(
        ".{name}.poopman-download-{}-{id}.part",
        std::process::id()
    ))
}

async fn download_response(response: reqwest::Response, destination: PathBuf) -> Result<HttpResponse> {
    if destination.exists() {
        return Err(anyhow::anyhow!(
            "The selected download destination already exists. Choose a new filename."
        ));
    }

    let (status, headers) = response_metadata(&response);
    let partial_path = partial_download_path(&destination);
    let mut partial = PartialDownload::new(partial_path.clone());
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial_path)
        .await
        .map_err(|e| anyhow::anyhow!("Could not create the selected download file: {e}"))?;

    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_reqwest_error)?;
        file.write_all(&chunk)
            .await
            .map_err(|e| anyhow::anyhow!("Could not write the download file: {e}"))?;
        downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
    }
    file.flush()
        .await
        .map_err(|e| anyhow::anyhow!("Could not finish writing the download file: {e}"))?;
    drop(file);
    tokio::fs::rename(&partial_path, &destination)
        .await
        .map_err(|e| anyhow::anyhow!("Could not finalize the download file: {e}"))?;
    partial.keep();

    Ok(HttpResponse {
        status,
        headers,
        body: vec![],
        downloaded_to: Some(destination),
        downloaded_bytes: Some(downloaded_bytes),
    })
}

fn classify_reqwest_error(error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        anyhow::Error::new(RequestTimedOut)
    } else {
        error.into()
    }
}

fn format_byte_limit(bytes: u64) -> String {
    if bytes % (1024 * 1024) == 0 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes % 1024 == 0 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AppSettings, BodyType, HttpMethod};
    use flate2::{Compression, write::GzEncoder};
    use std::io::{Read as _, Write as _};
    use std::time::Duration;

    fn test_settings() -> AppSettings {
        AppSettings::default()
    }

    /// Block on a future using the same runtime `start_send` spawned onto.
    /// (Awaiting a JoinHandle from outside the runtime is exactly what the
    /// gpui side does in production.)
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        RUNTIME
            .get()
            .expect("start_send initializes the runtime")
            .block_on(fut)
    }

    #[test]
    fn abort_maps_to_request_canceled_error() {
        // A listener that accepts but never responds: the request hangs
        // until aborted, no matter how fast or slow the test thread is.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());

        let client = HttpClient::new(test_settings());
        let inflight = client.start_send(HttpMethod::GET, url, vec![], BodyType::None);
        inflight.abort_handle().abort();

        let err = block_on(inflight.wait()).expect_err("aborted request must fail");
        assert!(
            err.downcast_ref::<RequestCanceled>().is_some(),
            "expected RequestCanceled, got: {err:#}"
        );
    }

    #[test]
    fn start_send_completes_normally_without_abort() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());

        // Minimal one-shot HTTP server on a plain thread.
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf); // consume the request
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
                )
                .unwrap();
        });

        let client = HttpClient::new(test_settings());
        let inflight = client.start_send(HttpMethod::GET, url, vec![], BodyType::None);

        let response = block_on(inflight.wait()).expect("request should succeed");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hi");
    }

    #[test]
    fn total_timeout_stops_a_server_that_never_responds() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_millis(180));
        });

        let mut settings = test_settings();
        settings.total_timeout_ms = 40;
        settings.read_timeout_ms = 500;
        let inflight = HttpClient::new(settings).start_send(HttpMethod::GET, url, vec![], BodyType::None);
        let error = block_on(inflight.wait()).expect_err("stalled request should time out");
        assert!(error.downcast_ref::<RequestTimedOut>().is_some(), "{error:#}");
    }

    #[test]
    fn read_idle_timeout_stops_a_stalled_response_body() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(180));
        });

        let mut settings = test_settings();
        settings.read_timeout_ms = 40;
        settings.total_timeout_ms = 500;
        let inflight = HttpClient::new(settings).start_send(HttpMethod::GET, url, vec![], BodyType::None);
        let error = block_on(inflight.wait()).expect_err("idle body should time out");
        assert!(error.downcast_ref::<RequestTimedOut>().is_some(), "{error:#}");
    }

    #[test]
    fn chunked_response_limit_does_not_trust_content_length() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n400\r\n",
                )
                .unwrap();
            stream.write_all(&vec![b'x'; 1024]).unwrap();
            stream.write_all(b"\r\n400\r\n").unwrap();
            stream.write_all(&vec![b'y'; 1024]).unwrap();
            stream.write_all(b"\r\n0\r\n\r\n").unwrap();
        });

        let mut settings = test_settings();
        settings.max_response_size_bytes = 1_024;
        let inflight = HttpClient::new(settings).start_send(HttpMethod::GET, url, vec![], BodyType::None);
        let error = block_on(inflight.wait()).expect_err("chunked body should exceed limit");
        let limit = error
            .downcast_ref::<ResponseLimitExceeded>()
            .expect("expected response limit error");
        assert_eq!(limit.limit_bytes, 1_024);
    }

    #[test]
    fn compressed_response_limit_counts_decoded_bytes() {
        let plain = vec![b'x'; 4 * 1024];
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&plain).unwrap();
        let compressed = gzip.finish().unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        compressed.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            stream.write_all(&compressed).unwrap();
        });

        let mut settings = test_settings();
        settings.max_response_size_bytes = 1_024;
        let inflight = HttpClient::new(settings).start_send(HttpMethod::GET, url, vec![], BodyType::None);
        let error = block_on(inflight.wait()).expect_err("decoded gzip body should exceed limit");
        assert!(error.downcast_ref::<ResponseLimitExceeded>().is_some(), "{error:#}");
    }

    #[test]
    fn download_streams_to_disk_without_retaining_body() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world",
                )
                .unwrap();
        });
        let destination = std::env::temp_dir().join(format!(
            "poopman-http-client-test-{}-{}.bin",
            std::process::id(),
            NEXT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed)
        ));

        let inflight = HttpClient::new(test_settings()).start_download(
            HttpMethod::GET,
            url,
            vec![],
            BodyType::None,
            destination.clone(),
        );
        let response = block_on(inflight.wait()).expect("download should succeed");
        assert!(response.body.is_empty());
        assert_eq!(response.downloaded_bytes, Some(11));
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello world");
        std::fs::remove_file(destination).unwrap();
    }
}
