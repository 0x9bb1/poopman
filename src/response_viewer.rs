use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{button::*, h_flex, input::*, scroll::ScrollableElement as _, v_flex, ActiveTheme as _};
use std::sync::Arc;

use crate::types::ResponseData;

/// Map a raw Content-Type header value to a gpui-renderable image format.
/// Strips `;`-parameters (e.g. charset), trims, and is case-insensitive.
fn image_format_for_content_type(content_type: &str) -> Option<ImageFormat> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    ImageFormat::from_mime_type(&mime)
}

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
    /// Shared with the owning tab, so setting/reading never copies the body.
    response: Option<Arc<ResponseData>>,
    /// True right after the user cancels a request; shows a notice instead of
    /// the usual empty state. Reset by the next set_response/clear_response.
    canceled: bool,
    /// Pre-built preview for image responses (constructed once per response —
    /// `Image::from_bytes` hashes the body for its asset id, too costly per frame).
    preview_image: Option<Arc<gpui::Image>>,
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
            canceled: false,
            preview_image: None,
            body_display,
            active_tab: 0,
            headers_scroll_handle: ScrollHandle::new(),
        }
    }

    /// Set response data
    pub fn set_response(
        &mut self,
        response: Arc<ResponseData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.canceled = false;
        // Pre-build an inline preview for image responses (binary only).
        self.preview_image = if response.is_text {
            None
        } else {
            response
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .and_then(|(_, v)| image_format_for_content_type(v))
                .map(|format| Arc::new(gpui::Image::from_bytes(format, response.body.clone())))
        };
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
    pub fn get_response(&self) -> Option<Arc<ResponseData>> {
        self.response.clone()
    }

    /// Clear response data
    pub fn clear_response(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.canceled = false;
        self.response = None;
        self.preview_image = None;
        self.body_display.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.active_tab = 0;
        cx.notify();
    }

    /// Clear the panel and show a "Request canceled" notice.
    pub fn show_canceled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_response(window, cx);
        self.canceled = true;
        cx.notify();
    }

    /// Save the (binary) response body to a file chosen via the OS dialog.
    fn save_binary(&mut self, _event: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(response) = self.response.clone() else {
            return;
        };
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
            if let Ok(Ok(Some(path))) = rx.await
                && let Err(e) = std::fs::write(&path, &response.body)
            {
                log::error!("Failed to save response to {:?}: {}", path, e);
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
                        .child(format!("Time: {}", crate::format::format_duration_ms(response.duration_ms))),
                )
                .when(!response.is_network_error(), |this| {
                    this.child(
                        div()
                            .text_sm()
                            .child(format!("Size: {}", crate::format::format_size(response.body.len()))),
                    )
                })
        } else {
            h_flex()
                .px_4()
                .py_2p5()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_color(cx.theme().muted_foreground)
                .child(if self.canceled { "Request canceled" } else { "No response yet" })
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
                                .items_start()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_sm()
                                        .flex_shrink_0()
                                        .child(format!("{}:", key)),
                                )
                                .child(
                                    // Wraps rather than ellipsizing — reading the whole
                                    // value is the point of looking at headers. min_w_0
                                    // is load-bearing: a value with no break
                                    // opportunities (a JWT, a long set-cookie) otherwise
                                    // has an automatic minimum width of the entire
                                    // string and blows the row out horizontally.
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .min_w_0()
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
                            let resp_is_text = self.response.as_ref().is_none_or(|r| r.is_text);
                            if resp_is_text {
                                let is_error = self
                                    .response
                                    .as_ref()
                                    .is_some_and(|r| r.is_network_error());
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
                                let preview = self.preview_image.clone();
                                this.child(
                                    v_flex()
                                        .flex_1()
                                        .w_full()
                                        .min_h_0()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .when_some(preview, |this, image| {
                                            // Inline preview, scaled to fit
                                            // (img defaults to object-fit: contain).
                                            this.child(
                                                div()
                                                    .flex_1()
                                                    .w_full()
                                                    .min_h_0()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(img(image).max_w_full().max_h_full()),
                                            )
                                        })
                                        .when(self.preview_image.is_none(), |this| {
                                            this.child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.foreground)
                                                    .child("Binary response"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(format!(
                                                    "{} · {}",
                                                    content_type,
                                                    crate::format::format_size(len)
                                                )),
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
                                    .min_h_0() // Let the list shrink so its overflow_scroll engages
                                    .w_full()
                                    .overflow_hidden()
                                    .child(self.render_headers(cx))
                                    .vertical_scrollbar(&self.headers_scroll_handle),
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
                        .child(if self.canceled {
                            "Request canceled"
                        } else {
                            "Send a request to see the response here"
                        }),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    // NOT `use super::*`: that would pull in `gpui::*`, whose `test` attribute
    // macro shadows the standard `#[test]`.
    use super::image_format_for_content_type;
    use gpui::ImageFormat;

    #[test]
    fn maps_supported_image_content_types() {
        assert_eq!(image_format_for_content_type("image/png"), Some(ImageFormat::Png));
        assert_eq!(image_format_for_content_type("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(image_format_for_content_type("image/jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(image_format_for_content_type("image/webp"), Some(ImageFormat::Webp));
        assert_eq!(image_format_for_content_type("image/gif"), Some(ImageFormat::Gif));
        assert_eq!(image_format_for_content_type("image/svg+xml"), Some(ImageFormat::Svg));
        assert_eq!(image_format_for_content_type("image/bmp"), Some(ImageFormat::Bmp));
        assert_eq!(image_format_for_content_type("image/tiff"), Some(ImageFormat::Tiff));
    }

    #[test]
    fn strips_parameters_whitespace_and_case() {
        assert_eq!(
            image_format_for_content_type("Image/PNG; charset=binary"),
            Some(ImageFormat::Png)
        );
        assert_eq!(image_format_for_content_type("  image/gif ; foo=bar"), Some(ImageFormat::Gif));
    }

    #[test]
    fn rejects_non_image_and_unknown_types() {
        assert_eq!(image_format_for_content_type("application/pdf"), None);
        assert_eq!(image_format_for_content_type("image/x-exotic"), None);
        assert_eq!(image_format_for_content_type(""), None);
        assert_eq!(image_format_for_content_type("text/html"), None);
    }
}
