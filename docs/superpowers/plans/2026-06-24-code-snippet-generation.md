# Code Snippet Generation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Postman's "Code" feature — a `</>` button in the URL bar that opens a right-docked slide-out panel showing the current request as runnable client code in a selectable language, with Copy.

**Architecture:** A pure, fully-unit-tested `code_gen` module turns a `RequestData` into source code for 6 targets. A new `CodeSnippetPanel` entity (language `Select` + read-only code editor + Copy/Close) is owned by `PoopmanApp` and shown as a right card column when open. `RequestEditor` gets a `</>` icon button that emits an `OpenCodeSnippet` event; `PoopmanApp` resolves `{{vars}}` against the active environment and feeds the resolved request to the panel. This mirrors the existing event-driven flow (`RequestCompleted`, `EnvironmentsChanged`).

**Tech Stack:** Rust 2024, GPUI 0.2.2, gpui-component 0.5 (`Select`, `InputState::code_editor`, `Icon`/`IconName`, `Button`), existing `variables::substitute`.

---

## UI Design (consistency notes)

- **Trigger:** a small `</>` ghost icon button placed in the URL bar, immediately to the **left of the Send button** (poopman has no Postman-style right rail; this fits the existing card layout and keeps discovery high). Icon asset: new `assets/icons/code.svg` (lucide "code" glyph, `currentColor`).
- **Presentation:** a **right-docked card column** (`crate::ui::card_panel`, fixed width `CODE_PANEL_WIDTH = 440px`) that appears beside the request/response splitter when open — the closest faithful match to Postman's right slide-over within poopman's floating-card style. (No slide animation in v1; it appears/disappears. Animation is an optional follow-up.)
- **Panel contents (top→bottom):** header row with title "Code snippet" + `Copy` button + `Close` (X) icon button; a language `Select`; a read-only code editor (`InputState::code_editor`) filling the rest, styled like the response body viewer (rounded, hairline border, `theme.popover` bg).
- **Body scope (v1):** `None` and `Raw` bodies are fully supported across all 6 targets. `FormData` bodies are **not** exported yet; generators prepend a one-line comment noting this. The generator is extensible (add a `CodeTarget` variant + one `gen_*` fn).

## Targets (6, in dropdown order)

1. cURL · 2. Rust — reqwest · 3. Python — Requests · 4. JavaScript — Fetch · 5. NodeJS — Axios · 6. Go — net/http

## File Structure

- **Create `src/code_gen.rs`** — pure code generation: `CodeTarget` enum, escaping helpers, body/header extraction, `generate()` dispatch + 6 `gen_*` functions. No GPUI types. Heavily unit-tested.
- **Create `src/code_snippet_panel.rs`** — `CodeSnippetPanel` entity (UI), `CloseCodeSnippet` event.
- **Create `assets/icons/code.svg`** — `</>` glyph for the trigger button.
- **Modify `src/variables.rs`** — add pure `substitute_request()` (resolves `{{vars}}` through URL/headers/body).
- **Modify `src/request_editor.rs`** — add `OpenCodeSnippet` event + `EventEmitter`, `resolved_request_data()`, and the `</>` button.
- **Modify `src/app.rs`** — own `code_panel` + `code_panel_open`, subscribe to `OpenCodeSnippet`/`CloseCodeSnippet`, render the right column.
- **Modify `src/theme.rs`** — add `CODE_PANEL_WIDTH` constant.
- **Modify `src/main.rs`** — declare `mod code_gen;` and `mod code_snippet_panel;`.

> **WSL2 note:** GPUI can't run headless here, so UI is verified by `cargo build` + `cargo test` (logic) and a manual run by the user on Windows. Pure modules (`code_gen`, `variables`) are fully covered by `cargo test`.

---

## Task 1: `code_gen` module — pure code generation

**Files:**
- Create: `src/code_gen.rs`
- Modify: `src/main.rs` (add `mod code_gen;`)

