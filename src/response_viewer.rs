use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{button::*, h_flex, input::*, v_flex, ActiveTheme as _};

use crate::types::ResponseData;

/// Response viewer panel
pub struct ResponseViewer {
    response: Option<ResponseData>,
    body_display: Entity<InputState>,
    active_tab: usize,
    headers_scroll_handle: ScrollHandle,
}

impl ResponseViewer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body_display = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .line_number(true)
                .multi_line()
        });

        Self {
            response: None,
            body_display,
            active_tab: 0,
            headers_scroll_handle: ScrollHandle::new(),
        }
    }

    /// Set response data
    pub fn set_response(
        &mut self,
        response: ResponseData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Try to format JSON body for better display
        let formatted_body =
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response.body) {
                serde_json::to_string_pretty(&json).unwrap_or(response.body.clone())
            } else {
                response.body.clone()
            };

        self.body_display.update(cx, |input, cx| {
            input.set_value(&formatted_body, window, cx);
        });

        self.response = Some(response);
        self.active_tab = 0; // Reset to Body tab
        cx.notify();
    }

    /// Get current response data
    pub fn get_response(&self) -> Option<ResponseData> {
        self.response.clone()
    }

    /// Clear response data
    pub fn clear_response(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.response = None;
        self.body_display.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.active_tab = 0;
        cx.notify();
    }

    fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        if let Some(response) = &self.response {
            let status_color = if response.is_network_error() {
                cx.theme().danger // Special color for network errors
            } else if response.is_success() {
                cx.theme().success
            } else if response.is_error() {
                cx.theme().danger
            } else {
                cx.theme().accent
            };

            let status_text = if response.is_network_error() {
                format!("ERROR - {}", response.status_text())
            } else {
                format!(
                    "{} {}",
                    response.status.unwrap_or(0),
                    response.status_text()
                )
            };

            h_flex()
                .gap_3()
                .items_center()
                .p_2()
                .bg(cx.theme().muted)
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded(cx.theme().radius)
                        .bg(status_color)
                        .text_color(gpui::white())
                        .text_sm()
                        .child(status_text),
                )
                .child(
                    div()
                        .text_sm()
                        .child(format!("Time: {} ms", response.duration_ms)),
                )
                .when(!response.is_network_error(), |this| {
                    this.child(
                        div()
                            .text_sm()
                            .child(format!("Size: {} bytes", response.body.len())),
                    )
                })
        } else {
            h_flex()
                .p_2()
                .bg(cx.theme().muted)
                .border_b_1()
                .border_color(cx.theme().border)
                .child("No response yet")
        }
    }

    fn render_headers(&self, _cx: &App) -> impl IntoElement {
        if let Some(response) = &self.response {
            v_flex()
                .id("response-headers-scroll")
                .flex_1()
                .w_full()
                .min_h_0()
                .track_scroll(&self.headers_scroll_handle)
                .overflow_scroll()
                .child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .children(response.headers.iter().map(|(key, value)| {
                            h_flex()
                                .gap_2()
                                .w_full()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_sm()
                                        .flex_shrink_0()
                                        .child(format!("{}:", key)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(value.clone()),
                                )
                        })),
                )
        } else {
            v_flex()
                .id("response-headers-empty")
                .flex_1()
                .child(v_flex().p_2().child("No headers"))
        }
    }
}

impl Render for ResponseViewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .id("response-viewer-root")
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .overflow_hidden() // Prevent content overflow
            .bg(theme.background)
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation())) // Prevent click events from propagating
            .child(
                // Response section with header
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .w_full()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        // Section title
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("RESPONSE"),
                    )
                    .child(self.render_status_bar(cx)),
            )
            .when_some(self.response.as_ref(), |this, _| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .flex_1()
                        .p_4()
                        .w_full()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_1()
                                .child(
                                    Button::new("tab-body")
                                        .ghost()
                                        .label("Body")
                                        .when(self.active_tab == 0, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("tab-headers")
                                        .ghost()
                                        .label("Headers")
                                        .when(self.active_tab == 1, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .when(self.active_tab == 0, |this| {
                            let is_error = self
                                .response
                                .as_ref()
                                .map_or(false, |r| r.is_network_error());
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .w_full()
                                    .child(
                                        Input::new(&self.body_display)
                                            .disabled(is_error)
                                            .w_full()
                                            .h_full(),
                                    ),
                            )
                        })
                        .when(self.active_tab == 1, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .w_full()
                                    .overflow_hidden()
                                    .child(self.render_headers(cx)),
                            )
                        }),
                )
            })
            .when(self.response.is_none(), |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .child("Send a request to see the response here"),
                )
            })
    }
}
