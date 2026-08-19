//! The environment selector shown in the title bar. Houses environment
//! switching (with a check mark on the active one) and an entry to open the
//! environment dialog. Item handlers call back into `PoopmanApp` via a captured
//! entity handle.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, IconName, Side, Sizable as _,
    button::Button,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use crate::app::PoopmanApp;
use crate::types::Environment;

const TRIGGER_LABEL_MAX_CHARS: usize = 20;
const MENU_LABEL_MAX_CHARS: usize = 40;

/// Collapse whitespace and cap user-provided environment names without slicing
/// through a UTF-8 code point. The returned string is presentation-only; the
/// stored environment name is never changed.
fn display_environment_name(name: &str, max_chars: usize) -> String {
    let normalized = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = if normalized.is_empty() {
        "Unnamed Environment"
    } else {
        normalized.as_str()
    };

    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized.to_string();
    }

    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "\u{2026}".to_string();
    }

    let mut compact = normalized.chars().take(max_chars - 1).collect::<String>();
    compact.push('\u{2026}');
    compact
}

fn active_environment(
    environments: &[Environment],
    active_id: Option<i64>,
) -> Option<&Environment> {
    active_id.and_then(|id| environments.iter().find(|environment| environment.id == id))
}

fn trigger_label(environments: &[Environment], active_id: Option<i64>) -> String {
    active_environment(environments, active_id)
        .map(|environment| display_environment_name(&environment.name, TRIGGER_LABEL_MAX_CHARS))
        .unwrap_or_else(|| "Environment".to_string())
}

fn section_label(label: &'static str) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        div()
            .w_full()
            .py_0p5()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .child(label)
    })
    .disabled(true)
}

/// Build the environment dropdown button for the title bar.
pub fn edit_menu(
    app: Entity<PoopmanApp>,
    environments: Vec<Environment>,
    active_id: Option<i64>,
) -> impl IntoElement {
    let trigger_label = trigger_label(&environments, active_id);
    Button::new("edit-menu")
        .small()
        .compact()
        .outline()
        .icon(IconName::Globe)
        .label(trigger_label)
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu
                .min_w(px(220.))
                .max_w(px(320.))
                .max_h(px(360.))
                .scrollable(true)
                .check_side(Side::Right)
                .item(section_label("ENVIRONMENTS"));

            if environments.is_empty() {
                menu = menu.item(PopupMenuItem::new("No saved environments").disabled(true));
            }

            for environment in &environments {
                let id = environment.id;
                let app = app.clone();
                let is_active = active_id == Some(id);
                menu = menu.item(
                    PopupMenuItem::new(display_environment_name(
                        &environment.name,
                        MENU_LABEL_MAX_CHARS,
                    ))
                    .icon(IconName::Globe)
                    .checked(is_active)
                    .on_click(move |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.set_active_environment(Some(id), window, cx);
                        });
                    }),
                );
            }

            menu = menu.separator();

            {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new("No Environment")
                        .icon(IconName::CircleX)
                        .checked(active_id.is_none())
                        .on_click(move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.set_active_environment(None, window, cx);
                            });
                        }),
                );
            }

            menu = menu.separator();

            {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new("Manage Environments\u{2026}")
                        .icon(IconName::Settings)
                        .on_click(move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.open_env_manager(window, cx);
                            });
                        }),
                );
            }

            menu
        })
}

#[cfg(test)]
mod tests {
    // Do not glob-import the parent: that would pull in `gpui::*`, whose
    // `test` attribute macro shadows Rust's built-in `#[test]`.
    use super::{display_environment_name, trigger_label};
    use crate::types::Environment;

    fn environment(id: i64, name: &str) -> Environment {
        Environment {
            id,
            name: name.to_string(),
            variables: vec![],
        }
    }

    #[test]
    fn trigger_uses_default_without_an_active_environment() {
        let environments = vec![environment(1, "Development")];

        assert_eq!(trigger_label(&environments, None), "Environment");
        assert_eq!(trigger_label(&environments, Some(99)), "Environment");
    }

    #[test]
    fn trigger_uses_the_active_environment_name() {
        let environments = vec![environment(1, "Development"), environment(2, "Staging")];

        assert_eq!(trigger_label(&environments, Some(2)), "Staging");
    }

    #[test]
    fn display_name_normalizes_whitespace_and_handles_blank_names() {
        assert_eq!(
            display_environment_name("  Team\n  Staging  ", 40),
            "Team Staging"
        );
        assert_eq!(display_environment_name(" \t ", 40), "Unnamed Environment");
    }

    #[test]
    fn display_name_truncates_unicode_on_character_boundaries() {
        let compact = display_environment_name("开发环境变量名称", 5);

        assert_eq!(compact, "开发环境\u{2026}");
        assert_eq!(compact.chars().count(), 5);
    }

    #[test]
    fn display_name_handles_tiny_limits() {
        assert_eq!(display_environment_name("Development", 1), "\u{2026}");
        assert_eq!(display_environment_name("Development", 0), "");
    }
}
