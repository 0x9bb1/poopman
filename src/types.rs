use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Limits and transfer behavior applied to every newly started HTTP request.
///
/// These values deliberately live outside individual requests: they are client
/// safeguards, not data that should be saved into a collection or history item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Maximum time allowed to establish a TCP/TLS connection.
    pub connect_timeout_ms: u64,
    /// Maximum period with no response-body bytes arriving.
    pub read_timeout_ms: u64,
    /// Wall-clock limit for the complete request, including the response body.
    pub total_timeout_ms: u64,
    /// Maximum decoded bytes retained for display in the response viewer.
    pub max_response_size_bytes: u64,
}

impl AppSettings {
    pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;
    pub const DEFAULT_READ_TIMEOUT_MS: u64 = 30_000;
    pub const DEFAULT_TOTAL_TIMEOUT_MS: u64 = 60_000;
    pub const DEFAULT_MAX_RESPONSE_SIZE_BYTES: u64 = 50 * 1024 * 1024;

    const MAX_TIMEOUT_MS: u64 = 3_600_000;
    const MIN_RESPONSE_SIZE_BYTES: u64 = 1_024;
    const MAX_RESPONSE_SIZE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

    /// Protect startup from a malformed or hand-edited persisted value. The UI
    /// performs the same validation before allowing a change to take effect.
    pub fn normalized(mut self) -> Self {
        self.connect_timeout_ms = Self::valid_timeout(
            self.connect_timeout_ms,
            Self::DEFAULT_CONNECT_TIMEOUT_MS,
        );
        self.read_timeout_ms =
            Self::valid_timeout(self.read_timeout_ms, Self::DEFAULT_READ_TIMEOUT_MS);
        self.total_timeout_ms =
            Self::valid_timeout(self.total_timeout_ms, Self::DEFAULT_TOTAL_TIMEOUT_MS);
        if !(Self::MIN_RESPONSE_SIZE_BYTES..=Self::MAX_RESPONSE_SIZE_BYTES)
            .contains(&self.max_response_size_bytes)
        {
            self.max_response_size_bytes = Self::DEFAULT_MAX_RESPONSE_SIZE_BYTES;
        }
        self
    }

    fn valid_timeout(value: u64, fallback: u64) -> u64 {
        if (1..=Self::MAX_TIMEOUT_MS).contains(&value) {
            value
        } else {
            fallback
        }
    }

    pub fn response_limit_mebibytes(&self) -> u64 {
        self.max_response_size_bytes.div_ceil(1024 * 1024)
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            connect_timeout_ms: Self::DEFAULT_CONNECT_TIMEOUT_MS,
            read_timeout_ms: Self::DEFAULT_READ_TIMEOUT_MS,
            total_timeout_ms: Self::DEFAULT_TOTAL_TIMEOUT_MS,
            max_response_size_bytes: Self::DEFAULT_MAX_RESPONSE_SIZE_BYTES,
        }
    }
}

/// Header type for distinguishing predefined vs custom headers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeaderType {
    /// Legacy persisted value for headers that could not be disabled.
    /// New editor rows never use this variant.
    Mandatory,
    /// Predefined header that can be toggled but not deleted
    Predefined,
    /// Custom user-defined header that can be toggled and deleted
    Custom,
}

/// Predefined header names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PredefinedHeader {
    CacheControl,
    ContentType,
    Accept,
    UserAgent,
    Connection,
    ContentLength,
}

