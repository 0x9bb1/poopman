use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::px;
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::{Input, InputState, InputEvent as InputChangeEvent, TabSize},
    scroll::ScrollableElement as _,
    select::*, v_flex, ActiveTheme as _, IndexPath, Sizable as _,
};

use std::{collections::{HashMap, HashSet}, rc::Rc, time::Duration};

use crate::body_editor_assistance::{
    BodyDiagnosticSeverity, VariableCompletionProvider, compute_body_diagnostics,
};
use crate::types::{BodyDraft, BodyKind, BodyType, FormDataRow, FormDataValue, RawSubtype};

use gpui::Subscription;

/// Event emitted when body type changes, carrying the computed Content-Type
#[derive(Clone, Debug)]
pub struct BodyTypeChanged {
    pub content_type: Option<String>, // Some("application/json") or None for BodyType::None
}

/// Get appropriate placeholder text for each raw subtype
fn get_placeholder_for_subtype(subtype: RawSubtype) -> &'static str {
    match subtype {
        RawSubtype::Json => r#"{"key": "value"}"#,
        RawSubtype::Xml => r#"<root><element>value</element></root>"#,
        RawSubtype::Text => "Enter plain text here...",
        RawSubtype::JavaScript => "console.log('Hello, world!');",
        RawSubtype::UrlEncoded => "key=value&another=value",
    }
}

