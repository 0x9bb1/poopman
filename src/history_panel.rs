use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::*, h_flex, Icon, scroll::ScrollbarAxis, v_flex, ActiveTheme as _, Sizable as _,
    StyledExt as _,
};
use std::sync::Arc;

use crate::db::Database;
use crate::types::HistoryItem;

/// Event emitted when a history item is clicked
#[derive(Clone)]
pub struct HistoryItemClicked {
    pub item: HistoryItem,
}

/// History panel component
pub struct HistoryPanel {
    db: Arc<Database>,
    history: Vec<HistoryItem>,
    selected_id: Option<i64>,
}

impl HistoryPanel {
    pub fn new(db: Arc<Database>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        // Load initial history from database
        let history = db.load_recent_history(100).unwrap_or_default();

        Self {
            db,
            history,
            selected_id: None,
        }
    }

    /// Reload history from database
    pub fn reload(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.history = self.db.load_recent_history(100).unwrap_or_default();
        cx.notify();
    }

    fn format_relative_time(timestamp: &str) -> String {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp) {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt);

            if duration.num_seconds() < 60 {
                "just now".to_string()
            } else if duration.num_minutes() < 60 {
                format!("{} min ago", duration.num_minutes())
            } else if duration.num_hours() < 24 {
                format!("{} hours ago", duration.num_hours())
            } else {
                format!("{} days ago", duration.num_days())
            }
        } else {
            timestamp.to_string()
        }
    }

    fn on_item_click(&mut self, item: &HistoryItem, _window: &mut Window, cx: &mut Context<Self>) {
        self.selected_id = Some(item.id);
        cx.emit(HistoryItemClicked { item: item.clone() });
        cx.notify();
    }

    fn clear_history(
        &mut self,
        _event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.db.clear_all_history() {
            log::error!("Failed to clear history: {}", e);
            return;
        }

        self.history.clear();
        self.selected_id = None;
        cx.notify();
    }
}

impl EventEmitter<HistoryItemClicked> for HistoryPanel {}

impl Render for HistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                // Header
                h_flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::default()
                                    .path("icons/logo.svg")
                                    .size_5()
                            )
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("History"))
                    )
                    .child(
                        Button::new("clear-btn")
                            .xsmall()
                            .ghost()
                            .label("Clear")
                            .on_click(cx.listener(Self::clear_history)),
                    ),
            )
            .when(self.history.is_empty(), |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .text_sm()
                        .child("No history yet\n\nSend a request to get started"),
                )
            })
            .when(!self.history.is_empty(), |this| {
                this.child(
                    // List - use size_full to fill available space
                    v_flex()
                        .size_full()
                        .gap_2()
                        .p_2()
                        .children(self.history.iter().map(|item| {
                            let item_id = item.id;
                            let is_selected = self.selected_id == Some(item_id);
                            let method = item.request.method.as_str();
                            let url = &item.request.url;
                            let time = Self::format_relative_time(&item.timestamp);

                            let method_color = match method {
                                "GET" => theme.success,
                                "POST" => theme.accent,
                                "PUT" => theme.warning,
                                "DELETE" => theme.danger,
                                _ => theme.muted_foreground,
                            };

                            let status_color = if let Some(response) = &item.response {
                                if response.is_success() {
                                    Some(theme.success)
                                } else if response.is_error() {
                                    Some(theme.danger)
                                } else {
                                    Some(theme.accent)
                                }
                            } else {
                                None
                            };

                            let item_clone = item.clone();

                            div()
                                .id(("history-item", item_id as u64))
                                .w_full()
                                .px_3()
                                .py_2()
                                .bg(if is_selected {
                                    theme.list_active
                                } else {
                                    theme.list
                                })
                                .border_1()
                                .border_color(if is_selected {
                                    theme.list_active_border
                                } else {
                                    theme.border
                                })
                                .rounded(theme.radius)
                                .cursor_pointer()
                                .on_click(cx.listener(
                                    move |this, _event: &gpui::ClickEvent, window, cx| {
                                        this.on_item_click(&item_clone, window, cx);
                                    },
                                ))
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .px_2()
                                                        .py_1()
                                                        .rounded(theme.radius)
                                                        .bg(method_color)
                                                        .text_color(gpui::white())
                                                        .text_xs()
                                                        .flex_shrink_0()
                                                        .child(method),
                                                )
                                                .when_some(status_color, |this, color| {
                                                    this.child(
                                                        div()
                                                            .px_2()
                                                            .py_1()
                                                            .rounded(theme.radius)
                                                            .bg(color)
                                                            .text_color(gpui::white())
                                                            .text_xs()
                                                            .flex_shrink_0()
                                                                                                                          .child(
                                                                                                                            item.response
                                                                                                                                .as_ref()
                                                                                                                                .map(|r| r.status_text())
                                                                                                                                .unwrap_or("NET ERR"),
                                                                                                                        ),                                                    )
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(theme.foreground)
                                                .overflow_x_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .child(url.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(time),
                                        ),
                                )
                        }))
                        .scrollable(ScrollbarAxis::Vertical)
                        .size_full(),
                )
            })
    }
}
