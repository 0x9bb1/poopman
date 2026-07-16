use gpui::*;
use gpui_component::{
    h_flex, v_flex, ActiveTheme as _, Root, TitleBar, WindowExt,
    resizable::{h_resizable, resizable_panel, v_resizable},
};
use gpui::px;
use std::sync::Arc;

use crate::code_snippet_panel::CodeSnippetPanel;
use crate::db::Database;
use crate::environment_manager::{EnvironmentManager, EnvironmentsChanged};
use crate::history_panel::{HistoryItemClicked, HistoryPanel};
use crate::request_editor::{OpenCodeSnippet, RequestCancelled, RequestCompleted, RequestEditor};
use crate::request_tab::RequestTab;
use crate::response_viewer::ResponseViewer;
use crate::tab_bar::{NewTabClicked, TabBar, TabClicked, TabCloseClicked};
use crate::theme::{
    REQUEST_INITIAL_HEIGHT, REQUEST_MAX, REQUEST_MIN, SIDEBAR_MAX, SIDEBAR_MIN, SIDEBAR_WIDTH,
};

actions!(poopman, [SendRequest, NewTab, CloseTab, NextTab, PrevTab, FocusUrl]);

/// Main application view
pub struct PoopmanApp {
    /// Focused at startup so the window's focus is never `None`.
    ///
    /// This is load-bearing for every keyboard shortcut, not a nicety.
    /// `Window::dispatch_key_event` (`gpui-0.2.2/src/window.rs:3735`) resolves the
    /// dispatch path from the focused node, and `focus_node_id_in_rendered_frame`
    /// falls back to the dispatch tree's *root* when focus is `None`. The path is
    /// then just that root — and our `on_action` handlers live on `PoopmanApp`'s own
    /// element, a descendant of it, so with no focus they are never reached and
    /// every shortcut silently does nothing.
    ///
    /// Tracked on the content area rather than the root — see the note at the
    /// `track_focus` call in `render`. Moving it back up kills the window controls.
    focus_handle: FocusHandle,
    db: Arc<Database>,
    history_panel: Entity<HistoryPanel>,
    request_editor: Entity<RequestEditor>,
    response_viewer: Entity<ResponseViewer>,
    tab_bar: Entity<TabBar>,
    request_tabs: Vec<RequestTab>,
    active_tab_index: usize,
    next_tab_id: usize,
    environments: Vec<crate::types::Environment>,
    active_environment_id: Option<i64>,
    env_manager: Entity<EnvironmentManager>,
    code_panel: Entity<CodeSnippetPanel>,
    _subscriptions: Vec<Subscription>,
}

