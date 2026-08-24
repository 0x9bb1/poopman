use gpui::px;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, IndexPath, Root, Sizable as _, TitleBar, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    resizable::{h_resizable, resizable_panel, v_resizable},
    select::{Select, SelectState},
    v_flex,
};
use std::sync::Arc;

use crate::code_snippet_panel::CodeSnippetPanel;
use crate::collections_panel::{
    CollectionTarget, CollectionsChanged, CollectionsPanel, NewRequestRequested,
    SavedRequestClicked,
};
use crate::db::Database;
use crate::environment_manager::{
    EnvironmentManager, EnvironmentsChanged, environment_dialog_geometry,
};
use crate::history_panel::{HistoryItemClicked, HistoryPanel};
use crate::request_editor::{
    OpenCodeSnippet, RequestCancelled, RequestCompleted, RequestEditor, RequestStarted,
    ToggleRequestBookmarkRequested,
};
use crate::request_tab::RequestTab;
use crate::response_viewer::ResponseViewer;
use crate::tab_bar::{NewTabClicked, TabBar, TabClicked, TabCloseClicked};
use crate::theme::{
    REQUEST_INITIAL_HEIGHT, REQUEST_MAX, REQUEST_MIN, SIDEBAR_MAX, SIDEBAR_MIN, SIDEBAR_WIDTH,
};