impl PredefinedHeader {
    pub fn name(&self) -> &'static str {
        match self {
            PredefinedHeader::CacheControl => "Cache-Control",
            PredefinedHeader::ContentType => "Content-Type",
            PredefinedHeader::Accept => "Accept",
            PredefinedHeader::UserAgent => "User-Agent",
            PredefinedHeader::Connection => "Connection",
            PredefinedHeader::ContentLength => "Content-Length",
        }
    }

    pub fn default_value(&self) -> &'static str {
        match self {
            PredefinedHeader::CacheControl => "no-cache",
            PredefinedHeader::ContentType => "application/json",
            PredefinedHeader::Accept => "*/*",
            PredefinedHeader::UserAgent => "PostmanRuntime/7.48.0",
            PredefinedHeader::Connection => "keep-alive",
            // Kept only so old serialized `PredefinedHeader` values still decode.
            // Content-Length is owned by the HTTP transport and has no editor value.
            PredefinedHeader::ContentLength => "",
        }
    }

    pub fn header_type(&self) -> HeaderType {
        HeaderType::Predefined
    }

    /// Headers represented by rows in the editor. The `ContentLength` enum
    /// variant remains for backward-compatible deserialization, but is never
    /// user-managed.
    pub fn editable() -> Vec<Self> {
        vec![
            PredefinedHeader::CacheControl,
            PredefinedHeader::ContentType,
            PredefinedHeader::Accept,
            PredefinedHeader::UserAgent,
            PredefinedHeader::Connection,
        ]
    }
}

/// Metadata calculated by the HTTP transport from the final encoded body.
/// Matching is case-insensitive because HTTP field names are case-insensitive.
pub fn is_transport_owned_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("content-length")
}

/// HTTP methods supported by the API client.
///
/// Variant names are all-caps on purpose: they match the wire format and are
/// serialized by name into the history database, so renaming them would break
/// previously saved requests.
#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::OPTIONS => "OPTIONS",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
        ]
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Some(HttpMethod::GET),
            "POST" => Some(HttpMethod::POST),
            "PUT" => Some(HttpMethod::PUT),
            "DELETE" => Some(HttpMethod::DELETE),
            "PATCH" => Some(HttpMethod::PATCH),
            "HEAD" => Some(HttpMethod::HEAD),
            "OPTIONS" => Some(HttpMethod::OPTIONS),
            _ => None,
        }
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Raw body subtype for syntax highlighting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RawSubtype {
    Json,
    Xml,
    Text,
    JavaScript,
    UrlEncoded,
}

impl RawSubtype {
    /// Returns the language string for syntax highlighting.
    ///
    /// Note: XML is not supported by gpui-component's tree-sitter-languages feature,
    /// so it falls back to "plain" (no syntax highlighting).
    pub fn as_str(&self) -> &'static str {
        match self {
            RawSubtype::Json => "json",
            RawSubtype::Xml => "plain", // XML not supported, fallback to plain
            RawSubtype::Text => "plain",
            RawSubtype::JavaScript => "javascript",
            RawSubtype::UrlEncoded => "plain",
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            RawSubtype::Json => "application/json",
            RawSubtype::Xml => "application/xml",
            RawSubtype::Text => "text/plain",
            RawSubtype::JavaScript => "application/javascript",
            RawSubtype::UrlEncoded => "application/x-www-form-urlencoded",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            RawSubtype::Json,
            RawSubtype::Xml,
            RawSubtype::Text,
            RawSubtype::JavaScript,
            RawSubtype::UrlEncoded,
        ]
    }
}

/// Form-data value type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormDataValue {
    Text(String),
    File { path: String },
}

/// Form-data row
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDataRow {
    pub enabled: bool,
    pub key: String,
    pub value: FormDataValue,
}

impl FormDataRow {
    /// Editor-only empty rows are affordances, not request data.
    pub fn is_blank(&self) -> bool {
        self.key.is_empty()
            && match &self.value {
                FormDataValue::Text(value) => value.is_empty(),
                FormDataValue::File { path } => path.is_empty(),
            }
    }
}

/// Request body type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyType {
    None,
    Raw {
        content: String,
        subtype: RawSubtype,
    },
    FormData(Vec<FormDataRow>),
}

impl Default for BodyType {
    fn default() -> Self {
        BodyType::Raw {
            content: String::new(),
            subtype: RawSubtype::Json,
        }
    }
}

