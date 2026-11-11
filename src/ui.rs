//! Shared visual primitives for the floating-card UI: panel cards and
//! segmented pill tab strips. All helpers are callback-free — callers attach
//! `.id(...)`/`.on_click(...)` themselves so the helpers stay generic.
//! Also the app-wide error toast, so every background failure looks the same.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::notification::Notification;
use gpui_component::{Theme, WindowExt as _, h_flex, v_flex};

/// Report a failed background operation the way Postman does: a toast that
/// hides itself, never a modal. Only pass database or file-system errors —
/// transport errors can embed a resolved URL carrying environment secrets.
pub fn error_toast(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    error: impl std::fmt::Display,
) {
    let title = title.into();
    log::error!("{title}: {error}");
    window.push_notification(Notification::error(error.to_string()).title(title), cx);
}

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

/// Turns a click-only element into a keyboard control.
///
/// - `track_focus` puts it in the Tab order when `tab_stop` is set. Members of
///   a group pass `tab_stop` only for the selected one, so the group is a
///   single stop and arrow keys move inside it (see [`rove_focus`]).
/// - Enter and Space then activate it for free: gpui sends a keyboard
///   `ClickEvent` to a focused element's `on_click` (`gpui/elements/div.rs`).
/// - The focus ring is a spread shadow, so it takes no layout space.
///
/// `track_focus` also makes gpui focus the element on mouse-down. That is
/// suppressed, as gpui-component's `Button` does: clicking leaves focus where
/// it was, and the ring only ever follows the keyboard.
pub fn keyboard_control(
    element: Stateful<Div>,
    focus: &FocusHandle,
    tab_stop: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let ring = BoxShadow {
        color: theme.ring,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
    };
    element
        .track_focus(&focus.clone().tab_stop(tab_stop))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .focus(move |style| style.shadow(vec![ring]))
}

/// Arrow-key navigation inside a group of [`keyboard_control`]s. If one of
/// `handles` has focus and `event` is an unmodified arrow key, focuses its
/// neighbour (wrapping) and returns the neighbour's index.
pub fn rove_focus(
    handles: &[FocusHandle],
    event: &KeyDownEvent,
    window: &mut Window,
) -> Option<usize> {
    if event.keystroke.modifiers.modified() {
        return None;
    }
    let forward = match event.keystroke.key.as_str() {
        "right" | "down" => true,
        "left" | "up" => false,
        _ => return None,
    };
    let current = handles
        .iter()
        .position(|handle| handle.is_focused(window))?;
    let next = cycle_index(current, handles.len(), forward);
    handles[next].focus(window);
    Some(next)
}

/// The index one step from `current` in a list of `len`, wrapping at both ends.
pub fn cycle_index(current: usize, len: usize, forward: bool) -> usize {
    if len <= 1 {
        return current;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

#[cfg(test)]
mod tests {
    // NOT `use super::*`: that would pull in `gpui::*`, whose `test` attribute
    // macro shadows the standard `#[test]`.
    use super::cycle_index;

    #[test]
    fn steps_forward_through_the_middle_of_the_list() {
        assert_eq!(cycle_index(0, 3, true), 1);
        assert_eq!(cycle_index(1, 3, true), 2);
    }

    #[test]
    fn wraps_forward_past_the_last_item() {
        assert_eq!(cycle_index(2, 3, true), 0);
    }

    #[test]
    fn steps_backward_through_the_middle_of_the_list() {
        assert_eq!(cycle_index(2, 3, false), 1);
        assert_eq!(cycle_index(1, 3, false), 0);
    }

    #[test]
    fn wraps_backward_past_the_first_item() {
        assert_eq!(cycle_index(0, 3, false), 2);
    }

    #[test]
    fn single_item_stays_put_in_both_directions() {
        assert_eq!(cycle_index(0, 1, true), 0);
        assert_eq!(cycle_index(0, 1, false), 0);
    }

    #[test]
    fn empty_list_returns_current_without_panicking() {
        assert_eq!(cycle_index(0, 0, true), 0);
        assert_eq!(cycle_index(0, 0, false), 0);
    }
}
