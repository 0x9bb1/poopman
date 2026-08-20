use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    button::*,
    h_flex,
    input::*,
    menu::{ContextMenuExt as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    text::{TextView, TextViewStyle},
    v_flex,
};
use std::{collections::VecDeque, sync::Arc};

use crate::types::ResponseData;

const BODY_CACHE_ENTRIES: usize = 6;
const BODY_CACHE_DISPLAY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
struct PreparedBody {
    text: Arc<str>,
}

fn decode_response_text(body: &[u8]) -> String {
    #[cfg(feature = "profile")]
    let profile_data = format!("{} bytes", body.len());
    #[cfg(feature = "profile")]
    profiling::scope!("response utf8 decode", profile_data.as_str());
    String::from_utf8_lossy(body).into_owned()
}

/// Prepare response text away from the UI thread.
///
/// Decode, parse, and pretty-print the complete response here. `InputState`
/// virtualizes visible-line layout, but it has no incremental data-source API,
/// so the complete prepared text is handed to it in one update.
#[cfg_attr(feature = "profile", profiling::function)]
fn prepare_response_body(body: &[u8]) -> PreparedBody {
    let text = decode_response_text(body);
    let display = if let Ok(json) = {
        #[cfg(feature = "profile")]
        let profile_data = format!("{} bytes", text.len());
        #[cfg(feature = "profile")]
        profiling::scope!("response json parse", profile_data.as_str());
        serde_json::from_str::<serde_json::Value>(&text)
    } {
        #[cfg(feature = "profile")]
        let profile_data = format!("{} bytes", text.len());
        #[cfg(feature = "profile")]
        profiling::scope!("response json pretty serialize", profile_data.as_str());
        crate::code_formatter::pretty_json_4(&json).unwrap_or(text)
    } else {
        text
    };
    PreparedBody {
        text: display.into(),
    }
}

#[derive(Clone)]
struct CachedBody {
    response: Arc<ResponseData>,
    text: Arc<str>,
    display_bytes: usize,
}

