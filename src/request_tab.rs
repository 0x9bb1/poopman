use std::sync::Arc;

use crate::types::{
    BodyDraft, BodyType, HeaderState, HeaderType, HistoryItem, HttpMethod, ParamState,
    PredefinedHeader, RequestData, RequestSection, ResponseData, ResponseSection,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSnapshot {
    pub request: RequestData,
    pub body_draft: BodyDraft,
    pub params_state: Vec<ParamState>,
    pub headers_state: Vec<HeaderState>,
    pub saved_name: Option<String>,
    pub collection_id: Option<i64>,
    pub folder_id: Option<i64>,
}

impl RequestSnapshot {
    fn matches(&self, tab: &RequestTab) -> bool {
        let (Some(params), Some(headers)) = (&tab.params_state, &tab.headers_state) else {
            return *self == tab.snapshot();
        };
        // Same answer as `*self == tab.snapshot()` without cloning the tab on
        // every render. Destructured without `..` so a field added to any of
        // these types fails to compile here instead of silently escaping dirty
        // tracking (and losing edits on close without a prompt).
        let Self {
            request,
            body_draft,
            params_state,
            headers_state,
            saved_name,
            collection_id,
            folder_id,
        } = self;
        let RequestData {
            method,
            url,
            // `snapshot()` rebuilds these from `headers_state`, compared below.
            headers: _,
            body,
            auth,
        } = request;
        let BodyDraft {
            kind,
            raw_content,
            raw_subtype,
            formdata_rows,
        } = body_draft;
        *method == tab.request.method
            && *url == tab.request.url
            && *body == tab.request.body
            && *auth == tab.request.auth
            && *kind == tab.body_draft.kind
            && *raw_content == tab.body_draft.raw_content
            && *raw_subtype == tab.body_draft.raw_subtype
            && formdata_rows.iter().eq(tab
                .body_draft
                .formdata_rows
                .iter()
                .filter(|row| !row.is_blank()))
            && params_state.iter().eq(params
                .iter()
                .filter(|param| !param.key.is_empty() || !param.value.is_empty()))
            && headers_state.iter().eq(headers.iter().filter(|header| {
                !header.is_transport_owned() && (!header.key.is_empty() || !header.value.is_empty())
            }))
            && *saved_name == tab.saved_name
            && *collection_id == tab.collection_id
            && *folder_id == tab.folder_id
    }
}

/// Everything a request tab owns in the shared `RequestEditor`. The editor's
/// `capture_state`/`restore_state` and this tab's `editor_state`/
/// `apply_editor_state` are the only transfer path, and all four build or
/// destructure it without `..`: a field added here fails to compile until both
/// the save and the restore side handle it.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorTabState {
    pub request: RequestData,
    pub body_draft: BodyDraft,
    /// `None` until the tab's rows were captured once; the editor then derives
    /// the rows from `request` (e.g. a history entry that was never edited).
    pub params_state: Option<Vec<ParamState>>,
    pub headers_state: Option<Vec<HeaderState>>,
    pub section: RequestSection,
}