- [ ] **Step 1: Declare the module**

In `src/main.rs`, add the module declaration in the existing alphabetical `mod` block (after `mod code_formatter;`):

```rust
mod code_formatter;
mod code_gen;
```

- [ ] **Step 2: Write `src/code_gen.rs` with the full module**

Create `src/code_gen.rs`:

```rust
//! Pure code-snippet generation (Postman's "Code" feature): turn a `RequestData`
//! into runnable client code for several languages/libraries. All functions are
//! stateless and unit-testable; no GPUI types here.
//!
//! v1 supports `None` and `Raw` request bodies across all targets. `FormData`
//! bodies are not exported yet — generators prepend a clarifying comment.

use crate::types::{BodyType, RequestData};

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
    if form_data_present(req) {
        lines.push("# NOTE: form-data body is not yet supported in code export".to_string());
    }
    lines.push(format!(
        "curl --location --request {} '{}'",
        req.method.as_str(),
        shell_single(&req.url)
    ));
    for (k, v) in headers(req) {
        lines.push(format!("  --header '{}: {}'", shell_single(k), shell_single(v)));
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
        s.push_str(&format!("        .body(\"{}\")\n", dq(&body)));
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
        s.push_str(&format!("payload = \"{}\"\n", dq(b)));
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
        s.push_str(&format!("  body: \"{}\",\n", dq(&b)));
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
        s.push_str(&format!("  data: \"{}\",\n", dq(&b)));
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
        // body double-quotes are escaped
        assert!(out.contains(".body(\"{\\\"name\\\": \\\"ada\\\"}\")"));
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
        assert!(out.contains("payload = \"{\\\"name\\\": \\\"ada\\\"}\""));
        assert!(out.contains("data=payload"));
    }

    #[test]
    fn fetch_uses_headers_and_method() {
        let out = generate(CodeTarget::JavaScriptFetch, &post_json_req());
        assert!(out.contains("myHeaders.append(\"Content-Type\", \"application/json\");"));
        assert!(out.contains("method: \"POST\","));
        assert!(out.contains("body: \"{\\\"name\\\": \\\"ada\\\"}\","));
        assert!(out.contains("fetch(\"https://api.example.com/users\", requestOptions)"));
    }

    #[test]
    fn axios_lowercases_method() {
        let out = generate(CodeTarget::NodeAxios, &post_json_req());
        assert!(out.contains("method: \"post\","));
        assert!(out.contains("data: \"{\\\"name\\\": \\\"ada\\\"}\","));
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
    fn blank_header_keys_are_skipped() {
        let mut req = get_req();
        req.headers.push(("".to_string(), "ignored".to_string()));
        let out = generate(CodeTarget::Curl, &req);
        assert!(!out.contains("ignored"));
    }
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --lib code_gen`
Expected: all `code_gen::tests::*` PASS.

