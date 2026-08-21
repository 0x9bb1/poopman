use std::sync::Arc;

use crate::types::{BodyDraft, BodyType, HeaderState, HistoryItem, HttpMethod, ParamState, RequestData, ResponseData};

/// Represents a single request tab
#[derive(Debug, Clone)]
pub struct RequestTab {
    pub id: usize,
    pub title: String,
    pub request: RequestData,
    /// Complete body editor state, including drafts for inactive body types.
    pub body_draft: BodyDraft,
    /// Response data for this tab (shared, so tab switches never copy the body)
    pub response: Option<Arc<ResponseData>>,
    /// The request currently owned by this tab. Completion/cancellation events
    /// must match this id before they may mutate the tab.
    pub active_request_id: Option<u64>,
    /// Persisted response-panel state for a canceled request.
    pub response_canceled: bool,
    // UI state (not persisted to database)
    pub params_state: Option<Vec<ParamState>>,
    pub headers_state: Option<Vec<HeaderState>>,
    /// Associated history item ID (if opened from history)
    pub history_id: Option<i64>,
    /// Associated collection request ID, if this tab was opened from a saved
    /// request. Collection saves are explicit, so edits remain in the tab
    /// until the user presses the save action.
    pub saved_request_id: Option<i64>,
    pub collection_id: Option<i64>,
    pub folder_id: Option<i64>,
    pub saved_name: Option<String>,
}

impl RequestTab {
    /// Create a new empty request tab
    pub fn new_empty(id: usize) -> Self {
        let request = RequestData {
            method: HttpMethod::GET,
            url: String::new(),
            headers: vec![],
            body: BodyType::default(),
            auth: crate::types::AuthConfig::default(),
        };
        Self {
            id,
            title: "New Request".to_string(),
            body_draft: BodyDraft::from_body(&request.body),
            request,
            response: None,
            active_request_id: None,
            response_canceled: false,
            params_state: None,
            headers_state: None,
            history_id: None,
            saved_request_id: None,
            collection_id: None,
            folder_id: None,
            saved_name: None,
        }
    }

    /// Create a request tab from history item
    pub fn from_history(id: usize, item: &HistoryItem) -> Self {
        Self {
            id,
            title: Self::generate_title(&item.request),
            request: item.request.clone(),
            body_draft: BodyDraft::from_body(&item.request.body),
            response: item.response.clone(),
            active_request_id: None,
            response_canceled: false,
            params_state: None,
            headers_state: None,
            history_id: Some(item.id),
            saved_request_id: None,
            collection_id: None,
            folder_id: None,
            saved_name: None,
        }
    }