actions!(
    poopman,
    [
        SendRequest,
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        FocusUrl,
        Quit
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    Collections,
    History,
}

/// Data loaded before the first window is opened. Startup disk and SQLite work
/// is performed on GPUI's background executor, so even schema migration cannot
/// stall an already-running UI event loop.
pub(crate) struct AppInitialState {
    db: Arc<Database>,
    environments: Vec<crate::types::Environment>,
    active_environment_id: Option<i64>,
    history: Vec<crate::types::HistoryItem>,
    collections: Vec<crate::types::Collection>,
}

impl AppInitialState {
    pub(crate) fn load() -> anyhow::Result<Self> {
        let db = Arc::new(Database::new()?);
        let environments = db.load_environments()?;
        let active_environment_id = db.get_active_environment_id()?;
        let history = db.load_recent_history(crate::history_panel::HISTORY_LIMIT)?;
        let collections = db.load_collections()?;
        Ok(Self {
            db,
            environments,
            active_environment_id,
            history,
            collections,
        })
    }
}

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
    collections_panel: Entity<CollectionsPanel>,
    sidebar_view: SidebarView,
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
    collections_reconcile_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl PoopmanApp {
    pub fn new(initial: AppInitialState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let AppInitialState {
            db,
            environments,
            active_environment_id,
            history,
            collections,
        } = initial;
        db.register_ui_thread();

        // Create components
        let request_editor = cx.new(|cx| RequestEditor::new(window, cx));
        let response_viewer = cx.new(|cx| ResponseViewer::new(window, cx));
        let history_panel = cx.new(|cx| HistoryPanel::new(db.clone(), history, window, cx));
        let collections_panel =
            cx.new(|cx| CollectionsPanel::new(db.clone(), collections, window, cx));
        let tab_bar = cx.new(|cx| TabBar::new(window, cx));
        let manager_environments = environments.clone();
        let env_manager = cx.new(|cx| {
            EnvironmentManager::new(
                db.clone(),
                manager_environments,
                active_environment_id,
                window,
                cx,
            )
        });
        let code_panel = cx.new(|cx| CodeSnippetPanel::new(window, cx));

        // Push the active environment's variables into the request editor.
        let initial_env_vars = Self::active_env_vars(&environments, active_environment_id);
        request_editor.update(cx, |editor, _| editor.set_env_vars(initial_env_vars));

        // Initialize with one empty tab
        let request_tabs = vec![RequestTab::new_empty(0)];
        let active_tab_index = 0;
        let next_tab_id = 1;
        let sidebar_view = SidebarView::Collections;

        // Capture stable ownership and the immutable editor snapshot before the
        // asynchronous request can complete.
        let request_started_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, _, event: &RequestStarted, window, cx| {
                let Some(tab_index) = this
                    .request_tabs
                    .iter()
                    .position(|tab| tab.id == event.tab_id)
                else {
                    return;
                };
                this.request_tabs[tab_index]
                    .begin_request(event.request_id, event.request.clone());
                if tab_index == this.active_tab_index
                    && this.response_viewer.read(cx).is_canceled()
                {
                    this.response_viewer.update(cx, |viewer, cx| {
                        viewer.clear_response(window, cx);
                    });
                }
                this.update_tab_bar(cx);
            },
        );

        // Subscribe to request completion events
        let db_clone = db.clone();
        let history_panel_clone = history_panel.clone();
        let request_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, _, event: &RequestCompleted, window, cx| {
                #[cfg(feature = "profile")]
                profiling::scope!("handle request completed");

                let Some(tab_index) = this
                    .request_tabs
                    .iter()
                    .position(|tab| tab.id == event.tab_id)
                else {
                    return;
                };
                if !this.request_tabs[tab_index]
                    .complete_request(event.request_id, event.response.clone())
                {
                    return;
                }

                // The response is stored on its originating tab. Paint the
                // shared viewer only if that tab is still active.
                if tab_index == this.active_tab_index {
                    this.response_viewer.update(cx, |viewer, cx| {
                        viewer.set_response(event.response.clone(), window, cx);
                    });
                }
                this.update_tab_bar(cx);

                // Postman behavior: every send is logged to History, including a
                // re-send of a request opened from history. Database::call is
                // synchronous by design, so run it only on GPUI's background
                // executor and refresh the panel after the write has completed.
                let db = db_clone.clone();
                let request = event.history_request.clone();
                let history_panel = history_panel_clone.clone();
                let persist = cx.background_spawn(async move { Self::persist_send(&db, &request) });
                cx.spawn_in(window, async move |_this, cx| {
                    match persist.await {
                        Ok(_) => {
                            history_panel
                                .update_in(cx, |panel, window, cx| panel.reload(window, cx))?;
                        }
                        Err(error) => log::error!("Failed to save history: {}", error),
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .detach();
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

        // Open a persisted collection request in a new tab (or reuse a blank
        // scratch tab), keeping collection metadata attached to the tab.
        let collections_sub = cx.subscribe_in(
            &collections_panel,
            window,
            move |this, _, event: &SavedRequestClicked, window, cx| {
                this.open_saved_request_in_new_tab(&event.request, window, cx);
            },
        );

        let new_collection_request_sub = cx.subscribe_in(
            &collections_panel,
            window,
            move |this, _, event: &NewRequestRequested, window, cx| {
                this.collections_panel.update(cx, |panel, cx| {
                    panel.select_target(&event.target, cx);
                });
                this.create_new_tab(window, cx);
            },
        );

        // Collection tree mutations can rename or remove rows while their
        // requests are open. Refresh tab metadata without replacing unsaved
        // editor contents.
        let collections_changed_sub = cx.subscribe_in(
            &collections_panel,
            window,
            move |this, _, event: &CollectionsChanged, window, cx| {
                this.reconcile_collection_tabs(&event.deleted_request_ids, window, cx);
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
            move |this, _, event: &EnvironmentsChanged, _window, cx| {
                this.apply_environment_state(event.environments.clone(), event.active_id, cx);
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
                this.code_panel
                    .update(cx, |panel, cx| panel.set_request(req, window, cx));
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
        let cancel_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, _, event: &RequestCancelled, window, cx| {
                let Some(tab_index) = this
                    .request_tabs
                    .iter()
                    .position(|tab| tab.id == event.tab_id)
                else {
                    return;
                };
                if !this.request_tabs[tab_index].cancel_request(event.request_id) {
                    return;
                }
                if tab_index == this.active_tab_index {
                    this.response_viewer.update(cx, |viewer, cx| {
                        viewer.show_canceled(window, cx);
                    });
                }
            },
        );

        let bookmark_toggle_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, _, _event: &ToggleRequestBookmarkRequested, window, cx| {
                this.toggle_request_bookmark(window, cx);
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
            collections_panel,
            sidebar_view,
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
            collections_reconcile_generation: 0,
            _subscriptions: vec![
                request_started_sub,
                request_sub,
                history_sub,
                collections_sub,
                new_collection_request_sub,
                collections_changed_sub,
                tab_clicked_sub,
                new_tab_sub,
                close_tab_sub,
                env_changed_sub,
                open_code_sub,
                cancel_sub,
                bookmark_toggle_sub,
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

    /// Append a completed send to History and return the new row id.
    ///
    /// Postman behavior: EVERY send is logged, including a re-send of a request
    /// opened from history. (Previously gated on `!is_from_history`, which
    /// silently dropped edits — e.g. added auth — made to a restored request.)
    /// Only the request is stored; response bodies are not.
    fn persist_send(db: &Database, request: &crate::types::RequestData) -> anyhow::Result<i64> {
        let request_headers = serde_json::to_string(&request.headers).unwrap_or_default();
        db.insert_history(
            request.method.as_str(),
            &request.url,
            &request_headers,
            &request.body,
            &request.auth,
        )
    }

    /// Apply the environment manager's in-memory state immediately. Persistence
    /// is asynchronous; reflecting an edit never requires a DB read-back.
    fn apply_environment_state(
        &mut self,
        environments: Vec<crate::types::Environment>,
        active_environment_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.environments = environments;
        self.active_environment_id = active_environment_id;
        let vars = Self::active_env_vars(&self.environments, self.active_environment_id);
        self.request_editor
            .update(cx, |editor, _| editor.set_env_vars(vars));
        cx.notify();
    }

    /// Open the environment management dialog.
    pub(crate) fn open_env_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let manager = self.env_manager.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let theme = cx.theme();
            let viewport = window.viewport_size();
            let geometry = environment_dialog_geometry(viewport.width, viewport.height);
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
                                .font_weight(gpui::FontWeight::NORMAL)
                                .text_color(theme.muted_foreground)
                                .child(
                                    "Create reusable values for URLs, headers, and request bodies.",
                                ),
                        ),
                )
                .w(geometry.width)
                .margin_top(geometry.margin_top)
                .max_h((viewport.height - px(32.)).max(px(400.)))
                .bg(theme.popover)
                .rounded(px(16.))
                .child(manager.clone())
        });
    }

    /// Switch the active environment (or clear it) from the Edit menu, then
    /// reload + refresh the request editor's variable map.
    pub(crate) fn set_active_environment(
        &mut self,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.env_manager.update(cx, |manager, cx| {
            manager.set_active(id, window, cx);
        });
    }

    /// Save current editor state to active tab
    fn save_current_tab_state(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.request_tabs.get_mut(self.active_tab_index) {
            let request_data = self.request_editor.read(cx).get_current_request_data(cx);
            let body_draft = self.request_editor.read(cx).get_body_draft(cx);
            let params_state = self.request_editor.read(cx).get_params_state(cx);
            let headers_state = self.request_editor.read(cx).get_headers_state(cx);
            let response = self.response_viewer.read(cx).get_response();
            let response_canceled = self.response_viewer.read(cx).is_canceled();

            tab.request = request_data;
            tab.body_draft = body_draft;
            tab.response = response;
            tab.response_canceled = response_canceled;
            tab.params_state = Some(params_state);
            tab.headers_state = Some(headers_state);
            tab.update_title_from_saved_name();
        }
    }

    /// Switch to a different tab
    #[cfg_attr(feature = "profile", profiling::function)]
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
            self.load_tab_into_editor(&tab, window, cx);

            self.load_tab_response(&tab, window, cx);
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
        self.load_tab_into_editor(&new_tab, window, cx);

        // Clear response for new tab
        self.response_viewer.update(cx, |viewer, cx| {
            viewer.clear_response(window, cx);
        });

        self.update_tab_bar(cx);
        cx.notify();
    }

    /// Close a tab
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closing_tab_id) = self.request_tabs.get(index).map(|tab| tab.id) else {
            return;
        };
        // Explicit policy: closing a tab aborts its in-flight request. The
        // editor removes ownership first, so its eventual task result is ignored.
        self.request_editor.update(cx, |editor, cx| {
            editor.close_request_tab(closing_tab_id, cx);
        });

        if self.request_tabs.len() <= 1 {
            // Don't close the last tab, just reset it to empty
            self.request_tabs[0] = RequestTab::new_empty(self.next_tab_id);
            self.next_tab_id += 1;
            self.active_tab_index = 0;

            let reset_tab = self.request_tabs[0].clone();
            self.load_tab_into_editor(&reset_tab, window, cx);

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
                self.load_tab_into_editor(&tab, window, cx);

                self.load_tab_response(&tab, window, cx);
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

        // If the active tab is a pristine scratch tab (e.g. the default tab at
        // startup), fill it in place instead of spawning a sibling.
        let new_tab = if self
            .request_tabs
            .get(self.active_tab_index)
            .is_some_and(RequestTab::is_blank)
        {
            let id = self.request_tabs[self.active_tab_index].id;
            let tab = RequestTab::from_history(id, item);
            self.request_tabs[self.active_tab_index] = tab.clone();
            tab
        } else {
            let tab = RequestTab::from_history(self.next_tab_id, item);
            self.next_tab_id += 1;
            self.request_tabs.push(tab.clone());
            self.active_tab_index = self.request_tabs.len() - 1;
            tab
        };

        // Load into editor
        self.load_tab_into_editor(&new_tab, window, cx);

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

    /// Open a saved collection request in a tab, reusing a pristine scratch tab
    /// when possible. The persisted row name remains the tab title even after
    /// the URL or method is edited.
    fn open_saved_request_in_new_tab(
        &mut self,
        saved: &crate::types::SavedRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing_index) = self
            .request_tabs
            .iter()
            .position(|tab| tab.saved_request_id == Some(saved.id))
        {
            self.switch_to_tab(existing_index, window, cx);
            return;
        }

        self.save_current_tab_state(cx);
        let new_tab = if self
            .request_tabs
            .get(self.active_tab_index)
            .is_some_and(RequestTab::is_blank)
        {
            let id = self.request_tabs[self.active_tab_index].id;
            let tab = RequestTab::from_saved_request(id, saved);
            self.request_tabs[self.active_tab_index] = tab.clone();
            tab
        } else {
            let tab = RequestTab::from_saved_request(self.next_tab_id, saved);
            self.next_tab_id += 1;
            self.request_tabs.push(tab.clone());
            self.active_tab_index = self.request_tabs.len() - 1;
            tab
        };

        self.load_tab_into_editor(&new_tab, window, cx);
        self.response_viewer.update(cx, |viewer, cx| {
            viewer.clear_response(window, cx);
        });
        self.update_tab_bar(cx);
        cx.notify();
    }

    #[cfg_attr(feature = "profile", profiling::function)]
    fn load_tab_into_editor(
        &mut self,
        tab: &RequestTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_editor.update(cx, |editor, cx| {
            editor.set_active_request_tab(tab.id, cx);
            editor.load_request_with_body_draft(
                &tab.request,
                Some(&tab.body_draft),
                window,
                cx,
            );
            editor.set_is_saved_request(tab.saved_request_id.is_some(), cx);
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
    }

    fn load_tab_response(
        &mut self,
        tab: &RequestTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.response_viewer.update(cx, |viewer, cx| {
            if tab.response_canceled {
                viewer.show_canceled(window, cx);
            } else if let Some(response) = &tab.response {
                viewer.set_response(response.clone(), window, cx);
            } else {
                viewer.clear_response(window, cx);
            }
        });
    }

    /// Keep tab metadata synchronized with collection-side rename/delete
    /// actions, while deliberately leaving the current editor values alone.
    fn reconcile_collection_tabs(
        &mut self,
        deleted_ids: &[i64],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.collections_reconcile_generation =
            self.collections_reconcile_generation.wrapping_add(1);
        let generation = self.collections_reconcile_generation;
        for tab in &mut self.request_tabs {
            let Some(saved_id) = tab.saved_request_id else {
                continue;
            };
            if deleted_ids.contains(&saved_id) {
                tab.saved_request_id = None;
                tab.collection_id = None;
                tab.folder_id = None;
                tab.saved_name = None;
                tab.update_title();
            }
        }
        self.refresh_saved_tab_ui(cx);

        let mut saved_ids = self
            .request_tabs
            .iter()
            .filter_map(|tab| tab.saved_request_id)
            .collect::<Vec<_>>();
        saved_ids.sort_unstable();
        saved_ids.dedup();
        if saved_ids.is_empty() {
            return;
        }
        let db = self.db.clone();
        let task = cx.background_spawn(async move {
            saved_ids
                .into_iter()
                .map(|id| db.load_saved_request(id).map(|saved| (id, saved)))
                .collect::<anyhow::Result<Vec<_>>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(saved_rows) => {
                    this.update(cx, |this, cx| {
                        if this.collections_reconcile_generation != generation {
                            return;
                        }
                        for (saved_id, saved) in saved_rows {
                            for tab in this
                                .request_tabs
                                .iter_mut()
                                .filter(|tab| tab.saved_request_id == Some(saved_id))
                            {
                                if let Some(saved) = &saved {
                                    tab.collection_id = Some(saved.collection_id);
                                    tab.folder_id = saved.folder_id;
                                    tab.saved_name = Some(saved.name.clone());
                                    tab.update_title_from_saved_name();
                                } else {
                                    tab.saved_request_id = None;
                                    tab.collection_id = None;
                                    tab.folder_id = None;
                                    tab.saved_name = None;
                                    tab.update_title();
                                }
                            }
                        }
                        this.refresh_saved_tab_ui(cx);
                    })?;
                }
                Err(error) => {
                    log::error!("Failed to refresh saved request metadata: {}", error)
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn refresh_saved_tab_ui(&mut self, cx: &mut Context<Self>) {
        let active_is_saved = self
            .request_tabs
            .get(self.active_tab_index)
            .is_some_and(|tab| tab.saved_request_id.is_some());
        self.request_editor.update(cx, |editor, cx| {
            editor.set_is_saved_request(active_is_saved, cx);
        });
        self.update_tab_bar(cx);
        cx.notify();
    }

    fn toggle_request_bookmark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_current_tab_state(cx);
        let Some(tab) = self.request_tabs.get(self.active_tab_index).cloned() else {
            return;
        };
        let params_state = tab.params_state.clone().unwrap_or_default();
        let headers_state = tab.headers_state.clone().unwrap_or_default();

        if let Some(saved_id) = tab.saved_request_id {
            let db = self.db.clone();
            let task = cx.background_spawn(async move { db.delete_saved_request(saved_id) });
            cx.spawn_in(window, async move |this, cx| {
                match task.await {
                    Ok(()) => {
                        this.update_in(cx, |this, window, cx| {
                            for tab in &mut this.request_tabs {
                                if tab.saved_request_id == Some(saved_id) {
                                    tab.saved_request_id = None;
                                    tab.collection_id = None;
                                    tab.folder_id = None;
                                    tab.saved_name = None;
                                    tab.update_title();
                                }
                            }
                            this.refresh_saved_tab_ui(cx);
                            this.collections_panel
                                .update(cx, |panel, cx| panel.reload(window, cx));
                        })?;
                    }
                    Err(error) => {
                        cx.update(|window, cx| {
                            app_notice(window, cx, "Remove bookmark failed", error.to_string())
                        })?;
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .detach();
            return;
        }

        let targets = self.collections_panel.read(cx).request_targets();
        if targets.is_empty() {
            let db = self.db.clone();
            let selected = self.collections_panel.read(cx).selected_target();
            let initial_name = if tab.title.trim().is_empty() || tab.title == "New Request" {
                format!("{} request", tab.request.method.as_str())
            } else {
                tab.title.clone()
            };
            let task = cx.background_spawn(async move { db.create_collection("My Collection") });
            cx.spawn_in(window, async move |this, cx| {
                match task.await {
                    Ok(collection_id) => {
                        this.update_in(cx, |this, window, cx| {
                            this.collections_panel
                                .update(cx, |panel, cx| panel.reload(window, cx));
                            this.open_new_save_dialog(
                                tab.id,
                                initial_name,
                                tab.request,
                                params_state,
                                headers_state,
                                vec![CollectionTarget {
                                    collection_id,
                                    folder_id: None,
                                    label: "My Collection".to_string(),
                                }],
                                selected,
                                window,
                                cx,
                            );
                        })?;
                    }
                    Err(error) => {
                        cx.update(|window, cx| {
                            app_notice(window, cx, "Save failed", error.to_string())
                        })?;
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .detach();
            return;
        }
        let selected = self.collections_panel.read(cx).selected_target();
        let initial_name = if tab.title.trim().is_empty() || tab.title == "New Request" {
            format!("{} request", tab.request.method.as_str())
        } else {
            tab.title.clone()
        };
        self.open_new_save_dialog(
            tab.id,
            initial_name,
            tab.request,
            params_state,
            headers_state,
            targets,
            selected,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn open_new_save_dialog(
        &mut self,
        tab_id: usize,
        initial_name: String,
        request: crate::types::RequestData,
        params_state: Vec<crate::types::ParamState>,
        headers_state: Vec<crate::types::HeaderState>,
        targets: Vec<CollectionTarget>,
        selected: Option<CollectionTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Request name");
            input.set_value(&initial_name, window, cx);
            input
        });
        let labels = targets
            .iter()
            .map(|target| target.label.clone())
            .collect::<Vec<_>>();
        let selected_index = selected
            .as_ref()
            .and_then(|selected| targets.iter().position(|target| target == selected))
            .unwrap_or(0);
        let target_select = cx.new(|cx| {
            SelectState::new(
                labels,
                Some(IndexPath::default().row(selected_index)),
                window,
                cx,
            )
        });
        // Dialogs are absolutely positioned by gpui-component. Without a
        // viewport-relative bound, the fixed 520px dialog can extend past a
        // compact window and its footer gets clipped by the window edge.
        let viewport = window.viewport_size();
        let max_dialog_width = (viewport.width - px(32.)).max(px(360.));
        let max_dialog_height = (viewport.height - px(32.)).max(px(220.));
        let app = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let name_input_for_ok = name_input.clone();
            let target_select_for_ok = target_select.clone();
            let targets_for_ok = targets.clone();
            let request_for_ok = request.clone();
            let params_for_ok = params_state.clone();
            let headers_for_ok = headers_state.clone();
            let app = app.clone();
            dialog
                .title(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("Save request"),
                )
                .w(px(520.))
                .max_w(max_dialog_width)
                .max_h(max_dialog_height)
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().child("Request name"))
                                .child(Input::new(&name_input)),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().child("Collection / folder"))
                                .child(Select::new(&target_select)),
                        ),
                )
                .confirm()
                .on_ok(move |_, window, cx: &mut App| {
                    let name = name_input_for_ok.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    let index = target_select_for_ok
                        .read(cx)
                        .selected_index(cx)
                        .map(|index| index.row)
                        .unwrap_or(0);
                    let Some(target) = targets_for_ok.get(index).cloned() else {
                        return false;
                    };
                    app.update(cx, |app, cx| {
                        app.persist_new_saved_request(
                            tab_id,
                            &name,
                            target,
                            &request_for_ok,
                            &params_for_ok,
                            &headers_for_ok,
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_new_saved_request(
        &mut self,
        tab_id: usize,
        name: &str,
        target: CollectionTarget,
        request: &crate::types::RequestData,
        params_state: &[crate::types::ParamState],
        headers_state: &[crate::types::HeaderState],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.db.clone();
        let collection_id = target.collection_id;
        let folder_id = target.folder_id;
        let name = name.to_string();
        let request = request.clone();
        let params_state = params_state.to_vec();
        let headers_state = headers_state.to_vec();
        let name_for_db = name.clone();
        let task = cx.background_spawn(async move {
            db.insert_saved_request(
                collection_id,
                folder_id,
                &name_for_db,
                &request,
                &params_state,
                &headers_state,
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(id) => {
                    this.update_in(cx, |this, window, cx| {
                        if let Some(tab) = this.request_tabs.iter_mut().find(|tab| tab.id == tab_id)
                        {
                            tab.saved_request_id = Some(id);
                            tab.collection_id = Some(collection_id);
                            tab.folder_id = folder_id;
                            tab.saved_name = Some(name);
                            tab.update_title_from_saved_name();
                        }
                        this.refresh_saved_tab_ui(cx);
                        this.collections_panel
                            .update(cx, |panel, cx| panel.reload(window, cx));
                    })?;
                }
                Err(error) => {
                    cx.update(|window, cx| {
                        app_notice(window, cx, "Save failed", error.to_string())
                    })?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    /// Update tab bar with current tabs
    #[cfg_attr(feature = "profile", profiling::function)]
    fn update_tab_bar(&mut self, cx: &mut Context<Self>) {
        self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.update_tabs(self.request_tabs.clone(), self.active_tab_index, cx);
            cx.notify();
        });
    }
}

impl Render for PoopmanApp {
    #[cfg_attr(feature = "profile", profiling::function)]
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
                                            .child(
                                                v_flex()
                                                    .size_full()
                                                    .child(
                                                    div()
                                                        .w_full()
                                                        .border_b_1()
                                                        .border_color(theme.border)
                                                        .child(
                                                            h_flex()
                                                                .w_full()
                                                                .gap_1()
                                                                .p_2()
                                                                .child(
                                                                    sidebar_switch(
                                                                        "Collections",
                                                                        self.sidebar_view == SidebarView::Collections,
                                                                        cx.listener(|this, _, _, cx| {
                                                                            this.sidebar_view = SidebarView::Collections;
                                                                            cx.notify();
                                                                        }),
                                                                    ),
                                                                )
                                                                .child(
                                                                    sidebar_switch(
                                                                        "History",
                                                                        self.sidebar_view == SidebarView::History,
                                                                        cx.listener(|this, _, _, cx| {
                                                                            this.sidebar_view = SidebarView::History;
                                                                            cx.notify();
                                                                        }),
                                                                    ),
                                                                ),
                                                        ),
                                                    )
                                                    .child(if self.sidebar_view == SidebarView::Collections {
                                                        div()
                                                            .flex_1()
                                                            .min_h_0()
                                                            .min_w_0()
                                                            .child(self.collections_panel.clone())
                                                            .into_any_element()
                                                    } else {
                                                        div()
                                                            .flex_1()
                                                            .min_h_0()
                                                            .min_w_0()
                                                            .child(self.history_panel.clone())
                                                            .into_any_element()
                                                    }),
                                            ),
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
                                        // w_full keeps this host's width definite, so the
                                        // width:100% that ResizablePanelGroup and
                                        // ResizablePanel both size themselves with has
                                        // something to resolve against.
                                        div().flex_1().w_full().overflow_hidden().child(
                                            v_resizable("request-response-splitter")
                                                .child(
                                                    resizable_panel()
                                                        .size(px(REQUEST_INITIAL_HEIGHT))
                                                        .size_range(px(REQUEST_MIN)..px(REQUEST_MAX))
                                                        .child(
                                                            // flex_1 rather than size_full: the
                                                            // panel is a flex ROW, so this is the
                                                            // main axis. size_full asks for
                                                            // width:100%, which only fills if that
                                                            // percentage resolves; flex-grow fills
                                                            // unconditionally. The response card
                                                            // below has always used flex_1 and has
                                                            // never collapsed, while this one has.
                                                            crate::ui::card_panel(theme)
                                                                .flex_1()
                                                                .h_full()
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

fn sidebar_switch<F>(label: &'static str, active: bool, on_click: F) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let button = Button::new(label).small().label(label).on_click(on_click);
    if active {
        button.primary()
    } else {
        button.ghost()
    }
}

fn app_notice(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<String>,
    message: impl Into<String>,
) {
    let title = title.into();
    let message = message.into();
    window.open_dialog(cx, move |dialog, _window, cx| {
        dialog
            .title(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(title.clone()),
            )
            .w(px(520.))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(message.clone()),
            )
            .alert()
    });
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

    // Postman behavior: EVERY send is logged to History, including a re-send of a
    // request opened from history. A previous `!is_from_history` gate silently
    // dropped edits (e.g. added auth) made to a restored request.
    #[test]
    fn every_send_appends_history_including_a_resend() {
        use super::PoopmanApp;
        use crate::db::Database;
        use crate::types::{AuthConfig, AuthType, HttpMethod, RequestData};

        let db = Database::new_in_memory();

        // First send: a fresh request, no auth.
        let original = RequestData::new(HttpMethod::GET, "https://api.test/x".to_string());
        PoopmanApp::persist_send(&db, &original).unwrap();

        // Same request re-opened from history, edited to add Bearer auth, re-sent.
        let mut edited = original.clone();
        edited.auth = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "t0ken".into(),
            ..Default::default()
        };
        PoopmanApp::persist_send(&db, &edited).unwrap();

        let items = db.load_recent_history(10).unwrap();
        assert_eq!(items.len(), 2, "each send must append its own history row");
        // Newest first: the edited re-send carries the Bearer auth...
        assert_eq!(items[0].request.auth.auth_type, AuthType::Bearer);
        assert_eq!(items[0].request.auth.bearer_token, "t0ken");
        // ...and the original row is untouched.
        assert_eq!(items[1].request.auth.auth_type, AuthType::None);
    }
}
