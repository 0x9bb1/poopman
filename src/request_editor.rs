use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::px;
use gpui_component::{
    button::*, checkbox::Checkbox, input::*,
    scroll::ScrollableElement as _,
    select::*, v_flex, ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _,
};
use gpui_component::input::InputEvent;

use crate::auth_editor::AuthEditor;
use crate::body_editor::{BodyEditor, BodyTypeChanged};
use crate::header_completion::HeaderCompletionProvider;
use crate::types::{HeaderType, HttpMethod, PredefinedHeader, RequestData, ResponseData};
use crate::url_params::{self, QueryParam};
use crate::theme::METHOD_SELECT_WIDTH;

/// Event emitted when a request is sent and response is received.
/// The response is `Arc`-shared so subscribers can store it without copying the body.
#[derive(Clone)]
pub struct RequestCompleted {
    pub request: RequestData,
    pub response: std::sync::Arc<ResponseData>,
}

/// Event emitted when the user asks to view the request as a code snippet.
#[derive(Clone)]
pub struct OpenCodeSnippet;

/// Event emitted when the user cancels an in-flight request.
#[derive(Clone)]
pub struct RequestCancelled;

/// Create a header-name input carrying the standard-header typeahead.
///
/// Custom rows get built in three places — loading a request, restoring saved
/// header state, and appending the trailing blank row. Routing all of them through
/// this helper is what stops the completion from being live on one path and dead on
/// the others.
fn custom_header_key_input<T: 'static>(
    value: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    // Owned because `cx.new` takes a 'static closure.
    let value = value.to_string();
    cx.new(move |cx| {
        let mut input = InputState::new(window, cx).placeholder("Header name");
        input.lsp.completion_provider = Some(std::rc::Rc::new(HeaderCompletionProvider));
        if !value.is_empty() {
            input.set_value(&value, window, cx);
        }
        input
    })
}

/// Header row with key-value inputs and enabled checkbox
struct HeaderRow {
    enabled: bool,
    key_input: Entity<InputState>,
    value_input: Entity<InputState>,
    header_type: HeaderType,
    predefined: Option<PredefinedHeader>,
    /// Character count of the key field at the previous change, used to tell an
    /// accepted completion (a multi-character replacement) from manual typing (one
    /// character at a time). See `maybe_advance_after_completion`.
    last_key_len: usize,
}

/// Query parameter row with key-value inputs and enabled checkbox
struct ParamRow {
    enabled: bool,
    key_input: Entity<InputState>,
    value_input: Entity<InputState>,
}

/// Request editor panel
pub struct RequestEditor {
    url_input: Entity<InputState>,
    method_select: Entity<SelectState<Vec<&'static str>>>,
    body_editor: Entity<BodyEditor>,
    auth_editor: Entity<AuthEditor>,
    headers: Vec<HeaderRow>,
    headers_scroll_handle: ScrollHandle,
    params: Vec<ParamRow>,
    params_scroll_handle: ScrollHandle,
    active_tab: usize,
    loading: bool,
    /// Abort handle for the in-flight request (Some only while loading).
    abort_handle: Option<tokio::task::AbortHandle>,
    /// Incremented on every send *and* cancel; spawned tasks capture their
    /// generation and bail out if it no longer matches, so a stale task can
    /// never clobber state owned by a newer send.
    send_generation: u64,
    _subscriptions: Vec<Subscription>,       // Permanent: URL input + body editor subscriptions
    _row_subscriptions: Vec<Subscription>,   // Header/param row subscriptions; rebuilt on load
    /// Active environment variables, pushed by PoopmanApp; used at send time.
    env_vars: std::collections::HashMap<String, String>,
}

