use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::px;
use gpui_component::{
    input::{Input, InputState},
    v_flex, h_flex, ActiveTheme as _,
};

use crate::types::{AuthConfig, AuthType};

/// Auth sub-tab editor. A flat set of input fields (one per auth field) plus a
/// type selector; only the active type's fields render. Values persist across
/// type switches because each field is its own always-alive `InputState`.
pub struct AuthEditor {
    /// 0 = None, 1 = Bearer, 2 = Basic, 3 = ApiKey.
    auth_type_index: usize,
    bearer_token: Entity<InputState>,
    basic_username: Entity<InputState>,
    basic_password: Entity<InputState>,
    api_key_name: Entity<InputState>,
    api_key_value: Entity<InputState>,
}

impl AuthEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            auth_type_index: 0,
            bearer_token: cx.new(|cx| InputState::new(window, cx).placeholder("Token")),
            basic_username: cx.new(|cx| InputState::new(window, cx).placeholder("Username")),
            basic_password: cx.new(|cx| InputState::new(window, cx).placeholder("Password")),
            api_key_name: cx.new(|cx| InputState::new(window, cx).placeholder("Key (e.g. X-API-Key)")),
            api_key_value: cx.new(|cx| InputState::new(window, cx).placeholder("Value")),
        }
    }

    /// Read the current auth configuration from the UI fields.
    pub fn get_auth(&self, cx: &App) -> AuthConfig {
        AuthConfig {
            auth_type: match self.auth_type_index {
                1 => AuthType::Bearer,
                2 => AuthType::Basic,
                3 => AuthType::ApiKey,
                _ => AuthType::None,
            },
            bearer_token: self.bearer_token.read(cx).value().to_string(),
            basic_username: self.basic_username.read(cx).value().to_string(),
            basic_password: self.basic_password.read(cx).value().to_string(),
            api_key_name: self.api_key_name.read(cx).value().to_string(),
            api_key_value: self.api_key_value.read(cx).value().to_string(),
        }
    }

    /// Load an auth configuration into the UI (used by `load_request`).
    pub fn set_auth(&mut self, auth: &AuthConfig, window: &mut Window, cx: &mut Context<Self>) {
        self.auth_type_index = match auth.auth_type {
            AuthType::None => 0,
            AuthType::Bearer => 1,
            AuthType::Basic => 2,
            AuthType::ApiKey => 3,
        };
        self.bearer_token.update(cx, |i, cx| i.set_value(&auth.bearer_token, window, cx));
        self.basic_username.update(cx, |i, cx| i.set_value(&auth.basic_username, window, cx));
        self.basic_password.update(cx, |i, cx| i.set_value(&auth.basic_password, window, cx));
        self.api_key_name.update(cx, |i, cx| i.set_value(&auth.api_key_name, window, cx));
        self.api_key_value.update(cx, |i, cx| i.set_value(&auth.api_key_value, window, cx));
        cx.notify();
    }

    /// A labelled input row (label on the left, field filling the rest).
    fn field_row(label: &'static str, input: &Entity<InputState>, theme: &gpui_component::Theme) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .w_full()
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(div().flex_1().child(Input::new(input)))
    }
}

impl Render for AuthEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .gap_3()
            .w_full()
            .flex_1()
            .min_h_0()
            // Type selector — muted radios, matching BodyEditor's body-type row.
            .child(
                h_flex().gap_4().items_center().children(
                    ["None", "Bearer", "Basic", "API Key"].into_iter().enumerate().map(|(i, label)| {
                        let selected = self.auth_type_index == i;
                        h_flex()
                            .id(("auth-type", i))
                            .gap_1p5()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.auth_type_index = i;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if selected { theme.primary } else { theme.border })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |d| {
                                        d.child(div().size(px(6.)).rounded_full().bg(theme.primary))
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if selected { theme.foreground } else { theme.muted_foreground })
                                    .child(label),
                            )
                    }),
                ),
            )
            // A one-line note: auth is injected at send time and wins over a manual header.
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("The auth header is added when the request is sent and overrides a manually-typed header of the same name."),
            )
            // Active type's fields.
            .when(self.auth_type_index == 0, |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .child("This request does not use authorization"),
                )
            })
            .when(self.auth_type_index == 1, |this| {
                this.child(Self::field_row("Token", &self.bearer_token, theme))
            })
            .when(self.auth_type_index == 2, |this| {
                this.child(Self::field_row("Username", &self.basic_username, theme))
                    .child(Self::field_row("Password", &self.basic_password, theme))
            })
            .when(self.auth_type_index == 3, |this| {
                this.child(Self::field_row("Key", &self.api_key_name, theme))
                    .child(Self::field_row("Value", &self.api_key_value, theme))
            })
    }
}