/// Per-row input entities for a form-data row: (key input, value input, type select).
type FormDataRowInputs = (Entity<InputState>, Entity<InputState>, Entity<SelectState<Vec<&'static str>>>);

pub struct BodyEditor {
    body_type_index: usize,
    raw_subtype_select: Entity<SelectState<Vec<&'static str>>>,
    raw_body_editor: Entity<InputState>,  // Single editor for all raw types
    current_raw_subtype: RawSubtype,      // Track current subtype
    formdata_rows: Vec<FormDataRow>,
    formdata_input_states: Vec<FormDataRowInputs>,
    /// Stable identities kept parallel with the row model and input entities.
    /// Event subscriptions capture these IDs, never mutable vector indices.
    formdata_row_ids: Vec<u64>,
    next_formdata_row_id: u64,
    formdata_scroll_handle: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    // Subscriptions owned by the current form-data rows. The raw subtype
    // subscription lives in `_subscriptions` and must survive loading tabs.
    _formdata_subscriptions: Vec<Subscription>,
    // Format/validation state
    validation_message: Option<String>,
    validation_error: bool,
    env_var_names: HashSet<String>,
    diagnostics_generation: u64,
}

impl BodyEditor {
    fn handle_input_event(
        &mut self,
        row_id: u64,
        is_key: bool,
        event: &InputChangeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputChangeEvent::Change = event
            && let Some(index) = self.formdata_row_index(row_id)
        {
            let (key_input, value_input, _) = &self.formdata_input_states[index];
            let value = if is_key { key_input } else { value_input }
                .read(cx)
                .value()
                .to_string();
            if is_key {
                self.update_formdata_key(index, value, cx);
            } else {
                self.update_formdata_value(index, value, cx);
            }
            self.normalize_trailing_formdata_row(window, cx);
        }
    }
}


impl BodyEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create Select for Raw subtypes
        let raw_subtype_select = cx.new(|cx| {
            SelectState::new(
                vec!["JSON", "XML", "Text", "JavaScript", "URL-encoded"],
                Some(IndexPath::default()), // Default to JSON
                window,
                cx,
            )
        });

        // Create single editor for all raw types (default to JSON)
        let current_raw_subtype = RawSubtype::Json;
        let raw_body_editor = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .code_editor(current_raw_subtype.as_str())
                .line_number(true)
                .indent_guides(true)
                .tab_size(TabSize { tab_size: 4, hard_tabs: false })
                .placeholder(get_placeholder_for_subtype(current_raw_subtype));
            input.lsp.completion_provider =
                Some(Rc::new(VariableCompletionProvider::new(Vec::new())));
            input
        });

        log::info!("Created single body editor with default language: 'json'");

        let mut editor = Self {
            body_type_index: 1, // Default to Raw
            raw_subtype_select: raw_subtype_select.clone(),
            raw_body_editor: raw_body_editor.clone(),
            current_raw_subtype,
            formdata_rows: vec![],
            formdata_input_states: vec![],
            formdata_row_ids: vec![],
            next_formdata_row_id: 1,
            formdata_scroll_handle: ScrollHandle::new(),
            _subscriptions: vec![],
            _formdata_subscriptions: vec![],
            validation_message: None,
            validation_error: false,
            env_var_names: HashSet::new(),
            diagnostics_generation: 0,
        };

        // Initialize with one empty form-data row for auto-add functionality
        editor.add_formdata_row(window, cx);

        // Subscribe to raw subtype changes to switch syntax highlighting
        let select_subscription = cx.subscribe_in(
            &raw_subtype_select,
            window,
            |this: &mut BodyEditor, _select, _event: &SelectEvent<Vec<&'static str>>, window, cx| {
                this.handle_subtype_change(window, cx);
            },
        );
        editor._subscriptions.push(select_subscription);
        let raw_input_subscription = cx.subscribe_in(
            &raw_body_editor,
            window,
            |this: &mut BodyEditor, _, event: &InputChangeEvent, window, cx| {
                if matches!(event, InputChangeEvent::Change) {
                    this.schedule_raw_diagnostics(true, window, cx);
                }
            },
        );
        editor._subscriptions.push(raw_input_subscription);

        editor
    }

    /// Replace completion candidates and re-check unknown variables whenever the
    /// active environment changes.
    pub fn set_env_vars(
        &mut self,
        vars: &HashMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.env_var_names = vars.keys().cloned().collect();
        let provider = VariableCompletionProvider::new(self.env_var_names.iter().cloned());
        self.raw_body_editor.update(cx, |input, _| {
            input.lsp.completion_provider = Some(Rc::new(provider));
        });
        self.schedule_raw_diagnostics(false, window, cx);
    }

    /// Coalesce typing, parse JSON off the UI thread, and discard stale results.
    fn schedule_raw_diagnostics(
        &mut self,
        debounce: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.diagnostics_generation = self.diagnostics_generation.wrapping_add(1);
        let generation = self.diagnostics_generation;

        cx.spawn_in(window, async move |this, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }

            // Snapshot only after the debounce. This avoids copying a 1 MB body
            // on every keystroke when a newer generation will supersede it.
            let snapshot = this.update(cx, |this, cx| {
                (this.diagnostics_generation == generation).then(|| {
                    (
                        this.raw_body_editor.read(cx).value().to_string(),
                        this.env_var_names.clone(),
                        this.current_raw_subtype == RawSubtype::Json,
                    )
                })
            })?;
            let Some((content, variable_names, validate_json)) = snapshot else {
                return Ok(());
            };

            let task = cx.background_spawn(async move {
                compute_body_diagnostics(&content, &variable_names, validate_json)
            });
            let diagnostics = task.await;

            this.update(cx, |this, cx| {
                if this.diagnostics_generation != generation {
                    return;
                }
                this.raw_body_editor.update(cx, |input, cx| {
                    let Some(target) = input.diagnostics_mut() else {
                        return;
                    };
                    target.clear();
                    target.extend(diagnostics.into_iter().map(|diagnostic| {
                        let severity = match diagnostic.severity {
                            BodyDiagnosticSeverity::Error => {
                                gpui_component::highlighter::DiagnosticSeverity::Error
                            }
                            BodyDiagnosticSeverity::Warning => {
                                gpui_component::highlighter::DiagnosticSeverity::Warning
                            }
                        };
                        gpui_component::highlighter::Diagnostic::new(
                            diagnostic.range,
                            diagnostic.message,
                        )
                        .with_severity(severity)
                        .with_source(diagnostic.source)
                    }));
                    cx.notify();
                });
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    }

    /// Handle raw subtype change - switch syntax highlighting and placeholder
    fn handle_subtype_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let subtype_index = self.raw_subtype_select
            .read(cx)
            .selected_index(cx)
            .map(|idx| idx.row)
            .unwrap_or(0);
        let new_subtype = RawSubtype::all()[subtype_index];

        if new_subtype != self.current_raw_subtype {
            log::info!("Switching body editor language from {:?} to {:?}",
                      self.current_raw_subtype, new_subtype);

            self.current_raw_subtype = new_subtype;
            self.raw_body_editor.update(cx, |state, cx| {
                state.set_highlighter(new_subtype.as_str(), cx);
                // Update placeholder based on subtype
                state.set_placeholder(get_placeholder_for_subtype(new_subtype), window, cx);
            });
            self.schedule_raw_diagnostics(false, window, cx);

            // Emit event to notify RequestEditor to update Content-Type header
            cx.emit(BodyTypeChanged {
                content_type: Some(new_subtype.content_type().to_string()),
            });

            cx.notify();
        }
    }

    /// Get current body type from UI state
    pub fn get_body(&self, cx: &App) -> BodyType {
        self.get_draft(cx).selected_body()
    }

    /// Snapshot both the active body and the inactive panel drafts for the
    /// request tab that currently owns this shared editor.
    pub fn get_draft(&self, cx: &App) -> BodyDraft {
        let formdata_rows = self
            .formdata_rows
            .iter()
            .zip(self.formdata_input_states.iter())
            .map(|(row, (key_input, value_input, _type_select))| {
                let mut updated_row = row.clone();
                updated_row.key = key_input.read(cx).value().to_string();
                let value = value_input.read(cx).value().to_string();
                updated_row.value = match &row.value {
                    FormDataValue::Text(_) => FormDataValue::Text(value),
                    FormDataValue::File { .. } => FormDataValue::File { path: value },
                };
                updated_row
            })
            // Never let the editor's auto-add placeholder cross a persistence,
            // export, tab, or send boundary.
            .filter(|row| !row.is_blank())
            .collect();

        BodyDraft {
            kind: match self.body_type_index {
                0 => BodyKind::None,
                1 => BodyKind::Raw,
                2 => BodyKind::FormData,
                _ => BodyKind::None,
            },
            raw_content: self.raw_body_editor.read(cx).value().to_string(),
            raw_subtype: self.current_raw_subtype,
            formdata_rows,
        }
    }

    /// Set body from loaded request
    pub fn set_body(&mut self, body: &BodyType, window: &mut Window, cx: &mut Context<Self>) {
        self.set_draft(&BodyDraft::from_body(body), window, cx);
    }

    /// Restore a tab's complete body draft and rebuild all row subscriptions.
    pub fn set_draft(&mut self, draft: &BodyDraft, window: &mut Window, cx: &mut Context<Self>) {
        self.body_type_index = match draft.kind {
            BodyKind::None => 0,
            BodyKind::Raw => 1,
            BodyKind::FormData => 2,
        };

        self.current_raw_subtype = draft.raw_subtype;
        let subtype_index = RawSubtype::all()
            .iter()
            .position(|subtype| *subtype == draft.raw_subtype)
            .unwrap_or(0);
        self.raw_subtype_select.update(cx, |select, cx| {
            select.set_selected_index(
                Some(IndexPath::default().row(subtype_index)),
                window,
                cx,
            );
        });
        self.raw_body_editor.update(cx, |input, cx| {
            input.set_value(&draft.raw_content, window, cx);
            input.set_highlighter(draft.raw_subtype.as_str(), cx);
            input.set_placeholder(
                get_placeholder_for_subtype(draft.raw_subtype),
                window,
                cx,
            );
        });
        self.schedule_raw_diagnostics(false, window, cx);

        self.formdata_rows.clear();
        self.formdata_input_states.clear();
        self.formdata_row_ids.clear();
        self._formdata_subscriptions.clear();
        for row in draft.formdata_rows.iter().filter(|row| !row.is_blank()) {
            self.add_formdata_row_with_value(row.clone(), window, cx);
        }
        self.normalize_trailing_formdata_row(window, cx);

        // Emit event after all state updates are complete
        let content_type = match draft.kind {
            BodyKind::None => None,
            BodyKind::Raw => Some(draft.raw_subtype.content_type().to_string()),
            BodyKind::FormData => Some("multipart/form-data; boundary=<auto>".to_string()),
        };

        cx.emit(BodyTypeChanged { content_type });
    }

    /// Calculate body content length
    pub fn calculate_length(&self, cx: &App) -> usize {
        match self.body_type_index {
            0 => 0, // None
            1 => {
                // Raw - read from single editor
                self.raw_body_editor.read(cx).value().len()
            }
            2 | 3 => 0, // Form-data and UrlEncoded - approximate
            _ => 0,
        }
    }

    // Form-data table methods
    fn add_formdata_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_formdata_row_with_value(FormDataRow {
            enabled: true,
            key: String::new(),
            value: FormDataValue::Text(String::new()),
        }, window, cx);
    }

    fn add_formdata_row_with_value(
        &mut self,
        row: FormDataRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row_id = self.next_formdata_row_id;
        self.next_formdata_row_id = self.next_formdata_row_id.wrapping_add(1);
        let key_value = row.key.clone();
        let value_string = match &row.value {
            FormDataValue::Text(value) => value.clone(),
            FormDataValue::File { path } => path.clone(),
        };
        let is_file = matches!(row.value, FormDataValue::File { .. });

        let key_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(&key_value, window, cx);
            input.set_placeholder("Key", window, cx);
            input
        });
        let value_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(&value_string, window, cx);
            input.set_placeholder(if is_file { "File Path" } else { "Value" }, window, cx);
            input
        });
        let type_select = cx.new(|cx| {
            SelectState::new(
                vec!["Text", "File"],
                Some(IndexPath::default().row(if is_file { 1 } else { 0 })),
                window,
                cx,
            )
        });

        self.formdata_rows.push(row);
        self.formdata_row_ids.push(row_id);
        self.formdata_input_states
            .push((key_input.clone(), value_input.clone(), type_select.clone()));

        self._formdata_subscriptions.push(cx.subscribe_in(
            &key_input,
            window,
            move |this, _, event: &InputChangeEvent, window, cx| {
                this.handle_input_event(row_id, true, event, window, cx);
            },
        ));
        self._formdata_subscriptions.push(cx.subscribe_in(
            &value_input,
            window,
            move |this, _, event: &InputChangeEvent, window, cx| {
                this.handle_input_event(row_id, false, event, window, cx);
            },
        ));

        let value_input_for_type = value_input.clone();
        self._formdata_subscriptions.push(cx.subscribe_in(
            &type_select,
            window,
            move |this, _entity, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                if let SelectEvent::Confirm(Some(selected_value)) = event {
                    let should_be_file = *selected_value == "File";
                    if let Some(index) = this.formdata_row_index(row_id) {
                        let current_is_file = matches!(
                            this.formdata_rows.get(index).map(|row| &row.value),
                            Some(FormDataValue::File { .. })
                        );
                        if should_be_file != current_is_file
                            && let Some(row) = this.formdata_rows.get_mut(index)
                        {
                            row.value = match &row.value {
                                FormDataValue::Text(text) => FormDataValue::File { path: text.clone() },
                                FormDataValue::File { path } => FormDataValue::Text(path.clone()),
                            };
                            value_input_for_type.update(cx, |input, cx| {
                                input.set_placeholder(
                                    if should_be_file { "File Path" } else { "Value" },
                                    window,
                                    cx,
                                );
                            });
                            cx.notify();
                        }
                    }
                }
            },
        ));

        cx.notify();
    }

    fn formdata_row_index(&self, row_id: u64) -> Option<usize> {
        crate::formdata::row_index(&self.formdata_row_ids, row_id)
    }

    /// Keep one (and only one) empty row after the last row containing data.
    fn normalize_trailing_formdata_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let desired_len = crate::formdata::desired_editor_len(&self.formdata_rows);

        if self.formdata_rows.len() > desired_len {
            self.formdata_rows.truncate(desired_len);
            self.formdata_input_states.truncate(desired_len);
            self.formdata_row_ids.truncate(desired_len);
        }
        while self.formdata_rows.len() < desired_len {
            self.add_formdata_row(window, cx);
        }
    }

    fn remove_formdata_row(
        &mut self,
        row_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.formdata_row_index(row_id) {
            self.formdata_rows.remove(index);
            self.formdata_input_states.remove(index);
            self.formdata_row_ids.remove(index);
            self.normalize_trailing_formdata_row(window, cx);
            cx.notify();
        }
    }

    fn toggle_formdata_row(&mut self, row_id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.formdata_row_index(row_id)
            && let Some(row) = self.formdata_rows.get_mut(index)
        {
            row.enabled = !row.enabled;
            cx.notify();
        }
    }

    fn update_formdata_key(&mut self, index: usize, new_key: String, cx: &mut Context<Self>) {
        if let Some(row) = self.formdata_rows.get_mut(index) {
            row.key = new_key;
            cx.notify();
        }
    }

    fn update_formdata_value(&mut self, index: usize, new_value: String, cx: &mut Context<Self>) {
        if let Some(row) = self.formdata_rows.get_mut(index) {
            row.value = match &row.value {
                FormDataValue::Text(_) => FormDataValue::Text(new_value),
                FormDataValue::File { .. } => FormDataValue::File { path: new_value },
            };
            cx.notify();
        }
    }

    /// Format current raw body content
    fn format_raw_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.raw_body_editor.read(cx).value().to_string();

        let result = match self.current_raw_subtype {
            RawSubtype::Json => crate::code_formatter::format_json(&content),
            RawSubtype::Xml => crate::code_formatter::format_xml(&content),
            _ => {
                self.validation_message = Some("Formatting not supported for this type".to_string());
                self.validation_error = true;
                cx.notify();
                return;
            }
        };

        match result {
            Ok(formatted) => {
                self.raw_body_editor.update(cx, |input, cx| {
                    input.set_value(&formatted, window, cx);
                });
                self.validation_message = Some(format!("{} formatted successfully", self.current_raw_subtype.as_str().to_uppercase()));
                self.validation_error = false;
            }
            Err(err) => {
                self.validation_message = Some(err);
                self.validation_error = true;
            }
        }
        cx.notify();
    }


    fn select_file_for_row(&mut self, row_id: u64, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let path = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a file".into()),
        });

        if let Some(index) = self.formdata_row_index(row_id)
            && let Some((_key_input, value_input, _type_select)) = self.formdata_input_states.get(index).cloned()
        {
            cx.spawn_in(window, async move |_, window| {
                if let Ok(Ok(Some(paths))) = path.await
                    && let Some(selected_path) = paths.first()
                {
                    // Store and display the full path (used directly when sending).
                    let path_str = selected_path.to_string_lossy().to_string();
                    let _ = window.update(|window, cx| {
                        value_input.update(cx, |input, cx| {
                            input.set_value(&path_str, window, cx);
                        });
                    });
                }
            })
            .detach();
        }
    }
}

