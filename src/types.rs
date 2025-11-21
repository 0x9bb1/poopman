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
    AcceptEncoding,
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
            PredefinedHeader::AcceptEncoding => "Accept-Encoding",
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
            PredefinedHeader::AcceptEncoding => "gzip, deflate, br",
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
            PredefinedHeader::AcceptEncoding,
            PredefinedHeader::UserAgent,
            PredefinedHeader::Connection,
            PredefinedHeader::ContentLength,
        ]
    }
}

/// HTTP methods supported by the API client
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
    pub fn as_str(&self) -> &'static str {
        match self {
            RawSubtype::Json => "json",
            RawSubtype::Xml => "xml",
            RawSubtype::Text => "text",
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
    pub body: String,
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
            status >= 200 && status < 300
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
#[derive(Debug, Clone)]
pub struct HistoryItem {
    pub id: i64,
    pub timestamp: String,
    pub request: RequestData,
    pub response: Option<ResponseData>,
}

impl HistoryItem {
    pub fn new(
        id: i64,
        timestamp: String,
        request: RequestData,
        response: Option<ResponseData>,
    ) -> Self {
        Self {
            id,
            timestamp,
            request,
            response,
        }
    }
}
