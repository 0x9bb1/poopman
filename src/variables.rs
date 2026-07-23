//! Pure `{{variable}}` substitution used at request-send time.

use std::collections::HashMap;

use crate::types::{AuthConfig, BodyType, FormDataRow, FormDataValue, RequestData};

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

/// Substitute `{{vars}}` in every auth field. `auth_type` is preserved as-is.
pub fn substitute_auth(auth: &AuthConfig, vars: &HashMap<String, String>) -> AuthConfig {
    AuthConfig {
        auth_type: auth.auth_type,
        bearer_token: substitute(&auth.bearer_token, vars),
        basic_username: substitute(&auth.basic_username, vars),
        basic_password: substitute(&auth.basic_password, vars),
        api_key_name: substitute(&auth.api_key_name, vars),
        api_key_value: substitute(&auth.api_key_value, vars),
    }
}

/// Substitute `{{vars}}` throughout a request — URL, header keys+values, and
/// raw/form body text — so generated code & previews use resolved values.
/// File form-data paths are left untouched.
pub fn substitute_request(req: &RequestData, vars: &HashMap<String, String>) -> RequestData {
    let headers = req
        .headers
        .iter()
        .map(|(k, v)| (substitute(k, vars), substitute(v, vars)))
        .collect();

    let body = match &req.body {
        BodyType::None => BodyType::None,
        BodyType::Raw { content, subtype } => BodyType::Raw {
            content: substitute(content, vars),
            subtype: *subtype,
        },
        BodyType::FormData(rows) => BodyType::FormData(
            rows.iter()
                .map(|r| FormDataRow {
                    enabled: r.enabled,
                    key: substitute(&r.key, vars),
                    value: match &r.value {
                        FormDataValue::Text(t) => FormDataValue::Text(substitute(t, vars)),
                        FormDataValue::File { path } => FormDataValue::File { path: path.clone() },
                    },
                })
                .collect(),
        ),
    };

    RequestData {
        method: req.method,
        url: substitute(&req.url, vars),
        headers,
        body,
        auth: substitute_auth(&req.auth, vars),
    }
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

    #[test]
    fn substitute_request_resolves_url_headers_and_raw_body() {
        use crate::types::{BodyType, HttpMethod, RawSubtype, RequestData};
        let req = RequestData {
            method: HttpMethod::POST,
            url: "{{base_url}}/users".to_string(),
            headers: vec![("Authorization".to_string(), "Bearer {{token}}".to_string())],
            body: BodyType::Raw {
                content: "{\"env\": \"{{env}}\"}".to_string(),
                subtype: RawSubtype::Json,
            },
            auth: crate::types::AuthConfig::default(),
        };
        let v = vars(&[("base_url", "https://api.test"), ("token", "abc"), ("env", "prod")]);
        let out = super::substitute_request(&req, &v);
        assert_eq!(out.url, "https://api.test/users");
        assert_eq!(out.headers[0].1, "Bearer abc");
        match out.body {
            BodyType::Raw { content, .. } => assert_eq!(content, "{\"env\": \"prod\"}"),
            _ => panic!("expected raw body"),
        }
    }

    #[test]
    fn substitute_auth_resolves_all_fields() {
        use crate::types::{AuthConfig, AuthType};
        let auth = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "{{token}}".into(),
            basic_username: "{{user}}".into(),
            basic_password: "{{pass}}".into(),
            api_key_name: "{{keyname}}".into(),
            api_key_value: "{{keyval}}".into(),
        };
        let v = vars(&[
            ("token", "abc"), ("user", "u"), ("pass", "p"),
            ("keyname", "X-Key"), ("keyval", "kv"),
        ]);
        let out = super::substitute_auth(&auth, &v);
        assert_eq!(out.auth_type, AuthType::Bearer);
        assert_eq!(out.bearer_token, "abc");
        assert_eq!(out.basic_username, "u");
        assert_eq!(out.basic_password, "p");
        assert_eq!(out.api_key_name, "X-Key");
        assert_eq!(out.api_key_value, "kv");
    }

    #[test]
    fn substitute_request_resolves_auth() {
        use crate::types::{AuthConfig, AuthType, BodyType, HttpMethod, RequestData};
        let req = RequestData {
            method: HttpMethod::GET,
            url: "https://api.test".into(),
            headers: vec![],
            body: BodyType::None,
            auth: AuthConfig { auth_type: AuthType::Bearer, bearer_token: "{{token}}".into(), ..Default::default() },
        };
        let out = super::substitute_request(&req, &vars(&[("token", "abc")]));
        assert_eq!(out.auth.bearer_token, "abc");
    }
}
