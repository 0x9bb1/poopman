use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::*,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use std::sync::Arc;
use std::time::Duration;

use crate::db::Database;
use crate::types::HistoryItem;

/// Maximum number of history rows loaded/searched at a time.
pub(crate) const HISTORY_LIMIT: usize = 100;

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
    search: Entity<InputState>,
    query: String,
    /// Invalidates stale search/reload completions. Database work is serialized,
    /// but foreground tasks may resume after a newer query has been entered.
    refresh_generation: u64,
    list_scroll_handle: ScrollHandle,
}

impl HistoryPanel {
    pub fn new(
        db: Arc<Database>,
        history: Vec<HistoryItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search history"));
        cx.subscribe_in(&search, window, Self::on_search_change)
            .detach();

        Self {
            db,
            history,
            selected_id: None,
            search,
            query: String::new(),
            refresh_generation: 0,
            list_scroll_handle: ScrollHandle::new(),
        }
    }

    /// Re-query without ever waiting for SQLite on the UI thread.
    ///
    /// Search changes are coalesced for a short interval, matching the pattern
    /// used by Telegram Desktop for delayed storage writes: rapid UI changes
    /// produce one useful database operation instead of a queue of stale work.
    fn refresh_list(&mut self, debounce: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        let generation = self.refresh_generation;
        let query = self.query.trim().to_string();
        let db = self.db.clone();

        cx.spawn_in(window, async move |this, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let is_current = this
                    .update(cx, |this, _| this.refresh_generation == generation)
                    .unwrap_or(false);
                if !is_current {
                    return Ok(());
                }
            }

            let query_for_db = query.clone();
            let task = cx.background_spawn(async move {
                if query_for_db.is_empty() {
                    db.load_recent_history(HISTORY_LIMIT)
                } else {
                    db.search_history(&query_for_db, HISTORY_LIMIT)
                }
            });
            let result = task.await;

            this.update(cx, |this, cx| {
                if this.refresh_generation != generation {
                    return;
                }
                match result {
                    Ok(history) => this.history = history,
                    Err(error) => log::error!("Failed to refresh history: {}", error),
                }
                cx.notify();
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn on_search_change(
        &mut self,
        _state: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.query = self.search.read(cx).value().to_string();
            self.refresh_list(true, window, cx);
        }
    }

    /// Reload history from database, honoring the active search query.
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_list(false, window, cx);
    }

    fn on_item_click(&mut self, item: &HistoryItem, _window: &mut Window, cx: &mut Context<Self>) {
        self.selected_id = Some(item.id);
        cx.emit(HistoryItemClicked { item: item.clone() });
        cx.notify();
    }

    fn clear_history(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        self.history.clear();
        self.selected_id = None;
        self.query = String::new();
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();

        let db = self.db.clone();
        let task = cx.background_spawn(async move { db.clear_all_history() });
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = task.await {
                log::error!("Failed to clear history: {}", error);
                // Restore authoritative state if the optimistic clear failed.
                this.update_in(cx, |this, window, cx| this.refresh_list(false, window, cx))?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    /// Render one history row. Split out of `render` so the list body stays
    /// shallow enough for rustfmt to format it.
    fn render_item(&self, item: &HistoryItem, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let item_id = item.id;
        let is_selected = self.selected_id == Some(item_id);
        let verb = item.request.method.as_str();
        let verb_color = crate::theme::method_color(item.request.method, theme);
        let url = item.request.url.clone();
        let time = crate::format::format_relative_time(&item.timestamp, chrono::Utc::now());
        let item_clone = item.clone();

        h_flex()
            .id(("history-item", item_id as u64))
            .gap_2()
            .items_start()
            .w_full()
            .px_2p5()
            .py_2()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(if is_selected {
                theme.list_active_border
            } else {
                gpui::transparent_black()
            })
            .bg(if is_selected {
                theme.list_active
            } else {
                gpui::transparent_black()
            })
            .cursor_pointer()
            .hover(|s| {
                s.bg(if is_selected {
                    theme.list_active
                } else {
                    theme.list_hover
                })
            })
            .on_click(
                cx.listener(move |this, _event: &gpui::ClickEvent, window, cx| {
                    this.on_item_click(&item_clone, window, cx);
                }),
            )
            .child(
                // small mono method label, no filled pill
                div()
                    .flex_shrink_0()
                    .w(px(34.))
                    .text_right()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(verb_color)
                    .child(verb),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .overflow_x_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(url),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(time),
                    ),
            )
    }
}

impl EventEmitter<HistoryItemClicked> for HistoryPanel {}

impl Render for HistoryPanel {
    #[cfg_attr(feature = "profile", profiling::function)]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .size_full()
            .child(
                // Header
                h_flex()
                    .items_center()
                    .w_full()
                    .gap_1()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("History"),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .small()
                                .w_full()
                                .cleanable(true)
                                .prefix(Icon::empty().path("icons/search.svg")),
                        ),
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
                let msg = if self.query.trim().is_empty() {
                    "No history yet\n\nSend a request to get started".to_string()
                } else {
                    format!("No history matches \"{}\"", self.query.trim())
                };
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .text_sm()
                        .child(msg),
                )
            })
            .when(!self.history.is_empty(), |this| {
                this.child(
                    // Viewport: bounded so the scroller inside it can overflow.
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0() // Let the list shrink so its overflow_scroll engages
                        .w_full()
                        .overflow_hidden()
                        .child(
                            v_flex()
                                .id("history-list-scroll")
                                .flex_1()
                                .w_full()
                                .min_h_0()
                                .track_scroll(&self.list_scroll_handle)
                                .overflow_scroll()
                                .child(v_flex().gap_0p5().px_2().py_1().children(
                                    self.history.iter().map(|item| self.render_item(item, cx)),
                                )),
                        )
                        .vertical_scrollbar(&self.list_scroll_handle),
                )
            })
    }
}
