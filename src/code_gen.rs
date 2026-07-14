//! Pure code-snippet generation (Postman's "Code" feature): turn a `RequestData`
//! into runnable client code for several languages/libraries. All functions are
//! stateless and unit-testable; no GPUI types here.
//!
//! v1 supports `None` and `Raw` request bodies across all targets. `FormData`
//! bodies are not exported yet — generators prepend a clarifying comment.

use crate::types::{BodyType, FormDataRow, FormDataValue, RequestData};

/// A language/library target for code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeTarget {
    Curl,
    RustReqwest,
    PythonRequests,
    JavaScriptFetch,
    NodeAxios,
    GoNetHttp,
}

impl CodeTarget {
    /// All targets in dropdown order.
    pub fn all() -> Vec<Self> {
        vec![
            CodeTarget::Curl,
            CodeTarget::RustReqwest,
            CodeTarget::PythonRequests,
            CodeTarget::JavaScriptFetch,
            CodeTarget::NodeAxios,
            CodeTarget::GoNetHttp,
        ]
    }

    /// Human-readable label for the language dropdown.
    pub fn label(&self) -> &'static str {
        match self {
            CodeTarget::Curl => "cURL",
            CodeTarget::RustReqwest => "Rust — reqwest",
            CodeTarget::PythonRequests => "Python — Requests",
            CodeTarget::JavaScriptFetch => "JavaScript — Fetch",
            CodeTarget::NodeAxios => "NodeJS — Axios",
            CodeTarget::GoNetHttp => "Go — net/http",
        }
    }

    /// Syntax-highlight language id for the code editor (falls back to plain text
    /// if the tree-sitter grammar isn't bundled — harmless).
    pub fn language(&self) -> &'static str {
        match self {
            CodeTarget::Curl => "bash",
            CodeTarget::RustReqwest => "rust",
            CodeTarget::PythonRequests => "python",
            CodeTarget::JavaScriptFetch | CodeTarget::NodeAxios => "javascript",
            CodeTarget::GoNetHttp => "go",
        }
    }

    /// All labels, in `all()` order — used to build the dropdown.
    pub fn labels() -> Vec<&'static str> {
        Self::all().iter().map(|t| t.label()).collect()
    }
}

/// Raw request body as a string, or `None` for empty/`None`/`FormData` bodies.
fn raw_body(req: &RequestData) -> Option<String> {
    match &req.body {
        BodyType::None => None,
        BodyType::Raw { content, .. } => {
            if content.trim().is_empty() {
                None
            } else {
                Some(content.clone())
            }
        }
        BodyType::FormData(_) => None,
    }
}

/// Whether the request has a (currently unsupported) non-empty form-data body.
fn form_data_present(req: &RequestData) -> bool {
    matches!(&req.body, BodyType::FormData(rows) if !rows.is_empty())
}

