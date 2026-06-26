use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{button::*, h_flex, input::*, v_flex, ActiveTheme as _};

use crate::types::ResponseData;

/// Pick a sensible file extension for a (lowercased, param-stripped) Content-Type.
///
/// Uses a curated map for common types because mime_guess's extension ordering is
/// unreliable (e.g. `image/jpeg` yields `jfif` first), falling back to mime_guess
/// for the long tail.
fn extension_for_content_type(ct: &str) -> Option<String> {
    let curated = match ct {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/javascript" | "text/javascript" => "js",
        "text/html" => "html",
        "text/css" => "css",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "audio/mpeg" => "mp3",
        "video/mp4" => "mp4",
        _ => "",
    };
    if !curated.is_empty() {
        return Some(curated.to_string());
    }
    mime_guess::get_mime_extensions_str(ct)
        .and_then(|exts| exts.first())
        .map(|e| e.to_string())
}

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
                .multi_line(true)
                .tab_size(TabSize { tab_size: 4, hard_tabs: false })
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
        // Only feed the text editor for text responses; binary is shown in a
        // dedicated panel and never decoded to (lossy) text.
        let display = if response.is_text {
            let text = response.body_text();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                crate::code_formatter::pretty_json_4(&json).unwrap_or_else(|_| text.to_string())
            } else {
                text.to_string()
            }
        } else {
            String::new()
        };

        self.body_display.update(cx, |input, cx| {
            input.set_value(&display, window, cx);
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

    /// Save the (binary) response body to a file chosen via the OS dialog.
    fn save_binary(&mut self, _event: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(response) = self.response.as_ref() else {
            return;
        };
        let bytes = response.body.clone();
        // Suggest a filename with the right extension based on Content-Type.
        let suggested = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
            .and_then(|ct| extension_for_content_type(&ct))
            .map(|ext| format!("response.{}", ext))
            .unwrap_or_else(|| "response.bin".to_string());
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn_in(window, async move |_this, _cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                if let Err(e) = std::fs::write(&path, &bytes) {
                    log::error!("Failed to save response to {:?}: {}", path, e);
                }
            }
        })
        .detach();
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
                .px_4()
                .py_2p5()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .px_2p5()
                        .py_0p5()
                        .rounded(cx.theme().radius)
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .bg(status_color.opacity(0.12))
                        .text_color(status_color)
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
                .px_4()
                .py_2p5()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_color(cx.theme().muted_foreground)
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
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation())) // Prevent click events from propagating
            .child(
                // Response status bar (self-styled with its own padding + bottom border)
                div()
                    .flex()
                    .flex_col()
                    .w_full()
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
                            crate::ui::segmented_bar(theme)
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 0)
                                        .id("resp-tab-body")
                                        .when(self.active_tab != 0, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Body"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 1)
                                        .id("resp-tab-headers")
                                        .when(self.active_tab != 1, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Headers"),
                                ),
                        )
                        .when(self.active_tab == 0, |this| {
                            let resp_is_text = self.response.as_ref().map_or(true, |r| r.is_text);
                            if resp_is_text {
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
                                        .rounded(theme.radius_lg)
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(theme.popover)
                                        .child(
                                            Input::new(&self.body_display)
                                                .disabled(is_error)
                                                .rounded(theme.radius_lg)
                                                .w_full()
                                                .h_full(),
                                        ),
                                )
                            } else {
                                // Binary response: don't decode to lossy text — show info + Save.
                                let (content_type, len) = self
                                    .response
                                    .as_ref()
                                    .map(|r| {
                                        let ct = r
                                            .headers
                                            .iter()
                                            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                                            .map(|(_, v)| v.clone())
                                            .unwrap_or_else(|| "application/octet-stream".to_string());
                                        (ct, r.body.len())
                                    })
                                    .unwrap_or_else(|| ("application/octet-stream".to_string(), 0));
                                this.child(
                                    v_flex()
                                        .flex_1()
                                        .w_full()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(theme.foreground)
                                                .child("Binary response"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(format!("{} · {} bytes", content_type, len)),
                                        )
                                        .child(
                                            Button::new("save-binary")
                                                .primary()
                                                .label("Save to file…")
                                                .on_click(cx.listener(Self::save_binary)),
                                        ),
                                )
                            }
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