impl PoopmanApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Initialize database
        let db = Arc::new(Database::new().expect("Failed to initialize database"));

        // Load environments + active selection
        let environments = db.load_environments().unwrap_or_default();
        let active_environment_id = db.get_active_environment_id().unwrap_or(None);

        // Create components
        let request_editor = cx.new(|cx| RequestEditor::new(window, cx));
        let response_viewer = cx.new(|cx| ResponseViewer::new(window, cx));
        let history_panel = cx.new(|cx| HistoryPanel::new(db.clone(), window, cx));
        let tab_bar = cx.new(|cx| TabBar::new(window, cx));
        let env_manager = cx.new(|cx| EnvironmentManager::new(db.clone(), window, cx));
        let code_panel = cx.new(|cx| CodeSnippetPanel::new(window, cx));

        // Push the active environment's variables into the request editor.
        let initial_env_vars = Self::active_env_vars(&environments, active_environment_id);
        request_editor.update(cx, |editor, _| editor.set_env_vars(initial_env_vars));

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

        // Reload environments + refresh editor vars whenever the manager changes them.
        let env_changed_sub = cx.subscribe_in(
            &env_manager,
            window,
            move |this, _, _e: &EnvironmentsChanged, _window, cx| {
                this.reload_environments(cx);
            },
        );

        // Open the code-snippet dialog when the request editor's </> button asks for
        // it; feed the panel the current request (env vars resolved) then show it.
        let code_panel_for_sub = code_panel.clone();
        let open_code_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, editor, _e: &OpenCodeSnippet, window, cx| {
                let req = editor.read(cx).resolved_request_data(cx);
                this.code_panel.update(cx, |panel, cx| panel.set_request(req, window, cx));
                let panel = code_panel_for_sub.clone();
                window.open_dialog(cx, move |dialog, _window, cx| {
                    let theme = cx.theme();
                    dialog
                        .title(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(theme.foreground)
                                .child("Code snippet"),
                        )
                        .w(px(760.))
                        .child(panel.clone())
                });
            },
        );

        // Show the canceled notice when the user aborts an in-flight request.
        // Canceled requests are never written to history (same as Postman).
        let response_viewer_for_cancel = response_viewer.clone();
        let cancel_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |_this, _, _e: &RequestCancelled, window, cx| {
                response_viewer_for_cancel.update(cx, |viewer, cx| {
                    viewer.show_canceled(window, cx);
                });
            },
        );

        // Push the initial tab into the tab bar so the first request shows as a
        // tab immediately (the TabBar entity starts empty; without this the bar
        // would show only the "+" until the first tab action).
        tab_bar.update(cx, |bar, cx| {
            bar.update_tabs(request_tabs.clone(), active_tab_index, cx);
        });

        // Focus the root so the window's focus is never `None` — see the field's
        // doc comment. Without this, shortcuts are dead until the user happens to
        // click something focusable.
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        Self {
            focus_handle,
            db,
            history_panel,
            request_editor,
            response_viewer,
            tab_bar,
            request_tabs,
            active_tab_index,
            next_tab_id,
            environments,
            active_environment_id,
            env_manager,
            code_panel,
            _subscriptions: vec![
                request_sub,
                history_sub,
                tab_clicked_sub,
                new_tab_sub,
                close_tab_sub,
                env_changed_sub,
                open_code_sub,
                cancel_sub,
            ],
        }
    }

    /// Build the active environment's enabled variables as a flat map.
    fn active_env_vars(
        environments: &[crate::types::Environment],
        active_id: Option<i64>,
    ) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if let Some(id) = active_id
            && let Some(env) = environments.iter().find(|e| e.id == id)
        {
            for v in &env.variables {
                if v.enabled && !v.key.is_empty() {
                    map.insert(v.key.clone(), v.value.clone());
                }
            }
        }
        map
    }

    /// Reload environments + active selection from the DB and push the active
    /// variable map to the request editor.
    fn reload_environments(&mut self, cx: &mut Context<Self>) {
        self.environments = self.db.load_environments().unwrap_or_default();
        self.active_environment_id = self.db.get_active_environment_id().unwrap_or(None);
        let vars = Self::active_env_vars(&self.environments, self.active_environment_id);
        self.request_editor.update(cx, |editor, _| editor.set_env_vars(vars));
        cx.notify();
    }

    /// Open the environment management dialog.
    pub(crate) fn open_env_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let manager = self.env_manager.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let theme = cx.theme();
            dialog
                .title(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(theme.foreground)
                                .child("Environments"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Define variables like {{base_url}} per environment"),
                        ),
                )
                .w(px(680.))
                .child(manager.clone())
        });
    }

    /// Switch the active environment (or clear it) from the Edit menu, then
    /// reload + refresh the request editor's variable map.
    pub(crate) fn set_active_environment(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        if let Err(e) = self.db.set_active_environment_id(id) {
            log::error!("Failed to set active environment: {}", e);
            return;
        }
        self.reload_environments(cx);
        self.env_manager.update(cx, |mgr, cx| {
            mgr.reload();
            cx.notify();
        });
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
                if let Some(params_state) = &tab.params_state
                    && !params_state.is_empty()
                {
                    editor.load_params_state(params_state, window, cx);
                }

                if let Some(headers_state) = &tab.headers_state
                    && !headers_state.is_empty()
                {
                    editor.load_headers_state(headers_state, window, cx);
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .key_context("Poopman")
            .on_action(cx.listener(|this, _: &SendRequest, window, cx| {
                this.request_editor.update(cx, |editor, cx| editor.send(window, cx));
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.create_new_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let index = this.active_tab_index;
                this.close_tab(index, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                let next = cycle_index(this.active_tab_index, this.request_tabs.len(), true);
                this.switch_to_tab(next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                let prev = cycle_index(this.active_tab_index, this.request_tabs.len(), false);
                this.switch_to_tab(prev, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusUrl, window, cx| {
                this.request_editor.update(cx, |editor, cx| editor.focus_url(window, cx));
            }))
            .size_full()
            .bg(theme.muted)
            .child(
                // Custom warm title bar (replaces the white native title bar).
                // Brand + Edit menu are grouped in one child so the TitleBar's
                // justify_between row keeps them together at the left (otherwise
                // two children get pushed to opposite ends).
                TitleBar::new().child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child("Poopman"),
                        )
                        .child(crate::menu_bar::edit_menu(
                            cx.entity(),
                            self.environments.clone(),
                            self.active_environment_id,
                        )),
                ),
            )
            .child(
                div()
                    // Focus lives here, on the content area, and deliberately NOT on the
                    // root — the root spans the title bar too, and that breaks the window
                    // controls. `track_focus` makes an element insert a hitbox
                    // (`div.rs:1699`) and registers a focus-on-mouse-down listener that
                    // calls `window.prevent_default()` (`div.rs:2035`). Windows delivers
                    // WM_NCLBUTTONDOWN on minimize/maximize/close through gpui as an
                    // ordinary MouseDownEvent first, and treats it as consumed when the
                    // default was prevented (`platform/windows/events.rs:976`) — so it
                    // returns early and never records `nc_button_pressed`, leaving the
                    // matching mouse-up with nothing to act on. All three buttons go dead
                    // while still painting their hover styles.
                    //
                    // The actions stay on the root: dispatch walks the whole focus path,
                    // and the root is still an ancestor of this element.
                    .track_focus(&self.focus_handle)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p_3()
                    .child(
                        h_resizable("history-main-splitter")
                            .child(
                                // Left: History panel with resizable width
                                resizable_panel()
                                    .size(px(SIDEBAR_WIDTH))
                                    .size_range(px(SIDEBAR_MIN)..px(SIDEBAR_MAX))
                                    .child(
                                        crate::ui::card_panel(theme)
                                            .size_full()
                                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                            .child(self.history_panel.clone()),
                                    ),
                            )
                            .child(
                                // Right: Tab bar + Request editor and response viewer.
                                // gap = space between the tab-bar card and the
                                // request/response area; ml = gap from the sidebar
                                // card (the resizable handle itself is only 1px).
                                div()
                                    .flex_1()
                                    .h_full()
                                    .flex()
                                    .flex_col()
                                    .gap(px(10.))
                                    .ml(px(10.))
                                    .overflow_hidden() // Prevent content overflow
                                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation()) // Isolate scroll events
                                    .child(
                                        // Tab bar card (its own floating row)
                                        crate::ui::card_panel(theme).child(
                                            h_flex()
                                                .w_full()
                                                .child(div().flex_1().min_w_0().child(self.tab_bar.clone())),
                                        ),
                                    )
                                    .child(
                                        // Request editor and response viewer with resizable splitter
                                        div().flex_1().overflow_hidden().child(
                                            v_resizable("request-response-splitter")
                                                .child(
                                                    resizable_panel()
                                                        .size(px(REQUEST_INITIAL_HEIGHT))
                                                        .size_range(px(REQUEST_MIN)..px(REQUEST_MAX))
                                                        .child(
                                                            crate::ui::card_panel(theme)
                                                                .size_full()
                                                                .child(self.request_editor.clone()),
                                                        ),
                                                )
                                                .child(
                                                    // mt = gap from the request card
                                                    // (the v_resizable handle is only 1px).
                                                    crate::ui::card_panel(theme)
                                                        .flex_1()
                                                        .min_h(px(200.))
                                                        .mt(px(10.))
                                                        .child(self.response_viewer.clone())
                                                        .into_any_element(),
                                                ),
                                        ),
                                    )
                                    .into_any_element(),
                            ),
                    ),
            )
            // gpui-component dialogs/modals are stored on Root but must be rendered
            // by the app's root view; embed the dialog overlay here.
            .children(Root::render_dialog_layer(window, cx))
    }
}

/// Next (`forward`) or previous tab index, wrapping at both ends.
///
/// Returns `current` unchanged when `len` is 0 or 1 — callers then hit
/// `switch_to_tab`'s `index == self.active_tab_index` early-return and no-op.
fn cycle_index(current: usize, len: usize, forward: bool) -> usize {
    if len <= 1 {
        return current;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

#[cfg(test)]
mod tests {
    // NOT `use super::*`: that would pull in `gpui::*`, whose `test` attribute
    // macro shadows the standard `#[test]`.
    use super::cycle_index;

    #[test]
    fn steps_forward_through_the_middle_of_the_list() {
        assert_eq!(cycle_index(0, 3, true), 1);
        assert_eq!(cycle_index(1, 3, true), 2);
    }

    #[test]
    fn wraps_forward_past_the_last_tab() {
        assert_eq!(cycle_index(2, 3, true), 0);
    }

    #[test]
    fn steps_backward_through_the_middle_of_the_list() {
        assert_eq!(cycle_index(2, 3, false), 1);
        assert_eq!(cycle_index(1, 3, false), 0);
    }

    #[test]
    fn wraps_backward_past_the_first_tab() {
        assert_eq!(cycle_index(0, 3, false), 2);
    }

    #[test]
    fn single_tab_stays_put_in_both_directions() {
        assert_eq!(cycle_index(0, 1, true), 0);
        assert_eq!(cycle_index(0, 1, false), 0);
    }

    #[test]
    fn empty_list_returns_current_without_panicking() {
        assert_eq!(cycle_index(0, 0, true), 0);
        assert_eq!(cycle_index(0, 0, false), 0);
    }
}