    /// Create a request tab from a persisted collection request.
    pub fn from_saved_request(id: usize, saved: &crate::types::SavedRequest) -> Self {
        Self {
            id,
            title: saved.name.clone(),
            request: saved.request.clone(),
            body_draft: BodyDraft::from_body(&saved.request.body),
            response: None,
            active_request_id: None,
            response_canceled: false,
            params_state: Some(saved.params_state.clone()),
            headers_state: Some(saved.headers_state.clone()),
            history_id: None,
            saved_request_id: Some(saved.id),
            collection_id: Some(saved.collection_id),
            folder_id: saved.folder_id,
            saved_name: Some(saved.name.clone()),
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

        // The tab bar renders the HTTP method in its own colored label. Keep
        // the title to the request path so a scratch/history tab does not
        // become `GET GET /path`.
        path.to_string()
    }

    /// Update title based on current request data
    pub fn update_title(&mut self) {
        self.title = Self::generate_title(&self.request);
    }

    /// Keep a collection request's explicit name when the editor changes.
    pub fn update_title_from_saved_name(&mut self) {
        if let Some(name) = &self.saved_name {
            self.title = name.clone();
        } else {
            self.update_title();
        }
    }

    /// Record the immutable editor snapshot associated with a new send.
    pub fn begin_request(&mut self, request_id: u64, request: RequestData) {
        self.request = request;
        self.active_request_id = Some(request_id);
        self.response_canceled = false;
        self.update_title_from_saved_name();
    }

    /// Apply a response only when it belongs to this tab's current request.
    pub fn complete_request(
        &mut self,
        request_id: u64,
        response: Arc<ResponseData>,
    ) -> bool {
        if self.active_request_id != Some(request_id) {
            return false;
        }
        self.active_request_id = None;
        self.response = Some(response);
        self.response_canceled = false;
        true
    }

    /// Apply cancellation only to the matching request in this tab.
    pub fn cancel_request(&mut self, request_id: u64) -> bool {
        if self.active_request_id != Some(request_id) {
            return false;
        }
        self.active_request_id = None;
        self.response = None;
        self.response_canceled = true;
        true
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
            && self.saved_request_id.is_none()
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
    use crate::types::{BodyKind, BodyType, RawSubtype, ResponseData};

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

    fn response(status: u16) -> Arc<ResponseData> {
        Arc::new(ResponseData {
            status: Some(status),
            duration_ms: 1,
            headers: vec![],
            body: status.to_string().into_bytes(),
            is_text: true,
        })
    }

    #[test]
    fn generated_tab_title_leaves_method_to_tab_bar() {
        let mut request = empty_request();
        request.method = HttpMethod::POST;
        request.url = "https://example.com/items?draft=true".to_string();
        let item = HistoryItem::new(8, "t".to_string(), request, None);

        assert_eq!(RequestTab::from_history(1, &item).title, "items");
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

    #[test]
    fn completion_after_switch_updates_only_its_originating_tab() {
        let mut tabs = [RequestTab::new_empty(10), RequestTab::new_empty(20)];
        tabs[0].begin_request(101, RequestData::new(HttpMethod::GET, "https://a.test".into()));

        // The user switched to tab B before A completed. Routing uses tab id,
        // never the active index.
        let active_index = 1;
        let origin = tabs.iter_mut().find(|tab| tab.id == 10).unwrap();
        assert!(origin.complete_request(101, response(201)));

        assert_eq!(tabs[0].response.as_ref().unwrap().status, Some(201));
        assert!(tabs[active_index].response.is_none());
    }

    #[test]
    fn send_keeps_an_immutable_request_snapshot() {
        let mut editor_request = RequestData::new(HttpMethod::GET, "https://a.test".into());
        let mut tab = RequestTab::new_empty(10);
        tab.begin_request(101, editor_request.clone());

        editor_request.url = "https://edited-later.test".into();

        assert_eq!(tab.request.url, "https://a.test");
    }

    #[test]
    fn concurrent_out_of_order_completions_stay_with_their_tabs() {
        let mut a = RequestTab::new_empty(10);
        let mut b = RequestTab::new_empty(20);
        a.begin_request(101, RequestData::new(HttpMethod::GET, "https://a.test".into()));
        b.begin_request(102, RequestData::new(HttpMethod::GET, "https://b.test".into()));

        assert!(b.complete_request(102, response(202)));
        assert!(a.complete_request(101, response(201)));
        assert_eq!(a.response.unwrap().status, Some(201));
        assert_eq!(b.response.unwrap().status, Some(202));
    }

    #[test]
    fn cancel_affects_only_the_matching_tab_and_request() {
        let mut a = RequestTab::new_empty(10);
        let mut b = RequestTab::new_empty(20);
        a.begin_request(101, RequestData::new(HttpMethod::GET, "https://a.test".into()));
        b.begin_request(102, RequestData::new(HttpMethod::GET, "https://b.test".into()));

        assert!(b.cancel_request(102));
        assert!(b.response_canceled);
        assert_eq!(a.active_request_id, Some(101));
        assert!(!a.response_canceled);
        assert!(!a.cancel_request(999));
    }

    #[test]
    fn stale_same_tab_completion_cannot_overwrite_a_newer_request() {
        let mut tab = RequestTab::new_empty(10);
        tab.begin_request(101, RequestData::new(HttpMethod::GET, "https://old.test".into()));
        tab.begin_request(102, RequestData::new(HttpMethod::GET, "https://new.test".into()));

        assert!(!tab.complete_request(101, response(201)));
        assert!(tab.response.is_none());
        assert!(tab.complete_request(102, response(202)));
        assert_eq!(tab.response.unwrap().status, Some(202));
    }

    #[test]
    fn closing_a_running_tab_leaves_no_completion_destination() {
        let mut tabs = vec![RequestTab::new_empty(10), RequestTab::new_empty(20)];
        tabs[0].begin_request(101, RequestData::new(HttpMethod::GET, "https://a.test".into()));

        tabs.remove(0);

        assert!(tabs.iter_mut().find(|tab| tab.id == 10).is_none());
        assert_eq!(tabs[0].id, 20);
        assert!(tabs[0].response.is_none());
    }

    #[test]
    fn inactive_body_drafts_are_isolated_per_tab() {
        let mut a = RequestTab::new_empty(10);
        let mut b = RequestTab::new_empty(20);
        a.body_draft.kind = BodyKind::None;
        a.body_draft.raw_content = "draft-a".into();
        b.body_draft.kind = BodyKind::FormData;
        b.body_draft.raw_content = "draft-b".into();

        let loaded_b = b.body_draft.clone();
        let loaded_a = a.body_draft.clone();

        assert_eq!(loaded_b.raw_content, "draft-b");
        assert_eq!(loaded_a.raw_content, "draft-a");
        assert_eq!(loaded_a.kind, BodyKind::None);
    }
}
