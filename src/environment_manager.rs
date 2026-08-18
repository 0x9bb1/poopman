//! Environment management UI (shown inside a Dialog): create/rename/delete
//! environments, edit their variables, and choose the active one. All mutations
//! are written to the DB immediately and an `EnvironmentsChanged` event is emitted
//! so `PoopmanApp` can reload and refresh the request editor's variable map.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::InputEvent;
use gpui_component::{
    ActiveTheme as _, Sizable as _, button::*, checkbox::Checkbox, h_flex, input::*,
    scroll::ScrollableElement as _, v_flex,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use crate::db::Database;
use crate::types::{EnvVar, Environment};

/// Emitted with the manager's current in-memory state. The UI never has to
/// synchronously read SQLite merely to reflect an edit it already owns.
#[derive(Clone)]
pub struct EnvironmentsChanged {
    pub environments: Vec<Environment>,
    pub active_id: Option<i64>,
}

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
    env_list_scroll_handle: ScrollHandle,
    var_list_scroll_handle: ScrollHandle,
    /// True while programmatically loading inputs, so their `Change` events don't
    /// trigger an auto-save of values we just set.
    suspend_autosave: bool,
    /// Invalidates delayed auto-saves when another keystroke arrives.
    save_generation: u64,
    /// Also checked on the database thread, closing the small race between a
    /// timer's foreground generation check and background task scheduling.
    save_epoch: Arc<AtomicU64>,
    /// Live input-change subscriptions (name + each var row), rewired on load.
    _subs: Vec<Subscription>,
}

impl EventEmitter<EnvironmentsChanged> for EnvironmentManager {}

