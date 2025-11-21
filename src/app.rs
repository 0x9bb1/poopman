use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    resizable::{h_resizable, resizable_panel, v_resizable},
};
use gpui::px;
use std::sync::Arc;

use crate::db::Database;
use crate::history_panel::{HistoryItemClicked, HistoryPanel};
use crate::request_editor::{RequestCompleted, RequestEditor};
use crate::request_tab::RequestTab;
use crate::response_viewer::ResponseViewer;
use crate::tab_bar::{NewTabClicked, TabBar, TabClicked, TabCloseClicked};

/// Main application view
pub struct PoopmanApp {
    #[allow(dead_code)]
    db: Arc<Database>,
    history_panel: Entity<HistoryPanel>,
    request_editor: Entity<RequestEditor>,
    response_viewer: Entity<ResponseViewer>,
    tab_bar: Entity<TabBar>,
    request_tabs: Vec<RequestTab>,
    active_tab_index: usize,
    next_tab_id: usize,
    _subscriptions: Vec<Subscription>,
}

impl PoopmanApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Initialize database
        let db = Arc::new(Database::new().expect("Failed to initialize database"));

        // Create components
        let request_editor = cx.new(|cx| RequestEditor::new(window, cx));
        let response_viewer = cx.new(|cx| ResponseViewer::new(window, cx));
        let history_panel = cx.new(|cx| HistoryPanel::new(db.clone(), window, cx));
        let tab_bar = cx.new(|cx| TabBar::new(window, cx));

        // Initialize with one empty tab
        let request_tabs = vec![RequestTab::new_empty(0)];
        let active_tab_index = 0;
        let next_tab_id = 1;

        // Subscribe to request completion events
        let db_clone = db.clone();
        let history_panel_clone = history_panel.clone();
        let response_viewer_clone = response_viewer.clone();
        let request_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, _, event: &RequestCompleted, window, cx| {
                // Check if current tab is from history (has history_id)
                let is_from_history = this
                    .request_tabs
                    .get(this.active_tab_index)
                    .map(|tab| tab.history_id.is_some())
                    .unwrap_or(false);

                // Only save to database if this is a new request (not from history)
                // Note: Response is not saved to history (aligned with Postman behavior)
                if !is_from_history {
                    let request_headers =
                        serde_json::to_string(&event.request.headers).unwrap_or_default();

                    if let Err(e) = db_clone.insert_history(
                        event.request.method.as_str(),
                        &event.request.url,
                        &request_headers,
                        &event.request.body,
                    ) {
                        log::error!("Failed to save history: {}", e);
                    }

                    // Reload history panel only when new history is created
                    history_panel_clone.update(cx, |panel, cx| {
                        panel.reload(window, cx);
                    });
                }

                // Update response viewer (always)
                response_viewer_clone.update(cx, |viewer, cx| {
                    viewer.set_response(event.response.clone(), window, cx);
                });

                // Update current tab data with the completed request and response (always)
                if let Some(tab) = this.request_tabs.get_mut(this.active_tab_index) {
                    tab.request = event.request.clone();
                    tab.response = Some(event.response.clone());
                    tab.update_title();
                    this.update_tab_bar(cx);
                }
            },
        );

        // Subscribe to history item click events - open in new tab
        let history_sub = cx.subscribe_in(
            &history_panel,
            window,
            move |this, _, event: &HistoryItemClicked, window, cx| {
                this.open_history_in_new_tab(&event.item, window, cx);
            },
        );

        // Subscribe to tab bar events
        let tab_clicked_sub = cx.subscribe_in(
            &tab_bar,
            window,
            move |this, _, event: &TabClicked, window, cx| {
                this.switch_to_tab(event.tab_index, window, cx);
            },
        );

        let new_tab_sub = cx.subscribe_in(
            &tab_bar,
            window,
            move |this, _, _event: &NewTabClicked, window, cx| {
                this.create_new_tab(window, cx);
            },
        );

        let close_tab_sub = cx.subscribe_in(
            &tab_bar,
            window,
            move |this, _, event: &TabCloseClicked, window, cx| {
                this.close_tab(event.tab_index, window, cx);
            },
        );

        Self {
            db,
            history_panel,
            request_editor,
            response_viewer,
            tab_bar,
            request_tabs,
            active_tab_index,
            next_tab_id,
            _subscriptions: vec![
                request_sub,
                history_sub,
                tab_clicked_sub,
                new_tab_sub,
                close_tab_sub,
            ],
        }
    }

    /// Save current editor state to active tab
    fn save_current_tab_state(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.request_tabs.get_mut(self.active_tab_index) {
            let request_data = self.request_editor.read(cx).get_current_request_data(cx);
            let params_state = self.request_editor.read(cx).get_params_state(cx);
            let headers_state = self.request_editor.read(cx).get_headers_state(cx);
            let response = self.response_viewer.read(cx).get_response();

            tab.request = request_data;
            tab.response = response;
            tab.params_state = Some(params_state);
            tab.headers_state = Some(headers_state);
            tab.update_title();
        }
    }

    /// Switch to a different tab
    fn switch_to_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.request_tabs.len() || index == self.active_tab_index {
            return;
        }

        // Save current tab state before switching
        self.save_current_tab_state(cx);

        // Update active index
        self.active_tab_index = index;

        // Load new tab data into editor
        if let Some(tab) = self.request_tabs.get(index).cloned() {
            self.request_editor.update(cx, |editor, cx| {
                // Load basic request data first
                editor.load_request(&tab.request, window, cx);

                // If we have saved UI state, load it (overrides parsed state from URL)
                if let Some(params_state) = &tab.params_state {
                    if !params_state.is_empty() {
                        editor.load_params_state(params_state, window, cx);
                    }
                }

                if let Some(headers_state) = &tab.headers_state {
                    if !headers_state.is_empty() {
                        editor.load_headers_state(headers_state, window, cx);
                    }
                }
            });

            // Load response data
            self.response_viewer.update(cx, |viewer, cx| {
                if let Some(response) = &tab.response {
                    viewer.set_response(response.clone(), window, cx);
                } else {
                    viewer.clear_response(window, cx);
                }
            });
        }

        self.update_tab_bar(cx);
        cx.notify();
    }

    /// Create a new empty tab
    fn create_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Save current tab state
        self.save_current_tab_state(cx);

        // Create new tab
        let new_tab = RequestTab::new_empty(self.next_tab_id);
        self.next_tab_id += 1;
        self.request_tabs.push(new_tab.clone());
        self.active_tab_index = self.request_tabs.len() - 1;

        // Load new tab into editor
        self.request_editor.update(cx, |editor, cx| {
            editor.load_request(&new_tab.request, window, cx);
        });

        // Clear response for new tab
        self.response_viewer.update(cx, |viewer, cx| {
            viewer.clear_response(window, cx);
        });

        self.update_tab_bar(cx);
        cx.notify();
    }

    /// Close a tab
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_tabs.len() <= 1 {
            // Don't close the last tab, just reset it to empty
            self.request_tabs[0] = RequestTab::new_empty(self.next_tab_id);
            self.next_tab_id += 1;
            self.active_tab_index = 0;

            self.request_editor.update(cx, |editor, cx| {
                editor.load_request(&self.request_tabs[0].request, window, cx);
            });

            // Clear response for reset tab
            self.response_viewer.update(cx, |viewer, cx| {
                viewer.clear_response(window, cx);
            });

            self.update_tab_bar(cx);
            cx.notify();
            return;
        }

        // Remove the tab
        self.request_tabs.remove(index);

        // Adjust active tab index
        if index < self.active_tab_index {
            self.active_tab_index -= 1;
        } else if index == self.active_tab_index {
            // If we closed the active tab, activate the tab to the left (or right if it was the first)
            if self.active_tab_index >= self.request_tabs.len() {
                self.active_tab_index = self.request_tabs.len().saturating_sub(1);
            }

            // Load the new active tab
            if let Some(tab) = self.request_tabs.get(self.active_tab_index).cloned() {
                self.request_editor.update(cx, |editor, cx| {
                    editor.load_request(&tab.request, window, cx);
                });

                // Load response for the new active tab
                self.response_viewer.update(cx, |viewer, cx| {
                    if let Some(response) = &tab.response {
                        viewer.set_response(response.clone(), window, cx);
                    } else {
                        viewer.clear_response(window, cx);
                    }
                });
            }
        }

        self.update_tab_bar(cx);
        cx.notify();
    }

    /// Open history item in a new tab (or switch to existing tab if already open)
    fn open_history_in_new_tab(
        &mut self,
        item: &crate::types::HistoryItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Check if this history item is already open in a tab
        if let Some(existing_index) = self
            .request_tabs
            .iter()
            .position(|tab| tab.history_id == Some(item.id))
        {
            // Switch to existing tab instead of creating a new one
            self.switch_to_tab(existing_index, window, cx);
            return;
        }

        // Save current tab state
        self.save_current_tab_state(cx);

        // Create new tab from history
        let new_tab = RequestTab::from_history(self.next_tab_id, item);
        self.next_tab_id += 1;
        self.request_tabs.push(new_tab.clone());
        self.active_tab_index = self.request_tabs.len() - 1;

        // Load into editor
        self.request_editor.update(cx, |editor, cx| {
            editor.load_request(&new_tab.request, window, cx);
        });

        // Load response from history
        self.response_viewer.update(cx, |viewer, cx| {
            if let Some(response) = &new_tab.response {
                viewer.set_response(response.clone(), window, cx);
            } else {
                viewer.clear_response(window, cx);
            }
        });

        self.update_tab_bar(cx);
        cx.notify();
    }

    /// Update tab bar with current tabs
    fn update_tab_bar(&mut self, cx: &mut Context<Self>) {
        self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.update_tabs(self.request_tabs.clone(), self.active_tab_index, cx);
        });
    }
}

