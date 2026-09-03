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
use reqwest::{StatusCode, Url, header::LOCATION};
use tokio::io::AsyncWriteExt as _;
use tokio::runtime::Runtime;

use crate::types::{AppSettings, BodyType, FormDataValue, HttpMethod};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static NEXT_DOWNLOAD_ID: AtomicU64 = AtomicU64::new(1);
const MAX_REDIRECTS: usize = 10;

/// Request headers reqwest already treats as credentials on cross-origin
/// redirects. Auth-configured API keys add their dynamic name to this list at
/// send time.
const STANDARD_CREDENTIAL_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "cookie2",
    "proxy-authorization",
    "www-authenticate",
];

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

/// A redirect chain exceeded the documented maximum number of hops.
#[derive(Debug)]
pub struct RedirectLimitExceeded;

impl std::fmt::Display for RedirectLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Request stopped after {MAX_REDIRECTS} redirects. Check the server for a redirect loop."
        )
    }
}

impl std::error::Error for RedirectLimitExceeded {}

/// A redirect target used a scheme the native HTTP client will not send to.
#[derive(Debug)]
pub struct UnsupportedRedirectScheme;

impl std::fmt::Display for UnsupportedRedirectScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Redirect blocked because its destination is not HTTP or HTTPS."
        )
    }
}

