use std::sync::OnceLock;
use anyhow::Result;
use tokio::runtime::Runtime;

use crate::types::{BodyType, FormDataValue, HttpMethod};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
/// Shared reqwest client. A `Client` owns the connection pool and is internally
/// reference-counted, so one instance is reused across all requests (keep-alive
/// / pooling / TLS setup are amortized) and cloning is cheap.
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// A fully-read HTTP response. The body is collected on the tokio runtime
/// (reqwest's body stream requires its reactor), so callers can use it freely.
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
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
}

impl HttpClient {
    pub fn new() -> Self {
        let client = CLIENT
            .get_or_init(|| {
                reqwest::Client::builder()
                    .build()
                    .expect("Failed to initialize HTTP client")
            })
            .clone();

        Self { client }
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

        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to initialize tokio runtime")
        });

        let handle = runtime
            .spawn(async move {
                let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())?;
                let mut req = client.request(reqwest_method, &url);

                let is_form = matches!(body, BodyType::FormData(_));
                for (key, value) in &headers {
                    // Never send a manual Content-Length — reqwest computes the correct
                    // one from the actual body. A stale value (e.g. the predefined "0")
                    // truncates the request body server-side (multipart boundary then
                    // can't be found -> 400). reqwest's .header() appends, so we must
                    // skip it here rather than rely on override.
                    if key.eq_ignore_ascii_case("content-length") {
                        continue;
                    }
                    // For multipart, let reqwest set Content-Type — it includes the
                    // boundary. A manually-set one would lack the boundary.
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
                                    // Reads the file and guesses MIME from its extension.
                                    form = form.file(row.key, &path).await.map_err(|e| {
                                        anyhow::anyhow!("Failed to read file '{}': {}", path, e)
                                    })?;
                                }
                            }
                        }
                        req = req.multipart(form);
                    }
                }

                let response = req.send().await?;
                let status = response.status().as_u16();
                let headers = response
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.to_string(), s.to_string())))
                    .collect::<Vec<_>>();
                let body = response.bytes().await?.to_vec();

                Ok::<HttpResponse, anyhow::Error>(HttpResponse {
                    status,
                    headers,
                    body,
                })
            });

        InFlightRequest { handle }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, HttpMethod};
    use std::io::{Read as _, Write as _};

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

        let client = HttpClient::new();
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

        let client = HttpClient::new();
        let inflight = client.start_send(HttpMethod::GET, url, vec![], BodyType::None);

        let response = block_on(inflight.wait()).expect("request should succeed");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hi");
    }
}
