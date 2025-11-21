use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    resizable::{resizable_panel, v_resizable},
};
use gpui::px;
use std::sync::Arc;

use crate::db::Database;
use crate::history_panel::{HistoryItemClicked, HistoryPanel};
use crate::request_editor::{RequestCompleted, RequestEditor};
use crate::response_viewer::ResponseViewer;

/// Main application view
pub struct PoopmanApp {
    #[allow(dead_code)]
    db: Arc<Database>,
    history_panel: Entity<HistoryPanel>,
    request_editor: Entity<RequestEditor>,
    response_viewer: Entity<ResponseViewer>,
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

        // Subscribe to request completion events
        let db_clone = db.clone();
        let history_panel_clone = history_panel.clone();
        let response_viewer_clone = response_viewer.clone();
        let request_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |_this, _, event: &RequestCompleted, window, cx| {
                // Save to database
                let request_headers =
                    serde_json::to_string(&event.request.headers).unwrap_or_default();
                let response_headers =
                    serde_json::to_string(&event.response.headers).unwrap_or_default();

                if let Err(e) = db_clone.insert_history(
                    event.request.method.as_str(),
                    &event.request.url,
                    &request_headers,
                    &event.request.body,
                    event.response.status,
                    Some(event.response.duration_ms),
                    Some(&response_headers),
                    Some(&event.response.body),
                ) {
                    log::error!("Failed to save history: {}", e);
                }

                // Update response viewer
                response_viewer_clone.update(cx, |viewer, cx| {
                    viewer.set_response(event.response.clone(), window, cx);
                });

                // Reload history panel
                history_panel_clone.update(cx, |panel, cx| {
                    panel.reload(window, cx);
                });
            },
        );

        // Subscribe to history item click events
        let request_editor_clone = request_editor.clone();
        let history_sub = cx.subscribe_in(
            &history_panel,
            window,
            move |_this, _, event: &HistoryItemClicked, window, cx| {
                request_editor_clone.update(cx, |editor, cx| {
                    editor.load_request(&event.item.request, window, cx);
                });
            },
        );

        Self {
            db,
            history_panel,
            request_editor,
            response_viewer,
            _subscriptions: vec![request_sub, history_sub],
        }
    }
}

impl Render for PoopmanApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(theme.background)
            .child(
                // Left: History panel - 1/4 width
                div()
                    .w(relative(0.25))
                    .h_full()
                    .border_r_1()
                    .border_color(theme.border)
                    .child(self.history_panel.clone()),
            )
            .child(
                // Right: Request editor and response viewer - 3/4 width with resizable splitter
                div()
                    .flex_1()
                    .h_full()
                    .child(
                        v_resizable("request-response-splitter")
                            .child(
                                resizable_panel()
                                    .size(px(400.)) // Request editor initial size
                                    .size_range(px(200.)..px(800.)) // Can resize between 200px-800px
                                    .child(self.request_editor.clone())
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .border_t_1()
                                    .border_color(theme.border)
                                    .child(self.response_viewer.clone())
                                    .into_any_element()
                            )
                    )
            )
    }
}