impl RequestEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://api.github.com/zen"));

        let method_select = cx.new(|cx| {
            SelectState::new(
                vec!["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"],
                Some(IndexPath::default()), // Default to GET
                window,
                cx,
            )
        });

        let body_editor = cx.new(|cx| BodyEditor::new(window, cx));
        let auth_editor = cx.new(|cx| AuthEditor::new(window, cx));

        // Subscribe to body type changes to auto-update Content-Type header
        let body_sub = cx.subscribe_in(&body_editor, window, |this: &mut RequestEditor, _, event: &BodyTypeChanged, window, cx| {
            this.update_content_type_from_body(&event.content_type, window, cx);
        });

        let mut editor = Self {
            url_input: url_input.clone(),
            method_select,
            body_editor,
            auth_editor,
            headers: vec![],
            headers_scroll_handle: ScrollHandle::new(),
            params: vec![],
            params_scroll_handle: ScrollHandle::new(),
            active_tab: 0,
            loading: false,
            abort_handle: None,
            send_generation: 0,
            _subscriptions: vec![],
            _row_subscriptions: vec![],
            env_vars: std::collections::HashMap::new(),
        };

        // Subscribe to URL input changes: a pasted `curl …` command imports the
        // whole request; anything else just re-parses query params.
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let value = this.url_input.read(cx).value().to_string();
                if value.trim_start().starts_with("curl ")
                    && let Some(request) = crate::curl_import::parse_curl(&value)
                {
                    // load_request rewrites the URL input, which re-fires
                    // Change — the new value no longer starts with "curl",
                    // so there is no loop.
                    this.load_request(&request, window, cx);
                    return;
                }
            }
            this.parse_url_to_params(window, cx);
        });
        editor._subscriptions.push(url_sub);
        editor._subscriptions.push(body_sub);

        // Initialize with predefined headers
        editor.init_predefined_headers(window, cx);

        // Add initial empty custom header row with subscription
        editor.add_custom_header_row(window, cx);

        // Initialize params with one empty row
        editor.add_param_row(window, cx);

        editor
    }

    /// Initialize all predefined headers
    fn init_predefined_headers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for predefined in PredefinedHeader::all() {
            let header_type = predefined.header_type();

            let key_input = cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(predefined.name(), window, cx);
                input
            });

            let value_input = cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(predefined.default_value(), window, cx);
                input
            });

            self.headers.push(HeaderRow {
                enabled: true, // All predefined headers are enabled by default
                key_input,
                value_input,
                header_type,
                predefined: Some(predefined),
                last_key_len: predefined.name().chars().count(),
            });
        }
    }

    /// Load a request from history
    pub fn load_request(
        &mut self,
        request: &RequestData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Set URL
        self.url_input.update(cx, |input, cx| {
            input.set_value(&request.url, window, cx);
        });

        // Set method
        let method_index = HttpMethod::all()
            .iter()
            .position(|m| *m == request.method)
            .unwrap_or(0);
        self.method_select.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::default().row(method_index)), window, cx);
        });

        // Set body via BodyEditor
        self.body_editor.update(cx, |editor, cx| {
            editor.set_body(&request.body, window, cx);
        });

        // Set auth via AuthEditor
        self.auth_editor.update(cx, |editor, cx| {
            editor.set_auth(&request.auth, window, cx);
        });

        // Set headers - reinitialize with predefined headers
        self.headers.clear();
        // Only clear ROW subscriptions (header/param rows). The permanent URL and body
        // subscriptions in self._subscriptions must survive, otherwise body Content-Type
        // sync and header auto-add silently break after switching tabs / loading history.
        self._row_subscriptions.clear();

        // Clear params to force rebuild with fresh subscriptions.
        self.params.clear();

        // First, add all predefined headers
        self.init_predefined_headers(window, cx);

        // Then, update predefined headers or add custom headers from the loaded request
        for (key, value) in &request.headers {
            // Check if this matches a predefined header
            let all_predefined = PredefinedHeader::all();
            let predefined_match = all_predefined
                .iter()
                .find(|p| p.name().eq_ignore_ascii_case(key));

            if let Some(&predefined) = predefined_match {
                // Update the predefined header's value and enable it
                for header in &mut self.headers {
                    if header.predefined == Some(predefined) {
                        header.value_input.update(cx, |input, cx| {
                            input.set_value(value, window, cx);
                        });
                        header.enabled = true;
                        break;
                    }
                }
            } else {
                // Add as custom header
                let key_input = custom_header_key_input(key, window, cx);
                let value_input = cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(value, window, cx);
                    input
                });

                self.headers.push(HeaderRow {
                    enabled: true,
                    key_input,
                    value_input,
                    header_type: HeaderType::Custom,
                    predefined: None,
                    last_key_len: key.chars().count(),
                });
            }
        }

        // Add one empty custom header row at the end with subscription
        self.add_custom_header_row(window, cx);

        // Populate params from the URL. Use the ungated rebuild directly: this is a
        // programmatic load, so the URL input does not hold focus and the focus-gated
        // parse_url_to_params would otherwise bail and leave Params empty.
        self.rebuild_params_from_url(window, cx);

        // Force sync Content-Type with body type to auto-correct any inconsistencies in history
        let content_type = match &request.body {
            crate::types::BodyType::None => None,
            crate::types::BodyType::Raw { subtype, .. } => Some(subtype.content_type().to_string()),
            crate::types::BodyType::FormData(_) => Some("multipart/form-data; boundary=<auto>".to_string()),
        };
        self.update_content_type_from_body(&content_type, window, cx);

        cx.notify();
    }

    /// Replace the active environment variable map (called by PoopmanApp).
    pub fn set_env_vars(&mut self, vars: std::collections::HashMap<String, String>) {
        self.env_vars = vars;
    }

    /// Extract current request data from the editor
    pub fn get_current_request_data(&self, cx: &App) -> RequestData {
        // Get URL
        let url = self.url_input.read(cx).value().to_string();

        // Get method
        let method_index = self
            .method_select
            .read(cx)
            .selected_index(cx).map(|idx| idx.row)
            .unwrap_or(0);
        let method = HttpMethod::all().get(method_index).copied().unwrap_or(HttpMethod::GET);

        // Get headers (only enabled ones, excluding empty custom headers)
        let mut headers = Vec::new();
        for header_row in &self.headers {
            if header_row.enabled {
                let key = header_row.key_input.read(cx).value().to_string();
                let value = header_row.value_input.read(cx).value().to_string();

                // Skip empty custom headers (the placeholder row)
                if !key.is_empty() || !matches!(header_row.header_type, HeaderType::Custom) {
                    headers.push((key, value));
                }
            }
        }

        // Get body
        let body = self.body_editor.read(cx).get_body(cx);

        RequestData {
            method,
            url,
            headers,
            body,
            auth: self.auth_editor.read(cx).get_auth(cx),
        }
    }

    /// Current request with `{{vars}}` resolved against the active environment,
    /// for code generation / previews.
    pub fn resolved_request_data(&self, cx: &App) -> RequestData {
        crate::variables::substitute_request(&self.get_current_request_data(cx), &self.env_vars)
    }

    /// Extract complete params state including disabled params
    pub fn get_params_state(&self, cx: &App) -> Vec<crate::types::ParamState> {
        self.params
            .iter()
            .map(|param_row| {
                let key = param_row.key_input.read(cx).value().to_string();
                let value = param_row.value_input.read(cx).value().to_string();
                crate::types::ParamState {
                    enabled: param_row.enabled,
                    key,
                    value,
                }
            })
            .filter(|state| !state.key.is_empty() || !state.value.is_empty())
            .collect()
    }

    /// Extract complete headers state including disabled headers
    pub fn get_headers_state(&self, cx: &App) -> Vec<crate::types::HeaderState> {
        self.headers
            .iter()
            .map(|header_row| {
                let key = header_row.key_input.read(cx).value().to_string();
                let value = header_row.value_input.read(cx).value().to_string();
                crate::types::HeaderState {
                    enabled: header_row.enabled,
                    key,
                    value,
                    header_type: header_row.header_type,
                    predefined: header_row.predefined,
                }
            })
            .collect()
    }

    /// Load params state (including disabled params)
    pub fn load_params_state(&mut self, state: &[crate::types::ParamState], window: &mut Window, cx: &mut Context<Self>) {
        // Clear existing params and subscriptions related to params
        self.params.clear();

        // Rebuild params from saved state
        for param_state in state {
            let param_row = ParamRow {
                enabled: param_state.enabled,
                key_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&param_state.key, window, cx);
                    input
                }),
                value_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&param_state.value, window, cx);
                    input
                }),
            };

            // Subscribe to changes for syncing back to URL
            let sub1 = cx.subscribe_in(&param_row.key_input, window, |this, _, _event: &InputEvent, window, cx| {
                this.sync_params_to_url(window, cx);
            });
            let sub2 = cx.subscribe_in(&param_row.value_input, window, |this, _, _event: &InputEvent, window, cx| {
                this.sync_params_to_url(window, cx);
            });

            self._row_subscriptions.push(sub1);
            self._row_subscriptions.push(sub2);
            self.params.push(param_row);
        }

        // Add one empty row for new params
        self.add_param_row(window, cx);

        cx.notify();
    }

    /// Load headers state (including disabled headers)
    pub fn load_headers_state(&mut self, state: &[crate::types::HeaderState], window: &mut Window, cx: &mut Context<Self>) {
        // Clear existing headers and subscriptions
        self.headers.clear();

        // Rebuild headers from saved state
        for header_state in state {
            // Predefined rows render their key field disabled, so only custom rows
            // get the typeahead.
            let key_input = if matches!(header_state.header_type, HeaderType::Custom) {
                custom_header_key_input(&header_state.key, window, cx)
            } else {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&header_state.key, window, cx);
                    input
                })
            };

            let header_row = HeaderRow {
                enabled: header_state.enabled,
                key_input,
                value_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&header_state.value, window, cx);
                    input
                }),
                header_type: header_state.header_type,
                predefined: header_state.predefined,
                last_key_len: header_state.key.chars().count(),
            };

            // Subscribe to key input change if it's a custom header
            if matches!(header_state.header_type, HeaderType::Custom) {
                let key_input = header_row.key_input.clone();
                let key_input_for_closure = key_input.clone();
                let sub = cx.subscribe_in(&key_input, window, move |this, emitter, _event: &InputEvent, window, cx| {
                    this.maybe_advance_after_completion(emitter, window, cx);

                    if let Some(last) = this.headers.last() {
                        let has_key = !last.key_input.read(cx).value().is_empty();
                        if has_key
                            && matches!(last.header_type, HeaderType::Custom)
                            && this.headers.last().map(|h| Entity::entity_id(&h.key_input)) == Some(Entity::entity_id(&key_input_for_closure))
                        {
                            this.add_custom_header_row(window, cx);
                        }
                    }
                });
                self._row_subscriptions.push(sub);
            }

            self.headers.push(header_row);
        }

        // Ensure there's at least one empty custom header row
        let has_custom_headers = self.headers.iter().any(|h| matches!(h.header_type, HeaderType::Custom));
        if !has_custom_headers {
            self.add_custom_header_row(window, cx);
        }

        cx.notify();
    }

    /// Detect an accepted header-name completion and move focus to the value field.
    ///
    /// The library exposes no "completion accepted" hook, so we infer one: a change
    /// that grows the key by more than one character and leaves it exactly equal to a
    /// standard header name is a menu insertion (or a paste of a full name), never
    /// manual typing, which advances one character at a time. This fires after the
    /// library re-focuses the key input (both run off the same Change), so focusing
    /// the value input here wins.
    fn maybe_advance_after_completion(
        &mut self,
        changed: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed_id = Entity::entity_id(changed);
        let Some(idx) = self
            .headers
            .iter()
            .position(|h| Entity::entity_id(&h.key_input) == changed_id)
        else {
            return;
        };

        let value = self.headers[idx].key_input.read(cx).value().to_string();
        let cur_len = value.chars().count();
        let grew_by_more_than_one = cur_len > self.headers[idx].last_key_len + 1;
        self.headers[idx].last_key_len = cur_len;

        if grew_by_more_than_one && crate::header_names::HEADER_NAMES.contains(&value.as_str()) {
            let value_input = self.headers[idx].value_input.clone();
            value_input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn add_custom_header_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_row = HeaderRow {
            enabled: true,
            key_input: custom_header_key_input("", window, cx),
            value_input: cx.new(|cx| InputState::new(window, cx).placeholder("Value")),
            header_type: HeaderType::Custom,
            predefined: None,
            last_key_len: 0,
        };

        // Subscribe to the key input change
        let key_input = new_row.key_input.clone();
        let key_input_for_closure = key_input.clone();
        let sub = cx.subscribe_in(&key_input, window, move |this, emitter, _event: &InputEvent, window, cx| {
            this.maybe_advance_after_completion(emitter, window, cx);

            // Check if this was the last row and it now has content
            if let Some(last) = this.headers.last() {
                let has_key = !last.key_input.read(cx).value().is_empty();
                // Only auto-add if the last row is a custom row
                if has_key
                    && matches!(last.header_type, HeaderType::Custom)
                    && this.headers.last().map(|h| Entity::entity_id(&h.key_input)) == Some(Entity::entity_id(&key_input_for_closure))
                {
                    this.add_custom_header_row(window, cx);

                    // Scroll to bottom after adding new row
                    let scroll_handle = this.headers_scroll_handle.clone();
                    cx.spawn_in(window, async move |_this, cx| {
                        // Wait for layout to stabilize by checking max_offset changes
                        let mut last_offset = px(0.);
                        let mut stable_count = 0;

                        for _ in 0..20 {  // Max 20 attempts (~20ms)
                            cx.background_executor().timer(std::time::Duration::from_millis(1)).await;

                            let current = scroll_handle.max_offset().height;
                            if (current - last_offset).abs() < px(0.1) {
                                stable_count += 1;
                                if stable_count >= 2 {
                                    // Offset stable for 2 checks, layout likely complete
                                    break;
                                }
                            } else {
                                stable_count = 0;
                            }
                            last_offset = current;
                        }

                        // Scroll to bottom
                        let _ = cx.update(|_, _cx| {
                            let max_offset = scroll_handle.max_offset();
                            scroll_handle.set_offset(point(px(0.), -max_offset.height));
                        });
                    }).detach();

                    cx.notify();
                }
            }
        });

        self._row_subscriptions.push(sub);
        self.headers.push(new_row);
        cx.notify();
    }

    fn toggle_header(&mut self, index: usize, _checked: &bool, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(header) = self.headers.get_mut(index) {
            // Cannot disable mandatory headers (e.g., Cache-Control)
            if !matches!(header.header_type, HeaderType::Mandatory) {
                header.enabled = !header.enabled;
                cx.notify();
            }
        }
    }

    fn remove_header_row(
        &mut self,
        index: usize,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only allow deletion of custom headers
        if let Some(header) = self.headers.get(index)
            && matches!(header.header_type, HeaderType::Custom)
        {
            self.headers.remove(index);

            // Check if there are any custom headers left
            let has_custom_headers = self.headers.iter().any(|h| matches!(h.header_type, HeaderType::Custom));

            // If no custom headers remain, add an empty one
            if !has_custom_headers {
                self.add_custom_header_row(window, cx);
            }

            cx.notify();
        }
    }

    /// Update Content-Length header with calculated value from body
    fn update_content_length(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content_length = self.body_editor.read(cx).calculate_length(cx).to_string();

        // Find Content-Length header and update it
        for header in &mut self.headers {
            if let Some(predefined) = header.predefined
                && matches!(predefined, PredefinedHeader::ContentLength)
            {
                header.value_input.update(cx, |input, cx| {
                    input.set_value(&content_length, window, cx);
                });
                break;
            }
        }
    }

    /// Update Content-Type header to match body type
    fn update_content_type_from_body(&mut self, content_type: &Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        // Find Content-Type header and update it
        let new_value = content_type.clone().unwrap_or_default();
        for header in &mut self.headers {
            if let Some(predefined) = header.predefined
                && matches!(predefined, PredefinedHeader::ContentType)
            {
                // Update Content-Type value
                let value_to_set = new_value.clone();
                header.value_input.update(cx, |input, cx| {
                    input.set_value(&value_to_set, window, cx);
                });

                log::debug!("Auto-updated Content-Type header to: {}", new_value);
                break;
            }
        }
    }

    /// Parse URL query parameters into params list.
    ///
    /// This function synchronizes the params list with the URL's query string.
    /// It uses pure functions from url_params module for parsing logic.
    fn parse_url_to_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Focus arbitration: only parse when the URL input is the focused widget.
        // sync_params_to_url's programmatic set_value also emits InputEvent::Change,
        // but the URL input is not focused then, so this returns early and the
        // bidirectional loop is broken without any reentrancy flags.
        if !self.url_input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }

        self.rebuild_params_from_url(window, cx);
    }

    /// Rebuild the params list from the URL's query string. No focus gating.
    ///
    /// Used by the focus-gated `parse_url_to_params` wrapper (live URL edits) and
    /// directly by `load_request`, where the URL is set programmatically and never
    /// holds focus — so it must populate params unconditionally.
    fn rebuild_params_from_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url_str = self.url_input.read(cx).value().to_string();
        let new_params = url_params::parse_query_params(&url_str);

        // URL is non-empty but has no query string (user still typing the base URL):
        // keep existing params instead of wiping them.
        if new_params.is_empty()
            && !url_str.is_empty()
            && !url_str.contains('?')
            && !self.params.is_empty()
        {
            return;
        }

        // Skip rebuild if the parsed params match current params (avoids disrupting
        // the user mid-edit and avoids needless entity churn).
        let current_params: Vec<(String, String)> = self
            .params
            .iter()
            .map(|p| {
                (
                    p.key_input.read(cx).value().to_string(),
                    p.value_input.read(cx).value().to_string(),
                )
            })
            .filter(|(k, v)| !k.is_empty() || !v.is_empty())
            .collect();
        if url_params::params_equal(&new_params, &current_params) && !self.params.is_empty() {
            return;
        }

        // Rebuild params list from the URL query string.
        self.params.clear();
        for (key_str, value_str) in new_params {
            self.add_param_row_with_values(&key_str, &value_str, true, window, cx);
        }
        // Always keep one trailing empty row for adding new params.
        self.add_param_row(window, cx);

        cx.notify();
    }

    /// Add a param row with specific values (helper for parse_url_to_params)
    fn add_param_row_with_values(
        &mut self,
        key: &str,
        value: &str,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Convert to String to avoid lifetime issues
        let key_string = key.to_string();
        let value_string = value.to_string();

        let param_row = ParamRow {
            enabled,
            key_input: cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(&key_string, window, cx);
                input
            }),
            value_input: cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(&value_string, window, cx);
                input
            }),
        };

        // Subscribe to changes for syncing back to URL
        let sub1 = cx.subscribe_in(&param_row.key_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.sync_params_to_url(window, cx);
        });
        let sub2 = cx.subscribe_in(&param_row.value_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.sync_params_to_url(window, cx);
        });

        self._row_subscriptions.push(sub1);
        self._row_subscriptions.push(sub2);
        self.params.push(param_row);
    }

    /// Sync params list to URL input box.
    ///
    /// This function rebuilds the URL query string from the current params list
    /// and updates the URL input. Uses pure functions from url_params module.
    fn sync_params_to_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Focus arbitration: only sync when a param input is the focused widget.
        // Otherwise this Change was triggered by a programmatic set_value (e.g. from
        // parse_url_to_params rebuilding rows), and syncing back would loop.
        let param_focused = self.params.iter().any(|p| {
            p.key_input.read(cx).focus_handle(cx).is_focused(window)
                || p.value_input.read(cx).focus_handle(cx).is_focused(window)
        });
        if !param_focused {
            return;
        }

        self.rebuild_url_from_params(window, cx);
    }

    /// Rebuild the URL input from the current params list. No focus gating.
    ///
    /// Used both by `sync_params_to_url` (the focus-gated wrapper for text edits)
    /// and directly by button callbacks (toggle/remove), where no text input holds
    /// focus. The resulting `set_value` emits InputEvent::Change, but the URL input
    /// is not focused, so `parse_url_to_params` short-circuits — no loop.
    fn rebuild_url_from_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_url = self.url_input.read(cx).value().to_string();
        let new_url = self.rebuild_url_with_params(&current_url, cx);
        self.url_input.update(cx, |input, cx| {
            input.set_value(&new_url, window, cx);
        });
    }

    /// Rebuild URL by combining base URL with current params.
    ///
    /// Uses pure functions from url_params module for URL building.
    fn rebuild_url_with_params(&self, url_str: &str, cx: &App) -> String {
        log::debug!("Rebuilding URL from: {}", url_str);

        // Extract base URL using pure function
        let base = url_params::extract_base_url(url_str);

        // Collect params as QueryParam structs
        let params: Vec<QueryParam> = self.params
            .iter()
            .map(|p| QueryParam::new(
                p.key_input.read(cx).value().to_string(),
                p.value_input.read(cx).value().to_string(),
                p.enabled,
            ))
            .collect();

        // Build URL using pure function
        let result = url_params::build_url_with_params(base, &params);

        log::debug!("Rebuilt URL to: {}", result);
        result
    }

    /// Add a new param row with auto-add functionality
    fn add_param_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_row = ParamRow {
            enabled: true,
            key_input: cx.new(|cx| InputState::new(window, cx).placeholder("Parameter")),
            value_input: cx.new(|cx| InputState::new(window, cx).placeholder("Value")),
        };

        // Subscribe to key input change for auto-add
        let key_input = new_row.key_input.clone();
        let key_input_for_closure = key_input.clone();
        let sub_key = cx.subscribe_in(&key_input, window, move |this, _, _event: &InputEvent, window, cx| {
            // Sync to URL
            this.sync_params_to_url(window, cx);

            // Auto-add new row if this is the last one and has content
            if let Some(last) = this.params.last() {
                let has_key = !last.key_input.read(cx).value().is_empty();
                if has_key
                    && this.params.last().map(|p| Entity::entity_id(&p.key_input)) == Some(Entity::entity_id(&key_input_for_closure))
                {
                    this.add_param_row(window, cx);

                    // Scroll to bottom
                    let scroll_handle = this.params_scroll_handle.clone();
                    cx.spawn_in(window, async move |_this, cx| {
                        let mut last_offset = px(0.);
                        let mut stable_count = 0;

                        for _ in 0..20 {
                            cx.background_executor().timer(std::time::Duration::from_millis(1)).await;

                            let current = scroll_handle.max_offset().height;
                            if (current - last_offset).abs() < px(0.1) {
                                stable_count += 1;
                                if stable_count >= 2 {
                                    break;
                                }
                            } else {
                                stable_count = 0;
                            }
                            last_offset = current;
                        }

                        let _ = cx.update(|_, _cx| {
                            let max_offset = scroll_handle.max_offset();
                            scroll_handle.set_offset(point(px(0.), -max_offset.height));
                        });
                    }).detach();

                    cx.notify();
                }
            }
        });

        // Subscribe to value input change for syncing
        let sub_value = cx.subscribe_in(&new_row.value_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.sync_params_to_url(window, cx);
        });

        self._row_subscriptions.push(sub_key);
        self._row_subscriptions.push(sub_value);
        self.params.push(new_row);
        cx.notify();
    }

    /// Toggle param enabled state
    fn toggle_param(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(param) = self.params.get_mut(index) {
            param.enabled = !param.enabled;
            self.rebuild_url_from_params(window, cx);
            cx.notify();
        }
    }

    /// Remove a param row
    fn remove_param(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.params.len() {
            self.params.remove(index);

            // Check if there are any non-empty params left
            let has_params = self.params.iter().any(|p| {
                let key = p.key_input.read(cx).value().to_string();
                let value = p.value_input.read(cx).value().to_string();
                !key.is_empty() || !value.is_empty()
            });

            // If no params remain, add an empty one
            if !has_params {
                self.add_param_row(window, cx);
            }

            self.rebuild_url_from_params(window, cx);
            cx.notify();
        }
    }

    /// Abort the in-flight request (the Send button shows Cancel while loading).
    fn cancel_request(
        &mut self,
        _event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(handle) = self.abort_handle.take() {
            handle.abort();
        }
        // Invalidate the spawned task so its completion can't touch state.
        self.send_generation = self.send_generation.wrapping_add(1);
        self.loading = false;
        cx.emit(RequestCancelled);
        cx.notify();
    }

    fn send_request(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send(window, cx);
    }

    /// Focus the URL input and select all of its text. Public so the ctrl-l
    /// action can trigger it from PoopmanApp.
    ///
    /// Select-all goes through action dispatch because `InputState::select_all`
    /// is `pub(super)` in gpui-component and unreachable from this crate; the
    /// `SelectAll` action itself is public.
    pub fn focus_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.url_input.update(cx, |input, cx| input.focus(window, cx));
        window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
    }

    /// Send the current request. Public so the ctrl-enter action can trigger
    /// it from PoopmanApp; no-op while a request is already in flight (the
    /// button is swapped to Cancel then, but the keyboard path isn't).
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let mut url = self.url_input.read(cx).value().to_string().trim().to_string();
        if url.is_empty() {
            log::warn!("Cannot send request: URL is empty");
            return;
        }

        // Substitute {{env vars}} BEFORE scheme normalization/validation, so a
        // value like "https://host" doesn't get an extra "http://" prefix.
        url = crate::variables::substitute(&url, &self.env_vars);

        // Auto-add scheme if missing (like Postman does) - default to http://
        if !url.starts_with("http://") && !url.starts_with("https://") {
            url = format!("http://{}", url);
            log::debug!("Auto-added http:// scheme to URL: {}", url);
        }

        // Validate URL format after normalization
        if url::Url::parse(&url).is_err() {
            log::error!("Invalid URL format even after normalization: '{}'", url);
            return;
        }

        log::debug!("Sending request to: {}", url);

        // Update Content-Length before sending
        self.update_content_length(window, cx);

        // Get selected method
        let method_index = self
            .method_select
            .read(cx)
            .selected_index(cx)
            .map(|idx| idx.row)
            .unwrap_or(0);
        let method_str = match method_index {
            0 => "GET",
            1 => "POST",
            2 => "PUT",
            3 => "DELETE",
            4 => "PATCH",
            5 => "HEAD",
            6 => "OPTIONS",
            _ => "GET",
        };
        let method = HttpMethod::from_str(method_str).unwrap_or(HttpMethod::GET);

        // Get current body from BodyEditor
        let body = self.body_editor.read(cx).get_body(cx);

        // Build headers from header rows - only include enabled headers
        let mut headers = vec![];
        for header in &self.headers {
            if header.enabled {
                let key = header.key_input.read(cx).value().to_string();
                let value = header.value_input.read(cx).value().to_string();
                if !key.is_empty() && !value.is_empty() {
                    headers.push((key, value));
                }
            }
        }

        // Note: Content-Type is now automatically synced via BodyTypeChanged event
        // No need to auto-add here as it's already in the headers list

        // Substitute {{env vars}} into headers / body at send time. (URL was
        // already substituted earlier, before scheme normalization.)
        let env = &self.env_vars;
        let headers: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| {
                (
                    crate::variables::substitute(k, env),
                    crate::variables::substitute(v, env),
                )
            })
            .collect();
        let body = match body {
            crate::types::BodyType::Raw { content, subtype } => crate::types::BodyType::Raw {
                content: crate::variables::substitute(&content, env),
                subtype,
            },
            crate::types::BodyType::FormData(rows) => crate::types::BodyType::FormData(
                rows.into_iter()
                    .map(|mut row| {
                        row.key = crate::variables::substitute(&row.key, env);
                        row.value = match row.value {
                            crate::types::FormDataValue::Text(t) => {
                                crate::types::FormDataValue::Text(crate::variables::substitute(&t, env))
                            }
                            other => other, // file path left as-is
                        };
                        row
                    })
                    .collect(),
            ),
            crate::types::BodyType::None => crate::types::BodyType::None,
        };

        // Resolve auth {{vars}} and compute the wire header. The saved request
        // keeps manual headers + the auth config; only the wire gets the merged
        // header set (auth wins over a manual same-name header).
        let resolved_auth = crate::variables::substitute_auth(&self.auth_editor.read(cx).get_auth(cx), env);

        let request = RequestData {
            method,
            url: url.clone(),
            headers: headers.clone(),
            body: body.clone(),
            auth: resolved_auth.clone(),
        };

        self.send_generation = self.send_generation.wrapping_add(1);
        let generation = self.send_generation;
        self.loading = true;

        log::debug!("Starting {} request to: {}", method.as_str(), url);

        // Spawn the HTTP work onto the tokio runtime *now* so we can hold an
        // abort handle; the gpui task below only awaits the outcome.
        let start = std::time::Instant::now();
        let client = crate::http_client::HttpClient::new();
        let wire_headers = crate::types::effective_wire_headers(&headers, &resolved_auth);
        let inflight = client.start_send(method, url, wire_headers, body);
        self.abort_handle = Some(inflight.abort_handle());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let response = match inflight.wait().await {
                Ok(r) => r,
                Err(e) => {
                    if e.downcast_ref::<crate::http_client::RequestCanceled>().is_some() {
                        // cancel_request() already reset the UI and bumped the
                        // generation; nothing left to do.
                        return Ok(());
                    }
                    // Handle request error (network error, file read error, etc.)
                    let duration = start.elapsed();
                    let error_message = format!("Request failed: {}", e);
                    log::error!("{}", error_message);

                    let error_response = ResponseData {
                        status: None, // Use None to indicate network error
                        duration_ms: duration.as_millis() as u64,
                        headers: vec![],
                        body: error_message.into_bytes(),
                        is_text: true,
                    };

                    this.update(cx, |this, cx| {
                        if this.send_generation != generation {
                            return; // superseded by a newer send/cancel
                        }
                        this.loading = false;
                        this.abort_handle = None;
                        cx.emit(RequestCompleted {
                            request,
                            response: std::sync::Arc::new(error_response),
                        });
                        cx.notify();
                    })?;
                    return Ok(());
                }
            };

            let duration = start.elapsed();
            let status = response.status;

            log::debug!("Request completed with status {} in {}ms", status, duration.as_millis());

            let is_text = crate::types::is_text_response(&response.headers, &response.body);
            log::debug!("Response body size: {} bytes (text={})", response.body.len(), is_text);

            let response_data = ResponseData {
                status: Some(status),
                duration_ms: duration.as_millis() as u64,
                headers: response.headers,
                body: response.body,
                is_text,
            };

            this.update(cx, |this, cx| {
                if this.send_generation != generation {
                    return; // superseded by a newer send/cancel
                }
                this.loading = false;
                this.abort_handle = None;
                cx.emit(RequestCompleted {
                    request,
                    response: std::sync::Arc::new(response_data),
                });
                cx.notify();
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }
}

