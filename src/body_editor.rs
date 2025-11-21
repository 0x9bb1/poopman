use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::px;
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::{Input, InputState, InputEvent as InputChangeEvent}, radio::RadioGroup,
    select::*, v_flex, ActiveTheme as _, IndexPath, Sizable as _,
};
use std::collections::HashMap;

use crate::types::{BodyType, FormDataRow, FormDataValue, RawSubtype};

use gpui::Subscription;

pub struct BodyEditor {
    body_type_index: usize,
    raw_subtype_select: Entity<SelectState<Vec<&'static str>>>,
    raw_inputs: HashMap<RawSubtype, Entity<InputState>>,
    formdata_rows: Vec<FormDataRow>,
    formdata_input_states: Vec<(Entity<InputState>, Entity<InputState>, Entity<SelectState<Vec<&'static str>>>)>,
    formdata_scroll_handle: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl BodyEditor {
    fn handle_input_event(
        &mut self,
        state_entity: Entity<InputState>,
        event: &InputChangeEvent,
        cx: &mut Context<Self>,
    ) {
        if let InputChangeEvent::Change = event {
            if let Some((index, (key_input, _value_input, _type_select))) = self
                .formdata_input_states
                .iter()
                .enumerate()
                .find(|(_, (k, v, _))| k.entity_id() == state_entity.entity_id() || v.entity_id() == state_entity.entity_id())
            {
                let is_key = key_input.entity_id() == state_entity.entity_id();
                let value = state_entity.read(cx).value().to_string();
                if is_key {
                    self.update_formdata_key(index, value, cx);
                } else {
                    self.update_formdata_value(index, value, cx);
                }
            }
        }
    }
}


impl BodyEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create Select for Raw subtypes
        let raw_subtype_select = cx.new(|cx| {
            SelectState::new(
                vec!["JSON", "XML", "Text", "JavaScript"],
                Some(IndexPath::default()), // Default to JSON
                window,
                cx,
            )
        });

        // Create input editors for each raw subtype
        let mut raw_inputs = HashMap::new();
        for subtype in RawSubtype::all() {
            let placeholder = match subtype {
                RawSubtype::Json => r#"{"key": "value"}"#,
                RawSubtype::Xml => r#"<root></root>"#,
                RawSubtype::Text => "Plain text...",
                RawSubtype::JavaScript => "console.log('Hello');",
            };

            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor(subtype.as_str())
                    .line_number(true)
                    .multi_line()
                    .placeholder(placeholder)
            });
            raw_inputs.insert(subtype, input);
        }

        let mut editor = Self {
            body_type_index: 1, // Default to Raw
            raw_subtype_select,
            raw_inputs,
            formdata_rows: vec![],
            formdata_input_states: vec![],
            formdata_scroll_handle: ScrollHandle::new(),
            _subscriptions: vec![],
        };

        // Initialize with one empty form-data row for auto-add functionality
        editor.add_formdata_row(window, cx);

        editor
    }

    /// Get current body type from UI state
    pub fn get_body(&self, cx: &App) -> BodyType {
        match self.body_type_index {
            0 => BodyType::None,
            1 => {
                // Raw
                let subtype_index = self.raw_subtype_select
                    .read(cx)
                    .selected_index(cx)
                    .map(|idx| idx.row)
                    .unwrap_or(0);
                let subtype = RawSubtype::all()[subtype_index];
                let content = self.raw_inputs.get(&subtype)
                    .map(|input| input.read(cx).value().to_string())
                    .unwrap_or_default();
                BodyType::Raw { content, subtype }
            }
            2 => {
                // Form-data
                // Update formdata_rows with current input values
                let mut updated_formdata_rows = Vec::new();
                for (index, row) in self.formdata_rows.iter().enumerate() {
                    let (key_input, value_input, _type_select) = &self.formdata_input_states[index];
                    let mut updated_row = row.clone();

                    updated_row.key = key_input.read(cx).value().to_string();
                    let value = value_input.read(cx).value().to_string();
                    updated_row.value = match &row.value {
                        FormDataValue::Text(_) => FormDataValue::Text(value),
                        FormDataValue::File { .. } => FormDataValue::File { path: value },
                    };
                    updated_formdata_rows.push(updated_row);
                }
                BodyType::FormData(updated_formdata_rows)
            }
            _ => BodyType::None,
        }
    }

    /// Set body from loaded request
    pub fn set_body(&mut self, body: &BodyType, window: &mut Window, cx: &mut Context<Self>) {
        match body {
            BodyType::None => {
                self.body_type_index = 0;
            }
            BodyType::Raw { content, subtype } => {
                self.body_type_index = 1;
                let subtype_index = RawSubtype::all().iter().position(|s| s == subtype).unwrap_or(0);
                self.raw_subtype_select.update(cx, |select, cx| {
                    select.set_selected_index(Some(IndexPath::default().row(subtype_index)), window, cx);
                });
                if let Some(input) = self.raw_inputs.get(subtype) {
                    input.update(cx, |input, cx| {
                        input.set_value(content, window, cx);
                    });
                }
            }
            BodyType::FormData(rows) => {
                self.body_type_index = 2;
                self.formdata_rows = rows.clone();
                // Clear existing input states and subscriptions
                self.formdata_input_states.clear();
                self._subscriptions.clear();

                // Create new input states for each row
                for (row_index, row) in rows.iter().enumerate() {
                    let key_value = row.key.clone();
                    let value_str = match &row.value {
                        FormDataValue::Text(t) => t.clone(),
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
                        input.set_value(&value_str, window, cx);
                        input.set_placeholder(if is_file { "File Path" } else { "Value" }, window, cx);
                        input
                    });

                    // Add type selector
                    let type_select = cx.new(|cx| {
                        SelectState::new(
                            vec!["Text", "File"],
                            Some(IndexPath::default().row(if is_file { 1 } else { 0 })),
                            window,
                            cx,
                        )
                    });

                    self._subscriptions.push(
                        cx.subscribe(&key_input, Self::handle_input_event)
                    );
                    self._subscriptions.push(
                        cx.subscribe(&value_input, Self::handle_input_event)
                    );

                    // Subscribe to type selector changes
                    self._subscriptions.push(
                        cx.subscribe(&type_select, move |this, _entity, event: &SelectEvent<Vec<&'static str>>, cx| {
                            if let SelectEvent::Confirm(Some(selected_value)) = event {
                                let should_be_file = *selected_value == "File";
                                let current_is_file = matches!(
                                    this.formdata_rows.get(row_index).map(|r| &r.value),
                                    Some(FormDataValue::File { .. })
                                );
                                if should_be_file != current_is_file {
                                    // We need window here but subscribe doesn't provide it
                                    // So we'll just update the data model, the UI will react
                                    if let Some(row) = this.formdata_rows.get_mut(row_index) {
                                        row.value = match &row.value {
                                            FormDataValue::Text(text) => FormDataValue::File { path: text.clone() },
                                            FormDataValue::File { path } => FormDataValue::Text(path.clone()),
                                        };
                                    }
                                    cx.notify();
                                }
                            }
                        })
                    );

                    self.formdata_input_states.push((key_input, value_input, type_select));
                }

                // Add one empty row at the end for auto-add functionality
                self.add_formdata_row(window, cx);
            }
        }
    }

    /// Calculate body content length
    pub fn calculate_length(&self, cx: &App) -> usize {
        match self.body_type_index {
            0 => 0, // None
            1 => {
                // Raw
                let subtype_index = self.raw_subtype_select
                    .read(cx)
                    .selected_index(cx)
                    .map(|idx| idx.row)
                    .unwrap_or(0);
                let subtype = RawSubtype::all()[subtype_index];
                self.raw_inputs.get(&subtype)
                    .map(|input| input.read(cx).value().len())
                    .unwrap_or(0)
            }
            2 | 3 => 0, // Form-data and UrlEncoded - approximate
            _ => 0,
        }
    }

    // Form-data table methods
    fn add_formdata_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row_index = self.formdata_rows.len();

        self.formdata_rows.push(FormDataRow {
            enabled: true,
            key: String::new(),
            value: FormDataValue::Text(String::new()),
        });
        // Add new InputStates for key and value
        let key_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Key")
        });
        let value_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Value")
        });

        // Add type selector (Text/File)
        let type_select = cx.new(|cx| {
            SelectState::new(
                vec!["Text", "File"],
                Some(IndexPath::default()), // Default to Text
                window,
                cx,
            )
        });

        // Clone for closure capture (auto-add logic)
        let key_input_for_auto_add = key_input.clone();

        // Subscribe to key input with auto-add logic (like headers)
        let auto_add_sub = cx.subscribe_in(&key_input, window, move |this, _, _event: &InputChangeEvent, window, cx| {
            // Check if this is the last row and it has content
            if let Some((last_key, _, _)) = this.formdata_input_states.last() {
                let has_key = !last_key.read(cx).value().is_empty();
                // Verify this is the last row by comparing entity IDs
                if has_key &&
                   this.formdata_input_states.last().map(|(k, _, _)| Entity::entity_id(k)) ==
                   Some(Entity::entity_id(&key_input_for_auto_add))
                {
                    // Auto-add a new empty row
                    this.add_formdata_row(window, cx);

                    // Scroll to bottom after adding new row
                    let scroll_handle = this.formdata_scroll_handle.clone();
                    cx.spawn_in(window, async move |_this, cx| {
                        // Wait for layout to stabilize by checking max_offset changes
                        let mut last_offset = px(0.);
                        let mut stable_count = 0;

                        for _ in 0..20 {  // Max 20 attempts (~20ms)
                            cx.background_executor().timer(std::time::Duration::from_millis(1)).await;

                            let current = scroll_handle.max_offset().height;
                            if (current - last_offset).abs() < px(0.1) {
                                stable_count += 1;
                                if stable_count >= 2 {
                                    // Offset stable for 2 checks, layout likely complete
                                    break;
                                }
                            } else {
                                stable_count = 0;
                            }
                            last_offset = current;
                        }

                        // Scroll to bottom
                        let _ = cx.update(|_, _cx| {
                            let max_offset = scroll_handle.max_offset();
                            scroll_handle.set_offset(point(px(0.), -max_offset.height));
                        });
                    }).detach();

                    cx.notify();
                }
            }
        });
        self._subscriptions.push(auto_add_sub);

        // Subscribe to inputs for data model updates
        self._subscriptions
            .push(cx.subscribe(&key_input, Self::handle_input_event));
        self._subscriptions
            .push(cx.subscribe(&value_input, Self::handle_input_event));

        // Subscribe to type selector changes
        self._subscriptions.push(
            cx.subscribe(&type_select, move |this, _entity, event: &SelectEvent<Vec<&'static str>>, cx| {
                if let SelectEvent::Confirm(Some(selected_value)) = event {
                    let should_be_file = *selected_value == "File";
                    let current_is_file = matches!(
                        this.formdata_rows.get(row_index).map(|r| &r.value),
                        Some(FormDataValue::File { .. })
                    );
                    if should_be_file != current_is_file {
                        // Update the data model directly
                        if let Some(row) = this.formdata_rows.get_mut(row_index) {
                            row.value = match &row.value {
                                FormDataValue::Text(text) => FormDataValue::File { path: text.clone() },
                                FormDataValue::File { path } => FormDataValue::Text(path.clone()),
                            };
                        }
                        cx.notify();
                    }
                }
            })
        );

        self.formdata_input_states
            .push((key_input, value_input, type_select));

        cx.notify();
    }

    fn remove_formdata_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.formdata_rows.len() {
            self.formdata_rows.remove(index);
            self.formdata_input_states.remove(index); // Remove corresponding input states
            cx.notify();
        }
    }

    fn toggle_formdata_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.formdata_rows.get_mut(index) {
            row.enabled = !row.enabled;
            cx.notify();
        }
    }

    fn toggle_formdata_type(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.formdata_rows.get_mut(index) {
            row.value = match &row.value {
                FormDataValue::Text(text) => FormDataValue::File { path: text.clone() },
                FormDataValue::File { path } => FormDataValue::Text(path.clone()),
            };
            let (_, value_input, _type_select) = &self.formdata_input_states[index];
            let is_file = matches!(row.value, FormDataValue::File { .. });
            value_input.update(cx, |input, cx| {
                input.set_placeholder(
                    if is_file { "File Path" } else { "Value" },
                    window,
                    cx,
                );
            });
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

    fn select_file_for_row(&mut self, index: usize, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let path = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a file".into()),
        });

        if let Some((_key_input, value_input, _type_select)) = self.formdata_input_states.get(index).cloned() {
            cx.spawn_in(window, async move |_, window| {
                if let Ok(Ok(Some(paths))) = path.await {
                    if let Some(selected_path) = paths.iter().next() {
                        // Store full path but display only filename
                        let path_str = selected_path.to_string_lossy().to_string();
                        let _ = window.update(|window, cx| {
                            value_input.update(cx, |input, cx| {
                                input.set_value(&path_str, window, cx);
                            });
                        });
                    }
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
                // Body type selector (RadioGroup) with Raw subtype dropdown always visible
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        RadioGroup::horizontal("body-type")
                            .children(vec!["None", "Raw", "Form-data"])
                            .selected_index(Some(self.body_type_index))
                            .on_click(cx.listener(|this, selected_ix: &usize, _window, cx| {
                                this.body_type_index = *selected_ix;
                                cx.notify();
                            }))
                    )
                    .when(self.body_type_index == 1, |this| {
                        // Raw subtype dropdown - only show when Raw is selected
                        this.child(
                            div()
                                .w(px(120.))
                                .child(Select::new(&self.raw_subtype_select))
                        )
                    })
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
                // Raw - show code editor for selected subtype
                this.child(
                    div()
                        .flex_1()
                        .w_full()
                        .flex()
                        .flex_col()
                        .min_h_0()  // Allow child to shrink and scroll
                        .child({
                            let subtype_index = self.raw_subtype_select
                                .read(cx)
                                .selected_index(cx)
                                .map(|idx| idx.row)
                                .unwrap_or(0);
                            let subtype = RawSubtype::all()[subtype_index];
                            if let Some(input) = self.raw_inputs.get(&subtype) {
                                Input::new(input).w_full().h_full()
                            } else {
                                // Fallback
                                Input::new(self.raw_inputs.values().next().unwrap()).w_full().h_full()
                            }
                        })
                )
            })
            .when(self.body_type_index == 2, |this| {
                // Form-data - show table (like headers layout)
                this.child(
                    v_flex()
                        .id("formdata-scroll-container")
                        .gap_2()
                        .p_2()
                        .pb_4()  // Bottom padding to prevent last row from being obscured
                        .flex_1()
                        .min_h_0()  // Allow scrolling to work
                        .w_full()
                        .track_scroll(&self.formdata_scroll_handle)
                        .overflow_scroll()
                        .children(self.formdata_rows.iter().enumerate().zip(self.formdata_input_states.iter()).map(|((index, row), (key_input_entity, value_input_entity, type_select_entity))| {
                                    let is_file = matches!(row.value, FormDataValue::File { .. });

                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .w_full()
                                        .child(
                                            // Checkbox - Enable/Disable row
                                            div().flex_shrink_0().child(
                                                Checkbox::new(("formdata-check", index))
                                                    .checked(row.enabled)
                                                    .on_click(cx.listener(move |this, _checked, _window, cx| {
                                                        this.toggle_formdata_row(index, cx);
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
                                                                        Button::new(("choose-file", index))
                                                                            .xsmall()
                                                                            .label("Choose Files")
                                                                            .on_click(cx.listener(move |this, event, window, cx| {
                                                                                this.select_file_for_row(index, event, window, cx);
                                                                            }))
                                                                    )
                                                                })
                                                                .child(
                                                                    // Type selector
                                                                    Select::new(type_select_entity).xsmall()
                                                                )
                                                                .child(
                                                                    // Delete button
                                                                    Button::new(("delete-formdata", index))
                                                                        .ghost()
                                                                        .xsmall()
                                                                        .label("×")
                                                                        .on_click(cx.listener(move |this, _event, _window, cx| {
                                                                            this.remove_formdata_row(index, cx);
                                                                        }))
                                                                )
                                                        )
                                                )
                                        )
                                }))
                )
            })
    }
}
