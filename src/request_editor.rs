use futures::AsyncReadExt;
use gpui::http_client::{http, AsyncBody};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::px;
use gpui_component::{
    button::*, checkbox::Checkbox, input::*,
    select::*, v_flex, ActiveTheme as _, Disableable as _, IndexPath, Sizable as _,
};
use gpui_component::input::InputEvent;
use url::Url;

use crate::body_editor::BodyEditor;
use crate::types::{HeaderType, HttpMethod, PredefinedHeader, RequestData, ResponseData};

/// Event emitted when a request is sent and response is received
#[derive(Clone)]
pub struct RequestCompleted {
    pub request: RequestData,
    pub response: ResponseData,
}

/// Header row with key-value inputs and enabled checkbox
struct HeaderRow {
    enabled: bool,
    key_input: Entity<InputState>,
    value_input: Entity<InputState>,
    header_type: HeaderType,
    predefined: Option<PredefinedHeader>,
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
    headers: Vec<HeaderRow>,
    headers_scroll_handle: ScrollHandle,
    params: Vec<ParamRow>,
    params_scroll_handle: ScrollHandle,
    active_tab: usize,
    loading: bool,
    _subscriptions: Vec<Subscription>,
    updating_url: bool, // Flag to prevent infinite loop between URL and params updates
    parsing_url: bool, // Flag to prevent syncing back to URL while parsing URL to params
    last_parsed_url: String, // Last URL that was parsed to params
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

        let mut editor = Self {
            url_input: url_input.clone(),
            method_select,
            body_editor,
            headers: vec![],
            headers_scroll_handle: ScrollHandle::new(),
            params: vec![],
            params_scroll_handle: ScrollHandle::new(),
            active_tab: 0,
            loading: false,
            _subscriptions: vec![],
            updating_url: false,
            parsing_url: false,
            last_parsed_url: String::new(),
        };

