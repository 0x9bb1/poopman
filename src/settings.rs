//! App-wide HTTP safety settings. The panel owns only the editable controls;
//! the live value is shared with the request editor, so a successful edit takes
//! effect for the very next request without restarting the app.

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, IndexPath, Sizable as _, h_flex,
    input::{Input, InputEvent as InputChangeEvent, InputState},
    scroll::ScrollableElement as _,
    select::{Select, SelectState},
    v_flex,
};

use crate::{db::Database, types::AppSettings};

const MAX_TIMEOUT_MS: u64 = 3_600_000;
const MIN_RESPONSE_LIMIT_MIB: u64 = 1;
const MAX_RESPONSE_LIMIT_MIB: u64 = 2_048;
const MIN_PANEL_HEIGHT: f32 = 320.;
const MAX_PANEL_HEIGHT: f32 = 550.;
// Dialog title, padding, and margins consume approximately this much height.
const DIALOG_CHROME_HEIGHT: f32 = 108.;

fn settings_panel_height(viewport_height: Pixels) -> Pixels {
    (viewport_height - px(DIALOG_CHROME_HEIGHT))
        .max(px(MIN_PANEL_HEIGHT))
        .min(px(MAX_PANEL_HEIGHT))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveStatus {
    Saved,
    Saving,
    Invalid,
    Failed,
}

impl SaveStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Saved => "Changes are saved automatically",
            Self::Saving => "Saving changes…",
            Self::Invalid => "Use a positive whole number within the supported range",
            Self::Failed => "Changes could not be saved",
        }
    }
}

/// The General page rendered inside Poopman's Settings dialog.
pub struct SettingsPanel {
    db: Arc<Database>,
    settings: Arc<RwLock<AppSettings>>,
    http_version: Entity<SelectState<Vec<&'static str>>>,
    connect_timeout: Entity<InputState>,
    read_timeout: Entity<InputState>,
    total_timeout: Entity<InputState>,
    response_limit: Entity<InputState>,
    content_scroll_handle: ScrollHandle,
    status: SaveStatus,
    save_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPanel {
    pub fn new(
        db: Arc<Database>,
        settings: Arc<RwLock<AppSettings>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let current = settings
            .read()
            .expect("settings lock poisoned")
            .clone()
            .normalized();
        let http_version = cx
            .new(|cx| SelectState::new(vec!["Automatic"], Some(IndexPath::default()), window, cx));
        let connect_timeout = number_input(
            &current.connect_timeout_ms.to_string(),
            "10,000",
            window,
            cx,
        );
        let read_timeout = number_input(&current.read_timeout_ms.to_string(), "30,000", window, cx);
        let total_timeout =
            number_input(&current.total_timeout_ms.to_string(), "60,000", window, cx);
        let response_limit = number_input(
            &current.response_limit_mebibytes().to_string(),
            "50",
            window,
            cx,
        );

        let mut panel = Self {
            db,
            settings,
            http_version,
            connect_timeout,
            read_timeout,
            total_timeout,
            response_limit,
            content_scroll_handle: ScrollHandle::new(),
            status: SaveStatus::Saved,
            save_generation: 0,
            _subscriptions: vec![],
        };
        panel.wire_inputs(window, cx);
        panel
    }

    fn wire_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = [
            self.connect_timeout.clone(),
            self.read_timeout.clone(),
            self.total_timeout.clone(),
            self.response_limit.clone(),
        ];
        self._subscriptions = inputs
            .iter()
            .map(|input| {
                cx.subscribe_in(
                    input,
                    window,
                    |this, _, event: &InputChangeEvent, window, cx| {
                        if matches!(event, InputChangeEvent::Change) {
                            this.commit(window, cx);
                        }
                    },
                )
            })
            .collect();
    }

    fn value(input: &Entity<InputState>, cx: &App) -> Option<u64> {
        input
            .read(cx)
            .value()
            .trim()
            .replace(',', "")
            .parse::<u64>()
            .ok()
    }

    fn form_settings(&self, cx: &App) -> Option<AppSettings> {
        let connect_timeout_ms = Self::value(&self.connect_timeout, cx)?;
        let read_timeout_ms = Self::value(&self.read_timeout, cx)?;
        let total_timeout_ms = Self::value(&self.total_timeout, cx)?;
        let response_limit_mib = Self::value(&self.response_limit, cx)?;
        if !(1..=MAX_TIMEOUT_MS).contains(&connect_timeout_ms)
            || !(1..=MAX_TIMEOUT_MS).contains(&read_timeout_ms)
            || !(1..=MAX_TIMEOUT_MS).contains(&total_timeout_ms)
            || !(MIN_RESPONSE_LIMIT_MIB..=MAX_RESPONSE_LIMIT_MIB).contains(&response_limit_mib)
        {
            return None;
        }
        Some(AppSettings {
            connect_timeout_ms,
            read_timeout_ms,
            total_timeout_ms,
            max_response_size_bytes: response_limit_mib * 1024 * 1024,
        })
    }