/// Selected body panel in the editor. Unlike [`BodyType`], this does not own
/// the panel contents, which lets inactive Raw and Form-data drafts survive a
/// tab switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    None,
    Raw,
    FormData,
}

/// Complete, per-tab body editor state. Only `selected_body` is persisted or
/// sent; the other fields are private drafts for inactive body panels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyDraft {
    pub kind: BodyKind,
    pub raw_content: String,
    pub raw_subtype: RawSubtype,
    pub formdata_rows: Vec<FormDataRow>,
}

impl BodyDraft {
    pub fn from_body(body: &BodyType) -> Self {
        match body {
            BodyType::None => Self {
                kind: BodyKind::None,
                raw_content: String::new(),
                raw_subtype: RawSubtype::Json,
                formdata_rows: Vec::new(),
            },
            BodyType::Raw { content, subtype } => Self {
                kind: BodyKind::Raw,
                raw_content: content.clone(),
                raw_subtype: *subtype,
                formdata_rows: Vec::new(),
            },
            BodyType::FormData(rows) => Self {
                kind: BodyKind::FormData,
                raw_content: String::new(),
                raw_subtype: RawSubtype::Json,
                formdata_rows: rows.iter().filter(|row| !row.is_blank()).cloned().collect(),
            },
        }
    }

    pub fn selected_body(&self) -> BodyType {
        match self.kind {
            BodyKind::None => BodyType::None,
            BodyKind::Raw => BodyType::Raw {
                content: self.raw_content.clone(),
                subtype: self.raw_subtype,
            },
            BodyKind::FormData => BodyType::FormData(
                self.formdata_rows
                    .iter()
                    .filter(|row| !row.is_blank())
                    .cloned()
                    .collect(),
            ),
        }
    }
}

/// Authentication scheme selected in the Auth sub-tab.
///
/// Variant names are serialized by name into the history database, so renaming
/// them would break previously saved requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthType {
    #[default]
    None,
    Bearer,
    Basic,
    ApiKey,
}

/// Config-based auth: a flat struct (all fields always present) so switching
/// type in the UI preserves each type's previously-typed values, matching
/// Postman. The wire header is *computed* from this — auth is never stored as a
/// header row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    pub auth_type: AuthType,
    pub bearer_token: String,
    pub basic_username: String,
    pub basic_password: String,
    /// Header name for API-Key auth, e.g. "X-API-Key".
    pub api_key_name: String,
    pub api_key_value: String,
}

impl AuthConfig {
    /// Return the portion of this configuration that belongs to the selected
    /// auth mode.
    ///
    /// The editor deliberately keeps every mode's inputs alive so users can
    /// switch between drafts. Those inactive drafts must not cross persistence
    /// or wire boundaries, where they would become unrelated credentials.
    pub fn active_only(&self) -> Self {
        match self.auth_type {
            AuthType::None => Self::default(),
            AuthType::Bearer => Self {
                auth_type: AuthType::Bearer,
                bearer_token: self.bearer_token.clone(),
                ..Self::default()
            },
            AuthType::Basic => Self {
                auth_type: AuthType::Basic,
                basic_username: self.basic_username.clone(),
                basic_password: self.basic_password.clone(),
                ..Self::default()
            },
            AuthType::ApiKey => Self {
                auth_type: AuthType::ApiKey,
                api_key_name: self.api_key_name.clone(),
                api_key_value: self.api_key_value.clone(),
                ..Self::default()
            },
        }
    }

