//! The "Code snippet" dialog body (Postman's Code feature). Shows generated
//! client code for the current request in a selectable language, with a Copy
//! action. Owned by `PoopmanApp` and shown inside a dialog opened from the
//! request editor's `</>` button.

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::*, input::*, select::*, h_flex, v_flex, ActiveTheme as _, IndexPath, Sizable as _,
};

use crate::code_gen::{generate, CodeTarget};
use crate::types::RequestData;

/// Height of the code view inside the dialog (dialog height is content-driven,
/// so the editor needs a definite height to render).
const CODE_VIEW_HEIGHT: f32 = 460.;

pub struct CodeSnippetPanel {
    request: Option<RequestData>,
    target: CodeTarget,
    code: String,
    language_select: Entity<SelectState<Vec<&'static str>>>,
    code_display: Entity<InputState>,
    /// True briefly after a Copy click, to show "Copied ✓" feedback.
    copied: bool,
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
                .tab_size(TabSize { tab_size: 4, hard_tabs: false })
        });

        let sub = cx.subscribe_in(
            &language_select,
            window,
            |this, _, _e: &SelectEvent<Vec<&'static str>>, window, cx| {
                this.on_language_changed(window, cx);
            },
        );

        Self {
            request: None,
            target,
            code: String::new(),
            language_select,
            code_display,
            copied: false,
            _subscriptions: vec![sub],
        }
    }

    /// Update the request shown and regenerate the snippet.
    pub fn set_request(&mut self, request: RequestData, window: &mut Window, cx: &mut Context<Self>) {
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
        self.target = CodeTarget::all().get(idx).copied().unwrap_or(CodeTarget::Curl);
        let lang = self.target.language();
        self.code_display.update(cx, |input, cx| input.set_highlighter(lang, cx));
        self.regenerate(window, cx);
    }

    fn regenerate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = match &self.request {
            Some(req) => generate(self.target, req),
            None => String::new(),
        };
        self.code = code.clone();
        self.copied = false; // new code => clear any stale "Copied" state
        self.code_display.update(cx, |input, cx| input.set_value(&code, window, cx));
        cx.notify();
    }

    fn copy(&mut self, _e: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.code.clone()));
        self.copied = true;
        cx.notify();
        // Revert the "Copied ✓" label after a short delay.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for CodeSnippetPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .id("code-snippet-panel")
            .w_full()
            .gap_3()
            .child(
                // Toolbar: language selector (left) + Copy (right)
                h_flex()
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
                    .flex()
                    .flex_col()
                    .h(px(CODE_VIEW_HEIGHT))
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
