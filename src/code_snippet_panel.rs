//! The "Code snippet" dialog body (Postman's Code feature). Shows generated
//! client code for the current request in a selectable language, with a Copy
//! action. Owned by `PoopmanApp` and shown inside a dialog opened from the
//! request editor's `</>` button.

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::InputEvent;
use gpui_component::{
    ActiveTheme as _, IndexPath, Sizable as _, button::*, h_flex, input::*, select::*, v_flex,
};

use crate::code_gen::{CodeTarget, generate};
use crate::types::RequestData;

const MAX_CODE_VIEW_HEIGHT: f32 = 460.;
const VIEWPORT_MARGIN: f32 = 16.;
// gpui-component's dialog entrance animation adds 30 px to margin_top.
const DIALOG_ANIMATION_OFFSET: f32 = 30.;
// 24 px padding on each side, 24 px title, 16 px gap, and two 1 px borders.
const DIALOG_CHROME_HEIGHT: f32 = 90.;
const TOOLBAR_HEIGHT: f32 = 32.;
const TOOLBAR_GAP: f32 = 12.;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CodeSnippetGeometry {
    pub width: Pixels,
    pub code_height: Pixels,
    pub margin_top: Pixels,
    pub max_height: Pixels,
}

pub(crate) fn code_snippet_geometry(viewport: Size<Pixels>) -> CodeSnippetGeometry {
    let max_height =
        (viewport.height - px(2. * VIEWPORT_MARGIN + DIALOG_ANIMATION_OFFSET)).max(px(0.));
    let code_height = (max_height - px(DIALOG_CHROME_HEIGHT + TOOLBAR_HEIGHT + TOOLBAR_GAP))
        .clamp(px(0.), px(MAX_CODE_VIEW_HEIGHT));
    let dialog_height = code_height + px(DIALOG_CHROME_HEIGHT + TOOLBAR_HEIGHT + TOOLBAR_GAP);

    CodeSnippetGeometry {
        width: (viewport.width - px(2. * VIEWPORT_MARGIN)).clamp(px(0.), px(760.)),
        code_height,
        margin_top: ((viewport.height - dialog_height) / 2. - px(DIALOG_ANIMATION_OFFSET))
            .max(px(VIEWPORT_MARGIN)),
        max_height,
    }
}

pub struct CodeSnippetPanel {
    request: Option<RequestData>,
    target: CodeTarget,
    language_select: Entity<SelectState<Vec<&'static str>>>,
    code_display: Entity<InputState>,
    /// True briefly after a Copy click, to show "Copied ✓" feedback.
    copied: bool,
    copy_feedback_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl CodeSnippetPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target = CodeTarget::all()[0]; // cURL

        let language_select = cx.new(|cx| {
            SelectState::new(CodeTarget::labels(), Some(IndexPath::default()), window, cx)
        });

        let code_display = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(target.language())
                .line_number(true)
                .multi_line(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
        });

        let sub = cx.subscribe_in(
            &language_select,
            window,
            |this, _, _e: &SelectEvent<Vec<&'static str>>, window, cx| {
                this.on_language_changed(window, cx);
            },
        );
        let edit_sub = cx.subscribe(&code_display, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.copied = false;
                this.copy_feedback_task = None;
                cx.notify();
            }
        });

        Self {
            request: None,
            target,
            language_select,
            code_display,
            copied: false,
            copy_feedback_task: None,
            _subscriptions: vec![sub, edit_sub],
        }
    }

    /// Update the request shown and regenerate the snippet.
    pub fn set_request(
        &mut self,
        request: RequestData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request = Some(request);
        self.regenerate(window, cx);
    }

    fn on_language_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let idx = self
            .language_select
            .read(cx)
            .selected_index(cx)
            .map(|i| i.row)
            .unwrap_or(0);
        self.target = CodeTarget::all()
            .get(idx)
            .copied()
            .unwrap_or(CodeTarget::Curl);
        let lang = self.target.language();
        self.code_display
            .update(cx, |input, cx| input.set_highlighter(lang, cx));
        self.regenerate(window, cx);
    }

    fn regenerate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = match &self.request {
            Some(req) => generate(self.target, req),
            None => String::new(),
        };
        self.copied = false; // new code => clear any stale "Copied" state
        self.copy_feedback_task = None;
        self.code_display
            .update(cx, |input, cx| input.set_value(&code, window, cx));
        cx.notify();
    }

    fn copy(&mut self, _e: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.code_display.read(cx).value().to_string(),
        ));
        self.copied = true;
        cx.notify();
        // Revert the "Copied ✓" label after a short delay.
        self.copy_feedback_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        }));
    }
}

