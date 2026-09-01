//! Native editing assistance for raw request bodies.
//!
//! This module deliberately stays small: environment-variable completion is
//! backed by gpui-component's existing completion provider, while diagnostics
//! are computed as plain data so JSON parsing can run off the UI thread.

use std::{collections::HashSet, ops::Range};

use anyhow::Result;
use gpui::{Context, Task, Window};
use gpui_component::input::{CompletionProvider, InputState, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Position, TextEdit,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodyDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BodyDiagnostic {
    pub range: Range<Position>,
    pub severity: BodyDiagnosticSeverity,
    pub message: String,
    pub source: &'static str,
}

#[derive(Debug, Eq, PartialEq)]
struct VariableCompletionContext {
    prefix: String,
    replace_range: Range<usize>,
}

/// Completes variable names inside an unfinished `{{variable}}` expression.
///
/// Values are intentionally excluded from completion details because environment
/// values commonly contain credentials and tokens.
pub(crate) struct VariableCompletionProvider {
    variable_names: Vec<String>,
}

impl VariableCompletionProvider {
    pub(crate) fn new(variable_names: impl IntoIterator<Item = String>) -> Self {
        let mut variable_names = variable_names
            .into_iter()
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        variable_names.sort();
        variable_names.dedup();
        Self { variable_names }
    }
}

impl CompletionProvider for VariableCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        // Inspect only the cursor's line. In the usual pretty-printed case this
        // keeps completion work independent of total body size.
        let cursor_position = rope.offset_to_position(offset);
        let line = cursor_position.line as usize;
        let line_start = rope.line_start_offset(line);
        let line_end = rope.line_end_offset(line);
        let line_text = rope.slice(line_start..line_end).to_string();
        let Some(context) = variable_completion_context(&line_text, offset - line_start) else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };

        let prefix_lower = context.prefix.to_lowercase();
        let replace_start = line_start + context.replace_range.start;
        let replace_end = line_start + context.replace_range.end;
        let range = lsp_types::Range {
            start: rope.offset_to_position(replace_start),
            end: rope.offset_to_position(replace_end),
        };

        let items = self
            .variable_names
            .iter()
            .filter(|name| name.to_lowercase().starts_with(&prefix_lower))
            .map(|name| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some("Environment variable".to_string()),
                filter_text: Some(context.prefix.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: format!("{{{{{name}}}}}"),
                })),
                ..Default::default()
            })
            .collect();

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        _new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        // The provider itself cheaply rejects cursors outside `{{...}}`. Always
        // checking also lets completion follow backspace and edits within an
        // existing expression.
        true
    }
}

fn variable_completion_context(line: &str, cursor: usize) -> Option<VariableCompletionContext> {
    if cursor > line.len() || !line.is_char_boundary(cursor) {
        return None;
    }

    let before_cursor = &line[..cursor];
    let open = before_cursor.rfind("{{")?;
    let typed = &before_cursor[open + 2..];
    if typed.contains('{') || typed.contains('}') {
        return None;
    }

    // If the cursor is inside an already-closed expression, replace its closing
    // braces too. Otherwise insert them along with the selected name.
    let after_cursor = &line[cursor..];
    let replace_end = after_cursor
        .find("}}")
        .filter(|close| {
            let before_close = &after_cursor[..*close];
            !before_close.contains('{') && !before_close.contains('}')
        })
        .map_or(cursor, |close| cursor + close + 2);

    Some(VariableCompletionContext {
        prefix: typed.trim_start().to_string(),
        replace_range: open..replace_end,
    })
}

#[derive(Debug)]
struct VariableToken<'a> {
    range: Range<usize>,
    name_range: Range<usize>,
    name: &'a str,
}

fn variable_tokens(text: &str) -> Vec<VariableToken<'_>> {
    let mut tokens = Vec::new();
    let mut cursor = 0;

    while let Some(relative_open) = text[cursor..].find("{{") {
        let open = cursor + relative_open;
        let inner_start = open + 2;
        let Some(relative_close) = text[inner_start..].find("}}") else {
            break;
        };
        let close = inner_start + relative_close;
        let end = close + 2;
        let inner = &text[inner_start..close];
        let trimmed = inner.trim();
        let leading_whitespace = inner.len() - inner.trim_start().len();
        let name_start = inner_start + leading_whitespace;

        tokens.push(VariableToken {
            range: open..end,
            name_range: name_start..name_start + trimmed.len(),
            name: trimmed,
        });
        cursor = end;
    }

    tokens
}

/// Compute variable and JSON diagnostics without touching GPUI state.
pub(crate) fn compute_body_diagnostics(
    text: &str,
    variable_names: &HashSet<String>,
    validate_json: bool,
) -> Vec<BodyDiagnostic> {
    let tokens = variable_tokens(text);
    let line_starts = line_starts(text);
    let mut diagnostics = Vec::new();

    for token in &tokens {
        let (range, message) = if token.name.is_empty() {
            (
                token.range.clone(),
                "Variable name cannot be empty".to_string(),
            )
        } else if !variable_names.contains(token.name) {
            (
                token.name_range.clone(),
                format!("Unknown environment variable: {}", token.name),
            )
        } else {
            continue;
        };

        diagnostics.push(BodyDiagnostic {
            range: position_at(text, &line_starts, range.start)
                ..position_at(text, &line_starts, range.end),
            severity: BodyDiagnosticSeverity::Warning,
            message,
            source: "poopman-variables",
        });
    }

    if validate_json && !text.trim().is_empty() {
        let normalized = normalize_variables_for_json(text, &tokens);
        if let Err(error) = serde_json::from_str::<serde_json::Value>(&normalized) {
            diagnostics.push(BodyDiagnostic {
                range: json_error_range(&normalized, &error),
                severity: BodyDiagnosticSeverity::Error,
                message: format!("Malformed JSON: {error}"),
                source: "json",
            });
        }
    }

    diagnostics.sort_by_key(|diagnostic| {
        (
            diagnostic.range.start.line,
            diagnostic.range.start.character,
            diagnostic.range.end.line,
            diagnostic.range.end.character,
        )
    });
    diagnostics
}