impl std::error::Error for UnsupportedRedirectScheme {}

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
        .or_else(|| {
            error
                .downcast_ref::<RedirectLimitExceeded>()
                .map(ToString::to_string)
        })
        .or_else(|| {
            error
                .downcast_ref::<UnsupportedRedirectScheme>()
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
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(settings.connect_timeout_ms))
            .read_timeout(Duration::from_millis(settings.read_timeout_ms))
            .timeout(Duration::from_millis(settings.total_timeout_ms))
            // Redirects are followed explicitly in `send_request` so an auth
            // header with an arbitrary name can be removed at an origin boundary.
            .redirect(reqwest::redirect::Policy::none());
        // Wire tests must reach their loopback listener directly even when the
        // developer machine has a system proxy configured.
        #[cfg(test)]
        let builder = builder.no_proxy();
        let client = builder.build().expect("Failed to initialize HTTP client");

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
        auth_header_name: Option<String>,
        body: BodyType,
    ) -> InFlightRequest {
        let client = self.client.clone();
        let max_response_size_bytes = self.settings.max_response_size_bytes;
        let total_timeout = Duration::from_millis(self.settings.total_timeout_ms);

        let handle = runtime().spawn(async move {
            match tokio::time::timeout(total_timeout, async move {
                let response =
                    send_request(client, method, url, headers, auth_header_name, body).await?;
                collect_response(response, max_response_size_bytes).await
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(anyhow::Error::new(RequestTimedOut)),
            }
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
        auth_header_name: Option<String>,
        body: BodyType,
        destination: PathBuf,
    ) -> InFlightRequest {
        let client = self.client.clone();
        let total_timeout = Duration::from_millis(self.settings.total_timeout_ms);
        let handle = runtime().spawn(async move {
            match tokio::time::timeout(total_timeout, async move {
                let response =
                    send_request(client, method, url, headers, auth_header_name, body).await?;
                download_response(response, destination).await
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(anyhow::Error::new(RequestTimedOut)),
            }
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
    mut headers: Vec<(String, String)>,
    auth_header_name: Option<String>,
    mut body: BodyType,
) -> Result<reqwest::Response> {
    let mut method = reqwest::Method::from_bytes(method.as_str().as_bytes())?;
    let mut url = Url::parse(&url)?;
    let mut redirect_count = 0;

    loop {
        let response = send_single_request(&client, &method, &url, &headers, &body).await?;
        let status = response.status();
        if !is_redirect(status) {
            return Ok(response);
        }

        let Some(next_url) = redirect_target(&url, &response) else {
            // Match normal user-agent behavior for a redirect without a usable
            // Location header: expose the 3xx response instead of inventing a target.
            return Ok(response);
        };
        if !matches!(next_url.scheme(), "http" | "https") {
            return Err(anyhow::Error::new(UnsupportedRedirectScheme));
        }
        if redirect_count >= MAX_REDIRECTS {
            return Err(anyhow::Error::new(RedirectLimitExceeded));
        }
        redirect_count += 1;

        // Same-origin means an exact scheme/host/effective-port tuple. On any
        // boundary (including HTTPS -> HTTP), credentials are removed before
        // constructing the next request and are never reintroduced later.
        protect_redirect_headers(
            &mut headers,
            auth_header_name.as_deref(),
            &url,
            &next_url,
        );

        adjust_redirect_method_and_body(status, &mut method, &mut body, &mut headers);
        url = next_url;
    }
}

async fn send_single_request(
    client: &reqwest::Client,
    method: &reqwest::Method,
    url: &Url,
    headers: &[(String, String)],
    body: &BodyType,
) -> Result<reqwest::Response> {
    let mut req = client.request(method.clone(), url.clone());

    let is_form = matches!(body, BodyType::FormData(_));
    for (key, value) in headers {
        // Never send a manual Content-Length — reqwest computes the correct one
        // from the actual body. A stale predefined value would truncate it.
        if crate::types::is_transport_owned_header(key) {
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
            req = req.body(content.clone().into_bytes());
        }
        BodyType::FormData(rows) => {
            let mut form = reqwest::multipart::Form::new();
            for row in rows {
                if !row.enabled || row.key.is_empty() {
                    continue;
                }
                match &row.value {
                    FormDataValue::Text(text) => {
                        form = form.text(row.key.clone(), text.clone());
                    }
                    FormDataValue::File { path } => {
                        if path.is_empty() {
                            continue;
                        }
                        form = form.file(row.key.clone(), path).await.map_err(|e| {
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

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn redirect_target(current: &Url, response: &reqwest::Response) -> Option<Url> {
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    current.join(location).ok()
}

/// RFC origin tuple used for credential forwarding. Default and explicit
/// ports compare equal (for example, `https://host` and `https://host:443`).
fn is_same_origin(previous: &Url, next: &Url) -> bool {
    previous.scheme() == next.scheme()
        && previous.host_str() == next.host_str()
        && previous.port_or_known_default() == next.port_or_known_default()
}

fn protect_redirect_headers(
    headers: &mut Vec<(String, String)>,
    auth_header_name: Option<&str>,
    previous: &Url,
    next: &Url,
) {
    if is_same_origin(previous, next) {
        return;
    }

    headers.retain(|(name, _)| {
        let is_standard = STANDARD_CREDENTIAL_HEADERS
            .iter()
            .any(|sensitive| name.eq_ignore_ascii_case(sensitive));
        let is_configured_auth =
            auth_header_name.is_some_and(|auth| name.eq_ignore_ascii_case(auth));
        !is_standard && !is_configured_auth
    });
}

fn adjust_redirect_method_and_body(
    status: StatusCode,
    method: &mut reqwest::Method,
    body: &mut BodyType,
    headers: &mut Vec<(String, String)>,
) {
    let switch_to_get = match status {
        StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND => *method == reqwest::Method::POST,
        StatusCode::SEE_OTHER => *method != reqwest::Method::HEAD,
        _ => false,
    };
    let drop_body = switch_to_get || status == StatusCode::SEE_OTHER;

    if switch_to_get {
        *method = reqwest::Method::GET;
    }
    if drop_body {
        *body = BodyType::None;
        headers.retain(|(name, _)| {
            ![
                "content-type",
                "content-length",
                "content-encoding",
                "transfer-encoding",
            ]
            .iter()
            .any(|payload| name.eq_ignore_ascii_case(payload))
        });
    }
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
    use crate::types::{AppSettings, BodyType, FormDataRow, HttpMethod, RequestData};
    use flate2::{Compression, write::GzEncoder};
    use std::io::{Read as _, Write as _};
    use std::sync::mpsc;
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

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8(request).unwrap()
    }

    #[derive(Debug)]
    struct CapturedRequest {
        head: String,
        body: Vec<u8>,
    }

    impl CapturedRequest {
        fn header(&self, expected_name: &str) -> Option<&str> {
            self.head.lines().skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case(expected_name)
                    .then_some(value.trim())
            })
        }
    }

    fn read_complete_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(offset) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break offset + 4;
            }
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "connection closed before request headers arrived");
            request.extend_from_slice(&chunk[..read]);
        };

        let head = String::from_utf8(request[..header_end].to_vec()).unwrap();
        let content_length = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "connection closed before request body arrived");
            request.extend_from_slice(&chunk[..read]);
        }

        CapturedRequest {
            head,
            body: request[header_end..header_end + content_length].to_vec(),
        }
    }

    fn capture_request(
        method: HttpMethod,
        headers: Vec<(String, String)>,
        body: BodyType,
    ) -> CapturedRequest {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let (request_tx, request_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            request_tx.send(read_complete_request(&mut stream)).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });

        let inflight =
            HttpClient::new(test_settings()).start_send(method, url, headers, None, body);
        block_on(inflight.wait()).expect("captured request should succeed");
        request_rx.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    #[test]
    fn variable_backed_unicode_body_gets_its_real_byte_length() {
        let unresolved = RequestData {
            method: HttpMethod::POST,
            url: "https://unused.test".into(),
            headers: vec![("Content-Length".into(), "1".into())],
            body: BodyType::Raw {
                content: "{{message}}".into(),
                subtype: crate::types::RawSubtype::Text,
            },
            auth: crate::types::AuthConfig::default(),
        };
        let env =
            std::collections::HashMap::from([("message".to_string(), "你好，Poopman".to_string())]);
        let resolved = crate::variables::substitute_request(&unresolved, &env);
        let expected_body = match &resolved.body {
            BodyType::Raw { content, .. } => content.as_bytes(),
            _ => unreachable!(),
        };
        let captured = capture_request(
            resolved.method,
            crate::types::effective_wire_headers(&resolved.headers, &resolved.auth),
            resolved.body.clone(),
        );

        assert_eq!(captured.body, expected_body);
        assert_eq!(
            captured
                .header("Content-Length")
                .and_then(|value| value.parse::<usize>().ok()),
            Some(expected_body.len())
        );
    }

    #[test]
    fn empty_body_has_no_injected_editor_headers() {
        let captured = capture_request(
            HttpMethod::POST,
            vec![("Content-Length".into(), "999".into())],
            BodyType::None,
        );

        assert!(captured.body.is_empty());
        if let Some(length) = captured.header("Content-Length") {
            assert_eq!(length, "0");
        }
        // reqwest 0.13 owns its own `Accept: */*` transport default. The
        // application-level defaults that used to come from editor rows must
        // not leak onto an otherwise headerless request.
        for name in ["Cache-Control", "Content-Type", "User-Agent", "Connection"] {
            assert_eq!(captured.header(name), None, "unexpected {name} header");
        }
    }

    #[test]
    fn multipart_boundary_and_length_are_generated_from_encoded_body() {
        let captured = capture_request(
            HttpMethod::POST,
            vec![
                ("Content-Length".into(), "0".into()),
                (
                    "Content-Type".into(),
                    "multipart/form-data; boundary=<auto>".into(),
                ),
            ],
            BodyType::FormData(vec![
                FormDataRow {
                    enabled: true,
                    key: "message".into(),
                    value: FormDataValue::Text("你好".into()),
                },
                FormDataRow {
                    enabled: false,
                    key: "ignored".into(),
                    value: FormDataValue::Text("not-on-wire".into()),
                },
            ]),
        );

        let content_type = captured.header("Content-Type").unwrap();
        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(!content_type.contains("<auto>"));
        assert_eq!(
            captured.header("Content-Length").unwrap(),
            captured.body.len().to_string()
        );
        let body = String::from_utf8(captured.body).unwrap();
        assert!(body.contains("name=\"message\""));
        assert!(body.contains("你好"));
        assert!(!body.contains("not-on-wire"));
    }

    #[test]
    fn abort_maps_to_request_canceled_error() {
        // A listener that accepts but never responds: the request hangs
        // until aborted, no matter how fast or slow the test thread is.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());

        let client = HttpClient::new(test_settings());
        let inflight = client.start_send(HttpMethod::GET, url, vec![], None, BodyType::None);
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
        let inflight = client.start_send(HttpMethod::GET, url, vec![], None, BodyType::None);

        let response = block_on(inflight.wait()).expect("request should succeed");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hi");
    }

    #[test]
    fn same_origin_redirect_preserves_custom_auth_header() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/start", listener.local_addr().unwrap());
        let (request_tx, request_rx) = mpsc::channel();

        std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let _ = read_request(&mut first);
            first
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /finish\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();

            let (mut second, _) = listener.accept().unwrap();
            request_tx.send(read_request(&mut second)).unwrap();
            second
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .unwrap();
        });

        let inflight = HttpClient::new(test_settings()).start_send(
            HttpMethod::GET,
            url,
            vec![("X-API-Key".into(), "same-origin-secret".into())],
            Some("X-API-Key".into()),
            BodyType::None,
        );
        let response = block_on(inflight.wait()).expect("same-origin redirect should succeed");
        let redirected_request = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        assert_eq!(response.status, 200);
        assert!(
            redirected_request
                .to_ascii_lowercase()
                .contains("x-api-key: same-origin-secret")
        );
    }

    #[test]
    fn cross_origin_redirect_removes_custom_and_standard_credentials() {
        let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target_url = format!("http://{}/finish", target.local_addr().unwrap());
        let source = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let source_url = format!("http://{}/start", source.local_addr().unwrap());
        let (request_tx, request_rx) = mpsc::channel();

        std::thread::spawn(move || {
            let (mut stream, _) = source.accept().unwrap();
            let _ = read_request(&mut stream);
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
        });
        std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            request_tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .unwrap();
        });

        let inflight = HttpClient::new(test_settings()).start_send(
            HttpMethod::GET,
            source_url,
            vec![
                ("X-API-Key".into(), "cross-origin-secret".into()),
                ("Authorization".into(), "Bearer standard-secret".into()),
                ("X-Trace".into(), "keep-me".into()),
            ],
            Some("X-API-Key".into()),
            BodyType::None,
        );
        let response = block_on(inflight.wait()).expect("sanitized redirect should succeed");
        let redirected_request = request_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .to_ascii_lowercase();

        assert_eq!(response.status, 200);
        assert!(!redirected_request.contains("x-api-key:"));
        assert!(!redirected_request.contains("authorization:"));
        assert!(redirected_request.contains("x-trace: keep-me"));
    }

    #[test]
    fn https_to_http_redirect_removes_custom_auth_header() {
        let previous = Url::parse("https://api.example.test:443/start").unwrap();
        let next = Url::parse("http://api.example.test:443/finish").unwrap();
        let mut headers = vec![
            ("X-Custom-Secret".into(), "secret".into()),
            ("Accept".into(), "application/json".into()),
        ];

        protect_redirect_headers(&mut headers, Some("X-Custom-Secret"), &previous, &next);

        assert_eq!(headers, vec![("Accept".into(), "application/json".into())]);
    }

    #[test]
    fn redirect_limit_has_a_safe_actionable_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/loop", listener.local_addr().unwrap());

        std::thread::spawn(move || {
            for _ in 0..=MAX_REDIRECTS {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = read_request(&mut stream);
                stream
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        });

        let inflight = HttpClient::new(test_settings()).start_send(
            HttpMethod::GET,
            url,
            vec![],
            None,
            BodyType::None,
        );
        let error = block_on(inflight.wait()).expect_err("redirect loop should stop");

        assert!(error.downcast_ref::<RedirectLimitExceeded>().is_some());
        assert_eq!(
            actionable_error_message(&error).as_deref(),
            Some("Request stopped after 10 redirects. Check the server for a redirect loop.")
        );
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
        let inflight = HttpClient::new(settings).start_send(
            HttpMethod::GET,
            url,
            vec![],
            None,
            BodyType::None,
        );
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
        let inflight = HttpClient::new(settings).start_send(
            HttpMethod::GET,
            url,
            vec![],
            None,
            BodyType::None,
        );
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
        let inflight = HttpClient::new(settings).start_send(
            HttpMethod::GET,
            url,
            vec![],
            None,
            BodyType::None,
        );
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
        let inflight = HttpClient::new(settings).start_send(
            HttpMethod::GET,
            url,
            vec![],
            None,
            BodyType::None,
        );
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
            None,
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