/// Everything a request tab owns in the shared `ResponseViewer`, under the
/// same exhaustive capture/restore contract as [`EditorTabState`].
#[derive(Debug, Clone)]
pub struct ResponseTabState {
    pub response: Option<Arc<ResponseData>>,
    pub canceled: bool,
    pub section: ResponseSection,
}

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
    pub params_state: Option<Vec<ParamState>>,
    pub headers_state: Option<Vec<HeaderState>>,
    pub editor_section: RequestSection,
    pub response_section: ResponseSection,
    /// Associated history item ID (if opened from history)
    pub history_id: Option<i64>,
    /// Associated collection request ID, if this tab was opened from a saved
    /// request. Collection saves are explicit, so edits remain in the tab
    /// until the user presses the save action.
    pub saved_request_id: Option<i64>,
    pub collection_id: Option<i64>,
    pub folder_id: Option<i64>,
    pub saved_name: Option<String>,
    baseline: Option<Arc<RequestSnapshot>>,
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
        let mut tab = Self {
            id,
            title: "New Request".to_string(),
            body_draft: BodyDraft::from_body(&request.body),
            request,
            response: None,
            active_request_id: None,
            response_canceled: false,
            params_state: None,
            headers_state: None,
            editor_section: RequestSection::default(),
            response_section: ResponseSection::default(),
            history_id: None,
            saved_request_id: None,
            collection_id: None,
            folder_id: None,
            saved_name: None,
            baseline: None,
        };
        tab.establish_baseline();
        tab
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
            editor_section: RequestSection::default(),
            response_section: ResponseSection::default(),
            history_id: Some(item.id),
            saved_request_id: None,
            collection_id: None,
            folder_id: None,
            saved_name: None,
            baseline: None,
        }
    }

    /// Create a request tab from a persisted collection request.
    pub fn from_saved_request(id: usize, saved: &crate::types::SavedRequest) -> Self {
        Self {
            id,
            title: saved.name.clone(),
            request: saved.request.clone(),
            body_draft: saved
                .body_draft
                .clone()
                .unwrap_or_else(|| BodyDraft::from_body(&saved.request.body)),
            response: None,
            active_request_id: None,
            response_canceled: false,
            params_state: Some(saved.params_state.clone()),
            headers_state: Some(saved.headers_state.clone()),
            editor_section: RequestSection::default(),
            response_section: ResponseSection::default(),
            history_id: None,
            saved_request_id: Some(saved.id),
            collection_id: Some(saved.collection_id),
            folder_id: saved.folder_id,
            saved_name: Some(saved.name.clone()),
            baseline: None,
        }
    }

    pub fn snapshot(&self) -> RequestSnapshot {
        let mut headers = self.headers_state.clone().unwrap_or_else(|| {
            let mut headers: Vec<_> = PredefinedHeader::editable()
                .into_iter()
                .map(|header| HeaderState {
                    enabled: true,
                    key: header.name().into(),
                    value: header.default_value().into(),
                    header_type: HeaderType::Predefined,
                    predefined: Some(header),
                })
                .collect();
            for (key, value) in &self.request.headers {
                if let Some(header) = headers
                    .iter_mut()
                    .find(|header| header.key.eq_ignore_ascii_case(key))
                {
                    header.value = value.clone();
                } else {
                    headers.push(HeaderState {
                        enabled: true,
                        key: key.clone(),
                        value: value.clone(),
                        header_type: HeaderType::Custom,
                        predefined: None,
                    });
                }
            }
            headers
        });
        headers.retain(|header| {
            !header.is_transport_owned() && (!header.key.is_empty() || !header.value.is_empty())
        });
        let mut request = self.request.clone();
        request.headers = headers
            .iter()
            .filter(|header| header.enabled && !header.key.is_empty())
            .map(|header| (header.key.clone(), header.value.clone()))
            .collect();
        let params = self
            .params_state
            .clone()
            .unwrap_or_else(|| {
                crate::url_params::parse_query_params(&request.url)
                    .into_iter()
                    .map(|(key, value)| ParamState {
                        enabled: true,
                        key,
                        value,
                    })
                    .collect()
            })
            .into_iter()
            .filter(|param| !param.key.is_empty() || !param.value.is_empty())
            .collect();
        let mut body_draft = self.body_draft.clone();
        body_draft.formdata_rows.retain(|row| !row.is_blank());
        RequestSnapshot {
            request,
            body_draft,
            params_state: params,
            headers_state: headers,
            saved_name: self.saved_name.clone(),
            collection_id: self.collection_id,
            folder_id: self.folder_id,
        }
    }

    pub fn editor_state(&self) -> EditorTabState {
        EditorTabState {
            request: self.request.clone(),
            body_draft: self.body_draft.clone(),
            params_state: self.params_state.clone(),
            headers_state: self.headers_state.clone(),
            section: self.editor_section,
        }
    }

    pub fn apply_editor_state(&mut self, state: EditorTabState) {
        let EditorTabState {
            request,
            body_draft,
            params_state,
            headers_state,
            section,
        } = state;
        self.request = request;
        self.body_draft = body_draft;
        self.params_state = params_state;
        self.headers_state = headers_state;
        self.editor_section = section;
    }

    pub fn response_state(&self) -> ResponseTabState {
        ResponseTabState {
            response: self.response.clone(),
            canceled: self.response_canceled,
            section: self.response_section,
        }
    }

    pub fn apply_response_state(&mut self, state: ResponseTabState) {
        let ResponseTabState {
            response,
            canceled,
            section,
        } = state;
        self.response = response;
        self.response_canceled = canceled;
        self.response_section = section;
    }

    pub fn is_dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|baseline| !baseline.matches(self))
    }

    pub fn needs_baseline(&self) -> bool {
        self.baseline.is_none()
    }

    pub fn establish_baseline(&mut self) {
        self.mark_saved(self.snapshot());
    }

    pub fn mark_saved(&mut self, snapshot: RequestSnapshot) {
        self.baseline = Some(Arc::new(snapshot));
    }

    pub fn reconcile_saved_metadata(&mut self, saved: &crate::types::SavedRequest) {
        self.collection_id = Some(saved.collection_id);
        self.folder_id = saved.folder_id;
        self.saved_name = Some(saved.name.clone());
        if let Some(baseline) = &mut self.baseline {
            let baseline = Arc::make_mut(baseline);
            baseline.collection_id = self.collection_id;
            baseline.folder_id = self.folder_id;
            baseline.saved_name = self.saved_name.clone();
        }
        self.update_title_from_saved_name();
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
    pub fn complete_request(&mut self, request_id: u64, response: Arc<ResponseData>) -> bool {
        if self.active_request_id != Some(request_id) {
            return false;
        }
        self.active_request_id = None;
        self.response = Some(response);
        self.response_canceled = false;
        self.response_section = ResponseSection::Body;
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
        self.response_section = ResponseSection::Body;
        true
    }

    pub fn is_blank(&self) -> bool {
        self.history_id.is_none()
            && self.saved_request_id.is_none()
            && self.response.is_none()
            && self.active_request_id.is_none()
            && !self.response_canceled
            && !self.is_dirty()
            && {
                static EMPTY: std::sync::OnceLock<RequestSnapshot> = std::sync::OnceLock::new();
                EMPTY
                    .get_or_init(|| Self::new_empty(0).snapshot())
                    .matches(self)
            }
    }
}