(If the crate has no lib target, use `cargo test code_gen` which runs the bin's unit tests.)

- [ ] **Step 4: Commit**

```bash
git add src/code_gen.rs src/main.rs
git commit -m "feat(code): add pure code-snippet generator for 6 targets"
```

---

## Task 2: `{{variable}}` resolution for a whole request

**Files:**
- Modify: `src/variables.rs` (add `substitute_request` + imports + tests)

- [ ] **Step 1: Add the failing test**

At the top of `src/variables.rs`, the imports currently are just `use std::collections::HashMap;`. Add the test first (append inside the existing `#[cfg(test)] mod tests`). Add this test function:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib substitute_request_resolves`
Expected: FAIL — `substitute_request` not found.

- [ ] **Step 3: Implement `substitute_request`**

In `src/variables.rs`, add the import near the top (after the existing `use std::collections::HashMap;`):

```rust
use crate::types::{BodyType, FormDataRow, FormDataValue, RequestData};
```

Then add this function after `substitute` (before the `#[cfg(test)]` module):

```rust
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
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --lib variables`
Expected: all `variables::tests::*` PASS (including the new test).

- [ ] **Step 5: Commit**

```bash
git add src/variables.rs
git commit -m "feat(vars): add substitute_request for whole-request var resolution"
```

---

## Task 3: `</>` trigger button + `OpenCodeSnippet` event + resolved data

**Files:**
- Create: `assets/icons/code.svg`
- Modify: `src/request_editor.rs` (event + EventEmitter + method + button)

- [ ] **Step 1: Create the icon asset**

Create `assets/icons/code.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
```

- [ ] **Step 2: Add the `OpenCodeSnippet` event + EventEmitter**

In `src/request_editor.rs`, after the `RequestCompleted` struct (around line 20), add:

```rust
/// Event emitted when the user asks to view the request as a code snippet.
#[derive(Clone)]
pub struct OpenCodeSnippet;
```

Then near the existing `impl EventEmitter<RequestCompleted> for RequestEditor {}` (around line 987), add:

```rust
impl EventEmitter<OpenCodeSnippet> for RequestEditor {}
```

- [ ] **Step 3: Add `resolved_request_data`**

In `src/request_editor.rs`, immediately after `get_current_request_data` (ends ~line 279), add:

```rust
    /// Current request with `{{vars}}` resolved against the active environment,
    /// for code generation / previews.
    pub fn resolved_request_data(&self, cx: &App) -> RequestData {
        crate::variables::substitute_request(&self.get_current_request_data(cx), &self.env_vars)
    }
```

- [ ] **Step 4: Add the `Icon` import**

In `src/request_editor.rs`, extend the `gpui_component` import (lines 4-7) to include `Icon`:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, input::*,
    select::*, v_flex, ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _,
};
```

- [ ] **Step 5: Add the `</>` button to the URL bar**

In `src/request_editor.rs` render, the URL bar adds the Send button as its last child (around lines 1026-1036). Insert the code button **before** the Send button child, i.e. add this child to the URL-bar `div` right before the `.child( /* Send button */ )`:

```rust
                        .child(
                            // Code snippet button - opens the code panel
                            div().flex_shrink_0().child(
                                Button::new("code-snippet-btn")
                                    .ghost()
                                    .icon(Icon::empty().path("icons/code.svg"))
                                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                                        cx.emit(OpenCodeSnippet);
                                    })),
                            ),
                        )
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build`
Expected: builds (warnings OK). The button has no visible effect yet — wiring happens in Task 5.

- [ ] **Step 7: Commit**

```bash
git add assets/icons/code.svg src/request_editor.rs
git commit -m "feat(code): add </> trigger button + OpenCodeSnippet event"
```

---

## Task 4: `CodeSnippetPanel` entity

**Files:**
- Create: `src/code_snippet_panel.rs`
- Modify: `src/main.rs` (add `mod code_snippet_panel;`)

- [ ] **Step 1: Declare the module**

In `src/main.rs`, add (after `mod code_gen;`):

```rust
mod code_gen;
mod code_snippet_panel;
```

- [ ] **Step 2: Write `src/code_snippet_panel.rs`**

Create `src/code_snippet_panel.rs`:

```rust
//! The "Code snippet" slide-out panel (Postman's Code feature). Shows generated
//! client code for the current request in a selectable language, with Copy and
//! Close actions. Owned by `PoopmanApp`, rendered as a right-docked card when open.

use gpui::*;
use gpui_component::{
    button::*, input::*, select::*, h_flex, v_flex, ActiveTheme as _, Icon, IconName, IndexPath,
    Sizable as _,
};

use crate::code_gen::{generate, CodeTarget};
use crate::types::RequestData;

/// Emitted when the user closes the code-snippet panel.
pub struct CloseCodeSnippet;

