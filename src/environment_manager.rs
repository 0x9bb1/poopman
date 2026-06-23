//! Environment management UI (shown inside a Dialog): create/rename/delete
//! environments, edit their variables, and choose the active one. All mutations
//! are written to the DB immediately and an `EnvironmentsChanged` event is emitted
//! so `PoopmanApp` can reload and refresh the request editor's variable map.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::*, v_flex, ActiveTheme as _, Sizable as _,
};
use std::sync::Arc;

use crate::db::Database;
use crate::types::{Environment, EnvVar};

/// Emitted whenever environments or the active selection change, so the app reloads.
#[derive(Clone)]
pub struct EnvironmentsChanged;

struct VarRow {
    enabled: bool,
    key_input: Entity<InputState>,
    value_input: Entity<InputState>,
}

pub struct EnvironmentManager {
    db: Arc<Database>,
    environments: Vec<Environment>,
    active_id: Option<i64>,
    selected_id: Option<i64>,
    name_input: Entity<InputState>,
    var_rows: Vec<VarRow>,
}

impl EventEmitter<EnvironmentsChanged> for EnvironmentManager {}

impl EnvironmentManager {
    pub fn new(db: Arc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let environments = db.load_environments().unwrap_or_default();
        let active_id = db.get_active_environment_id().unwrap_or(None);
        let selected_id = environments.first().map(|e| e.id);
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));

        let mut this = Self {
            db,
            environments,
            active_id,
            selected_id,
            name_input,
            var_rows: vec![],
        };
        this.load_selected_into_editor(window, cx);
        this
    }

    fn reload(&mut self) {
        self.environments = self.db.load_environments().unwrap_or_default();
        self.active_id = self.db.get_active_environment_id().unwrap_or(None);
    }

    /// Populate name_input + var_rows from the currently selected environment.
    fn load_selected_into_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self
            .selected_id
            .and_then(|id| self.environments.iter().find(|e| e.id == id))
            .cloned();

        let name = selected.as_ref().map(|e| e.name.clone()).unwrap_or_default();
        self.name_input.update(cx, |input, cx| {
            input.set_value(&name, window, cx);
        });

        self.var_rows.clear();
        if let Some(env) = selected {
            for v in &env.variables {
                self.var_rows.push(self.make_var_row(v.enabled, &v.key, &v.value, window, cx));
            }
        }
    }

    fn make_var_row(
        &self,
        enabled: bool,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> VarRow {
        let key = key.to_string();
        let value = value.to_string();
        VarRow {
            enabled,
            key_input: cx.new(|cx| {
                let mut i = InputState::new(window, cx).placeholder("Key");
                i.set_value(&key, window, cx);
                i
            }),
            value_input: cx.new(|cx| {
                let mut i = InputState::new(window, cx).placeholder("Value");
                i.set_value(&value, window, cx);
                i
            }),
        }
    }

    fn select(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // Persist current edits before switching away.
        self.save(cx);
        self.selected_id = Some(id);
        self.load_selected_into_editor(window, cx);
        cx.notify();
    }

    fn add_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
        match self.db.create_environment("New Environment") {
            Ok(id) => {
                self.reload();
                self.selected_id = Some(id);
                self.load_selected_into_editor(window, cx);
                cx.emit(EnvironmentsChanged);
                cx.notify();
            }
            Err(e) => log::error!("Failed to create environment: {}", e),
        }
    }

    fn delete_environment(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.db.delete_environment(id) {
            log::error!("Failed to delete environment: {}", e);
            return;
        }
        self.reload();
        if self.selected_id == Some(id) {
            self.selected_id = self.environments.first().map(|e| e.id);
            self.load_selected_into_editor(window, cx);
        }
        cx.emit(EnvironmentsChanged);
        cx.notify();
    }

    fn set_active(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        if let Err(e) = self.db.set_active_environment_id(id) {
            log::error!("Failed to set active environment: {}", e);
            return;
        }
        self.active_id = id;
        cx.emit(EnvironmentsChanged);
        cx.notify();
    }

    /// Persist the currently selected environment's name + variables.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id else {
            return;
        };
        let name = self.name_input.read(cx).value().to_string();
        if !name.is_empty() {
            let _ = self.db.rename_environment(id, &name);
        }
        let vars: Vec<EnvVar> = self
            .var_rows
            .iter()
            .map(|r| EnvVar {
                enabled: r.enabled,
                key: r.key_input.read(cx).value().to_string(),
                value: r.value_input.read(cx).value().to_string(),
            })
            .filter(|v| !v.key.is_empty() || !v.value.is_empty())
            .collect();
        let _ = self.db.replace_variables(id, &vars);
    }

    fn save_and_notify(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
        self.reload();
        cx.emit(EnvironmentsChanged);
        cx.notify();
    }

    fn add_var_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.make_var_row(true, "", "", window, cx);
        self.var_rows.push(row);
        cx.notify();
    }

    fn remove_var_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.var_rows.len() {
            self.var_rows.remove(index);
            cx.notify();
        }
    }

    fn toggle_var(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.var_rows.get_mut(index) {
            row.enabled = !row.enabled;
            cx.notify();
        }
    }
}