impl Render for PoopmanApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .size_full()
            .bg(theme.background)
            .child(
                h_resizable("history-main-splitter")
                    .child(
                        // Left: History panel with resizable width
                        resizable_panel()
                            .size(px(280.)) // Initial width
                            .size_range(px(200.)..px(500.)) // Can resize between 200px-500px
                            .child(
                                div()
                                    .size_full()
                                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation()) // Isolate scroll events
                                    .child(self.history_panel.clone()),
                            ),
                    )
                    .child(
                        // Right: Tab bar + Request editor and response viewer
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .flex_col()
                            .overflow_hidden() // Prevent content overflow
                            .bg(theme.background) // Capture mouse events, prevent passthrough
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation()) // Isolate scroll events
                            .child(
                                // Tab bar at the top
                                self.tab_bar.clone(),
                            )
                            .child(
                                // Request editor and response viewer with resizable splitter
                                div().flex_1().overflow_hidden().child(
                                    v_resizable("request-response-splitter")
                                        .child(
                                            resizable_panel()
                                                .size(px(350.)) // Request editor initial size
                                                .size_range(px(150.)..px(700.)) // Can resize between 150px-700px
                                                .child(self.request_editor.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_h(px(200.)) // Minimum height to prevent collapse
                                                .overflow_hidden() // Prevent content overflow
                                                .border_t_1()
                                                .border_color(theme.border)
                                                .child(self.response_viewer.clone())
                                                .into_any_element(),
                                        ),
                                ),
                            )
                            .into_any_element(),
                    ),
            )
    }
}