        // Subscribe to URL input changes to parse params
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.parse_url_to_params(window, cx);
        });
        editor._subscriptions.push(url_sub);

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

        // Set headers - reinitialize with predefined headers
        self.headers.clear();
        self._subscriptions.clear();

        // Re-subscribe to URL input for URL → Params sync (cleared above)
        let url_input = self.url_input.clone();
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.parse_url_to_params(window, cx);
        });
        self._subscriptions.push(url_sub);

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
                let key_input = cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(key, window, cx);
                    input
                });
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
                });
            }
        }

        // Add one empty custom header row at the end with subscription
        self.add_custom_header_row(window, cx);

        // Parse URL to populate params (this will also add subscriptions)
        self.parse_url_to_params(window, cx);

        cx.notify();
    }

    /// Extract current request data from the editor
    pub fn get_current_request_data(&self, cx: &App) -> RequestData {
        // Get URL
        let url = self.url_input.read(cx).value().to_string();

        // Get method
        let method_index = self
            .method_select
            .read(cx)
            .selected_index(cx)
            .and_then(|idx| Some(idx.row))
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
        }
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

        // Set flag to prevent syncing back to URL while we're building params
        self.parsing_url = true;

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

            self._subscriptions.push(sub1);
            self._subscriptions.push(sub2);
            self.params.push(param_row);
        }

        // Add one empty row for new params
        self.add_param_row(window, cx);

        // Reset flag after params are built
        self.parsing_url = false;

        // Re-subscribe to URL input for URL → Params sync
        // This is needed because load_request() may have cleared it
        let url_input = self.url_input.clone();
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.parse_url_to_params(window, cx);
        });
        self._subscriptions.push(url_sub);

        cx.notify();
    }

    /// Load headers state (including disabled headers)
    pub fn load_headers_state(&mut self, state: &[crate::types::HeaderState], window: &mut Window, cx: &mut Context<Self>) {
        // Clear existing headers and subscriptions
        self.headers.clear();

        // Rebuild headers from saved state
        for header_state in state {
            let header_row = HeaderRow {
                enabled: header_state.enabled,
                key_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&header_state.key, window, cx);
                    input
                }),
                value_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&header_state.value, window, cx);
                    input
                }),
                header_type: header_state.header_type,
                predefined: header_state.predefined,
            };

            // Subscribe to key input change if it's a custom header
            if matches!(header_state.header_type, HeaderType::Custom) {
                let key_input = header_row.key_input.clone();
                let key_input_for_closure = key_input.clone();
                let sub = cx.subscribe_in(&key_input, window, move |this, _, _event: &InputEvent, window, cx| {
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
                self._subscriptions.push(sub);
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

    fn add_custom_header_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_row = HeaderRow {
            enabled: true,
            key_input: cx.new(|cx| InputState::new(window, cx).placeholder("Header name")),
            value_input: cx.new(|cx| InputState::new(window, cx).placeholder("Value")),
            header_type: HeaderType::Custom,
            predefined: None,
        };

        // Subscribe to the key input change
        let key_input = new_row.key_input.clone();
        let key_input_for_closure = key_input.clone();
        let sub = cx.subscribe_in(&key_input, window, move |this, _, _event: &InputEvent, window, cx| {
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

        self._subscriptions.push(sub);
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
        if let Some(header) = self.headers.get(index) {
            if matches!(header.header_type, HeaderType::Custom) {
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
    }

    /// Update Content-Length header with calculated value from body
    fn update_content_length(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content_length = self.body_editor.read(cx).calculate_length(cx).to_string();

        // Find Content-Length header and update it
        for header in &mut self.headers {
            if let Some(predefined) = header.predefined {
                if matches!(predefined, PredefinedHeader::ContentLength) {
                    header.value_input.update(cx, |input, cx| {
                        input.set_value(&content_length, window, cx);
                    });
                    break;
                }
            }
        }
    }

    /// Parse URL query parameters into params list
    fn parse_url_to_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updating_url || self.parsing_url {
            return; // Prevent infinite loop
        }

        let url_str = self.url_input.read(cx).value().to_string();

        // Check if URL actually changed since last parse
        if url_str == self.last_parsed_url {
            return; // URL hasn't changed, skip parsing
        }

        // Parse URL to get new params
        let mut new_params = Vec::new();

        if let Ok(url) = Url::parse(&url_str) {
            // URL is valid, extract query parameters
            for (key, value) in url.query_pairs() {
                new_params.push((key.to_string(), value.to_string()));
            }
        } else {
            // URL parsing failed, try to parse as query string manually
            if let Some(query_start) = url_str.find('?') {
                let query = &url_str[query_start + 1..];
                for pair in query.split('&') {
                    if let Some(eq_pos) = pair.find('=') {
                        let key = urlencoding::decode(&pair[..eq_pos])
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let value = urlencoding::decode(&pair[eq_pos + 1..])
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        if !key.is_empty() {
                            new_params.push((key, value));
                        }
                    }
                }
            } else if !url_str.is_empty() {
                // URL is not empty but has no query string and isn't parseable
                // Keep existing params unchanged
                return;
            }
            // If url_str is empty, new_params remains empty, which is correct
        }

        // Compare with existing params (excluding empty last row)
        let mut current_params = Vec::new();
        for param in &self.params {
            let key = param.key_input.read(cx).value().to_string();
            let value = param.value_input.read(cx).value().to_string();
            if !key.is_empty() || !value.is_empty() {
                current_params.push((key, value));
            }
        }

        // If params are the same, don't rebuild (avoids destroying input focus)
        if new_params == current_params {
            self.last_parsed_url = url_str; // Update last parsed URL
            return;
        }

        // Set flag to prevent syncing back to URL while we're building params
        self.parsing_url = true;
        self.last_parsed_url = url_str; // Update last parsed URL

        // Clear and rebuild params
        self.params.clear();

        for (key_str, value_str) in new_params {
            let param_row = ParamRow {
                enabled: true,
                key_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&key_str, window, cx);
                    input
                }),
                value_input: cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(&value_str, window, cx);
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

            self._subscriptions.push(sub1);
            self._subscriptions.push(sub2);
            self.params.push(param_row);
        }

        // Add one empty row for new params
        self.add_param_row(window, cx);

        // Reset flag after params are built
        self.parsing_url = false;

        cx.notify();
    }

    /// Sync params list to URL input box
    fn sync_params_to_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updating_url || self.parsing_url {
            return; // Prevent infinite loop and avoid syncing while parsing URL
        }

        self.updating_url = true;

        let current_url = self.url_input.read(cx).value().to_string();
        let new_url = self.rebuild_url_with_params(&current_url, cx);

        self.url_input.update(cx, |input, cx| {
            input.set_value(&new_url, window, cx);
        });

        // Reset flag asynchronously to ensure InputEvent is processed first
        cx.spawn_in(window, async move |this, cx| {
            let _ = this.update(cx, |this, _| {
                this.updating_url = false;
            });
        }).detach();
    }

    /// Rebuild URL with current params
    fn rebuild_url_with_params(&self, url_str: &str, cx: &App) -> String {
        // Extract base URL (without query string)
        let base = if let Some(pos) = url_str.find('?') {
            &url_str[..pos]
        } else {
            url_str
        };

        // Collect enabled params with non-empty keys
        let mut param_parts = vec![];
        for param in &self.params {
            if param.enabled {
                let key = param.key_input.read(cx).value().to_string();
                let value = param.value_input.read(cx).value().to_string();
                if !key.is_empty() {
                    param_parts.push(format!(
                        "{}={}",
                        urlencoding::encode(&key),
                        urlencoding::encode(&value)
                    ));
                }
            }
        }

        if param_parts.is_empty() {
            base.to_string()
        } else {
            format!("{}?{}", base, param_parts.join("&"))
        }
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

        self._subscriptions.push(sub_key);
        self._subscriptions.push(sub_value);
        self.params.push(new_row);
        cx.notify();
    }

    /// Toggle param enabled state
    fn toggle_param(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(param) = self.params.get_mut(index) {
            param.enabled = !param.enabled;
            self.sync_params_to_url(window, cx);
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

            self.sync_params_to_url(window, cx);
            cx.notify();
        }
    }

    fn send_request(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url = self.url_input.read(cx).value().to_string();
        if url.is_empty() {
            return;
        }

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

        // Auto-add Content-Type header based on body type if not already present
        let has_content_type = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Content-Type"));
        if !has_content_type {
            match &body {
                crate::types::BodyType::Raw { subtype, .. } => {
                    headers.push(("Content-Type".to_string(), subtype.content_type().to_string()));
                }
                crate::types::BodyType::FormData(_) => {
                    // Will be set by multipart builder
                }
                _ => {}
            }
        }

        let request = RequestData {
            method,
            url: url.clone(),
            headers: headers.clone(),
            body: body.clone(),
        };

        self.loading = true;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let start = std::time::Instant::now();

            // Use HttpClient which manages its own tokio runtime
            let client = crate::http_client::HttpClient::new();

            // Build HTTP request using http crate from gpui
            let mut request_builder = http::Request::builder().method(method.as_str()).uri(&url);

            // Add headers
            for (key, value) in &headers {
                request_builder = request_builder.header(key.as_str(), value.as_str());
            }

            // Build body based on type
            let body_bytes = match &body {
                crate::types::BodyType::None => Vec::new(),
                crate::types::BodyType::Raw { content, .. } => content.clone().into_bytes(),
                crate::types::BodyType::FormData(rows) => {
                    // Build form data (simplified, actual multipart would be more complex)
                    let mut form_parts = vec![];
                    for row in rows {
                        if row.enabled {
                            match &row.value {
                                crate::types::FormDataValue::Text(text) => {
                                    form_parts.push(format!("{}={}", row.key, text));
                                }
                                crate::types::FormDataValue::File { path } => {
                                    form_parts.push(format!("{}=@{}", row.key, path));
                                }
                            }
                        }
                    }
                    form_parts.join("&").into_bytes()
                }
            };

            // Add body
            let http_request = if !body_bytes.is_empty() {
                request_builder
                    .body(AsyncBody::from(body_bytes))
                    .unwrap()
            } else {
                request_builder.body(AsyncBody::default()).unwrap()
            };

            // Send request
            let response = match client.send(http_request).await {
                Ok(r) => r,
                Err(e) => {
                    // Handle request error (timeout, network error, etc.)
                    let duration = start.elapsed();
                    let error_message = format!("Request failed: {}", e);
                    log::error!("{}", error_message);

                    let error_response = ResponseData {
                        status: None, // Use None to indicate network error
                        duration_ms: duration.as_millis() as u64,
                        headers: vec![],
                        body: error_message,
                    };

                    this.update(cx, |this, cx| {
                        this.loading = false;
                        cx.emit(RequestCompleted {
                            request,
                            response: error_response,
                        });
                        cx.notify();
                    })?;
                    return Ok(());
                }
            };

            let duration = start.elapsed();
            let status = response.status().as_u16();

            // Collect response headers
            let mut resp_headers = vec![];
            for (key, value) in response.headers() {
                if let Ok(v) = value.to_str() {
                    resp_headers.push((key.to_string(), v.to_string()));
                }
            }

            // Read response body
            let mut body_reader = response.into_body();
            let mut body_bytes = Vec::new();
            body_reader
                .read_to_end(&mut body_bytes)
                .await
                .unwrap_or_default();
            let response_body = String::from_utf8_lossy(&body_bytes).to_string();

            let response_data = ResponseData {
                status: Some(status),
                duration_ms: duration.as_millis() as u64,
                headers: resp_headers,
                body: response_body,
            };

            this.update(cx, |this, cx| {
                this.loading = false;
                cx.emit(RequestCompleted {
                    request,
                    response: response_data,
                });
                cx.notify();
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }
}

impl EventEmitter<RequestCompleted> for RequestEditor {}

impl Render for RequestEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div().flex().flex_col().w_full().h_full().bg(theme.background).child(
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
                    // Section title
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child("REQUEST"),
                )
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
                                .w(px(100.))
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
                            // Send button - prevent it from shrinking
                            div().flex_shrink_0().child(
                                Button::new("send-btn")
                                    .primary()
                                    .label("Send")
                                    .disabled(self.loading)
                                    .loading(self.loading)
                                    .on_click(cx.listener(Self::send_request)),
                            ),
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
                            div()
                                .flex()
                                .flex_row()
                                .gap_1()
                                .child(
                                    Button::new("tab-headers")
                                        .ghost()
                                        .label("Headers")
                                        .when(self.active_tab == 0, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("tab-params")
                                        .ghost()
                                        .label("Params")
                                        .when(self.active_tab == 1, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("tab-body")
                                        .ghost()
                                        .label("Body")
                                        .when(self.active_tab == 2, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 2;
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .when(self.active_tab == 0, |this| {
                            this.child(
                                // Scrollable headers list
                                v_flex()
                                    .id("headers-scroll-container")
                                    .gap_2()
                                    .p_2()
                                    .pb_4()  // Bottom padding to prevent last row from being obscured
                                    .flex_1()
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
                                                .child(
                                                    // Key input - disabled for predefined headers
                                                    div()
                                                        .flex_1()
                                                        .child(Input::new(&header.key_input).disabled(is_predefined)),
                                                )
                                                .child(
                                                    // Value input - disabled for auto-calculated headers
                                                    // Delete button embedded as suffix for custom headers
                                                    div()
                                                        .flex_1()
                                                        .child(
                                                            Input::new(&header.value_input)
                                                                .disabled(is_auto_calculated)
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
                        })
                        .when(self.active_tab == 1, |this| {
                            this.child(
                                // Scrollable params list
                                v_flex()
                                    .id("params-scroll-container")
                                    .gap_2()
                                    .p_2()
                                    .pb_4()
                                    .flex_1()
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
                        })
                        .when(self.active_tab == 2, |this| {
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