    /// Apply a valid value immediately to the shared runtime settings, then
    /// coalesce rapid typing into one SQLite write. A generation check prevents
    /// an older debounce from overwriting a newer value.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self.form_settings(cx) else {
            self.status = SaveStatus::Invalid;
            cx.notify();
            return;
        };
        *self.settings.write().expect("settings lock poisoned") = settings.clone();
        self.save_generation = self.save_generation.wrapping_add(1);
        let generation = self.save_generation;
        self.status = SaveStatus::Saving;
        cx.notify();

        let db = self.db.clone();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let is_current = this
                .update(cx, |this, _| this.save_generation == generation)
                .unwrap_or(false);
            if !is_current {
                return Ok(());
            }
            let save = cx.background_spawn(async move { db.save_app_settings(&settings) });
            let result = save.await;
            this.update(cx, |this, cx| {
                if this.save_generation != generation {
                    return;
                }
                this.status = if result.is_ok() {
                    SaveStatus::Saved
                } else {
                    log::error!("Failed to save settings: {}", result.unwrap_err());
                    SaveStatus::Failed
                };
                cx.notify();
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn timeout_row(
        theme: &gpui_component::Theme,
        label: &'static str,
        description: &'static str,
        input: Entity<InputState>,
        top_border: bool,
    ) -> Div {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_6()
            .px_5()
            .py_3()
            .when(top_border, |row| {
                row.border_t_1().border_color(theme.border)
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(description),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .child(div().w(px(126.)).child(Input::new(&input).small()))
                    .child(
                        div()
                            .w(px(28.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("ms"),
                    ),
            )
    }
}

fn number_input(
    value: &str,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<SettingsPanel>,
) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(move |cx| {
        let mut input = InputState::new(window, cx).placeholder(placeholder);
        input.set_value(&value, window, cx);
        input
    })
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let panel_height = settings_panel_height(window.viewport_size().height);
        let (status_color, status_text_color) = match self.status {
            SaveStatus::Saved => (theme.success, theme.muted_foreground),
            SaveStatus::Saving => (theme.warning, theme.muted_foreground),
            SaveStatus::Invalid | SaveStatus::Failed => (theme.danger, theme.danger),
        };

        v_flex()
            .w_full()
            .h(panel_height)
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_3()
                    .px_1()
                    .child(
                        div()
                            .px_3()
                            .py_1p5()
                            .rounded(theme.radius)
                            .bg(theme.primary.opacity(0.12))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.primary)
                            .child("General"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Request defaults and safeguards"),
                    ),
            )
            .child(
                crate::ui::inset_panel(theme)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("settings-general-scroll")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .track_scroll(&self.content_scroll_handle)
                            .overflow_scroll()
                            .child(
                        div()
                            .px_5()
                            .pt_4()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("REQUEST"),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_6()
                            .px_5()
                            .py_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.foreground)
                                            .child("HTTP version"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child("Choose the protocol preference used for outgoing requests."),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(172.))
                                    .flex_shrink_0()
                                    .child(Select::new(&self.http_version)),
                            ),
                    )
                    .child(
                        div()
                            .px_5()
                            .pt_3()
                            .border_t_1()
                            .border_color(theme.border)
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("TIMEOUTS"),
                    )
                    .child(Self::timeout_row(
                        theme,
                        "Connect timeout",
                        "Stop if a connection cannot be established in time.",
                        self.connect_timeout.clone(),
                        false,
                    ))
                    .child(Self::timeout_row(
                        theme,
                        "Read idle timeout",
                        "Stop when a connected server stops sending response data.",
                        self.read_timeout.clone(),
                        true,
                    ))
                    .child(Self::timeout_row(
                        theme,
                        "Total request timeout",
                        "A final limit for the complete request, including download time.",
                        self.total_timeout.clone(),
                        true,
                    ))
                    .child(
                        div()
                            .px_5()
                            .pt_3()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .border_t_1()
                            .border_color(theme.border)
                            .child("RESPONSE LIMITS & DOWNLOADS"),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_6()
                            .px_5()
                            .py_4()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.foreground)
                                            .child("Viewer response limit"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child("Decoded responses above this limit stop safely. Download streams directly to disk."),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .flex_shrink_0()
                                    .child(
                                        div()
                                            .w(px(126.))
                                            .child(Input::new(&self.response_limit).small()),
                                    )
                                    .child(
                                        div()
                                            .w(px(28.))
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child("MiB"),
                                    ),
                            ),
                    ),
                    )
                    .vertical_scrollbar(&self.content_scroll_handle),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .child(div().size(px(7.)).rounded_full().bg(status_color))
                    .child(
                        div()
                            .text_xs()
                            .text_color(status_text_color)
                            .child(self.status.label()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Use Download beside Send for large files"),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_RESPONSE_LIMIT_MIB, MIN_RESPONSE_LIMIT_MIB, settings_panel_height};
    use gpui::px;

    #[test]
    fn response_limit_bounds_cover_a_useful_range() {
        assert_eq!(MIN_RESPONSE_LIMIT_MIB, 1);
        assert!(MAX_RESPONSE_LIMIT_MIB >= 1_024);
    }

    #[test]
    fn settings_panel_height_tracks_compact_viewports_and_caps_on_desktop() {
        assert_eq!(settings_panel_height(px(1_040.)), px(550.));
        assert_eq!(settings_panel_height(px(658.)), px(550.));
        assert_eq!(settings_panel_height(px(480.)), px(372.));
        assert_eq!(settings_panel_height(px(300.)), px(320.));
    }
}
