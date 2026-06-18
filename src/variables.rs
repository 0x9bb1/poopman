//! Pure `{{variable}}` substitution used at request-send time.

use std::collections::HashMap;

/// Replace `{{key}}` / `{{ key }}` (key trimmed) with values from `vars`.
///
/// - Unknown variables are left literal (so a typo / missing var is visible).
/// - Non-recursive: substituted values are not themselves re-scanned.
/// - An unclosed `{{` is emitted literally.
pub fn substitute(input: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        if let Some(close) = after.find("}}") {
            let key = after[..close].trim();
            match vars.get(key) {
                Some(val) => out.push_str(val),
                None => {
                    // keep the original token literally
                    out.push_str("{{");
                    out.push_str(&after[..close]);
                    out.push_str("}}");
                }
            }
            rest = &after[close + 2..];
        } else {
            // unclosed "{{" — emit the rest literally and stop
            out.push_str("{{");
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn replaces_known_var() {
        assert_eq!(substitute("{{a}}", &vars(&[("a", "1")])), "1");
    }

    #[test]
    fn trims_inner_whitespace() {
        assert_eq!(substitute("{{ a }}", &vars(&[("a", "1")])), "1");
    }

    #[test]
    fn replaces_multiple_and_keeps_surrounding_text() {
        assert_eq!(
            substitute("x{{a}}y{{b}}z", &vars(&[("a", "1"), ("b", "2")])),
            "x1y2z"
        );
    }

    #[test]
    fn unknown_var_left_literal() {
        assert_eq!(substitute("{{missing}}", &vars(&[])), "{{missing}}");
    }

    #[test]
    fn no_vars_unchanged() {
        assert_eq!(substitute("plain text", &vars(&[])), "plain text");
    }

    #[test]
    fn non_recursive() {
        // value containing {{b}} must NOT be re-substituted
        assert_eq!(
            substitute("{{a}}", &vars(&[("a", "{{b}}"), ("b", "X")])),
            "{{b}}"
        );
    }

    #[test]
    fn unclosed_brace_is_literal() {
        assert_eq!(substitute("{{ unclosed", &vars(&[])), "{{ unclosed");
    }
}