    /// The header this auth would put on the wire, or `None`.
    ///
    /// Emitted only when the relevant field(s) are non-empty, so an in-progress
    /// edit never sends a placeholder header (e.g. a dangling `Bearer `). This
    /// differs slightly from Postman, which emits once a type is selected.
    pub fn compute_header(&self) -> Option<(String, String)> {
        match self.auth_type {
            AuthType::None => None,
            AuthType::Bearer => {
                if self.bearer_token.is_empty() {
                    None
                } else {
                    Some((
                        "Authorization".to_string(),
                        format!("Bearer {}", self.bearer_token),
                    ))
                }
            }
            AuthType::Basic => {
                if self.basic_username.is_empty() && self.basic_password.is_empty() {
                    None
                } else {
                    let encoded =
                        BASE64.encode(format!("{}:{}", self.basic_username, self.basic_password));
                    Some(("Authorization".to_string(), format!("Basic {}", encoded)))
                }
            }
            AuthType::ApiKey => {
                if self.api_key_name.is_empty() || self.api_key_value.is_empty() {
                    None
                } else {
                    Some((self.api_key_name.clone(), self.api_key_value.clone()))
                }
            }
        }
    }
}

/// Manual headers with the computed auth header merged in.
///
/// Selecting an auth mode claims its header even while its fields are empty:
/// a stale manual credential must not become a fallback. If the selected auth
/// is complete, its computed header is appended after the conflict is removed.
pub fn effective_wire_headers(
    headers: &[(String, String)],
    auth: &AuthConfig,
) -> Vec<(String, String)> {
    let claimed_header = match auth.auth_type {
        AuthType::None => None,
        AuthType::Bearer | AuthType::Basic => Some("Authorization"),
        AuthType::ApiKey if !auth.api_key_name.is_empty() => Some(auth.api_key_name.as_str()),
        AuthType::ApiKey => None,
    };

    let mut out: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            !is_transport_owned_header(name)
                && claimed_header.is_none_or(|claimed| !name.eq_ignore_ascii_case(claimed))
        })
        .cloned()
        .collect();

    if let Some((name, value)) = auth
        .compute_header()
        .filter(|(name, _)| !is_transport_owned_header(name))
    {
        out.push((name, value));
    }
    out
}

/// Request data structure
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestData {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: BodyType,
    /// Config-based auth. `#[serde(default)]` so requests serialized before this
    /// feature (history rows / saved tabs) still deserialize — missing → `None`.
    #[serde(default)]
    pub auth: AuthConfig,
}

impl RequestData {
    #[allow(dead_code)]
    pub fn new(method: HttpMethod, url: String) -> Self {
        Self {
            method,
            url,
            headers: vec![],
            body: BodyType::default(),
            auth: AuthConfig::default(),
        }
    }

    /// Build the unresolved snapshot that is safe to write to request history.
    /// Environment substitution happens only on the separate wire request.
    pub fn history_snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.strip_transport_owned_headers();
        snapshot.auth = snapshot.auth.active_only();
        snapshot
    }

    /// Remove headers whose values must be computed from the final wire body.
    /// This also sanitizes records created by older versions of Poopman.
    pub fn strip_transport_owned_headers(&mut self) {
        self.headers
            .retain(|(name, _)| !is_transport_owned_header(name));
    }
}

/// Response data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseData {
    pub status: Option<u16>,
    pub duration_ms: u64,
    pub headers: Vec<(String, String)>,
    /// Raw response bytes (lossless — preserves binary payloads).
    pub body: Vec<u8>,
    /// Whether the body should be shown as text (vs treated as binary).
    pub is_text: bool,
    /// Destination selected for a streamed download. Such responses never keep
    /// their full body in memory.
    #[serde(default)]
    pub downloaded_to: Option<String>,
    /// Number of decoded bytes written by a streamed download.
    #[serde(default)]
    pub downloaded_bytes: Option<u64>,
}

/// Decide whether a response body should be shown as text.
///
/// Uses the `Content-Type` header first (clear text vs clear binary families),
/// falling back to a UTF-8 validity sniff when the type is missing/ambiguous.
pub fn is_text_response(headers: &[(String, String)], body: &[u8]) -> bool {
    let content_type = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });

    if let Some(ct) = content_type.as_deref() {
        // Clearly text
        if ct.starts_with("text/")
            || ct == "application/json"
            || ct == "application/xml"
            || ct == "application/javascript"
            || ct == "application/x-www-form-urlencoded"
            || ct == "image/svg+xml"
            || ct.ends_with("+json")
            || ct.ends_with("+xml")
        {
            return true;
        }
        // Clearly binary
        if ct.starts_with("image/")
            || ct.starts_with("audio/")
            || ct.starts_with("video/")
            || ct.starts_with("font/")
            || ct == "application/octet-stream"
            || ct == "application/pdf"
            || ct == "application/zip"
            || ct == "application/gzip"
        {
            return false;
        }
        // else: unknown application/* — fall through to UTF-8 sniff
    }

    std::str::from_utf8(body).is_ok()
}