/// Non-empty headers (skip blank keys left by placeholder rows).
fn headers(req: &RequestData) -> Vec<(&str, &str)> {
    req.headers
        .iter()
        .filter(|(k, _)| !k.trim().is_empty())
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// Enabled, non-blank-key form-data rows — the rows that export.
fn form_rows(req: &RequestData) -> Vec<&FormDataRow> {
    match &req.body {
        BodyType::FormData(rows) => rows
            .iter()
            .filter(|r| r.enabled && !r.key.trim().is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Headers to export: non-blank keys, minus Content-Type for form-data
/// bodies — the UI pins `multipart/form-data; boundary=<auto>` on such
/// requests, and each target's library must generate its own boundary.
fn export_headers(req: &RequestData) -> Vec<(&str, &str)> {
    let skip_content_type = matches!(&req.body, BodyType::FormData(_));
    req.headers
        .iter()
        .filter(|(k, _)| !k.trim().is_empty())
        .filter(|(k, _)| !(skip_content_type && k.eq_ignore_ascii_case("content-type")))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// Escape a string for a single-quoted shell context (the `'\''` trick).
fn shell_single(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Escape a string for a double-quoted source string (Rust/Python/JS):
/// backslash, double-quote, newline, carriage return, tab.
fn dq(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Rust raw string literal (`r#"..."#`) — keeps a multi-line body readable with
/// no escaping. Bumps the `#` count if the body contains the closing delimiter.
fn rust_raw(s: &str) -> String {
    let mut n = 1;
    while s.contains(&format!("\"{}", "#".repeat(n))) {
        n += 1;
    }
    let h = "#".repeat(n);
    format!("r{h}\"{s}\"{h}")
}

/// Python triple-quoted string — preserves newlines. Falls back to an escaped
/// double-quoted string when the body would break the triple-quote.
fn py_string(s: &str) -> String {
    if s.contains("\"\"\"") || s.ends_with('"') {
        format!("\"{}\"", dq(s))
    } else {
        format!("\"\"\"{}\"\"\"", s)
    }
}

/// JS template literal (backticks) — preserves newlines. Falls back to an escaped
/// double-quoted string when the body contains a backtick or `${`.
fn js_string(s: &str) -> String {
    if s.contains('`') || s.contains("${") {
        format!("\"{}\"", dq(s))
    } else {
        format!("`{}`", s)
    }
}

/// Top-level dispatch: generate source code for `target` from `req`.
pub fn generate(target: CodeTarget, req: &RequestData) -> String {
    match target {
        CodeTarget::Curl => gen_curl(req),
        CodeTarget::RustReqwest => gen_rust(req),
        CodeTarget::PythonRequests => gen_python(req),
        CodeTarget::JavaScriptFetch => gen_fetch(req),
        CodeTarget::NodeAxios => gen_axios(req),
        CodeTarget::GoNetHttp => gen_go(req),
    }
}

fn gen_curl(req: &RequestData) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "curl --location --request {} '{}'",
        req.method.as_str(),
        shell_single(&req.url)
    ));
    for (k, v) in export_headers(req) {
        lines.push(format!("  --header '{}: {}'", shell_single(k), shell_single(v)));
    }
    for row in form_rows(req) {
        match &row.value {
            FormDataValue::Text(v) => lines.push(format!(
                "  --form-string '{}={}'",
                shell_single(&row.key),
                shell_single(v)
            )),
            FormDataValue::File { path } => lines.push(format!(
                "  --form '{}=@\"{}\"'",
                shell_single(&row.key),
                shell_single(path)
            )),
        }
    }
    if let Some(body) = raw_body(req) {
        lines.push(format!("  --data '{}'", shell_single(&body)));
    }
    lines.join(" \\\n")
}

fn gen_rust(req: &RequestData) -> String {
    let mut s = String::new();
    if form_data_present(req) {
        s.push_str("// NOTE: form-data body is not yet supported in code export\n");
    }
    s.push_str("use reqwest::blocking::Client;\n");
    s.push_str("use reqwest::header::{HeaderMap, HeaderValue};\n\n");
    s.push_str("fn main() -> Result<(), Box<dyn std::error::Error>> {\n");
    s.push_str("    let client = Client::new();\n\n");
    s.push_str("    let mut headers = HeaderMap::new();\n");
    for (k, v) in headers(req) {
        s.push_str(&format!(
            "    headers.insert(\"{}\", HeaderValue::from_str(\"{}\")?);\n",
            dq(k),
            dq(v)
        ));
    }
    s.push('\n');
    s.push_str(&format!(
        "    let response = client\n        .request(reqwest::Method::{}, \"{}\")\n        .headers(headers)\n",
        req.method.as_str(),
        dq(&req.url)
    ));
    if let Some(body) = raw_body(req) {
        s.push_str(&format!("        .body({})\n", rust_raw(&body)));
    }
    s.push_str("        .send()?;\n\n");
    s.push_str("    println!(\"{}\", response.text()?);\n");
    s.push_str("    Ok(())\n");
    s.push_str("}\n");
    s
}

fn gen_python(req: &RequestData) -> String {
    let mut s = String::new();
    if form_data_present(req) {
        s.push_str("# NOTE: form-data body is not yet supported in code export\n");
    }
    s.push_str("import requests\n\n");
    s.push_str(&format!("url = \"{}\"\n\n", dq(&req.url)));
    let hs = headers(req);
    if hs.is_empty() {
        s.push_str("headers = {}\n");
    } else {
        s.push_str("headers = {\n");
        for (k, v) in hs {
            s.push_str(&format!("    \"{}\": \"{}\",\n", dq(k), dq(v)));
        }
        s.push_str("}\n");
    }
    let body = raw_body(req);
    if let Some(b) = &body {
        s.push_str(&format!("payload = {}\n", py_string(b)));
    }
    s.push('\n');
    if body.is_some() {
        s.push_str(&format!(
            "response = requests.request(\"{}\", url, headers=headers, data=payload)\n",
            req.method.as_str()
        ));
    } else {
        s.push_str(&format!(
            "response = requests.request(\"{}\", url, headers=headers)\n",
            req.method.as_str()
        ));
    }
    s.push_str("print(response.text)\n");
    s
}

fn gen_fetch(req: &RequestData) -> String {
    let mut s = String::new();
    if form_data_present(req) {
        s.push_str("// NOTE: form-data body is not yet supported in code export\n");
    }
    s.push_str("const myHeaders = new Headers();\n");
    for (k, v) in headers(req) {
        s.push_str(&format!("myHeaders.append(\"{}\", \"{}\");\n", dq(k), dq(v)));
    }
    s.push_str("\nconst requestOptions = {\n");
    s.push_str(&format!("  method: \"{}\",\n", req.method.as_str()));
    s.push_str("  headers: myHeaders,\n");
    if let Some(b) = raw_body(req) {
        s.push_str(&format!("  body: {},\n", js_string(&b)));
    }
    s.push_str("  redirect: \"follow\",\n");
    s.push_str("};\n\n");
    s.push_str(&format!("fetch(\"{}\", requestOptions)\n", dq(&req.url)));
    s.push_str("  .then((response) => response.text())\n");
    s.push_str("  .then((result) => console.log(result))\n");
    s.push_str("  .catch((error) => console.error(error));\n");
    s
}

fn gen_axios(req: &RequestData) -> String {
    let mut s = String::new();
    if form_data_present(req) {
        s.push_str("// NOTE: form-data body is not yet supported in code export\n");
    }
    s.push_str("const axios = require(\"axios\");\n\n");
    s.push_str("const config = {\n");
    s.push_str(&format!("  method: \"{}\",\n", req.method.as_str().to_lowercase()));
    s.push_str(&format!("  url: \"{}\",\n", dq(&req.url)));
    let hs = headers(req);
    if hs.is_empty() {
        s.push_str("  headers: {},\n");
    } else {
        s.push_str("  headers: {\n");
        for (k, v) in hs {
            s.push_str(&format!("    \"{}\": \"{}\",\n", dq(k), dq(v)));
        }
        s.push_str("  },\n");
    }
    if let Some(b) = raw_body(req) {
        s.push_str(&format!("  data: {},\n", js_string(&b)));
    }
    s.push_str("};\n\n");
    s.push_str("axios(config)\n");
    s.push_str("  .then((response) => {\n");
    s.push_str("    console.log(JSON.stringify(response.data));\n");
    s.push_str("  })\n");
    s.push_str("  .catch((error) => {\n");
    s.push_str("    console.log(error);\n");
    s.push_str("  });\n");
    s
}

fn gen_go(req: &RequestData) -> String {
    let mut s = String::new();
    if form_data_present(req) {
        s.push_str("// NOTE: form-data body is not yet supported in code export\n");
    }
    let body = raw_body(req);
    s.push_str("package main\n\n");
    s.push_str("import (\n");
    s.push_str("\t\"fmt\"\n");
    s.push_str("\t\"io\"\n");
    s.push_str("\t\"net/http\"\n");
    if body.is_some() {
        s.push_str("\t\"strings\"\n");
    }
    s.push_str(")\n\n");
    s.push_str("func main() {\n");
    s.push_str(&format!("\turl := \"{}\"\n", dq(&req.url)));
    s.push_str(&format!("\tmethod := \"{}\"\n\n", req.method.as_str()));
    if let Some(b) = &body {
        // Go raw string literal in backticks; backticks can't be escaped, so fall
        // back to a quoted Go string if the body itself contains a backtick.
        if b.contains('`') {
            s.push_str(&format!("\tpayload := strings.NewReader(\"{}\")\n\n", dq(b)));
        } else {
            s.push_str(&format!("\tpayload := strings.NewReader(`{}`)\n\n", b));
        }
        s.push_str("\tclient := &http.Client{}\n");
        s.push_str("\treq, err := http.NewRequest(method, url, payload)\n");
    } else {
        s.push_str("\tclient := &http.Client{}\n");
        s.push_str("\treq, err := http.NewRequest(method, url, nil)\n");
    }
    s.push_str("\tif err != nil {\n\t\tfmt.Println(err)\n\t\treturn\n\t}\n");
    for (k, v) in headers(req) {
        s.push_str(&format!("\treq.Header.Add(\"{}\", \"{}\")\n", dq(k), dq(v)));
    }
    s.push('\n');
    s.push_str("\tres, err := client.Do(req)\n");
    s.push_str("\tif err != nil {\n\t\tfmt.Println(err)\n\t\treturn\n\t}\n");
    s.push_str("\tdefer res.Body.Close()\n\n");
    s.push_str("\tbody, err := io.ReadAll(res.Body)\n");
    s.push_str("\tif err != nil {\n\t\tfmt.Println(err)\n\t\treturn\n\t}\n");
    s.push_str("\tfmt.Println(string(body))\n");
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, FormDataRow, FormDataValue, HttpMethod, RawSubtype};

    fn get_req() -> RequestData {
        RequestData {
            method: HttpMethod::GET,
            url: "https://api.example.com/users".to_string(),
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: BodyType::None,
        }
    }

    fn post_json_req() -> RequestData {
        RequestData {
            method: HttpMethod::POST,
            url: "https://api.example.com/users".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: BodyType::Raw {
                content: "{\"name\": \"ada\"}".to_string(),
                subtype: RawSubtype::Json,
            },
        }
    }

    /// POST with one text row, one file row, one disabled row, and the
    /// UI-pinned multipart Content-Type header that must NOT be exported.
    fn form_req() -> RequestData {
        RequestData {
            method: HttpMethod::POST,
            url: "https://api.example.com/upload".to_string(),
            headers: vec![
                ("Accept".to_string(), "application/json".to_string()),
                (
                    "Content-Type".to_string(),
                    "multipart/form-data; boundary=<auto>".to_string(),
                ),
            ],
            body: BodyType::FormData(vec![
                FormDataRow {
                    enabled: true,
                    key: "note".to_string(),
                    value: FormDataValue::Text("hello world".to_string()),
                },
                FormDataRow {
                    enabled: true,
                    key: "avatar".to_string(),
                    value: FormDataValue::File { path: "C:\\pics\\me.png".to_string() },
                },
                FormDataRow {
                    enabled: false,
                    key: "skipme".to_string(),
                    value: FormDataValue::Text("nope".to_string()),
                },
            ]),
        }
    }

    #[test]
    fn targets_have_six_and_unique_labels() {
        let all = CodeTarget::all();
        assert_eq!(all.len(), 6);
        let labels = CodeTarget::labels();
        assert_eq!(labels.len(), 6);
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 6, "labels must be unique");
    }

    #[test]
    fn curl_get_has_method_url_and_header() {
        let out = generate(CodeTarget::Curl, &get_req());
        assert!(out.contains("--request GET 'https://api.example.com/users'"));
        assert!(out.contains("--header 'Accept: application/json'"));
        assert!(!out.contains("--data"));
    }

    #[test]
    fn curl_post_includes_data_body() {
        let out = generate(CodeTarget::Curl, &post_json_req());
        assert!(out.contains("--request POST"));
        assert!(out.contains("--data '{\"name\": \"ada\"}'"));
    }

    #[test]
    fn curl_escapes_single_quotes_in_url() {
        let mut req = get_req();
        req.url = "https://x.test/a'b".to_string();
        let out = generate(CodeTarget::Curl, &req);
        assert!(out.contains("'https://x.test/a'\\''b'"));
    }

    #[test]
    fn rust_generates_blocking_client_and_escaped_body() {
        let out = generate(CodeTarget::RustReqwest, &post_json_req());
        assert!(out.contains("use reqwest::blocking::Client;"));
        assert!(out.contains("reqwest::Method::POST"));
        assert!(out.contains("HeaderValue::from_str(\"application/json\")?"));
        // body uses a raw string literal (no escaping)
        assert!(out.contains(".body(r#\"{\"name\": \"ada\"}\"#)"));
    }

    #[test]
    fn python_omits_payload_when_no_body() {
        let out = generate(CodeTarget::PythonRequests, &get_req());
        assert!(out.contains("import requests"));
        assert!(out.contains("requests.request(\"GET\", url, headers=headers)"));
        assert!(!out.contains("payload"));
    }

    #[test]
    fn python_includes_payload_when_body() {
        let out = generate(CodeTarget::PythonRequests, &post_json_req());
        assert!(out.contains("payload = \"\"\"{\"name\": \"ada\"}\"\"\""));
        assert!(out.contains("data=payload"));
    }

    #[test]
    fn fetch_uses_headers_and_method() {
        let out = generate(CodeTarget::JavaScriptFetch, &post_json_req());
        assert!(out.contains("myHeaders.append(\"Content-Type\", \"application/json\");"));
        assert!(out.contains("method: \"POST\","));
        assert!(out.contains("body: `{\"name\": \"ada\"}`,"));
        assert!(out.contains("fetch(\"https://api.example.com/users\", requestOptions)"));
    }

    #[test]
    fn axios_lowercases_method() {
        let out = generate(CodeTarget::NodeAxios, &post_json_req());
        assert!(out.contains("method: \"post\","));
        assert!(out.contains("data: `{\"name\": \"ada\"}`,"));
    }

    #[test]
    fn multiline_body_stays_readable_not_escaped() {
        // A pretty-printed JSON body must keep real newlines in each language's
        // idiomatic raw/multi-line string — never literal "\n".
        let mut req = post_json_req();
        let pretty = "{\n    \"userId\": 2204668,\n    \"salesFlag\": true\n}";
        req.body = BodyType::Raw {
            content: pretty.to_string(),
            subtype: RawSubtype::Json,
        };

        let rust = generate(CodeTarget::RustReqwest, &req);
        assert!(rust.contains(&format!(".body(r#\"{}\"#)", pretty)));
        assert!(!rust.contains("\\n"));

        let py = generate(CodeTarget::PythonRequests, &req);
        assert!(py.contains(&format!("payload = \"\"\"{}\"\"\"", pretty)));

        let fetch = generate(CodeTarget::JavaScriptFetch, &req);
        assert!(fetch.contains(&format!("body: `{}`,", pretty)));

        let go = generate(CodeTarget::GoNetHttp, &req);
        assert!(go.contains(&format!("strings.NewReader(`{}`)", pretty)));
    }

    #[test]
    fn go_uses_backtick_body_and_strings_import() {
        let out = generate(CodeTarget::GoNetHttp, &post_json_req());
        assert!(out.contains("\"strings\""));
        assert!(out.contains("payload := strings.NewReader(`{\"name\": \"ada\"}`)"));
        assert!(out.contains("req.Header.Add(\"Content-Type\", \"application/json\")"));
    }

    #[test]
    fn go_uses_nil_body_without_strings_import() {
        let out = generate(CodeTarget::GoNetHttp, &get_req());
        assert!(out.contains("http.NewRequest(method, url, nil)"));
        assert!(!out.contains("\"strings\""));
    }

    #[test]
    fn form_data_adds_note_comment() {
        let mut req = post_json_req();
        req.body = BodyType::FormData(vec![FormDataRow {
            enabled: true,
            key: "file".to_string(),
            value: FormDataValue::Text("x".to_string()),
        }]);
        let out = generate(CodeTarget::PythonRequests, &req);
        assert!(out.contains("form-data body is not yet supported"));
        assert!(!out.contains("payload"));
    }

    #[test]
    fn curl_form_data_uses_form_flags() {
        let out = generate(CodeTarget::Curl, &form_req());
        assert!(out.contains("--form-string 'note=hello world'"));
        assert!(out.contains("--form 'avatar=@\"C:\\pics\\me.png\"'"));
        assert!(!out.contains("skipme"));
        assert!(!out.contains("not yet supported"));
        assert!(!out.contains("Content-Type"), "boundary header must not export");
        assert!(out.contains("--header 'Accept: application/json'"));
        assert!(!out.contains("--data"));
    }

    #[test]
    fn curl_form_text_leading_at_stays_literal() {
        // --form-string never does curl's @file / <file expansion.
        let mut req = form_req();
        req.body = BodyType::FormData(vec![FormDataRow {
            enabled: true,
            key: "handle".to_string(),
            value: FormDataValue::Text("@ada".to_string()),
        }]);
        let out = generate(CodeTarget::Curl, &req);
        assert!(out.contains("--form-string 'handle=@ada'"));
    }

    #[test]
    fn blank_header_keys_are_skipped() {
        let mut req = get_req();
        req.headers.push(("".to_string(), "ignored".to_string()));
        let out = generate(CodeTarget::Curl, &req);
        assert!(!out.contains("ignored"));
    }
}