impl Render for BodyEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .gap_3()
            .w_full()
            .flex_1()
            .min_h_0()  // Critical for scrolling to work in form-data
            .child(
                // Body type selector (custom muted radios) + Raw controls right-aligned
                h_flex()
                    .justify_between()
                    .items_center()
                    .w_full()
                    .child(
                        h_flex().gap_4().items_center().children(
                            ["none", "raw", "form-data"].into_iter().enumerate().map(|(i, label)| {
                                let selected = self.body_type_index == i;
                                h_flex()
                                    .id(("body-type", i))
                                    .gap_1p5()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.body_type_index = i;
                                        let content_type = match i {
                                            0 => None,
                                            1 => Some(this.current_raw_subtype.content_type().to_string()),
                                            2 => Some("multipart/form-data; boundary=<auto>".to_string()),
                                            _ => None,
                                        };
                                        cx.emit(BodyTypeChanged { content_type });
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
                                            .text_color(if selected {
                                                theme.foreground
                                            } else {
                                                theme.muted_foreground
                                            })
                                            .child(label),
                                    )
                            })
                        )
                        .when(self.body_type_index == 1, |this| {
                            // JSON subtype dropdown, slightly separated from the radios.
                            // Explicit menu_width so longer names (JavaScript) aren't truncated.
                            this.child(
                                div().ml_2().child(
                                    Select::new(&self.raw_subtype_select)
                                        .small()
                                        .appearance(false)
                                        .menu_width(px(130.)),
                                ),
                            )
                        }),
                    )
                    .child(
                        // Right-aligned action, like Postman's Beautify
                        h_flex().items_center().when(self.body_type_index == 1, |this| {
                            this.child(
                                Button::new("beautify-button")
                                    .small()
                                    .ghost()
                                    .label("Beautify")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.format_raw_body(window, cx);
                                    })),
                            )
                        }),
                    )
            )
            // Body content based on selected type
            .when(self.body_type_index == 0, |this| {
                // None - show placeholder
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .child("This request does not have a body")
                )
            })
            .when(self.body_type_index == 1, |this| {
                // Raw - use single editor with dynamic syntax highlighting
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .w_full()
                        .rounded(theme.radius_lg)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.popover)
                        .child(
                            Input::new(&self.raw_body_editor)
                                .rounded(theme.radius_lg)
                                .w_full()
                                .h_full()
                        )
                )
            })
            .when(self.body_type_index == 2, |this| {
                // Form-data - show table (like headers layout)
                this.child(
                    div()
                        .flex_1()
                        .min_h_0()  // Allow scrolling to work
                        .child(
                            v_flex()
                                .id("formdata-scroll-container")
                                .gap_2()
                                .p_2()
                                .pb_4()  // Bottom padding to prevent last row from being obscured
                                .size_full()
                                .track_scroll(&self.formdata_scroll_handle)
                                .overflow_scroll()
                                .children(self.formdata_rows.iter().zip(self.formdata_input_states.iter()).zip(self.formdata_row_ids.iter()).map(|((row, (key_input_entity, value_input_entity, type_select_entity)), row_id)| {
                                    let row_id = *row_id;
                                    let is_file = matches!(row.value, FormDataValue::File { .. });

                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .w_full()
                                        .child(
                                            // Checkbox - Enable/Disable row
                                            div().flex_shrink_0().child(
                                                Checkbox::new(("formdata-check", row_id))
                                                    .checked(row.enabled)
                                                    .on_click(cx.listener(move |this, _checked, _window, cx| {
                                                        this.toggle_formdata_row(row_id, cx);
                                                    }))
                                            )
                                        )
                                        .child(
                                            // Key Input - same flex_1 ratio as headers
                                            div()
                                                .flex_1()
                                                .child(
                                                    Input::new(key_input_entity)
                                                )
                                        )
                                        .child(
                                            // Value Input - same flex_1 ratio as headers
                                            // Type selector and Delete button embedded in suffix
                                            div()
                                                .flex_1()
                                                .child(
                                                    Input::new(value_input_entity)
                                                        .when(is_file, |input| input.disabled(true))
                                                        .suffix(
                                                            h_flex()
                                                                .gap_1()
                                                                .items_center()
                                                                .when(is_file, |this| {
                                                                    // Choose File button when in file mode
                                                                    this.child(
                                                                        Button::new(("choose-file", row_id))
                                                                            .xsmall()
                                                                            .label("Choose Files")
                                                                            .on_click(cx.listener(move |this, event, window, cx| {
                                                                                this.select_file_for_row(row_id, event, window, cx);
                                                                            }))
                                                                    )
                                                                })
                                                                .child(
                                                                    // Type selector
                                                                    Select::new(type_select_entity).xsmall()
                                                                )
                                                                .child(
                                                                    // Delete button
                                                                    Button::new(("delete-formdata", row_id))
                                                                        .ghost()
                                                                        .xsmall()
                                                                        .label("×")
                                                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                                                            this.remove_formdata_row(row_id, window, cx);
                                                                        }))
                                                                )
                                                        )
                                                )
                                        )
                                }))
                        )
                        .vertical_scrollbar(&self.formdata_scroll_handle),
                )
            })
    }
}

impl EventEmitter<BodyTypeChanged> for BodyEditor {}