impl Render for CodeSnippetPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let geometry = code_snippet_geometry(window.viewport_size());

        v_flex()
            .id("code-snippet-panel")
            .w_full()
            .gap(px(TOOLBAR_GAP))
            .child(
                // Toolbar: language selector (left) + Copy (right)
                h_flex()
                    .id("code-snippet-toolbar")
                    .h(px(TOOLBAR_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().w(px(220.)).child(Select::new(&self.language_select)))
                    .child(
                        Button::new("code-copy")
                            .small()
                            .when(self.copied, |b| b.success())
                            .label(if self.copied { "Copied ✓" } else { "Copy" })
                            .on_click(cx.listener(Self::copy)),
                    ),
            )
            .child(
                div()
                    .id("code-snippet-editor")
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .h(geometry.code_height)
                    .w_full()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .overflow_hidden()
                    .child(Input::new(&self.code_display).w_full().h_full()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CodeSnippetPanel, DIALOG_ANIMATION_OFFSET, DIALOG_CHROME_HEIGHT, TOOLBAR_GAP,
        TOOLBAR_HEIGHT, VIEWPORT_MARGIN, code_snippet_geometry,
    };
    use crate::code_gen::{CodeTarget, generate};
    use crate::types::{HttpMethod, RequestData};
    use gpui::{AppContext as _, ClickEvent, TestAppContext, px, size};

    #[test]
    fn dialog_fits_minimum_viewport_including_entrance_animation() {
        let viewport = size(px(720.), px(480.));
        let geometry = code_snippet_geometry(viewport);
        let dialog_height =
            geometry.code_height + px(DIALOG_CHROME_HEIGHT + TOOLBAR_HEIGHT + TOOLBAR_GAP);

        assert_eq!(geometry.width, px(688.));
        assert!(geometry.code_height >= px(200.));
        assert!(dialog_height <= geometry.max_height);
        assert!(geometry.margin_top >= px(VIEWPORT_MARGIN));
        assert!(
            geometry.margin_top + px(DIALOG_ANIMATION_OFFSET) + dialog_height
                <= viewport.height - px(VIEWPORT_MARGIN)
        );
    }

    #[test]
    fn geometry_recomputes_on_resize_and_caps_large_windows() {
        let desktop = code_snippet_geometry(size(px(1440.), px(900.)));
        assert_eq!(desktop.width, px(760.));
        assert_eq!(desktop.code_height, px(460.));

        for height in [480., 540., 600., 768., 900.] {
            for width in [720., 800., 1024., 1440.] {
                let geometry = code_snippet_geometry(size(px(width), px(height)));
                assert!(geometry.width <= px(width - 32.));
                assert!(geometry.code_height <= desktop.code_height);
                assert!(
                    geometry.margin_top
                        + px(DIALOG_ANIMATION_OFFSET
                            + DIALOG_CHROME_HEIGHT
                            + TOOLBAR_HEIGHT
                            + TOOLBAR_GAP)
                        + geometry.code_height
                        <= px(height - VIEWPORT_MARGIN)
                );
            }
        }
    }

    #[gpui::test]
    fn toolbar_copy_uses_current_editor_text(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let cx = cx.add_empty_window();
        let panel = cx.update(|window, cx| cx.new(|cx| CodeSnippetPanel::new(window, cx)));
        let request = RequestData::new(HttpMethod::GET, "https://example.com".into());
        let generated = generate(CodeTarget::Curl, &request);
        cx.update(|window, cx| {
            panel.update(cx, |panel, cx| panel.set_request(request, window, cx));
        });

        // Include whitespace, Unicode and an empty editor: Copy must preserve
        // the entire current value, even when it differs from generated code.
        for displayed in [
            generated.as_str(),
            "  # edited 请求\n\tcurl https://example.org\n",
            "",
        ] {
            cx.update(|window, cx| {
                panel.update(cx, |panel, cx| {
                    panel.code_display.update(cx, |input, cx| {
                        input.set_value(displayed.to_string(), window, cx);
                    });
                });
            });
            cx.update(|window, cx| {
                panel.update(cx, |panel, cx| {
                    panel.copy(&ClickEvent::default(), window, cx)
                });
            });
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), displayed);
        }

        cx.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                assert!(panel.copied);
                panel
                    .code_display
                    .update(cx, |input, cx| input.set_value("another edit", window, cx));
            });
        });
        cx.update(|_, cx| {
            assert!(!panel.read(cx).copied);
            assert!(panel.read(cx).copy_feedback_task.is_none());
        });
    }

    #[gpui::test]
    fn keyboard_copy_preserves_selection_behavior(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let mut panel = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| CodeSnippetPanel::new(window, cx));
            panel = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let panel = panel.unwrap();
        cx.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                panel.code_display.update(cx, |input, cx| {
                    input.set_value("abc tail\nsecond line", window, cx);
                    input.focus(window, cx);
                });
            });
        });
        cx.run_until_parked();
        cx.dispatch_action(gpui_component::input::MoveToStart);
        cx.simulate_keystrokes("shift-right shift-right shift-right");
        if cfg!(target_os = "macos") {
            cx.simulate_keystrokes("cmd-c");
        } else {
            cx.simulate_keystrokes("ctrl-c");
        }
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "abc");
        cx.update(|window, cx| {
            panel.update(cx, |panel, cx| {
                panel.copy(&ClickEvent::default(), window, cx)
            });
        });
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "abc tail\nsecond line"
        );
    }
}