impl Render for EnvironmentManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_id = self.selected_id;
        let active_id = self.active_id;

        h_flex()
            .w_full()
            .h(px(440.))
            // ---- Left: environment list ----
            .child(
                v_flex()
                    .w(px(190.))
                    .h_full()
                    .flex_shrink_0()
                    .pr_3()
                    .mr_3()
                    .border_r_1()
                    .border_color(theme.border)
                    .gap_0p5()
                    // "+ New environment" — same row geometry as the env rows below
                    // (full width, px_2/py_1p5, 6px leading column, gap_2) so they
                    // align in left edge, width, and height.
                    .child(
                        h_flex()
                            .id("env-add")
                            .w_full()
                            .px_2()
                            .py_1p5()
                            .gap_2()
                            .items_center()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_dashed()
                            .border_color(theme.primary)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.list_active))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_environment(window, cx);
                            }))
                            .child(
                                // indicator column (centered "+"), same width as env rows
                                div()
                                    .w(px(14.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(theme.primary)
                                    .child("+"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(theme.primary)
                                    .child("New environment"),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("env-list")
                            .flex_1()
                            .gap_0p5()
                            .overflow_scroll()
                            .children(self.environments.iter().map(|env| {
                                let id = env.id;
                                let is_selected = selected_id == Some(id);
                                let is_active = active_id == Some(id);
                                h_flex()
                                    .id(("env-row", id as u64))
                                    .w_full()
                                    .px_2()
                                    .py_1p5()
                                    .gap_2()
                                    .items_center()
                                    .rounded(theme.radius)
                                    .cursor_pointer()
                                    .when(is_selected, |s| s.bg(theme.list_active))
                                    .hover(|s| s.bg(theme.list_hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select(id, window, cx);
                                    }))
                                    .child(
                                        // Dot = activation toggle (stops row-select propagation)
                                        div()
                                            .id(("env-active-dot", id as u64))
                                            .w(px(16.))
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                cx.stop_propagation();
                                                let new = if this.active_id == Some(id) {
                                                    None
                                                } else {
                                                    Some(id)
                                                };
                                                this.set_active(new, cx);
                                            }))
                                            .child(
                                                div()
                                                    .w(px(7.))
                                                    .h(px(7.))
                                                    .rounded_full()
                                                    .when(is_active, |d| d.bg(theme.primary))
                                                    .when(!is_active, |d| {
                                                        d.border_1().border_color(theme.muted_foreground)
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .when(is_active, |d| {
                                                d.font_weight(FontWeight::SEMIBOLD)
                                            })
                                            .text_color(theme.foreground)
                                            .child(env.name.clone()),
                                    )
                                    .when(is_active, |row| {
                                        row.child(
                                            div()
                                                .flex_shrink_0()
                                                .px_1p5()
                                                .rounded(theme.radius)
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .bg(theme.primary.opacity(0.12))
                                                .text_color(theme.primary)
                                                .child("ACTIVE"),
                                        )
                                    })
                            })),
                    ),
            )
            // ---- Right: selected environment editor ----
            .child(if let Some(sel_id) = selected_id {
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .gap_3()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.name_input)))
                            .child(
                                Button::new("env-delete")
                                    .small()
                                    .ghost()
                                    .label("Delete")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.delete_environment(sel_id, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_color(theme.border)
                            .overflow_hidden()
                            .child(
                                // header strip
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .px_3()
                                    .py_1p5()
                                    .bg(theme.muted)
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(div().w(px(20.)).flex_shrink_0())
                                    .child(div().flex_1().child("KEY"))
                                    .child(div().flex_1().child("VALUE"))
                                    .child(div().w(px(24.)).flex_shrink_0()),
                            )
                            .child(
                                v_flex()
                                    .id("env-vars")
                                    .flex_1()
                                    .overflow_scroll()
                                    .children(self.var_rows.iter().enumerate().map(|(index, row)| {
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .items_center()
                                            .px_3()
                                            .py_1p5()
                                            .when(index % 2 == 1, |r| r.bg(theme.muted.opacity(0.4)))
                                            .border_t_1()
                                            .border_color(theme.border)
                                            .child(
                                                div().w(px(20.)).flex_shrink_0().flex().justify_center().child(
                                                    Checkbox::new(("var-check", index))
                                                        .checked(row.enabled)
                                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                                            this.toggle_var(index, cx);
                                                        })),
                                                ),
                                            )
                                            .child(div().flex_1().min_w_0().child(Input::new(&row.key_input)))
                                            .child(div().flex_1().min_w_0().child(Input::new(&row.value_input)))
                                            .child(
                                                div().w(px(24.)).flex_shrink_0().flex().justify_center().child(
                                                    Button::new(("var-del", index))
                                                        .ghost()
                                                        .xsmall()
                                                        .label("×")
                                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                                            this.remove_var_row(index, cx);
                                                        })),
                                                ),
                                            )
                                    })),
                            ),
                    )
                    .child(
                        Button::new("env-add-var")
                            .small()
                            .ghost()
                            .label("+ Add variable")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_var_row(window, cx);
                            })),
                    )
                    .child(
                        // Footer: only a right-aligned Save (no hint text).
                        h_flex()
                            .w_full()
                            .justify_end()
                            .pt_2()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(
                                Button::new("env-save")
                                    .small()
                                    .primary()
                                    .label("Save")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.save_and_notify(cx);
                                    })),
                            ),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .text_sm()
                    .child("No environments yet — create one")
                    .into_any_element()
            })
    }
}