/// Replace a single-line variable token with `null` plus padding. The character
/// count stays stable, so a serde_json error can be mapped back to the editor.
/// Tokens inside strings remain valid string content; tokens used as raw values
/// become a valid JSON null placeholder.
fn normalize_variables_for_json(text: &str, tokens: &[VariableToken<'_>]) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut cursor = 0;

    for token in tokens {
        let token_text = &text[token.range.clone()];
        if token_text.contains(['\n', '\r']) {
            continue;
        }

        normalized.push_str(&text[cursor..token.range.start]);
        normalized.push_str("null");
        normalized.extend(std::iter::repeat_n(
            ' ',
            token_text.chars().count().saturating_sub(4),
        ));
        cursor = token.range.end;
    }

    normalized.push_str(&text[cursor..]);
    normalized
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    starts
}

fn position_at(text: &str, line_starts: &[usize], offset: usize) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    let line = line_starts
        .partition_point(|start| *start <= offset)
        .saturating_sub(1);
    let character = text[line_starts[line]..offset].chars().count();
    Position::new(line as u32, character as u32)
}

fn json_error_range(text: &str, error: &serde_json::Error) -> Range<Position> {
    let starts = line_starts(text);
    let line = error.line().saturating_sub(1).min(starts.len() - 1);
    let line_start = starts[line];
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |relative| line_start + relative);
    let line_text = &text[line_start..line_end];
    let mut byte_column = error.column().saturating_sub(1).min(line_text.len());
    while !line_text.is_char_boundary(byte_column) {
        byte_column = byte_column.saturating_sub(1);
    }

    let character_count = line_text.chars().count();
    let mut start_character = line_text[..byte_column].chars().count();
    if start_character == character_count && start_character > 0 {
        start_character -= 1;
    }

    if character_count > 0 {
        return Position::new(line as u32, start_character as u32)
            ..Position::new(line as u32, (start_character + 1) as u32);
    }

    // EOF immediately after a newline has column 0. Anchor the diagnostic on
    // the previous line so it still has a visible underline and hover target.
    if line > 0 {
        let previous_start = starts[line - 1];
        let previous_text = text[previous_start..line_start].trim_end_matches(['\r', '\n']);
        let previous_len = previous_text.chars().count();
        if previous_len > 0 {
            return Position::new((line - 1) as u32, (previous_len - 1) as u32)
                ..Position::new((line - 1) as u32, previous_len as u32);
        }
    }

    Position::new(line as u32, 0)..Position::new(line as u32, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> HashSet<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn completion_replaces_unfinished_expression_and_adds_closing_braces() {
        assert_eq!(
            variable_completion_context(r#"{"url":"{{ba"}"#, 12),
            Some(VariableCompletionContext {
                prefix: "ba".to_string(),
                replace_range: 8..12,
            })
        );
    }

    #[test]
    fn completion_replaces_existing_closing_braces() {
        assert_eq!(
            variable_completion_context("{{base_ul}}", 9),
            Some(VariableCompletionContext {
                prefix: "base_ul".to_string(),
                replace_range: 0..11,
            })
        );
    }

    #[test]
    fn completion_ignores_cursor_outside_variable_expression() {
        assert_eq!(variable_completion_context("{{base_url}} x", 14), None);
        assert_eq!(variable_completion_context("plain", 5), None);
    }

    #[test]
    fn reports_only_unknown_variables() {
        let diagnostics = compute_body_diagnostics(
            r#"{"known":"{{ token }}","missing":"{{other}}"}"#,
            &names(&["token"]),
            true,
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, BodyDiagnosticSeverity::Warning);
        assert_eq!(
            diagnostics[0].message,
            "Unknown environment variable: other"
        );
    }

    #[test]
    fn variable_placeholders_are_valid_in_json_string_or_value_positions() {
        let diagnostics = compute_body_diagnostics(
            r#"{"name":"{{name}}","count":{{count}}}"#,
            &names(&["name", "count"]),
            true,
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn malformed_json_gets_an_error_diagnostic() {
        let diagnostics = compute_body_diagnostics(r#"{"name": }"#, &names(&[]), true);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, BodyDiagnosticSeverity::Error);
        assert!(diagnostics[0].message.starts_with("Malformed JSON:"));
    }

    #[test]
    fn empty_json_body_has_no_diagnostic() {
        assert!(compute_body_diagnostics("  \n", &names(&[]), true).is_empty());
    }

    #[test]
    fn non_json_body_only_gets_variable_diagnostics() {
        let diagnostics = compute_body_diagnostics("not json {{missing}}", &names(&[]), false);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, BodyDiagnosticSeverity::Warning);
    }
}
