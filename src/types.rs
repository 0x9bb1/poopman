use serde::{Deserialize, Serialize};
use std::fmt;

/// Header type for distinguishing predefined vs custom headers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderType {
    /// Mandatory header that cannot be disabled or deleted (e.g., Cache-Control)
    Mandatory,
    /// Predefined header that can be toggled but not deleted
    Predefined,
    /// Custom user-defined header that can be toggled and deleted
    Custom,
}

/// Predefined header names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            PredefinedHeader::UserAgent => "Poopman/1.0",
            PredefinedHeader::Connection => "keep-alive",
            PredefinedHeader::ContentLength => "0",
        }
    }

    pub fn is_auto_calculated(&self) -> bool {
        matches!(self, PredefinedHeader::ContentLength)
    }

    pub fn header_type(&self) -> HeaderType {
        match self {
            PredefinedHeader::CacheControl => HeaderType::Mandatory,
            _ => HeaderType::Predefined,
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            PredefinedHeader::CacheControl,
            PredefinedHeader::ContentType,
            PredefinedHeader::Accept,
            PredefinedHeader::UserAgent,
            PredefinedHeader::Connection,
            PredefinedHeader::ContentLength,
        ]
    }
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
}

impl RawSubtype {
    /// Returns the language string for syntax highlighting.
    ///
    /// Note: XML is not supported by gpui-component's tree-sitter-languages feature,
    /// so it falls back to "plain" (no syntax highlighting).
    pub fn as_str(&self) -> &'static str {
        match self {
            RawSubtype::Json => "json",
            RawSubtype::Xml => "plain",  // XML not supported, fallback to plain
            RawSubtype::Text => "plain",
            RawSubtype::JavaScript => "javascript",
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            RawSubtype::Json => "application/json",
            RawSubtype::Xml => "application/xml",
            RawSubtype::Text => "text/plain",
            RawSubtype::JavaScript => "application/javascript",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            RawSubtype::Json,
            RawSubtype::Xml,
            RawSubtype::Text,
            RawSubtype::JavaScript,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormDataRow {
    pub enabled: bool,
    pub key: String,
    pub value: FormDataValue,
}

/// Request body type
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Request data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestData {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: BodyType,
}

impl RequestData {
    #[allow(dead_code)]
    pub fn new(method: HttpMethod, url: String) -> Self {
        Self {
            method,
            url,
            headers: vec![],
            body: BodyType::default(),
        }
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
}

/// Decide whether a response body should be shown as text.
///
/// Uses the `Content-Type` header first (clear text vs clear binary families),
/// falling back to a UTF-8 validity sniff when the type is missing/ambiguous.
pub fn is_text_response(headers: &[(String, String)], body: &[u8]) -> bool {
    let content_type = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());

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
    /// Lossy text view of the body (for display when `is_text`).
    pub fn body_text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

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
#[derive(Debug, Clone)]
pub struct ParamState {
    pub enabled: bool,
    pub key: String,
    pub value: String,
}

/// Header state for UI (including enabled/disabled state and header type)
#[derive(Debug, Clone)]
pub struct HeaderState {
    pub enabled: bool,
    pub key: String,
    pub value: String,
    pub header_type: HeaderType,
    pub predefined: Option<PredefinedHeader>,
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
        assert!(!is_text_response(&h("application/octet-stream"), &[0, 1, 2]));
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
}