pub struct CodeSnippetPanel {
    request: Option<RequestData>,
    target: CodeTarget,
    code: String,
    language_select: Entity<SelectState<Vec<&'static str>>>,
    code_display: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CloseCodeSnippet> for CodeSnippetPanel {}

impl CodeSnippetPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target = CodeTarget::all()[0]; // cURL

        let language_select = cx.new(|cx| {
            SelectState::new(CodeTarget::labels(), Some(IndexPath::default()), window, cx)
        });

        let code_display = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(target.language())
                .line_number(true)
                .multi_line(true)
                .tab_size(TabSize { tab_size: 4, hard_tabs: false })
        });

        let sub = cx.subscribe_in(
            &language_select,
            window,
            |this, _, _e: &SelectEvent<Vec<&'static str>>, window, cx| {
                this.on_language_changed(window, cx);
            },
        );

        Self {
            request: None,
            target,
            code: String::new(),
            language_select,
            code_display,
            _subscriptions: vec![sub],
        }
    }

    /// Update the request shown and regenerate the snippet.
    pub fn set_request(&mut self, request: RequestData, window: &mut Window, cx: &mut Context<Self>) {
        self.request = Some(request);
        self.regenerate(window, cx);
    }

    fn on_language_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let idx = self
            .language_select
            .read(cx)
            .selected_index(cx)
            .map(|i| i.row)
            .unwrap_or(0);
        self.target = CodeTarget::all().get(idx).copied().unwrap_or(CodeTarget::Curl);
        let lang = self.target.language();
        self.code_display.update(cx, |input, cx| input.set_highlighter(lang, cx));
        self.regenerate(window, cx);
    }

    fn regenerate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = match &self.request {
            Some(req) => generate(self.target, req),
            None => String::new(),
        };
        self.code = code.clone();
        self.code_display.update(cx, |input, cx| input.set_value(&code, window, cx));
        cx.notify();
    }

    fn copy(&mut self, _e: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.code.clone()));
    }

    fn close(&mut self, _e: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CloseCodeSnippet);
    }
}

impl Render for CodeSnippetPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .id("code-snippet-panel")
            .size_full()
            .gap_3()
            .p_4()
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                // Header: title + Copy + Close
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("Code snippet"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("code-copy")
                                    .small()
                                    .label("Copy")
                                    .on_click(cx.listener(Self::copy)),
                            )
                            .child(
                                Button::new("code-close")
                                    .small()
                                    .ghost()
                                    .icon(Icon::new(IconName::Close))
                                    .on_click(cx.listener(Self::close)),
                            ),
                    ),
            )
            .child(Select::new(&self.language_select))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .overflow_hidden()
                    .child(Input::new(&self.code_display).w_full().h_full()),
            )
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds (warnings OK). Panel isn't shown yet — wiring is Task 5.

- [ ] **Step 4: Commit**

```bash
git add src/code_snippet_panel.rs src/main.rs
git commit -m "feat(code): add CodeSnippetPanel entity (Select + code editor + copy)"
```

---

## Task 5: Wire panel into `PoopmanApp` (state, subscriptions, layout)

**Files:**
- Modify: `src/theme.rs` (add `CODE_PANEL_WIDTH`)
- Modify: `src/app.rs` (field, constructor, subscriptions, handlers, render)

- [ ] **Step 1: Add the layout constant**

In `src/theme.rs`, after `RAW_SUBTYPE_WIDTH` (line 42), add:

```rust
#[allow(dead_code)]
pub const CODE_PANEL_WIDTH: f32 = 440.;
```

- [ ] **Step 2: Add imports + struct fields in `app.rs`**

In `src/app.rs`, extend the use block at line 12 to also import the new event:

```rust
use crate::request_editor::{OpenCodeSnippet, RequestCompleted, RequestEditor};
```