impl EventEmitter<RequestCompleted> for RequestEditor {}
impl EventEmitter<OpenCodeSnippet> for RequestEditor {}
impl EventEmitter<RequestCancelled> for RequestEditor {}

impl Render for RequestEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div().id("request-editor-root").flex().flex_col().w_full().h_full().on_click(cx.listener(|_, _, _, cx| cx.stop_propagation())).child(
            // Request section with header
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .w_full()
                .h_full()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    // URL bar
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .w_full()
                        .child(
                            // Method selector - prevent it from growing
                            div()
                                .flex_shrink_0()
                                .w(px(METHOD_SELECT_WIDTH))
                                .child(Select::new(&self.method_select)),
                        )
                        .child(
                            // URL input - takes all remaining space
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .child(Input::new(&self.url_input)),
                        )
                        .child(
                            // Code snippet button (</>) - opens the code dialog
                            div().flex_shrink_0().child(
                                Button::new("code-snippet-btn")
                                    .ghost()
                                    .icon(Icon::empty().path("icons/code.svg"))
                                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                                        cx.emit(OpenCodeSnippet);
                                    })),
                            ),
                        )
                        .child(
                            // Send button - prevent it from shrinking.
                            // While loading it becomes a Cancel button.
                            div().flex_shrink_0().child(if self.loading {
                                Button::new("cancel-btn")
                                    .danger()
                                    .label("Cancel")
                                    .on_click(cx.listener(Self::cancel_request))
                            } else {
                                Button::new("send-btn")
                                    .primary()
                                    .label("Send")
                                    .on_click(cx.listener(Self::send_request))
                            }),
                        ),
                )
                .child(
                    // Tabs for Headers and Body
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .w_full()
                        .flex_1()
                        .min_h_0()  // Critical for scrolling to work
                        .child(
                            crate::ui::segmented_bar(theme)
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 0)
                                        .id("tab-headers")
                                        .when(self.active_tab != 0, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Headers"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 1)
                                        .id("tab-auth")
                                        .when(self.active_tab != 1, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Auth"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 2)
                                        .id("tab-params")
                                        .when(self.active_tab != 2, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 2;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Params"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 3)
                                        .id("tab-body")
                                        .when(self.active_tab != 3, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 3;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Body"),
                                ),
                        )
                        .when(self.active_tab == 0, |this| {
                            this.child(
                                // Viewport: owns the size constraint so the list can
                                // shrink and actually scroll; also hosts the scrollbar,
                                // which must be the scroller's sibling rather than its
                                // child (an absolute layer inside the scroller scrolls
                                // away with the content).
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        // Scrollable headers list
                                        v_flex()
                                            .id("headers-scroll-container")
                                            .gap_2()
                                            .p_2()
                                            .pb_4()  // Bottom padding to prevent last row from being obscured
                                            .size_full()
                                            .track_scroll(&self.headers_scroll_handle)
                                            .overflow_scroll()
                                            .children(self.headers.iter().enumerate().map(
                                        |(index, header)| {
                                            let enabled = header.enabled;
                                            let is_mandatory = matches!(header.header_type, HeaderType::Mandatory);
                                            let is_predefined = !matches!(header.header_type, HeaderType::Custom);
                                            let is_custom = matches!(header.header_type, HeaderType::Custom);
                                            let is_auto_calculated = header.predefined.map(|p| p.is_auto_calculated()).unwrap_or(false);

                                            div()
                                                .flex()
                                                .flex_row()
                                                .gap_2()
                                                .items_center() // Vertical center alignment
                                                .w_full()
                                                .child(
                                                    // Checkbox - disabled for mandatory headers
                                                    div().flex_shrink_0().child(
                                                        Checkbox::new(("header-checkbox", index))
                                                            .checked(enabled)
                                                            .disabled(is_mandatory)
                                                            .on_click(cx.listener(
                                                                move |this, checked, window, cx| {
                                                                    this.toggle_header(index, checked, window, cx);
                                                                },
                                                            ))
                                                    )
                                                )
                                                .child({
                                                    // Key input - disabled for predefined headers.
                                                    //
                                                    // gpui-component registers the up/down action
                                                    // handlers only for multi-line inputs (input.rs
                                                    // `.when(is_multi_line)`), so on a single-line
                                                    // field the arrow keys never reach the completion
                                                    // menu and the highlight cannot move. Enter/Escape
                                                    // work because their handlers are unconditional.
                                                    // We bridge the two arrow actions to the menu via
                                                    // the public `handle_action_for_context_menu`; the
                                                    // single-line Input ignores them, so they bubble
                                                    // up to this wrapper.
                                                    let key_input = header.key_input.clone();
                                                    div()
                                                        .flex_1()
                                                        .when(is_custom, |this| {
                                                            let down_input = key_input.clone();
                                                            let up_input = key_input.clone();
                                                            this.on_action(move |_: &MoveDown, window, cx| {
                                                                down_input.update(cx, |state, cx| {
                                                                    state.handle_action_for_context_menu(Box::new(MoveDown), window, cx);
                                                                });
                                                            })
                                                            .on_action(move |_: &MoveUp, window, cx| {
                                                                up_input.update(cx, |state, cx| {
                                                                    state.handle_action_for_context_menu(Box::new(MoveUp), window, cx);
                                                                });
                                                            })
                                                        })
                                                        .child(Input::new(&header.key_input).disabled(is_predefined))
                                                })
                                                .child(
                                                    // Value input - disabled for auto-calculated headers and Content-Type
                                                    // Delete button embedded as suffix for custom headers
                                                    div()
                                                        .flex_1()
                                                        .child(
                                                            Input::new(&header.value_input)
                                                                .disabled(is_auto_calculated || header.predefined == Some(PredefinedHeader::ContentType))
                                                                .when(is_custom, |input| {
                                                                    input.suffix(
                                                                        Button::new(("delete-header", index))
                                                                            .ghost()
                                                                            .xsmall()
                                                                            .label("×")
                                                                            .on_click(cx.listener(
                                                                                move |this, event, window, cx| {
                                                                                    this.remove_header_row(
                                                                                        index, event, window, cx,
                                                                                    );
                                                                                },
                                                                            ))
                                                                    )
                                                                })
                                                        ),
                                                )
                                        },
                                    ))
                                    )
                                    .vertical_scrollbar(&self.headers_scroll_handle),
                            )
                        })
                        .when(self.active_tab == 1, |this| {
                            this.child(
                                div()
                                    .p_2()
                                    .w_full()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .min_h_0()
                                    .child(self.auth_editor.clone()),
                            )
                        })
                        .when(self.active_tab == 2, |this| {
                            this.child(
                                // Viewport: owns the size constraint so the list can
                                // shrink and actually scroll; also hosts the scrollbar,
                                // which must be the scroller's sibling rather than its
                                // child (an absolute layer inside the scroller scrolls
                                // away with the content).
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        // Scrollable params list
                                        v_flex()
                                            .id("params-scroll-container")
                                            .gap_2()
                                            .p_2()
                                            .pb_4()
                                            .size_full()
                                            .track_scroll(&self.params_scroll_handle)
                                            .overflow_scroll()
                                    .children(self.params.iter().enumerate().map(
                                        |(index, param)| {
                                            let enabled = param.enabled;

                                            div()
                                                .flex()
                                                .flex_row()
                                                .gap_2()
                                                .items_center()
                                                .w_full()
                                                .child(
                                                    // Checkbox
                                                    div().flex_shrink_0().child(
                                                        Checkbox::new(("param-checkbox", index))
                                                            .checked(enabled)
                                                            .on_click(cx.listener(
                                                                move |this, _, window, cx| {
                                                                    this.toggle_param(index, window, cx);
                                                                },
                                                            ))
                                                    )
                                                )
                                                .child(
                                                    // Key input
                                                    div()
                                                        .flex_1()
                                                        .child(Input::new(&param.key_input)),
                                                )
                                                .child(
                                                    // Value input with delete button
                                                    div()
                                                        .flex_1()
                                                        .child(
                                                            Input::new(&param.value_input)
                                                                .suffix(
                                                                    Button::new(("delete-param", index))
                                                                        .ghost()
                                                                        .xsmall()
                                                                        .label("×")
                                                                        .on_click(cx.listener(
                                                                            move |this, _, window, cx| {
                                                                                this.remove_param(index, window, cx);
                                                                            },
                                                                        ))
                                                                )
                                                        ),
                                                )
                                        },
                                    ))
                                    )
                                    .vertical_scrollbar(&self.params_scroll_handle),
                            )
                        })
                        .when(self.active_tab == 3, |this| {
                            // Body tab - render BodyEditor component
                            this.child(
                                div()
                                    .p_2()
                                    .w_full()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .min_h_0()  // Critical for scrolling to work
                                    .child(self.body_editor.clone())
                            )
                        }),
                ),
        )
    }
}

