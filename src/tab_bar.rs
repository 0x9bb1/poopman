use gpui::*;
use gpui::px;
use gpui_component::{h_flex, ActiveTheme as _};

use crate::request_tab::RequestTab;

/// Event emitted when a tab is clicked
#[derive(Clone)]
pub struct TabClicked {
    pub tab_index: usize,
}

/// Event emitted when a new tab button is clicked
#[derive(Clone)]
pub struct NewTabClicked;

/// Event emitted when a tab close button is clicked
#[derive(Clone)]
pub struct TabCloseClicked {
    pub tab_index: usize,
}

/// Tab bar component for managing multiple request tabs
pub struct TabBar {
    tabs: Vec<RequestTab>,
    active_tab_index: usize,
}

impl TabBar {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            tabs: vec![],
            active_tab_index: 0,
        }
    }

    /// Update tabs and active index
    pub fn update_tabs(&mut self, tabs: Vec<RequestTab>, active_index: usize, _cx: &mut Context<Self>) {
        self.tabs = tabs;
        self.active_tab_index = active_index;
    }

    fn on_tab_click(&mut self, tab_index: usize, _event: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TabClicked { tab_index });
        cx.notify();
    }

    fn on_new_tab_click(&mut self, _event: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(NewTabClicked);
        cx.notify();
    }

    fn on_close_tab_click(&mut self, tab_index: usize, _event: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TabCloseClicked { tab_index });
        cx.notify();
    }
}

impl EventEmitter<TabClicked> for TabBar {}
impl EventEmitter<NewTabClicked> for TabBar {}
impl EventEmitter<TabCloseClicked> for TabBar {}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active_index = self.active_tab_index;

        h_flex()
            .gap_1()
            .items_center()
            .px_2()
            .py_1()
            .bg(theme.background)
            .border_b_1()
            .border_color(theme.border)
            .child(
                // Render all tabs
                h_flex()
                    .gap_1()
                    .items_center()
                    .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                        let is_active = index == active_index;
                        let tab_index = index;
                        let method = tab.request.method.as_str();

                        // Method color badge
                        let method_color = match tab.request.method {
                            crate::types::HttpMethod::GET => gpui::rgb(0x61affe),
                            crate::types::HttpMethod::POST => gpui::rgb(0x49cc90),
                            crate::types::HttpMethod::PUT => gpui::rgb(0xfca130),
                            crate::types::HttpMethod::DELETE => gpui::rgb(0xf93e3e),
                            crate::types::HttpMethod::PATCH => gpui::rgb(0x50e3c2),
                            crate::types::HttpMethod::HEAD => gpui::rgb(0x9012fe),
                            crate::types::HttpMethod::OPTIONS => gpui::rgb(0x0d5aa7),
                        };

                        h_flex()
                            .id(("tab", tab.id))
                            .gap_2()
                            .items_center()
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .border_1()
                            .border_color(if is_active { theme.border } else { gpui::transparent_black() })
                            .bg(if is_active { theme.list_active } else { theme.background })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, event, window, cx| {
                                this.on_tab_click(tab_index, event, window, cx);
                            }))
                            .child(
                                // Method badge
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(method_color)
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(gpui::white())
                                            .child(method)
                                    )
                            )
                            .child(
                                // Tab title
                                div()
                                    .text_sm()
                                    .text_color(if is_active { theme.foreground } else { theme.muted_foreground })
                                    .max_w(px(150.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(tab.title.clone())
                            )
                            .child(
                                // Close button
                                div()
                                    .id(("close-tab", tab.id))
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .cursor_pointer()
                                    .hover(|style| style.text_color(theme.foreground))
                                    .on_click(cx.listener(move |this, event, window, cx| {
                                        cx.stop_propagation();
                                        this.on_close_tab_click(tab_index, event, window, cx);
                                    }))
                                    .child("×")
                            )
                    }))
            )
            .child(
                // New tab button
                div()
                    .id("new-tab-button")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.list_hover).text_color(theme.foreground))
                    .on_click(cx.listener(|this, event, window, cx| {
                        this.on_new_tab_click(event, window, cx);
                    }))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("+")
                    )
            )
    }
}