#[cfg(test)]
mod snapshot_tests;

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
            downloaded_to: None,
            downloaded_bytes: None,
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
            downloaded_to: None,
            downloaded_bytes: None,
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
        tabs[0].begin_request(
            101,
            RequestData::new(HttpMethod::GET, "https://a.test".into()),
        );

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
        a.begin_request(
            101,
            RequestData::new(HttpMethod::GET, "https://a.test".into()),
        );
        b.begin_request(
            102,
            RequestData::new(HttpMethod::GET, "https://b.test".into()),
        );

        assert!(b.complete_request(102, response(202)));
        assert!(a.complete_request(101, response(201)));
        assert_eq!(a.response.unwrap().status, Some(201));
        assert_eq!(b.response.unwrap().status, Some(202));
    }

    #[test]
    fn cancel_affects_only_the_matching_tab_and_request() {
        let mut a = RequestTab::new_empty(10);
        let mut b = RequestTab::new_empty(20);
        a.begin_request(
            101,
            RequestData::new(HttpMethod::GET, "https://a.test".into()),
        );
        b.begin_request(
            102,
            RequestData::new(HttpMethod::GET, "https://b.test".into()),
        );

        assert!(b.cancel_request(102));
        assert!(b.response_canceled);
        assert_eq!(a.active_request_id, Some(101));
        assert!(!a.response_canceled);
        assert!(!a.cancel_request(999));
    }

    #[test]
    fn stale_same_tab_completion_cannot_overwrite_a_newer_request() {
        let mut tab = RequestTab::new_empty(10);
        tab.begin_request(
            101,
            RequestData::new(HttpMethod::GET, "https://old.test".into()),
        );
        tab.begin_request(
            102,
            RequestData::new(HttpMethod::GET, "https://new.test".into()),
        );

        assert!(!tab.complete_request(101, response(201)));
        assert!(tab.response.is_none());
        assert!(tab.complete_request(102, response(202)));
        assert_eq!(tab.response.unwrap().status, Some(202));
    }

    #[test]
    fn closing_a_running_tab_leaves_no_completion_destination() {
        let mut tabs = vec![RequestTab::new_empty(10), RequestTab::new_empty(20)];
        tabs[0].begin_request(
            101,
            RequestData::new(HttpMethod::GET, "https://a.test".into()),
        );

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

    #[test]
    fn editor_and_response_state_round_trip_through_the_tab() {
        let mut tab = RequestTab::new_empty(0);
        let editor = EditorTabState {
            request: RequestData::new(HttpMethod::PUT, "https://a.test/x?q=1".into()),
            body_draft: BodyDraft::from_body(&BodyType::default()),
            params_state: Some(vec![ParamState {
                enabled: false,
                key: "q".into(),
                value: "1".into(),
            }]),
            headers_state: Some(vec![]),
            section: RequestSection::Auth,
        };
        tab.apply_editor_state(editor.clone());
        assert_eq!(tab.editor_state(), editor);

        tab.apply_response_state(ResponseTabState {
            response: Some(response(201)),
            canceled: false,
            section: ResponseSection::Headers,
        });
        let state = tab.response_state();
        assert_eq!(state.response.map(|r| r.status), Some(Some(201)));
        assert!(!state.canceled);
        assert_eq!(state.section, ResponseSection::Headers);
    }

    #[test]
    fn new_response_or_cancel_reopens_the_body_section() {
        let mut tab = RequestTab::new_empty(0);
        tab.response_section = ResponseSection::Headers;
        tab.begin_request(1, empty_request());
        assert!(tab.complete_request(1, response(200)));
        assert_eq!(tab.response_section, ResponseSection::Body);

        tab.response_section = ResponseSection::Headers;
        tab.begin_request(2, empty_request());
        assert!(tab.cancel_request(2));
        assert_eq!(tab.response_section, ResponseSection::Body);

        // A stale completion must not touch the tab, section included.
        tab.response_section = ResponseSection::Headers;
        assert!(!tab.complete_request(1, response(500)));
        assert_eq!(tab.response_section, ResponseSection::Headers);
    }
}