impl ResponseData {
    pub fn status_text(&self) -> &'static str {
        match self.status {
            Some(200) => "OK",
            Some(201) => "Created",
            Some(204) => "No Content",
            Some(400) => "Bad Request",
            Some(401) => "Unauthorized",
            Some(403) => "Forbidden",
            Some(404) => "Not Found",
            Some(500) => "Internal Server Error",
            Some(502) => "Bad Gateway",
            Some(503) => "Service Unavailable",
            Some(_) => "Unknown",
            None => "Network Error",
        }
    }

    pub fn is_success(&self) -> bool {
        if let Some(status) = self.status {
            (200..300).contains(&status)
        } else {
            false
        }
    }

    pub fn is_error(&self) -> bool {
        if let Some(status) = self.status {
            status >= 400
        } else {
            true // Network error is considered an error
        }
    }

    pub fn is_network_error(&self) -> bool {
        self.status.is_none()
    }
}

/// History item stored in database
///
/// The response is shared via `Arc`: tabs and the viewer all hold the same
/// allocation, so cloning an item never copies the (potentially large) body.
#[derive(Debug, Clone)]
pub struct HistoryItem {
    pub id: i64,
    pub timestamp: String,
    pub request: RequestData,
    pub response: Option<std::sync::Arc<ResponseData>>,
}

impl HistoryItem {
    pub fn new(
        id: i64,
        timestamp: String,
        request: RequestData,
        response: Option<std::sync::Arc<ResponseData>>,
    ) -> Self {
        Self {
            id,
            timestamp,
            request,
            response,
        }
    }
}

/// Query parameter state for UI (including enabled/disabled state)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamState {
    pub enabled: bool,
    pub key: String,
    pub value: String,
}

/// Header state for UI (including enabled/disabled state and header type)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderState {
    pub enabled: bool,
    pub key: String,
    pub value: String,
    pub header_type: HeaderType,
    pub predefined: Option<PredefinedHeader>,
}

impl HeaderState {
    pub fn is_transport_owned(&self) -> bool {
        self.predefined == Some(PredefinedHeader::ContentLength)
            || is_transport_owned_header(&self.key)
    }
}

/// A request persisted inside a collection. The request payload and the
/// complete editor row state are stored separately so disabled headers/params
/// survive a save/load round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRequest {
    pub id: i64,
    pub collection_id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub request: RequestData,
    pub params_state: Vec<ParamState>,
    pub headers_state: Vec<HeaderState>,
    pub position: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// A folder node in a collection tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionFolder {
    pub id: i64,
    pub collection_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub position: i64,
    pub folders: Vec<CollectionFolder>,
    pub requests: Vec<SavedRequest>,
}

/// A top-level request collection. Requests may live directly under the
/// collection or inside any nested folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub position: i64,
    pub folders: Vec<CollectionFolder>,
    pub requests: Vec<SavedRequest>,
}

/// A named environment holding a set of variables.
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub variables: Vec<EnvVar>,
}

/// A single environment variable (key/value, toggleable).
#[derive(Debug, Clone)]
pub struct EnvVar {
    pub enabled: bool,
    pub key: String,
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(ct: &str) -> Vec<(String, String)> {
        vec![("Content-Type".to_string(), ct.to_string())]
    }

