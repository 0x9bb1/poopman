use crate::types::{BodyType, HeaderState, HistoryItem, HttpMethod, ParamState, RequestData};

/// Represents a single request tab
#[derive(Debug, Clone)]
pub struct RequestTab {
    pub id: usize,
    pub title: String,
    pub request: RequestData,
    // UI state (not persisted to database)
    pub params_state: Option<Vec<ParamState>>,
    pub headers_state: Option<Vec<HeaderState>>,
}

impl RequestTab {
    /// Create a new empty request tab
    pub fn new_empty(id: usize) -> Self {
        Self {
            id,
            title: "New Request".to_string(),
            request: RequestData {
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![],
                body: BodyType::default(),
            },
            params_state: None,
            headers_state: None,
        }
    }

    /// Create a request tab from history item
    pub fn from_history(id: usize, item: &HistoryItem) -> Self {
        Self {
            id,
            title: Self::generate_title(&item.request),
            request: item.request.clone(),
            params_state: None,
            headers_state: None,
        }
    }

    /// Create a request tab from request data
    pub fn from_request(id: usize, request: RequestData) -> Self {
        Self {
            id,
            title: Self::generate_title(&request),
            request,
            params_state: None,
            headers_state: None,
        }
    }

    /// Generate a display title from request data
    fn generate_title(request: &RequestData) -> String {
        if request.url.is_empty() {
            return "New Request".to_string();
        }

        // Extract path from URL
        let path = request
            .url
            .split('?')
            .next()
            .and_then(|s| {
                let parts: Vec<&str> = s.split('/').collect();
                parts.last().copied()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or("Untitled");

        format!("{} {}", request.method.as_str(), path)
    }

    /// Update title based on current request data
    pub fn update_title(&mut self) {
        self.title = Self::generate_title(&self.request);
    }
}
