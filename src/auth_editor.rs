use gpui::prelude::FluentBuilder as _;
use gpui::px;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Selectable as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use crate::types::{ApiKeyLocation, AuthConfig, AuthType};

#[cfg(test)]
mod tests;

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
    api_key_location: ApiKeyLocation,
    _subscriptions: Vec<Subscription>,
}

impl AuthEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editor = Self {
            auth_type_index: 0,
            api_key_location: ApiKeyLocation::Header,
            bearer_token: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Token")
                    .masked(true)
            }),
            basic_username: cx.new(|cx| InputState::new(window, cx).placeholder("Username")),
            basic_password: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Password")
                    .masked(true)
            }),
            api_key_name: cx
                .new(|cx| InputState::new(window, cx).placeholder("Key (e.g. X-API-Key)")),
            api_key_value: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Value")
                    .masked(true)
            }),
            _subscriptions: Vec::new(),
        };
        for input in [
            &editor.bearer_token,
            &editor.basic_username,
            &editor.basic_password,
            &editor.api_key_name,
            &editor.api_key_value,
        ] {
            editor
                ._subscriptions
                .push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
        }
        editor
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
            api_key_location: self.api_key_location,
        }
    }

    /// Load an auth configuration into the UI (used by `load_request`).
    pub fn set_auth(&mut self, auth: &AuthConfig, window: &mut Window, cx: &mut Context<Self>) {
        self.api_key_location = auth.api_key_location;
        self.auth_type_index = match auth.auth_type {
            AuthType::None => 0,
            AuthType::Bearer => 1,
            AuthType::Basic => 2,
            AuthType::ApiKey => 3,
        };
        self.bearer_token
            .update(cx, |i, cx| i.set_value(&auth.bearer_token, window, cx));
        self.basic_username
            .update(cx, |i, cx| i.set_value(&auth.basic_username, window, cx));
        self.basic_password
            .update(cx, |i, cx| i.set_value(&auth.basic_password, window, cx));
        self.api_key_name
            .update(cx, |i, cx| i.set_value(&auth.api_key_name, window, cx));
        self.api_key_value
            .update(cx, |i, cx| i.set_value(&auth.api_key_value, window, cx));
        cx.notify();
    }

    /// A labelled input row (label on the left, field filling the rest).
    fn field_row(
        label: &'static str,
        input: &Entity<InputState>,
        secret: bool,
        theme: &gpui_component::Theme,
    ) -> impl IntoElement {
        let input = if secret {
            Input::new(input).mask_toggle()
        } else {
            Input::new(input)
        };
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
            .child(div().flex_1().child(input))
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
                    .child(if self.auth_type_index == 3 && self.api_key_location == ApiKeyLocation::Query {
                        "The API key is added when sending and replaces query parameters with the same name."
                    } else {
                        "The auth header is added when the request is sent and overrides a manually-typed header of the same name."
                    }),
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
                this.child(Self::field_row("Token", &self.bearer_token, true, theme))
            })
            .when(self.auth_type_index == 2, |this| {
                this.child(Self::field_row(
                    "Username",
                    &self.basic_username,
                    false,
                    theme,
                ))
                .child(Self::field_row(
                    "Password",
                    &self.basic_password,
                    true,
                    theme,
                ))
            })
            .when(self.auth_type_index == 3, |this| {
                this.child(Self::field_row("Key", &self.api_key_name, false, theme))
                    .child(Self::field_row("Value", &self.api_key_value, true, theme))
                    .child(
                        h_flex().gap_3().items_center()
                            .child(div().w(px(120.)).flex_shrink_0().text_sm()
                                .text_color(theme.muted_foreground).child("Add to"))
                            .children(
                                [(ApiKeyLocation::Header, "Header"), (ApiKeyLocation::Query, "Query")]
                                    .into_iter().enumerate().map(|(index, (location, label))| {
                                        gpui_component::button::Button::new(("api-key-location", index))
                                            .label(label)
                                            .selected(self.api_key_location == location)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.api_key_location = location;
                                                cx.notify();
                                            }))
                                    }),
                            ),
                    )
            })
    }
}
