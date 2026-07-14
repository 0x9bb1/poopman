# Form-data Code Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** All six code-export targets (cURL, Rust, Python, JS Fetch, Node Axios, Go) generate real multipart form-data code instead of the current "not yet supported" NOTE comment, per `docs/superpowers/specs/2026-07-14-formdata-code-export-design.md`.

**Architecture:** Everything happens in the pure module `src/code_gen.rs` (no UI/send-path changes). Two shared helpers (`form_rows`, `export_headers`) feed six per-target emitters. `export_headers` drops `Content-Type` whenever the body is `BodyType::FormData` — the pinned `multipart/form-data; boundary=<auto>` UI header must never be exported because each target's library generates its own boundary.

**Tech Stack:** Rust; tests are plain `#[test]` in `code_gen::tests`.

**Environment constraints (from CLAUDE.md/memory):**
- WSL2 can run `cargo check` / `cargo clippy` but CANNOT link/test the crate.
- Test gate (run from WSL): `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"` — expected: all tests pass.
- Work on branch `feat/formdata-code-export` in the existing checkout `/mnt/e/code/poopman`. Do NOT use a git worktree: the Windows-side test command is hard-wired to `E:\code\poopman`.

**Existing code you'll touch (read these spans first):**
- `src/code_gen.rs:64-82` — `raw_body` / `form_data_present` (the latter gets deleted at the end)
- `src/code_gen.rs:84-92` — `headers()` (deleted at the end, replaced by `export_headers`)
- `src/code_gen.rs:158-349` — the six `gen_*` functions
- `src/code_gen.rs:351-513` — tests (`get_req`/`post_json_req` fixture style)
- Top-of-file imports: extend the existing `use crate::types::…` to include `FormDataRow, FormDataValue`.

---

### Task 1: Branch, shared helpers, cURL emitter

**Files:**
- Modify: `src/code_gen.rs` (imports, helpers, `gen_curl`, tests)

- [ ] **Step 1: Create the branch**

```bash
cd /mnt/e/code/poopman
git checkout -b feat/formdata-code-export
```

- [ ] **Step 2: Add the shared test fixture and failing cURL tests**

In `code_gen::tests`, next to `post_json_req()`:

