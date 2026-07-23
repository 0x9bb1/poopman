use std::sync::Arc;

use crate::types::{BodyType, HeaderState, HistoryItem, HttpMethod, ParamState, RequestData, ResponseData};

/// Represents a single request tab
#[derive(Debug, Clone)]
pub struct RequestTab {
    pub id: usize,
    pub title: String,
    pub request: RequestData,
    /// Response data for this tab (shared, so tab switches never copy the body)
    pub response: Option<Arc<ResponseData>>,
    // UI state (not persisted to database)
    pub params_state: Option<Vec<ParamState>>,
    pub headers_state: Option<Vec<HeaderState>>,
    /// Associated history item ID (if opened from history)
    pub history_id: Option<i64>,
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
                auth: crate::types::AuthConfig::default(),
            },
            response: None,
            params_state: None,
            headers_state: None,
            history_id: None,
        }
    }

    /// Create a request tab from history item
    pub fn from_history(id: usize, item: &HistoryItem) -> Self {
        Self {
            id,
            title: Self::generate_title(&item.request),
            request: item.request.clone(),
            response: item.response.clone(),
            params_state: None,
            headers_state: None,
            history_id: Some(item.id),
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

    /// A pristine scratch tab — the default tab at startup, or an untouched
    /// "New Request". Opening a history item fills such a tab in place instead
    /// of spawning a sibling.
    ///
    /// Headers are ignored on purpose: a fresh tab's saved request always
    /// carries the enabled predefined headers (Content-Type, Cache-Control,
    /// ...), so those are not a signal that the user has done anything. What
    /// marks a tab as used is a typed URL, body content, a response, or having
    /// been opened from history.
    pub fn is_blank(&self) -> bool {
        self.history_id.is_none()
            && self.response.is_none()
            && self.request.url.trim().is_empty()
            && match &self.request.body {
                BodyType::None => true,
                BodyType::Raw { content, .. } => content.trim().is_empty(),
                BodyType::FormData(rows) => rows.is_empty(),
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, RawSubtype, ResponseData};

    fn empty_request() -> RequestData {
        RequestData {
            method: HttpMethod::GET,
            url: String::new(),
            headers: vec![],
            body: BodyType::default(),
            auth: crate::types::AuthConfig::default(),
        }
    }

    #[test]
    fn new_empty_tab_is_blank() {
        assert!(RequestTab::new_empty(0).is_blank());
    }

    #[test]
    fn tab_with_url_is_not_blank() {
        let mut tab = RequestTab::new_empty(0);
        tab.request.url = "https://api.test/x".to_string();
        assert!(!tab.is_blank());
    }

    #[test]
    fn tab_opened_from_history_is_not_blank() {
        // Even a history item with an empty URL is not a scratch tab.
        let item = HistoryItem::new(7, "t".to_string(), empty_request(), None);
        assert!(!RequestTab::from_history(1, &item).is_blank());
    }

    #[test]
    fn tab_with_a_response_is_not_blank() {
        let mut tab = RequestTab::new_empty(0);
        tab.response = Some(Arc::new(ResponseData {
            status: Some(200),
            duration_ms: 0,
            headers: vec![],
            body: vec![],
            is_text: true,
        }));
        assert!(!tab.is_blank());
    }

    #[test]
    fn tab_with_body_content_is_not_blank() {
        let mut tab = RequestTab::new_empty(0);
        tab.request.body = BodyType::Raw {
            content: "{}".to_string(),
            subtype: RawSubtype::Json,
        };
        assert!(!tab.is_blank());
    }

    #[test]
    fn default_predefined_headers_do_not_count_as_used() {
        // A fresh tab's saved request always carries the enabled predefined
        // headers (Content-Type, Cache-Control, ...). Those must not make the
        // tab look "used", or history would never reuse the startup tab.
        let mut tab = RequestTab::new_empty(0);
        tab.request.headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Cache-Control".to_string(), "no-cache".to_string()),
        ];
        assert!(tab.is_blank());
    }
}