    #[test]
    fn text_content_types_are_text() {
        assert!(is_text_response(&h("application/json"), b"{}"));
        assert!(is_text_response(&h("text/html; charset=utf-8"), b"<html>"));
        assert!(is_text_response(&h("application/xml"), b"<x/>"));
        assert!(is_text_response(&h("application/problem+json"), b"{}"));
        assert!(is_text_response(&h("image/svg+xml"), b"<svg/>"));
    }

    #[test]
    fn binary_content_types_are_binary() {
        assert!(!is_text_response(&h("image/png"), &[0x89, 0x50]));
        assert!(!is_text_response(
            &h("application/octet-stream"),
            &[0, 1, 2]
        ));
        assert!(!is_text_response(&h("application/pdf"), b"%PDF"));
        assert!(!is_text_response(&h("application/zip"), &[0x50, 0x4b]));
    }

    #[test]
    fn unknown_or_missing_type_falls_back_to_utf8_sniff() {
        assert!(is_text_response(&[], b"plain text"));
        assert!(!is_text_response(&[], &[0xff, 0xfe, 0x00]));
        // unknown application/* defers to sniff
        assert!(is_text_response(&h("application/weird"), b"readable"));
        assert!(!is_text_response(&h("application/weird"), &[0xff, 0x00]));
    }

