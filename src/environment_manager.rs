//! Environment management UI (shown inside a Dialog): create/rename/delete
//! environments, edit their variables, and choose the active one. All mutations
//! are written to the DB immediately and an `EnvironmentsChanged` event is emitted
//! so `PoopmanApp` can reload and refresh the request editor's variable map.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::InputEvent;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, button::*, checkbox::Checkbox, h_flex, input::*,
    scroll::ScrollableElement as _, v_flex,
};
use std::time::Duration;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

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

const DIALOG_HORIZONTAL_MARGIN: f32 = 16.;
const DIALOG_MAX_WIDTH: f32 = 780.;
const DIALOG_MIN_WIDTH: f32 = 520.;
const DIALOG_MIN_CONTENT_HEIGHT: f32 = 300.;
const DIALOG_MAX_CONTENT_HEIGHT: f32 = 520.;
// Dialog title, gaps, and vertical padding consume approximately this much.
const DIALOG_CHROME_HEIGHT: f32 = 104.;

/// Viewport-derived dimensions shared by the dialog wrapper and its content.
/// Keeping this pure makes the compact-window contract directly testable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EnvironmentDialogGeometry {
    pub width: Pixels,
    pub content_height: Pixels,
    pub margin_top: Pixels,
    pub rail_width: Pixels,
}

pub(crate) fn environment_dialog_geometry(
    viewport_width: Pixels,
    viewport_height: Pixels,
) -> EnvironmentDialogGeometry {
    let available_width =
        (viewport_width - px(DIALOG_HORIZONTAL_MARGIN * 2.)).max(px(DIALOG_MIN_WIDTH));
    let width = available_width.min(px(DIALOG_MAX_WIDTH));
    let content_height = (viewport_height - px(160.))
        .max(px(DIALOG_MIN_CONTENT_HEIGHT))
        .min(px(DIALOG_MAX_CONTENT_HEIGHT));
    let estimated_height = content_height + px(DIALOG_CHROME_HEIGHT);
    let margin_top = ((viewport_height - estimated_height) / 2.).max(px(16.));
    let rail_width = if width < px(720.) { px(178.) } else { px(210.) };

    EnvironmentDialogGeometry {
        width,
        content_height,
        margin_top,
        rail_width,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveStatus {
    Saved,
    Saving,
    Failed,
    InvalidName,
}

impl SaveStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Saved => "All changes saved",
            Self::Saving => "Saving changes…",
            Self::Failed => "Changes could not be saved",
            Self::InvalidName => "Environment name is required",
        }
    }
}

struct EnvironmentSaveState {
    generation: u64,
    epoch: Arc<AtomicU64>,
    status: SaveStatus,
}

impl Default for EnvironmentSaveState {
    fn default() -> Self {
        Self {
            generation: 0,
            epoch: Arc::new(AtomicU64::new(0)),
            status: SaveStatus::Saved,
        }
    }
}

/// Tracks debounce generations per environment. A save started for environment
/// A must remain current even if the user immediately edits environment B.
#[derive(Default)]
struct SaveTracker {
    environments: HashMap<i64, EnvironmentSaveState>,
}

impl SaveTracker {
    fn begin(&mut self, id: i64, status: SaveStatus) -> (u64, Arc<AtomicU64>) {
        let state = self.environments.entry(id).or_default();
        state.generation = state.generation.wrapping_add(1);
        state.epoch.store(state.generation, Ordering::Release);
        state.status = status;
        (state.generation, state.epoch.clone())
    }

    fn is_current(&self, id: i64, generation: u64) -> bool {
        self.environments
            .get(&id)
            .is_some_and(|state| state.generation == generation)
    }

    fn finish(&mut self, id: i64, generation: u64, status: SaveStatus) -> bool {
        let Some(state) = self.environments.get_mut(&id) else {
            return false;
        };
        if state.generation != generation {
            return false;
        }
        state.status = status;
        true
    }

    fn set_status(&mut self, id: i64, status: SaveStatus) {
        self.environments.entry(id).or_default().status = status;
    }

    fn status(&self, id: i64) -> SaveStatus {
        self.environments
            .get(&id)
            .map(|state| state.status)
            .unwrap_or(SaveStatus::Saved)
    }

    fn invalidate(&mut self, id: i64) {
        if let Some(state) = self.environments.remove(&id) {
            state.epoch.fetch_add(1, Ordering::AcqRel);
        }
    }
}

