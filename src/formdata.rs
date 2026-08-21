//! Pure form-data editor invariants shared by the GPUI component and tests.

use crate::types::FormDataRow;

/// Resolve a row by stable identity after arbitrary vector reindexing.
pub fn row_index(row_ids: &[u64], row_id: u64) -> Option<usize> {
    row_ids.iter().position(|id| *id == row_id)
}

/// Number of editor rows needed to retain all content plus exactly one blank
/// trailing row. Extra blank rows at the tail are discarded.
pub fn desired_editor_len(rows: &[FormDataRow]) -> usize {
    rows.iter()
        .rposition(|row| !row.is_blank())
        .map_or(1, |index| index + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FormDataValue;

    fn text_row(key: &str, value: &str) -> FormDataRow {
        FormDataRow {
            enabled: true,
            key: key.to_string(),
            value: FormDataValue::Text(value.to_string()),
        }
    }

    #[test]
    fn stable_row_id_resolves_after_preceding_row_is_deleted() {
        let mut ids = vec![41, 42];
        ids.remove(0);

        assert_eq!(row_index(&ids, 42), Some(0));
        assert_eq!(row_index(&ids, 41), None);
    }

    #[test]
    fn trailing_placeholder_count_is_idempotent() {
        let rows = vec![
            text_row("name", "alice"),
            text_row("", ""),
            text_row("", ""),
        ];
        assert_eq!(desired_editor_len(&rows), 2);
        assert_eq!(desired_editor_len(&rows[..2]), 2);
        assert_eq!(desired_editor_len(&[]), 1);
    }
}
