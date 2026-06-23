//! The Edit menu shown in the title bar. Houses environment switching (with a
//! check mark on the active one) and an entry to open the environment dialog.
//! Item handlers call back into `PoopmanApp` via a captured entity handle.

use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    Sizable as _,
};

use crate::app::PoopmanApp;
use crate::types::Environment;

/// Build the "Edit" dropdown button for the title bar.
pub fn edit_menu(
    app: Entity<PoopmanApp>,
    environments: Vec<Environment>,
    active_id: Option<i64>,
) -> impl IntoElement {
    Button::new("edit-menu")
        .ghost()
        .small()
        .label("Edit")
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu.label("Environment");

            for env in &environments {
                let id = env.id;
                let app = app.clone();
                let is_active = active_id == Some(id);
                menu = menu.item(
                    PopupMenuItem::new(env.name.clone())
                        .checked(is_active)
                        .on_click(move |_, _window, cx| {
                            app.update(cx, |app, cx| {
                                app.set_active_environment(Some(id), cx);
                            });
                        }),
                );
            }

            {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new("No Environment")
                        .checked(active_id.is_none())
                        .on_click(move |_, _window, cx| {
                            app.update(cx, |app, cx| {
                                app.set_active_environment(None, cx);
                            });
                        }),
                );
            }

            menu = menu.separator();

            {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new("Manage Environments\u{2026}").on_click(
                        move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.open_env_manager(window, cx);
                            });
                        },
                    ),
                );
            }

            menu
        })
}