fn environment_name_is_valid(name: &str) -> bool {
    !name.trim().is_empty()
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
    /// Independent per-environment debounce and save status.
    save_tracker: SaveTracker,
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
            save_tracker: SaveTracker::default(),
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
                // Environment values frequently contain tokens or credentials.
                // Keep them private during screen sharing, with an explicit
                // reveal control in the rendered input.
                let mut i = InputState::new(window, cx)
                    .placeholder("Value")
                    .masked(true);
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
                        this.save_tracker.set_status(id, SaveStatus::Saved);
                        cx.emit(this.changed_event());
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    log::error!("Failed to create environment: {}", error);
                    this.update(cx, |this, cx| {
                        if let Some(id) = this.selected_id {
                            this.save_tracker.set_status(id, SaveStatus::Failed);
                        }
                        cx.notify();
                    })?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    fn delete_environment(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.save_tracker.invalidate(id);
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
                Err(error) => {
                    log::error!("Failed to delete environment: {}", error);
                    this.update(cx, |this, cx| {
                        this.save_tracker.set_status(id, SaveStatus::Failed);
                        cx.notify();
                    })?;
                }
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
        let id = self.selected_id?;
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
            if environment_name_is_valid(&name) {
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
        let name_is_valid = environment_name_is_valid(&name);
        let pending_status = if name_is_valid {
            SaveStatus::Saving
        } else {
            SaveStatus::InvalidName
        };
        let (generation, save_epoch) = self.save_tracker.begin(id, pending_status);
        cx.emit(self.changed_event());
        cx.notify();

        let db = self.db.clone();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let is_current = this
                .update(cx, |this, _| this.save_tracker.is_current(id, generation))
                .unwrap_or(false);
            if !is_current {
                return Ok(());
            }

            let task = cx.background_spawn(async move {
                db.save_environment_if_current(id, &name, &vars, save_epoch, generation)
            });
            let result = task.await;
            if let Err(error) = &result {
                log::error!("Failed to save environment: {}", error);
            }
            this.update(cx, |this, cx| {
                let status = if result.is_ok() {
                    if name_is_valid {
                        SaveStatus::Saved
                    } else {
                        SaveStatus::InvalidName
                    }
                } else {
                    SaveStatus::Failed
                };
                if this.save_tracker.finish(id, generation, status) {
                    cx.notify();
                }
            })?;
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_id = self.selected_id;
        let active_id = self.active_id;
        let viewport = window.viewport_size();
        let geometry = environment_dialog_geometry(viewport.width, viewport.height);
        let save_status = selected_id
            .map(|id| self.save_tracker.status(id))
            .unwrap_or(SaveStatus::Saved);
        let status_color = match save_status {
            SaveStatus::Saved => theme.success,
            SaveStatus::Saving => theme.warning,
            SaveStatus::Failed | SaveStatus::InvalidName => theme.danger,
        };

        h_flex()
            .w_full()
            .h(geometry.content_height)
            .gap_4()
            // ---- Left: environment list ----
            .child(
                crate::ui::inset_panel(theme)
                    .flex()
                    .flex_col()
                    .w(geometry.rail_width)
                    .h_full()
                    .flex_shrink_0()
                    .p_2()
                    .gap_2()
                    .bg(theme.muted.opacity(0.58))
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .px_1()
                            .child(crate::ui::section_label(theme, "Environments"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(self.environments.len().to_string()),
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
                                let active_icon = Icon::empty()
                                    .path(if is_active {
                                        "icons/check-circle.svg"
                                    } else {
                                        "icons/circle.svg"
                                    })
                                    .text_color(if is_active {
                                        theme.primary
                                    } else {
                                        theme.muted_foreground
                                    });
                                h_flex()
                                    .id(("env-row", id as u64))
                                    .w_full()
                                    .pr_2()
                                    .py_1()
                                    .gap_1()
                                    .items_center()
                                    .rounded(theme.radius)
                                    .border_1()
                                    .border_color(if is_selected {
                                        theme.list_active_border
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .when(is_selected, |s| s.bg(theme.list_active))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.list_hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select(id, window, cx);
                                    }))
                                    .child(
                                        Button::new(("env-active", id as u64))
                                            .small()
                                            .ghost()
                                            .icon(active_icon)
                                            .tooltip(if is_active {
                                                "Disable this environment"
                                            } else {
                                                "Use this environment"
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                let new = if this.active_id == Some(id) {
                                                    None
                                                } else {
                                                    Some(id)
                                                };
                                                this.set_active(new, window, cx);
                                            })),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_sm()
                                            .text_color(theme.foreground)
                                            .child(env.name.clone()),
                                    )
                            })),
                        )
                        .vertical_scrollbar(&self.env_list_scroll_handle),
                    )
                    .child(
                        Button::new("env-add")
                            .small()
                            .primary()
                            .outline()
                            .w_full()
                            .icon(Icon::empty().path("icons/plus.svg"))
                            .label("New environment")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_environment(window, cx);
                            })),
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
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(crate::ui::section_label(theme, "Environment name"))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(Input::new(&self.name_input)),
                                    )
                                    .child(
                                        Button::new("env-delete")
                                            .small()
                                            .danger()
                                            .outline()
                                            .icon(Icon::empty().path("icons/trash.svg"))
                                            .label("Delete")
                                            .tooltip("Delete this environment")
                                            .on_click(cx.listener(
                                                move |this, _, window, cx| {
                                                    this.delete_environment(
                                                        sel_id, window, cx,
                                                    );
                                                },
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        crate::ui::inset_panel(theme)
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .px_3()
                                    .py_2()
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.foreground)
                                            .child("Variables"),
                                    )
                                    .child(
                                        div()
                                            .px_1p5()
                                            .rounded_full()
                                            .bg(theme.muted)
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(self.var_rows.len().to_string()),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("env-add-var")
                                            .small()
                                            .primary()
                                            .outline()
                                            .icon(Icon::empty().path("icons/plus.svg"))
                                            .label("Add variable")
                                            .on_click(cx.listener(
                                                |this, _, window, cx| {
                                                    this.add_var_row(window, cx);
                                                },
                                            )),
                                    ),
                            )
                            .when(self.var_rows.is_empty(), |panel| {
                                panel.child(crate::ui::empty_state(
                                    theme,
                                    "No variables yet",
                                    "Add a key and value to use it as {{variable}} in requests.",
                                ))
                            })
                            .when(!self.var_rows.is_empty(), |panel| {
                                panel
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .items_center()
                                            .px_3()
                                            .py_1p5()
                                            .bg(theme.muted.opacity(0.72))
                                            .child(div().w(px(24.)).flex_shrink_0())
                                            .child(
                                                crate::ui::section_label(theme, "Key")
                                                    .flex_1(),
                                            )
                                            .child(
                                                crate::ui::section_label(theme, "Value")
                                                    .flex_1(),
                                            )
                                            .child(div().w(px(28.)).flex_shrink_0()),
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
                                                    .children(self.var_rows.iter().enumerate().map(
                                                        |(index, row)| {
                                                            h_flex()
                                                                .w_full()
                                                                .gap_2()
                                                                .items_center()
                                                                .px_3()
                                                                .py_1()
                                                                .when(index > 0, |r| {
                                                                    r.border_t_1()
                                                                        .border_color(theme.border)
                                                                })
                                                                .child(
                                                                    div()
                                                                        .w(px(24.))
                                                                        .flex_shrink_0()
                                                                        .flex()
                                                                        .justify_center()
                                                                        .child(
                                                                            Checkbox::new((
                                                                                "var-check",
                                                                                index,
                                                                            ))
                                                                            .checked(row.enabled)
                                                                            .on_click(cx.listener(
                                                                                move |this, _, window, cx| {
                                                                                    this.toggle_var(index, window, cx);
                                                                                },
                                                                            )),
                                                                        ),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .flex_1()
                                                                        .min_w_0()
                                                                        .child(
                                                                            Input::new(&row.key_input)
                                                                                .small(),
                                                                        ),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .flex_1()
                                                                        .min_w_0()
                                                                        .child(
                                                                            Input::new(&row.value_input)
                                                                                .small()
                                                                                .mask_toggle(),
                                                                        ),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .w(px(28.))
                                                                        .flex_shrink_0()
                                                                        .flex()
                                                                        .justify_center()
                                                                        .child(
                                                                            Button::new((
                                                                                "var-del",
                                                                                index,
                                                                            ))
                                                                            .ghost()
                                                                            .xsmall()
                                                                            .icon(
                                                                                Icon::empty()
                                                                                    .path("icons/trash.svg")
                                                                                    .text_color(theme.muted_foreground),
                                                                            )
                                                                            .tooltip("Remove variable")
                                                                            .on_click(cx.listener(
                                                                                move |this, _, window, cx| {
                                                                                    this.remove_var_row(index, window, cx);
                                                                                },
                                                                            )),
                                                                        ),
                                                                )
                                                        },
                                                    )),
                                            )
                                            .vertical_scrollbar(&self.var_list_scroll_handle),
                                    )
                            }),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_1()
                            .child(
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(status_color),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if matches!(
                                        save_status,
                                        SaveStatus::Failed | SaveStatus::InvalidName
                                    ) {
                                        theme.danger
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(save_status.label()),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Changes save automatically"),
                            ),
                    )
                    .into_any_element()
            } else {
                crate::ui::inset_panel(theme)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .child(crate::ui::empty_state(
                        theme,
                        "No environments yet",
                        "Create one to group reusable request variables.",
                    ))
                    .into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{SaveStatus, SaveTracker, environment_dialog_geometry, environment_name_is_valid};
    use gpui::px;
    use std::sync::atomic::Ordering;

    #[test]
    fn desktop_dialog_uses_readable_maximum_dimensions() {
        let geometry = environment_dialog_geometry(px(1920.), px(1040.));
        assert_eq!(geometry.width, px(780.));
        assert_eq!(geometry.content_height, px(520.));
        assert_eq!(geometry.rail_width, px(210.));
        assert!(geometry.margin_top >= px(16.));
    }

    #[test]
    fn compact_dialog_stays_inside_the_viewport() {
        let viewport_width = px(720.);
        let viewport_height = px(480.);
        let geometry = environment_dialog_geometry(viewport_width, viewport_height);

        assert_eq!(geometry.width, px(688.));
        assert_eq!(geometry.content_height, px(320.));
        assert_eq!(geometry.rail_width, px(178.));
        assert!(geometry.width + px(32.) <= viewport_width);
        assert!(
            geometry.margin_top + geometry.content_height + px(104.) <= viewport_height - px(16.)
        );
    }

    #[test]
    fn medium_dialog_scales_before_reaching_its_caps() {
        let geometry = environment_dialog_geometry(px(900.), px(640.));
        assert_eq!(geometry.width, px(780.));
        assert_eq!(geometry.content_height, px(480.));
    }

    #[test]
    fn save_status_copy_is_specific() {
        assert_eq!(SaveStatus::Saved.label(), "All changes saved");
        assert_eq!(SaveStatus::Saving.label(), "Saving changes…");
        assert_eq!(SaveStatus::Failed.label(), "Changes could not be saved");
        assert_eq!(
            SaveStatus::InvalidName.label(),
            "Environment name is required"
        );
    }

    #[test]
    fn save_generations_are_independent_between_environments() {
        let mut tracker = SaveTracker::default();
        let (a_generation, a_epoch) = tracker.begin(10, SaveStatus::Saving);
        let (b_generation, b_epoch) = tracker.begin(20, SaveStatus::Saving);

        assert!(tracker.is_current(10, a_generation));
        assert!(tracker.is_current(20, b_generation));
        assert_eq!(a_epoch.load(Ordering::Acquire), a_generation);
        assert_eq!(b_epoch.load(Ordering::Acquire), b_generation);

        assert!(tracker.finish(10, a_generation, SaveStatus::Saved));
        assert_eq!(tracker.status(10), SaveStatus::Saved);
        assert_eq!(tracker.status(20), SaveStatus::Saving);
    }

    #[test]
    fn stale_save_cannot_overwrite_the_latest_status() {
        let mut tracker = SaveTracker::default();
        let (stale_generation, _) = tracker.begin(10, SaveStatus::Saving);
        let (current_generation, _) = tracker.begin(10, SaveStatus::Saving);

        assert!(!tracker.finish(10, stale_generation, SaveStatus::Failed));
        assert_eq!(tracker.status(10), SaveStatus::Saving);
        assert!(tracker.finish(10, current_generation, SaveStatus::Saved));
        assert_eq!(tracker.status(10), SaveStatus::Saved);
    }

    #[test]
    fn invalidating_an_environment_cancels_its_database_epoch() {
        let mut tracker = SaveTracker::default();
        let (generation, epoch) = tracker.begin(10, SaveStatus::Saving);

        tracker.invalidate(10);

        assert!(!tracker.is_current(10, generation));
        assert_ne!(epoch.load(Ordering::Acquire), generation);
    }

    #[test]
    fn blank_environment_names_are_rejected() {
        assert!(!environment_name_is_valid(""));
        assert!(!environment_name_is_valid(" \t\n "));
        assert!(environment_name_is_valid("Development"));
    }
}