    #[test]
    fn compute_header_none_and_empty_fields_emit_nothing() {
        assert_eq!(AuthConfig::default().compute_header(), None);
        // Bearer with empty token → nothing (don't send a dangling "Bearer ")
        let a = AuthConfig {
            auth_type: AuthType::Bearer,
            ..Default::default()
        };
        assert_eq!(a.compute_header(), None);
        // Basic with both fields empty → nothing
        let a = AuthConfig {
            auth_type: AuthType::Basic,
            ..Default::default()
        };
        assert_eq!(a.compute_header(), None);
        // ApiKey with empty name → nothing
        let a = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_value: "v".into(),
            ..Default::default()
        };
        assert_eq!(a.compute_header(), None);
        // ApiKey with empty value → nothing
        let a = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_name: "X-API-Key".into(),
            ..Default::default()
        };
        assert_eq!(a.compute_header(), None);
    }

    #[test]
    fn compute_header_bearer() {
        let a = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "t0ken".into(),
            ..Default::default()
        };
        assert_eq!(
            a.compute_header(),
            Some(("Authorization".into(), "Bearer t0ken".into()))
        );
    }

    #[test]
    fn compute_header_basic_base64() {
        let a = AuthConfig {
            auth_type: AuthType::Basic,
            basic_username: "user".into(),
            basic_password: "pass".into(),
            ..Default::default()
        };
        // base64("user:pass") == "dXNlcjpwYXNz"
        assert_eq!(
            a.compute_header(),
            Some(("Authorization".into(), "Basic dXNlcjpwYXNz".into()))
        );
    }

    #[test]
    fn compute_header_basic_username_only() {
        let a = AuthConfig {
            auth_type: AuthType::Basic,
            basic_username: "user".into(),
            ..Default::default()
        };
        // base64("user:") == "dXNlcjo="
        assert_eq!(
            a.compute_header(),
            Some(("Authorization".into(), "Basic dXNlcjo=".into()))
        );
    }

    #[test]
    fn compute_header_api_key_uses_custom_name() {
        let a = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_name: "X-API-Key".into(),
            api_key_value: "secret".into(),
            ..Default::default()
        };
        assert_eq!(
            a.compute_header(),
            Some(("X-API-Key".into(), "secret".into()))
        );
    }

    #[test]
    fn effective_headers_none_leaves_manual_untouched() {
        let manual = vec![("Accept".to_string(), "*/*".to_string())];
        let out = effective_wire_headers(&manual, &AuthConfig::default());
        assert_eq!(out, manual);
    }

    #[test]
    fn content_length_is_transport_owned_and_not_editable() {
        assert!(is_transport_owned_header("Content-Length"));
        assert!(is_transport_owned_header("content-length"));
        assert!(!is_transport_owned_header("Content-Type"));
        assert_eq!(
            serde_json::from_str::<PredefinedHeader>("\"ContentLength\"").unwrap(),
            PredefinedHeader::ContentLength
        );
        assert!(!PredefinedHeader::editable().contains(&PredefinedHeader::ContentLength));
        assert_eq!(
            PredefinedHeader::CacheControl.header_type(),
            HeaderType::Predefined
        );
    }

    #[test]
    fn effective_headers_drop_manual_content_length_case_insensitively() {
        let manual = vec![
            ("content-length".to_string(), "1".to_string()),
            ("X-Trace".to_string(), "kept".to_string()),
        ];

        assert_eq!(
            effective_wire_headers(&manual, &AuthConfig::default()),
            vec![("X-Trace".to_string(), "kept".to_string())]
        );
    }

    #[test]
    fn effective_headers_reject_content_length_as_an_api_key_name() {
        let auth = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_name: "Content-Length".into(),
            api_key_value: "999".into(),
            ..Default::default()
        };

        assert!(effective_wire_headers(&[], &auth).is_empty());
    }

    #[test]
    fn effective_headers_appends_auth() {
        let manual = vec![("Accept".to_string(), "*/*".to_string())];
        let auth = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "t".into(),
            ..Default::default()
        };
        let out = effective_wire_headers(&manual, &auth);
        assert_eq!(
            out,
            vec![
                ("Accept".to_string(), "*/*".to_string()),
                ("Authorization".to_string(), "Bearer t".to_string()),
            ]
        );
    }

    #[test]
    fn effective_headers_auth_wins_over_same_name_manual_case_insensitive() {
        // A manually-typed "authorization" is dropped in favor of the computed one.
        let manual = vec![
            ("Accept".to_string(), "*/*".to_string()),
            ("authorization".to_string(), "Bearer OLD".to_string()),
        ];
        let auth = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "NEW".into(),
            ..Default::default()
        };
        let out = effective_wire_headers(&manual, &auth);
        assert_eq!(
            out,
            vec![
                ("Accept".to_string(), "*/*".to_string()),
                ("Authorization".to_string(), "Bearer NEW".to_string()),
            ]
        );
    }

    #[test]
    fn incomplete_bearer_suppresses_stale_manual_authorization() {
        let manual = vec![
            ("Accept".to_string(), "*/*".to_string()),
            ("authorization".to_string(), "Bearer OLD".to_string()),
        ];
        let auth = AuthConfig {
            auth_type: AuthType::Bearer,
            ..Default::default()
        };

        assert_eq!(
            effective_wire_headers(&manual, &auth),
            vec![("Accept".to_string(), "*/*".to_string())]
        );
    }

    #[test]
    fn incomplete_basic_suppresses_stale_manual_authorization() {
        let manual = vec![(
            "Authorization".to_string(),
            "Basic c3RhbGU6c2VjcmV0".to_string(),
        )];
        let auth = AuthConfig {
            auth_type: AuthType::Basic,
            ..Default::default()
        };

        assert!(effective_wire_headers(&manual, &auth).is_empty());
    }

    #[test]
    fn incomplete_api_key_suppresses_its_stale_manual_header() {
        let manual = vec![("X-API-Key".to_string(), "OLD".to_string())];
        let auth = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_name: "X-API-Key".into(),
            api_key_value: String::new(),
            ..Default::default()
        };

        // The selected mode still claims the name, so neither the stale value
        // nor an empty replacement goes onto the wire.
        assert!(effective_wire_headers(&manual, &auth).is_empty());
    }

    #[test]
    fn effective_headers_api_key_custom_name_dedupes() {
        let manual = vec![("X-API-Key".to_string(), "old".to_string())];
        let auth = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_name: "X-API-Key".into(),
            api_key_value: "new".into(),
            ..Default::default()
        };
        let out = effective_wire_headers(&manual, &auth);
        assert_eq!(out, vec![("X-API-Key".to_string(), "new".to_string())]);
    }

    #[test]
    fn history_snapshot_preserves_templates_and_drops_inactive_auth() {
        let request = RequestData {
            method: HttpMethod::POST,
            url: "{{base_url}}/users?token={{query_token}}".into(),
            headers: vec![("X-Token".into(), "{{header_token}}".into())],
            body: BodyType::Raw {
                content: "{\"token\":\"{{body_token}}\"}".into(),
                subtype: RawSubtype::Json,
            },
            auth: AuthConfig {
                auth_type: AuthType::Bearer,
                bearer_token: "{{bearer_token}}".into(),
                basic_username: "inactive-user".into(),
                basic_password: "inactive-password".into(),
                api_key_name: "X-Inactive-Key".into(),
                api_key_value: "inactive-api-key".into(),
            },
        };

        let snapshot = request.history_snapshot();
        assert_eq!(snapshot.url, request.url);
        assert_eq!(snapshot.headers, request.headers);
        assert_eq!(snapshot.body, request.body);
        assert_eq!(snapshot.auth.auth_type, AuthType::Bearer);
        assert_eq!(snapshot.auth.bearer_token, "{{bearer_token}}");
        assert!(snapshot.auth.basic_username.is_empty());
        assert!(snapshot.auth.basic_password.is_empty());
        assert!(snapshot.auth.api_key_name.is_empty());
        assert!(snapshot.auth.api_key_value.is_empty());
    }

    #[test]
    fn history_snapshot_with_none_drops_every_auth_draft() {
        let mut request = RequestData::new(HttpMethod::GET, "{{base_url}}".into());
        request.auth = AuthConfig {
            auth_type: AuthType::None,
            bearer_token: "inactive-bearer".into(),
            basic_username: "inactive-user".into(),
            basic_password: "inactive-password".into(),
            api_key_name: "X-Inactive-Key".into(),
            api_key_value: "inactive-api-key".into(),
        };

        assert_eq!(request.history_snapshot().auth, AuthConfig::default());
    }

    #[test]
    fn history_snapshot_drops_stale_content_length() {
        let mut request = RequestData::new(HttpMethod::POST, "https://api.test".into());
        request.headers = vec![
            ("Content-Length".into(), "0".into()),
            ("X-Trace".into(), "kept".into()),
        ];

        assert_eq!(
            request.history_snapshot().headers,
            vec![("X-Trace".into(), "kept".into())]
        );
    }

    #[test]
    fn active_only_keeps_basic_and_api_key_fields_separate() {
        let all = AuthConfig {
            auth_type: AuthType::Basic,
            bearer_token: "bearer".into(),
            basic_username: "user".into(),
            basic_password: "pass".into(),
            api_key_name: "X-Key".into(),
            api_key_value: "key-value".into(),
        };

        assert_eq!(
            all.active_only(),
            AuthConfig {
                auth_type: AuthType::Basic,
                basic_username: "user".into(),
                basic_password: "pass".into(),
                ..Default::default()
            }
        );

        let api_key = AuthConfig {
            auth_type: AuthType::ApiKey,
            ..all
        };
        assert_eq!(
            api_key.active_only(),
            AuthConfig {
                auth_type: AuthType::ApiKey,
                api_key_name: "X-Key".into(),
                api_key_value: "key-value".into(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn body_draft_never_exposes_formdata_placeholders() {
        let real = FormDataRow {
            enabled: true,
            key: "name".into(),
            value: FormDataValue::Text("alice".into()),
        };
        let blank = FormDataRow {
            enabled: true,
            key: String::new(),
            value: FormDataValue::Text(String::new()),
        };
        let body = BodyType::FormData(vec![real.clone(), blank.clone(), blank]);

        let first = BodyDraft::from_body(&body);
        assert_eq!(first.formdata_rows, vec![real.clone()]);
        assert_eq!(first.selected_body(), BodyType::FormData(vec![real]));
        assert_eq!(BodyDraft::from_body(&first.selected_body()), first);
    }
}
