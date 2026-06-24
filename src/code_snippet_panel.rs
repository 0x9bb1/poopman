//! The "Code snippet" slide-out panel (Postman's Code feature). Shows generated
//! client code for the current request in a selectable language, with Copy and
//! Close actions. Owned by `PoopmanApp`, rendered as a right-docked card when open.

use gpui::*;
use gpui_component::{
    button::*, input::*, select::*, h_flex, v_flex, ActiveTheme as _, Icon, IconName, IndexPath,
    Sizable as _,
};

use crate::code_gen::{generate, CodeTarget};
use crate::types::RequestData;

/// Emitted when the user closes the code-snippet panel.
pub struct CloseCodeSnippet;

pub struct CodeSnippetPanel {
    request: Option<RequestData>,
    target: CodeTarget,
    code: String,
    language_select: Entity<SelectState<Vec<&'static str>>>,
    code_display: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CloseCodeSnippet> for CodeSnippetPanel {}

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
        self.code_display.update(cx, |input, cx| input.set_value(&code, window, cx));
        cx.notify();
    }

    fn copy(&mut self, _e: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.code.clone()));
    }

    fn close(&mut self, _e: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CloseCodeSnippet);
    }
}

impl Render for CodeSnippetPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .id("code-snippet-panel")
            .size_full()
            .gap_3()
            .p_4()
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                // Header: title + Copy + Close
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("Code snippet"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("code-copy")
                                    .small()
                                    .label("Copy")
                                    .on_click(cx.listener(Self::copy)),
                            )
                            .child(
                                Button::new("code-close")
                                    .small()
                                    .ghost()
                                    .icon(Icon::new(IconName::Close))
                                    .on_click(cx.listener(Self::close)),
                            ),
                    ),
            )
            .child(Select::new(&self.language_select))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
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
