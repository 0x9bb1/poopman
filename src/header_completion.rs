//! Typeahead for custom header name fields.
//!
//! Wraps [`crate::header_names::suggest`] in gpui-component's LSP-shaped
//! [`CompletionProvider`] so the library's completion menu (keyboard navigation,
//! prefix highlighting, insertion) drives the UI. All matching logic lives in
//! `header_names`; this file only adapts it.

use anyhow::Result;
use gpui::{Context, Task, Window};
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    TextEdit,
};

use crate::header_names::suggest;

/// Suggests standard HTTP header names in a single-line header-name input.
pub struct HeaderCompletionProvider;

impl CompletionProvider for HeaderCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        _offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        // The field holds nothing but the header name, so the whole text is the
        // prefix regardless of where the cursor sits. Using the full range also
        // means the edit replaces what was typed rather than appending to it,
        // which is what puts canonical casing in the field after "au".
        let prefix = rope.to_string();
        let range = lsp_types::Range {
            start: rope.offset_to_position(0),
            end: rope.offset_to_position(rope.len()),
        };

        let items = suggest(&prefix)
            .into_iter()
            .map(|name| CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::FIELD),
                filter_text: Some(prefix.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name.to_string(),
                })),
                ..Default::default()
            })
            .collect::<Vec<_>>();

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        _new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        // Every keystroke is a candidate trigger; `suggest` returns nothing for an
        // empty field, which is what keeps the menu shut on a merely-focused row.
        true
    }
}