```rust
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: the two new tests FAIL (output still contains the NOTE line, no `--form`).

- [ ] **Step 4: Implement helpers + cURL**

Top-of-file: extend the types import to
`use crate::types::{BodyType, FormDataRow, FormDataValue, RequestData};`
(keep whatever else is already imported there).

Replace `form_data_present` (keep it for now — other generators still call it; it goes away in Task 7) by ADDING these two helpers next to it:

```rust
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
```

Replace `gen_curl` entirely:

```rust
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
```

- [ ] **Step 5: Compile-check in WSL, then run the Windows test gate**

Run: `cargo check 2>&1 | tail -5`
Expected: no errors (warnings about the still-unused helpers are OK only if none — `form_rows`/`export_headers` are already used by gen_curl, `headers`/`form_data_present` still used by other generators, so expect a clean check).

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL tests pass, including the two new ones (existing `curl_*` tests must stay green).

- [ ] **Step 6: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(export): multipart form-data in cURL snippets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Rust (reqwest) emitter

**Files:**
- Modify: `src/code_gen.rs` (`gen_rust`, tests)

- [ ] **Step 1: Add the failing test**

```rust
    #[test]
    fn rust_form_data_builds_multipart() {
        let out = generate(CodeTarget::RustReqwest, &form_req());
        assert!(out.contains("use reqwest::blocking::multipart;"));
        assert!(out.contains(".text(\"note\", \"hello world\")"));
        assert!(out.contains(".file(\"avatar\", \"C:\\\\pics\\\\me.png\")?"));
        assert!(out.contains(".multipart(form)"));
        assert!(!out.contains(".body("));
        assert!(!out.contains("skipme"));
        assert!(!out.contains("Content-Type"));
        assert!(!out.contains("not yet supported"));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: `rust_form_data_builds_multipart` FAILS.

- [ ] **Step 3: Replace `gen_rust`**

```rust
fn gen_rust(req: &RequestData) -> String {
    let form = form_rows(req);
    let mut s = String::new();
    s.push_str("use reqwest::blocking::Client;\n");
    if !form.is_empty() {
        s.push_str("use reqwest::blocking::multipart;\n");
    }
    s.push_str("use reqwest::header::{HeaderMap, HeaderValue};\n\n");
    s.push_str("fn main() -> Result<(), Box<dyn std::error::Error>> {\n");
    s.push_str("    let client = Client::new();\n\n");
    s.push_str("    let mut headers = HeaderMap::new();\n");
    for (k, v) in export_headers(req) {
        s.push_str(&format!(
            "    headers.insert(\"{}\", HeaderValue::from_str(\"{}\")?);\n",
            dq(k),
            dq(v)
        ));
    }
    s.push('\n');
    if !form.is_empty() {
        let mut chain: Vec<String> = vec!["    let form = multipart::Form::new()".to_string()];
        for row in &form {
            match &row.value {
                FormDataValue::Text(v) => chain.push(format!(
                    "        .text(\"{}\", \"{}\")",
                    dq(&row.key),
                    dq(v)
                )),
                // .file() reads the file at send time; `?` bubbles its io::Result.
                FormDataValue::File { path } => chain.push(format!(
                    "        .file(\"{}\", \"{}\")?",
                    dq(&row.key),
                    dq(path)
                )),
            }
        }
        s.push_str(&chain.join("\n"));
        s.push_str(";\n\n");
    }
    s.push_str(&format!(
        "    let response = client\n        .request(reqwest::Method::{}, \"{}\")\n        .headers(headers)\n",
        req.method.as_str(),
        dq(&req.url)
    ));
    if !form.is_empty() {
        s.push_str("        .multipart(form)\n");
    } else if let Some(body) = raw_body(req) {
        s.push_str(&format!("        .body({})\n", rust_raw(&body)));
    }
    s.push_str("        .send()?;\n\n");
    s.push_str("    println!(\"{}\", response.text()?);\n");
    s.push_str("    Ok(())\n");
    s.push_str("}\n");
    s
}
```

- [ ] **Step 4: Run the test gate**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL pass (existing `rust_generates_blocking_client_and_escaped_body` and `multiline_body_stays_readable_not_escaped` stay green — raw-body path is untouched).

- [ ] **Step 5: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(export): multipart form-data in Rust reqwest snippets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Python (Requests) emitter

**Files:**
- Modify: `src/code_gen.rs` (`gen_python`, tests — REPLACES the old NOTE test)

- [ ] **Step 1: Replace the old NOTE test and add the new ones**

DELETE the test `form_data_adds_note_comment` (it asserts the NOTE that this task removes). ADD:

```rust
    #[test]
    fn python_form_data_uses_files_dict() {
        let out = generate(CodeTarget::PythonRequests, &form_req());
        assert!(out.contains("\"note\": (None, \"hello world\"),"));
        assert!(out.contains("\"avatar\": open(\"C:\\\\pics\\\\me.png\", \"rb\"),"));
        assert!(out.contains("files=files"));
        assert!(!out.contains("data="));
        assert!(!out.contains("skipme"));
        assert!(!out.contains("Content-Type"));
        assert!(!out.contains("not yet supported"));
    }

    #[test]
    fn python_all_text_form_still_multipart() {
        // Without files=, requests would send urlencoded — the (None, value)
        // tuple form forces a real multipart body even for text-only forms.
        let mut req = form_req();
        req.body = BodyType::FormData(vec![FormDataRow {
            enabled: true,
            key: "a".to_string(),
            value: FormDataValue::Text("1".to_string()),
        }]);
        let out = generate(CodeTarget::PythonRequests, &req);
        assert!(out.contains("files = {"));
        assert!(out.contains("\"a\": (None, \"1\"),"));
        assert!(out.contains("files=files"));
        assert!(!out.contains("payload"));
    }
```

- [ ] **Step 2: Run to verify the new tests fail**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: both new tests FAIL; everything else passes.

- [ ] **Step 3: Replace `gen_python`**

```rust
fn gen_python(req: &RequestData) -> String {
    let form = form_rows(req);
    let mut s = String::new();
    s.push_str("import requests\n\n");
    s.push_str(&format!("url = \"{}\"\n\n", dq(&req.url)));
    let hs = export_headers(req);
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
    if !form.is_empty() {
        // (None, value) tuples force multipart encoding even for text-only
        // forms; a bare data= dict would send x-www-form-urlencoded instead.
        s.push_str("files = {\n");
        for row in &form {
            match &row.value {
                FormDataValue::Text(v) => s.push_str(&format!(
                    "    \"{}\": (None, \"{}\"),\n",
                    dq(&row.key),
                    dq(v)
                )),
                FormDataValue::File { path } => s.push_str(&format!(
                    "    \"{}\": open(\"{}\", \"rb\"),\n",
                    dq(&row.key),
                    dq(path)
                )),
            }
        }
        s.push_str("}\n");
    }
    s.push('\n');
    if !form.is_empty() {
        s.push_str(&format!(
            "response = requests.request(\"{}\", url, headers=headers, files=files)\n",
            req.method.as_str()
        ));
    } else if body.is_some() {
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
```

- [ ] **Step 4: Run the test gate**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL pass.

- [ ] **Step 5: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(export): multipart form-data in Python Requests snippets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: JavaScript (Fetch) emitter + `file_basename` helper

**Files:**
- Modify: `src/code_gen.rs` (`gen_fetch`, new helper, tests)

- [ ] **Step 1: Add the failing tests**

```rust
    #[test]
    fn fetch_form_data_appends_fields() {
        let out = generate(CodeTarget::JavaScriptFetch, &form_req());
        assert!(out.contains("const formdata = new FormData();"));
        assert!(out.contains("formdata.append(\"note\", \"hello world\");"));
        // Browsers can't open local paths — the file row is a commented hint
        // that carries the field name and the file's basename.
        assert!(out.contains("// formdata.append(\"avatar\", fileInput.files[0], \"me.png\");"));
        assert!(out.contains("body: formdata,"));
        assert!(!out.contains("skipme"));
        assert!(!out.contains("Content-Type"));
        assert!(!out.contains("not yet supported"));
    }

    #[test]
    fn file_basename_handles_both_separators() {
        assert_eq!(file_basename("C:\\pics\\me.png"), "me.png");
        assert_eq!(file_basename("/home/u/me.png"), "me.png");
        assert_eq!(file_basename("me.png"), "me.png");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: both FAIL (`file_basename` doesn't compile yet — that's the expected "fails because it doesn't exist" state; comment out the second test if it blocks compilation of the rest, then restore it in Step 3).

- [ ] **Step 3: Implement**

Helper next to `py_string`/`js_string`:

```rust
/// Final path component of a local file path, tolerating / and \ separators
/// (paths come from the form-data UI and may be Windows- or POSIX-style).
fn file_basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
```

Replace `gen_fetch`:

```rust
fn gen_fetch(req: &RequestData) -> String {
    let form = form_rows(req);
    let mut s = String::new();
    s.push_str("const myHeaders = new Headers();\n");
    for (k, v) in export_headers(req) {
        s.push_str(&format!("myHeaders.append(\"{}\", \"{}\");\n", dq(k), dq(v)));
    }
    if !form.is_empty() {
        s.push_str("\nconst formdata = new FormData();\n");
        for row in &form {
            match &row.value {
                FormDataValue::Text(v) => s.push_str(&format!(
                    "formdata.append(\"{}\", \"{}\");\n",
                    dq(&row.key),
                    dq(v)
                )),
                FormDataValue::File { path } => s.push_str(&format!(
                    "// Browsers can't read local paths — wire this to an <input type=\"file\">:\n// formdata.append(\"{}\", fileInput.files[0], \"{}\");\n",
                    dq(&row.key),
                    dq(file_basename(path))
                )),
            }
        }
    }
    s.push_str("\nconst requestOptions = {\n");
    s.push_str(&format!("  method: \"{}\",\n", req.method.as_str()));
    s.push_str("  headers: myHeaders,\n");
    if !form.is_empty() {
        s.push_str("  body: formdata,\n");
    } else if let Some(b) = raw_body(req) {
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
```

- [ ] **Step 4: Run the test gate**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL pass.

- [ ] **Step 5: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(export): multipart form-data in JS fetch snippets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: NodeJS (Axios) emitter

**Files:**
- Modify: `src/code_gen.rs` (`gen_axios`, tests)

- [ ] **Step 1: Add the failing test**

```rust
    #[test]
    fn axios_form_data_uses_form_data_package() {
        let out = generate(CodeTarget::NodeAxios, &form_req());
        assert!(out.contains("const FormData = require(\"form-data\");"));
        assert!(out.contains("const fs = require(\"fs\");"));
        assert!(out.contains("formdata.append(\"note\", \"hello world\");"));
        assert!(out.contains(
            "formdata.append(\"avatar\", fs.createReadStream(\"C:\\\\pics\\\\me.png\"));"
        ));
        assert!(out.contains("...formdata.getHeaders(),"));
        assert!(out.contains("data: formdata,"));
        assert!(!out.contains("skipme"));
        assert!(!out.contains("\"Content-Type\""));
        assert!(!out.contains("not yet supported"));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: the new test FAILS.

- [ ] **Step 3: Replace `gen_axios`**

```rust
fn gen_axios(req: &RequestData) -> String {
    let form = form_rows(req);
    let has_file = form
        .iter()
        .any(|r| matches!(r.value, FormDataValue::File { .. }));
    let mut s = String::new();
    s.push_str("const axios = require(\"axios\");\n");
    if !form.is_empty() {
        s.push_str("const FormData = require(\"form-data\");\n");
        if has_file {
            s.push_str("const fs = require(\"fs\");\n");
        }
        s.push_str("\nconst formdata = new FormData();\n");
        for row in &form {
            match &row.value {
                FormDataValue::Text(v) => s.push_str(&format!(
                    "formdata.append(\"{}\", \"{}\");\n",
                    dq(&row.key),
                    dq(v)
                )),
                FormDataValue::File { path } => s.push_str(&format!(
                    "formdata.append(\"{}\", fs.createReadStream(\"{}\"));\n",
                    dq(&row.key),
                    dq(path)
                )),
            }
        }
    }
    s.push('\n');
    s.push_str("const config = {\n");
    s.push_str(&format!("  method: \"{}\",\n", req.method.as_str().to_lowercase()));
    s.push_str(&format!("  url: \"{}\",\n", dq(&req.url)));
    let hs = export_headers(req);
    if !form.is_empty() {
        // form-data's getHeaders() provides the multipart Content-Type with
        // the generated boundary; exported headers merge after it.
        s.push_str("  headers: {\n    ...formdata.getHeaders(),\n");
        for (k, v) in hs {
            s.push_str(&format!("    \"{}\": \"{}\",\n", dq(k), dq(v)));
        }
        s.push_str("  },\n");
    } else if hs.is_empty() {
        s.push_str("  headers: {},\n");
    } else {
        s.push_str("  headers: {\n");
        for (k, v) in hs {
            s.push_str(&format!("    \"{}\": \"{}\",\n", dq(k), dq(v)));
        }
        s.push_str("  },\n");
    }
    if !form.is_empty() {
        s.push_str("  data: formdata,\n");
    } else if let Some(b) = raw_body(req) {
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
```

Note the emission-order change: the old version printed `const axios = ...` then a blank line immediately; the new version prints the form block between them, then one blank line before `const config`. The old no-form output shape stays byte-identical (`const axios...` + `\n` + blank + `const config`), keeping the existing `axios_lowercases_method` test green.

- [ ] **Step 4: Run the test gate**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL pass.

- [ ] **Step 5: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(export): multipart form-data in Node axios snippets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Go (net/http) emitter

**Files:**
- Modify: `src/code_gen.rs` (`gen_go`, tests)

- [ ] **Step 1: Add the failing test**

```rust
    #[test]
    fn go_form_data_uses_multipart_writer() {
        let out = generate(CodeTarget::GoNetHttp, &form_req());
        assert!(out.contains("\"bytes\""));
        assert!(out.contains("\"mime/multipart\""));
        assert!(out.contains("\"os\""));
        assert!(out.contains("\"path/filepath\""));
        assert!(out.contains("writer := multipart.NewWriter(payload)"));
        assert!(out.contains("_ = writer.WriteField(\"note\", \"hello world\")"));
        assert!(out.contains("file0, err := os.Open(\"C:\\\\pics\\\\me.png\")"));
        assert!(out.contains(
            "part0, err := writer.CreateFormFile(\"avatar\", filepath.Base(\"C:\\\\pics\\\\me.png\"))"
        ));
        assert!(out.contains("req.Header.Set(\"Content-Type\", writer.FormDataContentType())"));
        assert!(out.contains("http.NewRequest(method, url, payload)"));
        assert!(!out.contains("skipme"));
        assert!(!out.contains("req.Header.Add(\"Content-Type\""));
        assert!(!out.contains("not yet supported"));
        assert!(!out.contains("\"strings\""));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: the new test FAILS.

- [ ] **Step 3: Replace `gen_go`**

```rust
fn gen_go(req: &RequestData) -> String {
    let form = form_rows(req);
    let has_file = form
        .iter()
        .any(|r| matches!(r.value, FormDataValue::File { .. }));
    let body = raw_body(req);
    let mut s = String::new();
    s.push_str("package main\n\n");
    s.push_str("import (\n");
    if !form.is_empty() {
        s.push_str("\t\"bytes\"\n");
    }
    s.push_str("\t\"fmt\"\n");
    s.push_str("\t\"io\"\n");
    if !form.is_empty() {
        s.push_str("\t\"mime/multipart\"\n");
    }
    s.push_str("\t\"net/http\"\n");
    if has_file {
        s.push_str("\t\"os\"\n");
        s.push_str("\t\"path/filepath\"\n");
    }
    if form.is_empty() && body.is_some() {
        s.push_str("\t\"strings\"\n");
    }
    s.push_str(")\n\n");
    s.push_str("func main() {\n");
    s.push_str(&format!("\turl := \"{}\"\n", dq(&req.url)));
    s.push_str(&format!("\tmethod := \"{}\"\n\n", req.method.as_str()));
    if !form.is_empty() {
        s.push_str("\tpayload := &bytes.Buffer{}\n");
        s.push_str("\twriter := multipart.NewWriter(payload)\n");
        let mut file_idx = 0usize;
        for row in &form {
            match &row.value {
                FormDataValue::Text(v) => s.push_str(&format!(
                    "\t_ = writer.WriteField(\"{}\", \"{}\")\n",
                    dq(&row.key),
                    dq(v)
                )),
                FormDataValue::File { path } => {
                    s.push_str(&format!(
                        "\tfile{i}, err := os.Open(\"{p}\")\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n",
                        i = file_idx,
                        p = dq(path)
                    ));
                    s.push_str(&format!(
                        "\tpart{i}, err := writer.CreateFormFile(\"{k}\", filepath.Base(\"{p}\"))\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n",
                        i = file_idx,
                        k = dq(&row.key),
                        p = dq(path)
                    ));
                    s.push_str(&format!(
                        "\t_, err = io.Copy(part{i}, file{i})\n\tfile{i}.Close()\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n",
                        i = file_idx
                    ));
                    file_idx += 1;
                }
            }
        }
        s.push_str("\tif err := writer.Close(); err != nil {\n\t\tfmt.Println(err)\n\t\treturn\n\t}\n\n");
        s.push_str("\tclient := &http.Client{}\n");
        s.push_str("\treq, err := http.NewRequest(method, url, payload)\n");
    } else if let Some(b) = &body {
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
    for (k, v) in export_headers(req) {
        s.push_str(&format!("\treq.Header.Add(\"{}\", \"{}\")\n", dq(k), dq(v)));
    }
    if !form.is_empty() {
        s.push_str("\treq.Header.Set(\"Content-Type\", writer.FormDataContentType())\n");
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
```

Note on the pre-file `err` variable: the first file block introduces `err` via `file0, err := os.Open(...)` — `:=` is legal because `file0` is new. A form with ONLY text fields declares no `err` before `req, err := http.NewRequest` (also `:=`, `req` is new). Both shapes compile.

- [ ] **Step 4: Run the test gate**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL pass (existing `go_uses_backtick_body_and_strings_import` / `go_uses_nil_body_without_strings_import` stay green).

- [ ] **Step 5: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(export): multipart form-data in Go net/http snippets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: Cleanup, edge-case tests, docs

**Files:**
- Modify: `src/code_gen.rs` (delete dead helpers, module doc, tests)
- Modify: `README.md`

- [ ] **Step 1: Add the edge-case tests**

```rust
    #[test]
    fn disabled_only_form_data_exports_like_no_body() {
        let mut req = form_req();
        req.body = BodyType::FormData(vec![FormDataRow {
            enabled: false,
            key: "x".to_string(),
            value: FormDataValue::Text("y".to_string()),
        }]);
        let curl = generate(CodeTarget::Curl, &req);
        assert!(!curl.contains("--form"));
        assert!(!curl.contains("--data"));
        // The pinned multipart Content-Type is skipped for ANY form-data body —
        // exporting it without a matching body would produce a broken request.
        assert!(!curl.contains("Content-Type"));
        let go = generate(CodeTarget::GoNetHttp, &req);
        assert!(go.contains("http.NewRequest(method, url, nil)"));
    }

    #[test]
    fn raw_body_still_exports_content_type() {
        // Control: the Content-Type skip must not leak to raw bodies.
        let out = generate(CodeTarget::Curl, &post_json_req());
        assert!(out.contains("--header 'Content-Type: application/json'"));
    }
```

- [ ] **Step 2: Delete the dead helpers**

Remove `form_data_present` (`src/code_gen.rs:79-82` in the pre-change numbering) and the old `headers()` function — every generator now uses `export_headers`/`form_rows`. `cargo check` must come back clean with no `dead_code` warnings.

- [ ] **Step 3: Update the module doc**

Replace the module-doc sentence at `src/code_gen.rs:5` (`//! v1 supports \`None\` and \`Raw\` request bodies across all targets. \`FormData\` …`) with:

```rust
//! Supports `None`, `Raw`, and multipart `FormData` bodies across all targets.
//! Form-data exports skip the UI-pinned Content-Type header — each target's
//! HTTP library generates its own multipart boundary.
```

(Keep the surrounding module-doc lines as they are.)

- [ ] **Step 4: Update README**

In `README.md`:
- Under **Possible Future Enhancements**, delete the line
  `- [ ] Form-data export in code snippets (currently Raw/None bodies only).`
- In the **Features** bullet for code snippet export, append a sentence so it reads:
  `- **Code snippet export** — turn the current request into runnable code via the \`</>\` button: **cURL, Rust (reqwest), Python (Requests), JavaScript (Fetch), NodeJS (Axios), Go (net/http)**, with Copy. Raw and multipart form-data bodies both export.`

- [ ] **Step 5: Run checks**

Run: `cargo check 2>&1 | tail -3` (WSL) — expected: clean, no warnings.
Run: `cargo clippy 2>&1 | tail -3` (WSL) — expected: no new warnings.
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`
Expected: ALL pass.

- [ ] **Step 6: Commit**

```bash
git add src/code_gen.rs README.md
git commit -m "refactor(export): drop dead form-data helpers, update docs

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: Full suite + PR

- [ ] **Step 1: Run the FULL test suite on Windows**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`
Expected: all crate tests pass (db, variables, url_params, curl_import, format, … — not just code_gen).

- [ ] **Step 2: Push and open the PR**

```bash
git push -u origin feat/formdata-code-export
gh pr create --title "feat: form-data body support in code snippet export" --body "$(cat <<'EOF'
All six code-export targets (cURL, Rust reqwest, Python Requests, JS fetch, Node axios, Go net/http) now generate real multipart form-data code instead of a "not yet supported" note.

Spec: docs/superpowers/specs/2026-07-14-formdata-code-export-design.md

- Only enabled rows with non-blank keys export; disabled-only forms export like a body-less request.
- The UI-pinned `multipart/form-data; boundary=<auto>` Content-Type header is skipped for form-data bodies in every target — each library generates its own boundary.
- cURL text fields use `--form-string` (immune to `@`/`<` file expansion); Python uses `(None, value)` file-tuples so text-only forms still send multipart; browser fetch emits a commented `fileInput.files[0]` placeholder for file fields; axios merges `formdata.getHeaders()`; Go indexes `fileN`/`partN` per file row.
- Pure `src/code_gen.rs` change, fully unit-tested (`cargo test code_gen`).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Report the PR URL to the user (via Telegram — they're on mobile)**