/// Render headers as `key: value` lines — what "Copy all" puts on the clipboard.
/// No trailing newline, so pasting into a single-line field stays clean.
fn headers_to_text(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Escape text for embedding in the HTML fed to `TextView`.
///
/// Header values are arbitrary bytes from the network: `&` shows up in every
/// URL-bearing header and `<` appears in Link/Report-To headers. Without this
/// they would be swallowed as markup.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// One paragraph per header, key in bold — as HTML so `TextView` can render it
/// with real text selection.
fn headers_to_html(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(k, v)| format!("<p><b>{}:</b> {}</p>", escape_html(k), escape_html(v)))
        .collect::<Vec<_>>()
        .join("")
}

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
    /// One editor is created with the window and reused for every response.
    /// Creating an InputState per response makes its first syntax/layout pass a
    /// cold path (30 ms even for 1.7 KiB in the new2 Tracy capture).
    body_display: Entity<InputState>,
    body_ready: bool,
    body_loading: bool,
    body_generation: u64,
    /// Small LRU of complete, background-prepared text. An entry larger than
    /// the byte budget remains as the sole entry so revisiting it does not
    /// repeat decode/parse/pretty work.
    body_cache: VecDeque<CachedBody>,
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
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
        });
        Self {
            response: None,
            canceled: false,
            preview_image: None,
            body_display,
            body_ready: false,
            body_loading: false,
            body_generation: 0,
            body_cache: VecDeque::new(),
            active_tab: 0,
            headers_scroll_handle: ScrollHandle::new(),
        }
    }

    /// Set response data
    #[cfg_attr(feature = "profile", profiling::function)]
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
        self.body_generation = self.body_generation.wrapping_add(1);
        self.body_loading = false;
        self.body_ready = false;
        self.response = Some(response.clone());
        self.active_tab = 0; // Reset to Body tab

        // Binary responses use their dedicated preview. Text response metadata
        // is visible immediately; its expensive display representation arrives
        // asynchronously or is restored from the prepared-state cache.
        if response.is_text {
            if let Some(cached) = self.take_cached_body(&response) {
                #[cfg(feature = "profile")]
                profiling::scope!("response body cache hit");
                self.body_loading = true;
                self.start_cached_body_apply(cached, window, cx);
            } else {
                #[cfg(feature = "profile")]
                profiling::scope!("response body cache miss");
                self.body_loading = true;
                self.start_body_prepare(response, window, cx);
            }
        }
        cx.notify();
    }

    fn take_cached_body(&mut self, response: &Arc<ResponseData>) -> Option<CachedBody> {
        let index = self
            .body_cache
            .iter()
            .position(|cached| Arc::ptr_eq(&cached.response, response))?;
        self.body_cache.remove(index)
    }

    fn insert_cached_body(&mut self, cached: CachedBody) {
        if let Some(index) = self
            .body_cache
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.response, &cached.response))
        {
            self.body_cache.remove(index);
        }
        self.body_cache.push_front(cached);

        let mut display_bytes = self
            .body_cache
            .iter()
            .map(|entry| entry.display_bytes)
            .sum::<usize>();
        while self.body_cache.len() > BODY_CACHE_ENTRIES
            || (display_bytes > BODY_CACHE_DISPLAY_BYTES && self.body_cache.len() > 1)
        {
            if let Some(evicted) = self.body_cache.pop_back() {
                display_bytes = display_bytes.saturating_sub(evicted.display_bytes);
            }
        }
    }

    fn start_cached_body_apply(
        &mut self,
        cached: CachedBody,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.body_generation;
        let response = cached.response.clone();
        let prepared = PreparedBody {
            text: cached.text.clone(),
        };
        // Keep the entry available if another tab action supersedes this
        // scheduled UI commit before it runs.
        self.body_cache.push_front(cached);
        cx.spawn_in(window, async move |this, cx| {
            this.update_in(cx, |this, window, cx| {
                this.apply_prepared_body(response, generation, prepared, window, cx)
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn start_body_prepare(
        &mut self,
        response: Arc<ResponseData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.body_generation;
        let response_for_task = response.clone();
        let task =
            cx.background_spawn(async move { prepare_response_body(&response_for_task.body) });
        cx.spawn_in(window, async move |this, cx| {
            let prepared = task.await;
            this.update_in(cx, |this, window, cx| {
                this.apply_prepared_body(response, generation, prepared, window, cx)
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn apply_prepared_body(
        &mut self,
        response: Arc<ResponseData>,
        generation: u64,
        prepared: PreparedBody,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let display_bytes = prepared.text.len();
        self.insert_cached_body(CachedBody {
            response: response.clone(),
            text: prepared.text.clone(),
            display_bytes,
        });

        let is_current = self.body_generation == generation
            && self
                .response
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &response));
        if !is_current {
            return;
        }

        let display_text = prepared.text.clone();
        self.body_display.update(cx, move |input, cx| {
            #[cfg(feature = "profile")]
            let profile_data = format!("{} bytes", display_bytes);
            #[cfg(feature = "profile")]
            profiling::scope!("response InputState::set_value", profile_data.as_str());
            input.set_value(display_text.clone(), window, cx);
        });
        self.body_ready = true;
        self.body_loading = false;
        cx.notify();
    }

    /// Get current response data
    pub fn get_response(&self) -> Option<Arc<ResponseData>> {
        self.response.clone()
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled
    }

    /// Clear response data
    pub fn clear_response(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.canceled = false;
        self.body_generation = self.body_generation.wrapping_add(1);
        self.response = None;
        self.preview_image = None;
        self.body_ready = false;
        self.body_loading = false;
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
    fn save_binary(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(response) = self.response.clone() else {
            return;
        };
        // Suggest a filename with the right extension based on Content-Type.
        let suggested = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .and_then(|ct| extension_for_content_type(&ct))
            .map(|ext| format!("response.{}", ext))
            .unwrap_or_else(|| "response.bin".to_string());
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn_in(window, async move |_this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let path_for_log = path.clone();
                let write =
                    cx.background_spawn(async move { std::fs::write(&path, &response.body) });
                if let Err(error) = write.await {
                    log::error!("Failed to save response to {:?}: {}", path_for_log, error);
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
                .child(div().text_sm().child(format!(
                    "Time: {}",
                    crate::format::format_duration_ms(response.duration_ms)
                )))
                .when(!response.is_network_error(), |this| {
                    this.child(div().text_sm().child(format!(
                        "Size: {}",
                        crate::format::format_size(response.body.len())
                    )))
                })
        } else {
            h_flex()
                .px_4()
                .py_2p5()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_color(cx.theme().muted_foreground)
                .child(if self.canceled {
                    "Request canceled"
                } else {
                    "No response yet"
                })
        }
    }

    fn render_headers(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if let Some(response) = &self.response {
            let all_headers = headers_to_text(&response.headers);
            v_flex()
                .id("response-headers-scroll")
                .flex_1()
                .w_full()
                .min_h_0()
                .track_scroll(&self.headers_scroll_handle)
                .overflow_scroll()
                .child(
                    div()
                        .p_2()
                        .w_full()
                        .text_sm()
                        // TextView, not a div list: gpui has no text selection outside
                        // it and inputs (gpui/src/elements/text.rs exposes no selection
                        // API at all). Selectable gives the I-beam cursor, click-drag
                        // selection and the ctrl-c binding.
                        .child(
                            TextView::html(
                                "response-headers",
                                headers_to_html(&response.headers),
                                window,
                                cx,
                            )
                            .selectable(true)
                            .style(TextViewStyle::default().paragraph_gap(rems(0.25))),
                        )
                        .context_menu(move |menu, _window, _cx| {
                            // Only "Copy all headers" -- a "Copy selection" item cannot
                            // work here: it would have to dispatch TextView's Copy
                            // action, and by the time the menu is open the TextView no
                            // longer holds focus, so the dispatch goes nowhere and the
                            // clipboard keeps whatever ctrl-c last put there. Use ctrl-c
                            // for the selection.
                            let all = all_headers.clone();
                            menu.item(PopupMenuItem::new("Copy all headers").on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(all.clone()));
                                },
                            ))
                        }),
                )
                .into_any_element()
        } else {
            v_flex()
                .id("response-headers-empty")
                .flex_1()
                .child(v_flex().p_2().child("No headers"))
                .into_any_element()
        }
    }
}

impl Render for ResponseViewer {
    #[cfg_attr(feature = "profile", profiling::function)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Built before `theme` borrows cx immutably -- TextView needs &mut App.
        // Only while the tab is showing, so the HTML is not parsed for nothing.
        let headers_el = (self.active_tab == 1 && self.response.is_some())
            .then(|| self.render_headers(window, cx));
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
                        // Load-bearing: a flex item's min-height defaults to auto, i.e.
                        // its content height, so without this the container grows to fit
                        // the header list and the scroller below it is never bounded --
                        // overflow_scroll then has nothing to overflow and the list
                        // cannot scroll however long it gets.
                        .min_h_0()
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
                                let is_error =
                                    self.response.as_ref().is_some_and(|r| r.is_network_error());
                                let body_display = self.body_display.clone();
                                let has_body_display = self.body_ready;
                                let body_loading = self.body_loading;
                                this.child(
                                    v_flex()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .rounded(theme.radius_lg)
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(theme.popover)
                                        .when(has_body_display, |this| {
                                            this.child(
                                                div().flex_1().min_h_0().w_full().child(
                                                    Input::new(&body_display)
                                                        .disabled(is_error)
                                                        .rounded(theme.radius_lg)
                                                        .w_full()
                                                        .h_full(),
                                                ),
                                            )
                                        })
                                        .when(!has_body_display, |this| {
                                            this.child(
                                                div()
                                                    .flex_1()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_sm()
                                                    .text_color(theme.muted_foreground)
                                                    .child(if body_loading {
                                                        "Preparing response…"
                                                    } else {
                                                        "No response body"
                                                    }),
                                            )
                                        }),
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
                                            .unwrap_or_else(|| {
                                                "application/octet-stream".to_string()
                                            });
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
                                    .children(headers_el)
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
    use super::headers_to_html;
    use super::headers_to_text;
    use super::image_format_for_content_type;
    use super::prepare_response_body;
    use gpui::ImageFormat;

    #[test]
    fn prepares_small_json_with_pretty_indentation() {
        let prepared = prepare_response_body(br#"{"a":1}"#);
        assert_eq!(prepared.text.as_ref(), "{\n    \"a\": 1\n}");
    }

    #[test]
    fn preserves_non_json_text() {
        let prepared = prepare_response_body(b"plain text");
        assert_eq!(prepared.text.as_ref(), "plain text");
    }

    #[test]
    fn preserves_complete_large_non_json_body() {
        let body = vec![b'x'; 128 * 1024 + 1];
        let prepared = prepare_response_body(&body);
        assert_eq!(prepared.text.len(), body.len());
        assert_eq!(prepared.text.as_bytes(), body.as_slice());
    }

    #[test]
    fn formats_complete_large_minified_json() {
        let item_count = 75_000;
        let body = format!("[{}]", vec!["0"; item_count].join(","));
        let prepared = prepare_response_body(body.as_bytes());
        assert!(prepared.text.starts_with("[\n"));
        assert!(prepared.text.ends_with("\n]"));
        assert_eq!(
            prepared
                .text
                .lines()
                .filter(|line| line.trim() == "0,")
                .count(),
            item_count - 1
        );
        assert!(prepared.text.len() > body.len());
    }

    #[test]
    fn maps_supported_image_content_types() {
        assert_eq!(
            image_format_for_content_type("image/png"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            image_format_for_content_type("image/jpeg"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            image_format_for_content_type("image/jpg"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            image_format_for_content_type("image/webp"),
            Some(ImageFormat::Webp)
        );
        assert_eq!(
            image_format_for_content_type("image/gif"),
            Some(ImageFormat::Gif)
        );
        assert_eq!(
            image_format_for_content_type("image/svg+xml"),
            Some(ImageFormat::Svg)
        );
        assert_eq!(
            image_format_for_content_type("image/bmp"),
            Some(ImageFormat::Bmp)
        );
        assert_eq!(
            image_format_for_content_type("image/tiff"),
            Some(ImageFormat::Tiff)
        );
    }

    #[test]
    fn strips_parameters_whitespace_and_case() {
        assert_eq!(
            image_format_for_content_type("Image/PNG; charset=binary"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            image_format_for_content_type("  image/gif ; foo=bar"),
            Some(ImageFormat::Gif)
        );
    }

    #[test]
    fn rejects_non_image_and_unknown_types() {
        assert_eq!(image_format_for_content_type("application/pdf"), None);
        assert_eq!(image_format_for_content_type("image/x-exotic"), None);
        assert_eq!(image_format_for_content_type(""), None);
        assert_eq!(image_format_for_content_type("text/html"), None);
    }

    // ===== headers_to_text ("Copy all") =====

    fn hs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn joins_headers_as_key_colon_value_lines() {
        assert_eq!(
            headers_to_text(&hs(&[("content-type", "text/html"), ("server", "nginx")])),
            "content-type: text/html\nserver: nginx"
        );
    }

    #[test]
    fn no_trailing_newline() {
        let out = headers_to_text(&hs(&[("a", "1"), ("b", "2")]));
        assert!(!out.ends_with('\n'), "got {out:?}");
    }

    #[test]
    fn empty_headers_give_empty_string() {
        assert_eq!(headers_to_text(&[]), "");
    }

    #[test]
    fn single_header_has_no_newline() {
        assert_eq!(
            headers_to_text(&hs(&[("date", "Mon, 20 Jul 2026")])),
            "date: Mon, 20 Jul 2026"
        );
    }

    #[test]
    fn preserves_duplicate_keys_and_order() {
        // set-cookie legitimately repeats; collapsing it would lose data.
        assert_eq!(
            headers_to_text(&hs(&[("set-cookie", "a=1"), ("set-cookie", "b=2")])),
            "set-cookie: a=1\nset-cookie: b=2"
        );
    }

    #[test]
    fn keeps_empty_values() {
        assert_eq!(headers_to_text(&hs(&[("x-empty", "")])), "x-empty: ");
    }

    // ===== headers_to_html (what TextView renders) =====

    #[test]
    fn one_bold_key_paragraph_per_header() {
        assert_eq!(
            headers_to_html(&hs(&[("content-type", "text/html"), ("server", "nginx")])),
            "<p><b>content-type:</b> text/html</p><p><b>server:</b> nginx</p>"
        );
    }

    #[test]
    fn escapes_ampersands_in_values() {
        // Every URL-bearing header carries these; unescaped they vanish as markup.
        assert_eq!(
            headers_to_html(&hs(&[("location", "/a?x=1&y=2")])),
            "<p><b>location:</b> /a?x=1&amp;y=2</p>"
        );
    }

    #[test]
    fn escapes_angle_brackets_in_values() {
        // Link and Report-To headers really do contain these.
        assert_eq!(
            headers_to_html(&hs(&[("link", "<https://a/b>; rel=preload")])),
            "<p><b>link:</b> &lt;https://a/b&gt;; rel=preload</p>"
        );
    }

    #[test]
    fn escapes_keys_too() {
        assert_eq!(
            headers_to_html(&hs(&[("x<evil>", "v")])),
            "<p><b>x&lt;evil&gt;:</b> v</p>"
        );
    }

    #[test]
    fn empty_headers_give_empty_html() {
        assert_eq!(headers_to_html(&[]), "");
    }
}
