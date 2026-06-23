//! Shared visual primitives for the floating-card UI: panel cards and
//! segmented pill tab strips. All helpers are callback-free — callers attach
//! `.id(...)`/`.on_click(...)` themselves so the helpers stay generic.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{h_flex, Theme};

/// A floating panel card: white-ish surface, hairline border, large radius,
/// soft shadow, clipped contents. Wrap a panel's content in this.
pub fn card_panel(theme: &Theme) -> Div {
    div()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius_lg)
        .shadow_sm()
        .overflow_hidden()
}

/// The container for a segmented pill tab strip (muted rounded track).
pub fn segmented_bar(theme: &Theme) -> Div {
    h_flex()
        .gap_1()
        .p_0p5()
        .rounded(theme.radius_lg)
        .bg(theme.muted)
}

/// A single segment pill. Caller adds `.id(...)`, `.on_click(...)`, `.child(label)`.
/// Active pills sit on the card surface with a soft shadow; inactive are muted.
pub fn segment_pill(theme: &Theme, active: bool) -> Div {
    div()
        .px_3()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(theme.background)
                .text_color(theme.foreground)
                .font_weight(FontWeight::SEMIBOLD)
                .shadow_sm()
        })
        .when(!active, |d| d.text_color(theme.muted_foreground))
}
