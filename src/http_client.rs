use std::sync::OnceLock;
use anyhow::Result;
use gpui::http_client::http;
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Simple HTTP client that manages its own tokio runtime
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

    pub async fn send(
        &self,
        request: http::Request<gpui::http_client::AsyncBody>,
    ) -> Result<http::Response<gpui::http_client::AsyncBody>> {
        let (parts, body) = request.into_parts();

        // Convert AsyncBody to bytes
        let body_bytes = match body.0 {
            gpui::http_client::Inner::Empty => vec![],
            gpui::http_client::Inner::Bytes(cursor) => cursor.into_inner().to_vec(),
            gpui::http_client::Inner::AsyncReader(mut reader) => {
                use futures::AsyncReadExt;
                let mut bytes = Vec::new();
                reader.read_to_end(&mut bytes).await?;
                bytes
            }
        };

        // Build reqwest request
        let mut req = self.client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers);

        if !body_bytes.is_empty() {
            req = req.body(body_bytes);
        }

        // Send request in tokio runtime
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to initialize tokio runtime")
        });

        let response = runtime.spawn(async move {
            req.send().await
        }).await??;

        // Convert response
        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response.bytes().await?;

        let mut builder = http::Response::builder()
            .status(status)
            .version(http::Version::HTTP_11);

        *builder.headers_mut().unwrap() = headers;

        let body = gpui::http_client::AsyncBody::from(body_bytes.to_vec());
        Ok(builder.body(body)?)
    }
}