impl EnvironmentManager {
    pub fn new(
        db: Arc<Database>,
        environments: Vec<Environment>,
        active_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_id = environments.first().map(|e| e.id);
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));

        let mut this = Self {
            db,
            environments,
            active_id,
            selected_id,
            name_input,
            var_rows: vec![],
            env_list_scroll_handle: ScrollHandle::new(),
            var_list_scroll_handle: ScrollHandle::new(),
            suspend_autosave: false,
            save_generation: 0,
            save_epoch: Arc::new(AtomicU64::new(0)),
            _subs: vec![],
        };
        this.load_selected_into_editor(window, cx);
        this
    }

    /// (Re)subscribe to the name + variable inputs so any edit auto-saves.
    fn wire_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.clone();
        let inputs: Vec<Entity<InputState>> = self
            .var_rows
            .iter()
            .flat_map(|r| [r.key_input.clone(), r.value_input.clone()])
            .collect();

        let mut subs = Vec::with_capacity(inputs.len() + 1);
        subs.push(
            cx.subscribe_in(&name, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) && !this.suspend_autosave {
                    this.commit(window, cx);
                }
            }),
        );
        for input in &inputs {
            subs.push(
                cx.subscribe_in(input, window, |this, _, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::Change) && !this.suspend_autosave {
                        this.commit(window, cx);
                    }
                }),
            );
        }
        // Assigning drops the previous subscriptions (unsubscribing stale inputs).
        self._subs = subs;
    }

    fn changed_event(&self) -> EnvironmentsChanged {
        EnvironmentsChanged {
            environments: self.environments.clone(),
            active_id: self.active_id,
        }
    }

    /// Populate name_input + var_rows from the currently selected environment.
    fn load_selected_into_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self
            .selected_id
            .and_then(|id| self.environments.iter().find(|e| e.id == id))
            .cloned();

        // Programmatic set_value below would otherwise auto-save the values we're
        // loading; suspend autosave for the duration.
        self.suspend_autosave = true;

        let name = selected
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_default();
        self.name_input.update(cx, |input, cx| {
            input.set_value(&name, window, cx);
        });

        self.var_rows.clear();
        if let Some(env) = selected {
            for v in &env.variables {
                self.var_rows
                    .push(self.make_var_row(v.enabled, &v.key, &v.value, window, cx));
            }
        }

        self.wire_inputs(window, cx);
        self.suspend_autosave = false;
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
        // Edits update the in-memory model immediately and persist in the
        // background, so selection never needs a read-back round trip.
        self.selected_id = Some(id);
        self.load_selected_into_editor(window, cx);
        cx.notify();
    }

    fn add_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let task = cx.background_spawn(async move { db.create_environment("New Environment") });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(id) => {
                    this.update_in(cx, |this, window, cx| {
                        this.environments.push(Environment {
                            id,
                            name: "New Environment".to_string(),
                            variables: Vec::new(),
                        });
                        this.selected_id = Some(id);
                        this.load_selected_into_editor(window, cx);
                        cx.emit(this.changed_event());
                        cx.notify();
                    })?;
                }
                Err(error) => log::error!("Failed to create environment: {}", error),
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn delete_environment(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.save_generation = self.save_generation.wrapping_add(1);
        self.save_epoch
            .store(self.save_generation, Ordering::Release);
        let was_active = self.active_id == Some(id);
        let db = self.db.clone();
        let task = cx.background_spawn(async move {
            db.delete_environment(id)?;
            if was_active {
                db.set_active_environment_id(None)?;
            }
            Ok::<_, anyhow::Error>(())
        });
        cx.spawn_in(window, async move |this, cx| {
            match task.await {
                Ok(()) => {
                    this.update_in(cx, |this, window, cx| {
                        this.environments.retain(|environment| environment.id != id);
                        if was_active {
                            this.active_id = None;
                        }
                        if this.selected_id == Some(id) {
                            this.selected_id = this.environments.first().map(|env| env.id);
                            this.load_selected_into_editor(window, cx);
                        }
                        cx.emit(this.changed_event());
                        cx.notify();
                    })?;
                }
                Err(error) => log::error!("Failed to delete environment: {}", error),
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    pub(crate) fn set_active(
        &mut self,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.active_id;
        self.active_id = id;
        cx.emit(self.changed_event());
        cx.notify();

        let db = self.db.clone();
        let task = cx.background_spawn(async move { db.set_active_environment_id(id) });
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = task.await {
                log::error!("Failed to set active environment: {}", error);
                this.update(cx, |this, cx| {
                    if this.active_id == id {
                        this.active_id = previous;
                        cx.emit(this.changed_event());
                        cx.notify();
                    }
                })?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    /// Snapshot the editor into the local model before persistence.
    fn snapshot_selected(&mut self, cx: &mut Context<Self>) -> Option<(i64, String, Vec<EnvVar>)> {
        let Some(id) = self.selected_id else {
            return None;
        };
        let name = self.name_input.read(cx).value().to_string();
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
        if let Some(environment) = self.environments.iter_mut().find(|env| env.id == id) {
            if !name.is_empty() {
                environment.name = name.clone();
            }
            environment.variables = vars.clone();
        }
        Some((id, name, vars))
    }

    /// Update the UI immediately, then coalesce rapid edits into one background
    /// persistence job. The generation check prevents stale timers from writing.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, name, vars)) = self.snapshot_selected(cx) else {
            return;
        };
        self.save_generation = self.save_generation.wrapping_add(1);
        let generation = self.save_generation;
        self.save_epoch.store(generation, Ordering::Release);
        cx.emit(self.changed_event());
        cx.notify();

        let db = self.db.clone();
        let save_epoch = self.save_epoch.clone();
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

            let task = cx.background_spawn(async move {
                db.save_environment_if_current(id, &name, &vars, save_epoch, generation)
            });
            if let Err(error) = task.await {
                log::error!("Failed to save environment: {}", error);
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn add_var_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.make_var_row(true, "", "", window, cx);
        self.var_rows.push(row);
        // The new row is empty (not yet persisted), but its inputs need change
        // subscriptions so typing into them auto-saves.
        self.wire_inputs(window, cx);
        cx.notify();
    }

    fn remove_var_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.var_rows.len() {
            self.var_rows.remove(index);
            self.commit(window, cx);
        }
    }

    fn toggle_var(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.var_rows.get_mut(index) {
            row.enabled = !row.enabled;
            self.commit(window, cx);
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
                                    .w(px(16.))
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
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(
                                v_flex()
                                    .id("env-list")
                                    .size_full()
                                    .gap_0p5()
                                    .track_scroll(&self.env_list_scroll_handle)
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
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                let new = if this.active_id == Some(id) {
                                                    None
                                                } else {
                                                    Some(id)
                                                };
                                                this.set_active(new, window, cx);
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
                        )
                        .vertical_scrollbar(&self.env_list_scroll_handle),
                    ),
            )
            // ---- Right: selected environment editor ----
            .child(if let Some(sel_id) = selected_id {
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_h_0()
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
                        // Inline card (no shadow/bg) — it sits inside the dialog surface, so card_panel's elevation would be wrong here.
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
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        v_flex()
                                            .id("env-vars")
                                            .size_full()
                                            .track_scroll(&self.var_list_scroll_handle)
                                            .overflow_scroll()
                                            .children(self.var_rows.iter().enumerate().map(|(index, row)| {
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .items_center()
                                            .px_3()
                                            .py_1p5()
                                            .when(index % 2 == 1, |r| r.bg(theme.muted.opacity(0.4)))
                                            .when(index > 0, |r| r.border_t_1().border_color(theme.border))
                                            .child(
                                                div().w(px(20.)).flex_shrink_0().flex().justify_center().child(
                                                    Checkbox::new(("var-check", index))
                                                        .checked(row.enabled)
                                                        .on_click(cx.listener(move |this, _, window, cx| {
                                                            this.toggle_var(index, window, cx);
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
                                                        .on_click(cx.listener(move |this, _, window, cx| {
                                                            this.remove_var_row(index, window, cx);
                                                        })),
                                                ),
                                            )
                                    })),
                                    )
                                    .vertical_scrollbar(&self.var_list_scroll_handle),
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