Add to the imports a line for the panel (after line 11's `history_panel` import or anywhere in the `use crate::...` group):

```rust
use crate::code_snippet_panel::{CloseCodeSnippet, CodeSnippetPanel};
```

Extend the theme import (line 16-18) to include the new constant:

```rust
use crate::theme::{
    CODE_PANEL_WIDTH, REQUEST_INITIAL_HEIGHT, REQUEST_MAX, REQUEST_MIN, SIDEBAR_MAX, SIDEBAR_MIN,
    SIDEBAR_WIDTH,
};
```

In the `PoopmanApp` struct (lines 21-34), add two fields after `env_manager`:

```rust
    env_manager: Entity<EnvironmentManager>,
    code_panel: Entity<CodeSnippetPanel>,
    code_panel_open: bool,
    _subscriptions: Vec<Subscription>,
```

- [ ] **Step 3: Construct the panel + subscriptions**

In `PoopmanApp::new`, after `let env_manager = cx.new(...)` (line 50), add:

```rust
        let code_panel = cx.new(|cx| CodeSnippetPanel::new(window, cx));
```

After the `env_changed_sub` subscription (ends ~line 153), add two more subscriptions:

```rust
        // Open the code-snippet panel when the request editor asks for it; feed it
        // the current request with environment variables resolved.
        let open_code_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |this, editor, _e: &OpenCodeSnippet, window, cx| {
                let req = editor.read(cx).resolved_request_data(cx);
                this.code_panel.update(cx, |panel, cx| {
                    panel.set_request(req, window, cx);
                });
                this.code_panel_open = true;
                cx.notify();
            },
        );

        // Close the code-snippet panel on its Close button.
        let close_code_sub = cx.subscribe_in(
            &code_panel,
            window,
            move |this, _, _e: &CloseCodeSnippet, _window, cx| {
                this.code_panel_open = false;
                cx.notify();
            },
        );
```

- [ ] **Step 4: Store the new fields + subscriptions in the returned struct**

Find where `PoopmanApp { ... }` is constructed and the `_subscriptions` vec is assembled (search for `_subscriptions:` in the constructor). Add `code_panel`, set `code_panel_open: false`, and push the two new subscriptions into the subscription list. For example, update the struct literal to include:

```rust
            env_manager,
            code_panel,
            code_panel_open: false,
```

and add the new subscriptions to the `_subscriptions` vec (alongside `request_sub`, `history_sub`, etc.):

```rust
            _subscriptions: vec![
                request_sub,
                history_sub,
                tab_clicked_sub,
                new_tab_sub,
                close_tab_sub,
                env_changed_sub,
                open_code_sub,
                close_code_sub,
            ],
```

> Note: match the exact existing field/var names in the constructor — if the `_subscriptions` vec is built differently (e.g. `.push(...)` calls), append `open_code_sub` and `close_code_sub` the same way.

- [ ] **Step 5: Render the right-docked panel column**

In `PoopmanApp::render`, the request/response splitter is currently a single child (lines 523-548):

```rust
                                    .child(
                                        // Request editor and response viewer with resizable splitter
                                        div().flex_1().overflow_hidden().child(
                                            v_resizable("request-response-splitter")
                                                .child( /* request panel */ )
                                                .child( /* response panel */ ),
                                        ),
                                    )
```

Replace that single `.child( div().flex_1().overflow_hidden().child(v_resizable(...)) )` with an `h_flex` row that holds the splitter and (conditionally) the code panel. Keep the `v_resizable(...)` contents exactly as they are — only wrap them:

```rust
                                    .child(
                                        // Request/response splitter + optional code panel (right)
                                        h_flex()
                                            .flex_1()
                                            .min_h_0()
                                            .w_full()
                                            .gap(px(10.))
                                            .child(
                                                div().flex_1().overflow_hidden().child(
                                                    v_resizable("request-response-splitter")
                                                        .child(
                                                            resizable_panel()
                                                                .size(px(REQUEST_INITIAL_HEIGHT))
                                                                .size_range(px(REQUEST_MIN)..px(REQUEST_MAX))
                                                                .child(
                                                                    crate::ui::card_panel(theme)
                                                                        .size_full()
                                                                        .child(self.request_editor.clone()),
                                                                ),
                                                        )
                                                        .child(
                                                            crate::ui::card_panel(theme)
                                                                .flex_1()
                                                                .min_h(px(200.))
                                                                .mt(px(10.))
                                                                .child(self.response_viewer.clone())
                                                                .into_any_element(),
                                                        ),
                                                ),
                                            )
                                            .when(self.code_panel_open, |row| {
                                                row.child(
                                                    crate::ui::card_panel(theme)
                                                        .w(px(CODE_PANEL_WIDTH))
                                                        .h_full()
                                                        .flex_shrink_0()
                                                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                                        .child(self.code_panel.clone()),
                                                )
                                            }),
                                    )
```

`when` comes from `gpui::prelude::FluentBuilder`. Add this import at the top of `app.rs` if not already present:

```rust
use gpui::prelude::FluentBuilder as _;
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly (warnings OK).

- [ ] **Step 7: Run the full test suite + clippy**

Run: `cargo test`
Expected: all tests PASS (code_gen + variables + existing).

Run: `cargo clippy --all-targets`
Expected: no new errors.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/theme.rs
git commit -m "feat(code): wire code-snippet panel into app (right-docked slide-out)"
```

---

## Task 6: Manual verification (on Windows — WSL2 can't run GPUI)

> This project does not run under WSL2 (GPU requirement). Ask the user to run on Windows.

- [ ] **Step 1: Build release & run**

Run (on Windows): `cargo run`

- [ ] **Step 2: Manual checklist**

Confirm each, matching Postman UX:
- A `</>` icon button appears to the left of **Send** in the URL bar.
- Clicking it opens a right-docked "Code snippet" card beside the request/response area.
- The language dropdown lists: cURL, Rust — reqwest, Python — Requests, JavaScript — Fetch, NodeJS — Axios, Go — net/http.
- Selecting a GET request with a header shows correct code; switching language updates the code and syntax highlighting.
- Entering a POST with a JSON raw body shows the body in `--data` / `payload` / `body` / `data` / `strings.NewReader(...)` per language.
- With an active environment, `{{vars}}` in the URL/headers/body appear **resolved** in the generated code.
- **Copy** puts the snippet on the clipboard (paste elsewhere to confirm).
- **Close** (X) hides the panel; request/response area expands back to full width.

- [ ] **Step 3: (Optional) Update CLAUDE.md**

If desired, add a one-line note to the "Component Structure" section describing `CodeSnippetPanel` and `code_gen`. Commit separately.

---

## Self-Review (completed during planning)

- **Spec coverage:** trigger button (Task 3), slide-out panel (Tasks 4–5), 6 languages (Task 1), var resolution (Task 2), Copy (Task 4), style consistency via `card_panel`/`segmented`/theme (Tasks 4–5). ✓
- **Type consistency:** `CodeTarget`/`generate`/`labels` used identically in Tasks 1, 4. `OpenCodeSnippet`/`CloseCodeSnippet`/`set_request`/`resolved_request_data` names match across Tasks 3–5. `CODE_PANEL_WIDTH` defined (Task 5 Step 1) before use (Step 5). ✓
- **API checks (verified against pinned crate sources):** `cx.write_to_clipboard(ClipboardItem::new_string(..))` (gpui 0.2.2), `Button::icon(impl Into<Icon>)` + `Icon::empty().path(..)` + `IconName::Close` (gpui-component 0.5.1, `icons/close.svg` present), `InputState::{code_editor, multi_line(bool), line_number(bool), tab_size, set_value, set_highlighter}`, `SelectState::new`/`SelectEvent` emitted on confirm. ✓
- **Known v1 limitation (intentional, documented):** `FormData` bodies are not exported — generators emit a `NOTE:` comment. Follow-up: add multipart generation per target.
