use std::sync::OnceLock;
use anyhow::Result;
use tokio::runtime::Runtime;

use crate::types::{BodyType, FormDataValue, HttpMethod};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// A fully-read HTTP response. The body is collected on the tokio runtime
/// (reqwest's body stream requires its reactor), so callers can use it freely.
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// HTTP client that builds reqwest requests natively and manages its own
/// tokio runtime.
pub struct HttpClient {
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .build()
            .expect("Failed to initialize HTTP client");

        Self { client }
    }

    /// Send a request built from our own model.
    ///
    /// - `BodyType::Raw` is sent as a raw byte body.
    /// - `BodyType::FormData` is sent as real `multipart/form-data` via
    ///   reqwest's `multipart::Form` (it generates the boundary and the
    ///   `Content-Type` header; file parts are read from disk with their MIME
    ///   guessed from the extension).
    pub async fn send(
        &self,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: BodyType,
    ) -> Result<HttpResponse> {
        let client = self.client.clone();

        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to initialize tokio runtime")
        });

        runtime
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
            })
            .await?
    }
}
