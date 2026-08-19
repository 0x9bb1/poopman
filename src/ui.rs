//! Shared visual primitives for the floating-card UI: panel cards and
//! segmented pill tab strips. All helpers are callback-free — callers attach
//! `.id(...)`/`.on_click(...)` themselves so the helpers stay generic.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{Theme, h_flex, v_flex};

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

/// A quiet panel for grouping controls inside a dialog or another card.
///
/// Unlike [`card_panel`], this has no shadow: nested elevation made the old
/// environment dialog look like a stack of unrelated floating surfaces.
pub fn inset_panel(theme: &Theme) -> Div {
    div()
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius_lg)
        .overflow_hidden()
}

/// Consistent small heading for sections inside dialogs and side rails.
pub fn section_label(theme: &Theme, label: impl Into<SharedString>) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(label.into())
}

/// A compact empty state that avoids leaving an unexplained blank panel.
pub fn empty_state(
    theme: &Theme,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> Div {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_1()
        .px_4()
        .text_center()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(title.into()),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(description.into()),
        )
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
