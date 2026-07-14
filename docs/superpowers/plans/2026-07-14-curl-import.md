# cURL Import (Smart Paste) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pasting a `curl …` command into the URL bar parses it and populates method/URL/headers/body via the existing `load_request` path.

**Architecture:** A pure parser module `src/curl_import.rs` (shell-style tokenizer + flag loop) returns `Option<RequestData>` — reusing the existing request model so the UI hook is three lines: detect the `curl ` prefix in the URL input's `Change` event, parse, call `RequestEditor::load_request`.

**Tech Stack:** Rust; `base64` crate (already in the dependency tree via reqwest — added as a direct dep for `-u` Basic Auth); existing `HttpMethod`/`BodyType`/`RequestData` types.

**Test gate:** tests run on Windows: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`. WSL: `cargo check --tests` / `cargo clippy --all-targets` only.

---

### Task 0: Branch + dependency

- [ ] **Step 1: Branch off local main** (must equal origin/main)

```bash
cd /mnt/e/code/poopman && git checkout -b feat/curl-import
```

- [ ] **Step 2: Add base64 as a direct dependency**

In `Cargo.toml` under `[dependencies]` (keep alphabetical-ish placement near the top deps):

```toml
base64 = "0.22"
```

Run: `cargo check` — expected: clean (crate already compiled in the tree).

---

### Task 1: Parser module (TDD)

**Files:**
- Create: `src/curl_import.rs`
- Modify: `src/main.rs` (add `mod curl_import;` next to the other mods)
- Test: same file, `#[cfg(test)] mod tests`

**Parsing rules (the contract the tests pin down):**

- First token must be exactly `curl`, else `None`. Empty/whitespace input → `None`.
- Tokenizer: POSIX-ish — single quotes (no escapes inside), double quotes (`\"` and `\\` escapes), bare-word backslash escapes… with ONE deliberate deviation: a backslash followed by whitespace is a token break, not an escaped space. Rationale: multi-line curl commands pasted into the single-line URL input may arrive with `\<newline>` flattened to `\<space>`; line continuations must survive that. (Escaped literal spaces in URLs are rare and should be `%20` anyway.)
- Flags (short flags accept attached values `-XPOST`; long flags accept `--flag=value`):
  - `-X` / `--request` → method (via `HttpMethod::from_str`, unknown → ignored)
  - `-H` / `--header` → split at first `:`, trim both sides
  - `-d` / `--data` / `--data-raw` / `--data-binary` / `--data-urlencode` → body part; multiple parts join with `&` (curl semantics); implies POST when no explicit `-X`
  - `-F` / `--form` → `key=value` (`@path` value → file part); body becomes FormData; implies POST
  - `-u` / `--user` → `Authorization: Basic base64(user:pass)` header
  - `--url` → URL
  - Bare token not starting with `-` → URL (first one wins)
  - Anything else → skipped silently (`-s`, `-L`, `--compressed`, …). Known limitation: an unknown flag that takes a value leaves that value as a stray token, which is only harmful if the URL hasn't been seen yet.
- Body subtype from a parsed `Content-Type` header: contains `json` → Json, `xml` → Xml, `javascript` → JavaScript, else Text.
- Returns `Option<RequestData>` — `None` when the input isn't a parsable curl command or has no URL.

- [ ] **Step 1: Write the failing tests**

Create `src/curl_import.rs` containing ONLY the tests plus a stub:

```rust
//! Parse a pasted `curl …` command line into a [`RequestData`].

use crate::types::RequestData;

pub fn parse_curl(_input: &str) -> Option<RequestData> {
    todo!("implemented in the GREEN step")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, FormDataValue, HttpMethod, RawSubtype};

    fn parse(s: &str) -> RequestData {
        parse_curl(s).expect("should parse")
    }

    #[test]
    fn simple_get() {
        let r = parse("curl https://example.com/api");
        assert_eq!(r.method, HttpMethod::GET);
        assert_eq!(r.url, "https://example.com/api");
        assert!(r.headers.is_empty());
        assert!(matches!(r.body, BodyType::None));
    }

    #[test]
    fn non_curl_input_is_rejected() {
        assert!(parse_curl("wget https://example.com").is_none());
        assert!(parse_curl("").is_none());
        assert!(parse_curl("curl").is_none()); // no URL
        assert!(parse_curl("https://example.com").is_none());
    }

    #[test]
    fn single_quotes_preserve_content() {
        let r = parse("curl 'https://example.com/a b?x=1&y=2'");
        assert_eq!(r.url, "https://example.com/a b?x=1&y=2");
    }

    #[test]
    fn double_quotes_with_escapes() {
        let r = parse(r#"curl -H "X-Note: say \"hi\"" https://example.com"#);
        assert_eq!(r.headers, vec![("X-Note".to_string(), r#"say "hi""#.to_string())]);
    }

    #[test]
    fn explicit_method_flag() {
        assert_eq!(parse("curl -X PUT https://example.com").method, HttpMethod::PUT);
        assert_eq!(parse("curl --request DELETE https://example.com").method, HttpMethod::DELETE);
    }

    #[test]
    fn attached_and_equals_forms() {
        assert_eq!(parse("curl -XPOST https://example.com").method, HttpMethod::POST);
        assert_eq!(parse("curl --request=PATCH https://example.com").method, HttpMethod::PATCH);
    }

    #[test]
    fn headers_split_at_first_colon_and_trim() {
        let r = parse("curl -H 'X-Time: 12:30:00' https://example.com");
        assert_eq!(r.headers, vec![("X-Time".to_string(), "12:30:00".to_string())]);
    }

    #[test]
    fn multiple_headers_keep_order() {
        let r = parse("curl -H 'A: 1' -H 'B: 2' https://example.com");
        assert_eq!(
            r.headers,
            vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]
        );
    }

    #[test]
    fn data_implies_post_and_json_subtype_from_header() {
        let r = parse(
            "curl -H 'Content-Type: application/json' -d '{\"a\":1}' https://example.com",
        );
        assert_eq!(r.method, HttpMethod::POST);
        match r.body {
            BodyType::Raw { content, subtype } => {
                assert_eq!(content, "{\"a\":1}");
                assert_eq!(subtype, RawSubtype::Json);
            }
            other => panic!("expected raw body, got {:?}", other),
        }
    }

    #[test]
    fn explicit_method_wins_over_data_implied_post() {
        let r = parse("curl -X PUT -d 'x=1' https://example.com");
        assert_eq!(r.method, HttpMethod::PUT);
    }

    #[test]
    fn multiple_data_parts_join_with_ampersand() {
        let r = parse("curl -d a=1 -d b=2 https://example.com");
        match r.body {
            BodyType::Raw { content, subtype } => {
                assert_eq!(content, "a=1&b=2");
                assert_eq!(subtype, RawSubtype::Text);
            }
            other => panic!("expected raw body, got {:?}", other),
        }
    }

    #[test]
    fn form_fields_text_and_file() {
        let r = parse("curl -F name=alice -F avatar=@/tmp/a.png https://example.com");
        assert_eq!(r.method, HttpMethod::POST);
        match r.body {
            BodyType::FormData(rows) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].key, "name");
                assert!(matches!(&rows[0].value, FormDataValue::Text(t) if t == "alice"));
                assert_eq!(rows[1].key, "avatar");
                assert!(matches!(&rows[1].value, FormDataValue::File { path } if path == "/tmp/a.png"));
                assert!(rows.iter().all(|row| row.enabled));
            }
            other => panic!("expected form body, got {:?}", other),
        }
    }

    #[test]
    fn user_flag_becomes_basic_auth_header() {
        let r = parse("curl -u user:pass https://example.com");
        assert_eq!(
            r.headers,
            vec![("Authorization".to_string(), "Basic dXNlcjpwYXNz".to_string())]
        );
    }

    #[test]
    fn url_flag_and_bare_url_first_wins() {
        assert_eq!(parse("curl --url https://a.example").url, "https://a.example");
        let r = parse("curl https://first.example https://second.example");
        assert_eq!(r.url, "https://first.example");
    }

    #[test]
    fn line_continuations_and_flattened_backslashes() {
        let cmd = "curl -X POST \\\n  -H 'A: 1' \\\n  https://example.com";
        let r = parse(cmd);
        assert_eq!(r.method, HttpMethod::POST);
        assert_eq!(r.url, "https://example.com");
        // Same command flattened to one line (single-line input paste).
        let flat = "curl -X POST \\ -H 'A: 1' \\ https://example.com";
        let r = parse(flat);
        assert_eq!(r.method, HttpMethod::POST);
        assert_eq!(r.url, "https://example.com");
    }

    #[test]
    fn unknown_flags_are_skipped() {
        let r = parse("curl -s -L --compressed https://example.com");
        assert_eq!(r.url, "https://example.com");
        assert_eq!(r.method, HttpMethod::GET);
    }
}
```

Add `mod curl_import;` to `src/main.rs` alongside the existing module declarations.

- [ ] **Step 2: Verify RED**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test curl_import"`
Expected: FAIL — every test panics at `todo!` (or compile error if signatures drift; fix signatures, not tests).

- [ ] **Step 3: Implement**

Replace the stub in `src/curl_import.rs`:

```rust
//! Parse a pasted `curl …` command line into a [`RequestData`].
//!
//! Deliberately lenient: unknown flags are skipped, and a backslash followed
//! by whitespace is treated as a token break (a multi-line command pasted
//! into the single-line URL input arrives with `\<newline>` flattened to
//! `\<space>`; POSIX "escaped space" semantics would corrupt it).

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use crate::types::{BodyType, FormDataRow, FormDataValue, HttpMethod, RawSubtype, RequestData};

/// Shell-style tokenizer. Single quotes take content verbatim; double quotes
/// honor `\"` and `\\`; outside quotes a backslash escapes the next char,
/// except before whitespace where it is a token break (see module docs).
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                has_token = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    current.push(q);
                }
            }
            '"' => {
                has_token = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(&next) = chars.peek()
                                && (next == '"' || next == '\\')
                            {
                                current.push(next);
                                chars.next();
                            } else {
                                current.push('\\');
                            }
                        }
                        _ => current.push(q),
                    }
                }
            }
            '\\' => {
                match chars.peek() {
                    // Line continuation / flattened continuation: token break.
                    Some(&next) if next.is_whitespace() => {
                        if has_token {
                            tokens.push(std::mem::take(&mut current));
                            has_token = false;
                        }
                    }
                    Some(&next) => {
                        current.push(next);
                        has_token = true;
                        chars.next();
                    }
                    None => {}
                }
            }
            c if c.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            _ => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        tokens.push(current);
    }
    tokens
}

/// For `-H` style flags: value may be attached (`-Hfoo`), separate (`-H foo`),
/// or `--header=foo`. Returns the value and advances `i` past it.
fn flag_value(tokens: &[String], i: &mut usize, short: &str, long: &str) -> Option<String> {
    let tok = &tokens[*i];
    if let Some(rest) = tok.strip_prefix(long) {
        if rest.is_empty() {
            *i += 1;
            return tokens.get(*i).cloned();
        }
        if let Some(v) = rest.strip_prefix('=') {
            return Some(v.to_string());
        }
        return None; // e.g. "--headers" is not "--header"
    }
    if !short.is_empty()
        && let Some(rest) = tok.strip_prefix(short)
    {
        if rest.is_empty() {
            *i += 1;
            return tokens.get(*i).cloned();
        }
        return Some(rest.to_string()); // attached: -XPOST
    }
    None
}

fn matches_flag(tok: &str, short: &str, long: &str) -> bool {
    tok == short
        || tok == long
        || (!short.is_empty() && tok.starts_with(short) && tok.len() > short.len() && !tok.starts_with(long))
        || tok.starts_with(&format!("{}=", long))
}

pub fn parse_curl(input: &str) -> Option<RequestData> {
    let tokens = tokenize(input);
    if tokens.first().map(String::as_str) != Some("curl") {
        return None;
    }

    let mut method: Option<HttpMethod> = None;
    let mut url = String::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data_parts: Vec<String> = Vec::new();
    let mut form_rows: Vec<FormDataRow> = Vec::new();

    let mut i = 1;
    while i < tokens.len() {
        let tok = tokens[i].clone();
        if matches_flag(&tok, "-X", "--request") {
            if let Some(v) = flag_value(&tokens, &mut i, "-X", "--request")
                && let Some(m) = HttpMethod::from_str(&v.to_ascii_uppercase())
            {
                method = Some(m);
            }
        } else if matches_flag(&tok, "-H", "--header") {
            if let Some(v) = flag_value(&tokens, &mut i, "-H", "--header")
                && let Some((k, val)) = v.split_once(':')
            {
                headers.push((k.trim().to_string(), val.trim().to_string()));
            }
        } else if matches_flag(&tok, "", "--data-raw")
            || matches_flag(&tok, "", "--data-binary")
            || matches_flag(&tok, "", "--data-urlencode")
        {
            let long = if tok.starts_with("--data-raw") {
                "--data-raw"
            } else if tok.starts_with("--data-binary") {
                "--data-binary"
            } else {
                "--data-urlencode"
            };
            if let Some(v) = flag_value(&tokens, &mut i, "", long) {
                data_parts.push(v);
            }
        } else if matches_flag(&tok, "-d", "--data") {
            if let Some(v) = flag_value(&tokens, &mut i, "-d", "--data") {
                data_parts.push(v);
            }
        } else if matches_flag(&tok, "-F", "--form") {
            if let Some(v) = flag_value(&tokens, &mut i, "-F", "--form")
                && let Some((k, val)) = v.split_once('=')
            {
                let value = match val.strip_prefix('@') {
                    Some(path) => FormDataValue::File { path: path.to_string() },
                    None => FormDataValue::Text(val.to_string()),
                };
                form_rows.push(FormDataRow { enabled: true, key: k.to_string(), value });
            }
        } else if matches_flag(&tok, "-u", "--user") {
            if let Some(v) = flag_value(&tokens, &mut i, "-u", "--user") {
                headers.push(("Authorization".to_string(), format!("Basic {}", BASE64.encode(v))));
            }
        } else if matches_flag(&tok, "", "--url") {
            if let Some(v) = flag_value(&tokens, &mut i, "", "--url")
                && url.is_empty()
            {
                url = v;
            }
        } else if !tok.starts_with('-') {
            if url.is_empty() {
                url = tok;
            }
        }
        // Unknown flags fall through and are skipped.
        i += 1;
    }

    if url.is_empty() {
        return None;
    }

    let body = if !form_rows.is_empty() {
        BodyType::FormData(form_rows)
    } else if !data_parts.is_empty() {
        let content_type = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.to_ascii_lowercase())
            .unwrap_or_default();
        let subtype = if content_type.contains("json") {
            RawSubtype::Json
        } else if content_type.contains("xml") {
            RawSubtype::Xml
        } else if content_type.contains("javascript") {
            RawSubtype::JavaScript
        } else {
            RawSubtype::Text
        };
        BodyType::Raw { content: data_parts.join("&"), subtype }
    } else {
        BodyType::None
    };

    let method = method.unwrap_or(if matches!(body, BodyType::None) {
        HttpMethod::GET
    } else {
        HttpMethod::POST
    });

    Some(RequestData { method, url, headers, body })
}
```

Implementation notes:
- Check `HttpMethod::from_str`'s actual signature in `src/types.rs` first — if it returns `Result`, use `.ok()`.
- `RawSubtype`/`HttpMethod` need `PartialEq` for the test asserts — check derives in `src/types.rs`, add if missing.
- `matches_flag` short-prefix check must not treat `--data` as attached `-d` value: the `!tok.starts_with(long)` guard handles it, and the dispatch order above tests the `--data-*` long variants before `-d/--data`.

- [ ] **Step 4: Verify GREEN**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test curl_import"`
Expected: 16 passed.
Then full suite: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` — expected 101 passed (85 + 16).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/curl_import.rs src/main.rs
git commit -m "feat(import): cURL command parser -> RequestData"
```

---

### Task 2: URL-bar smart-paste hook

**Files:**
- Modify: `src/request_editor.rs` (the URL input subscription in `new()`, ~line 97)

- [ ] **Step 1: Extend the subscription**

Replace:

```rust
        // Subscribe to URL input changes to parse params
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.parse_url_to_params(window, cx);
        });
```

with:

```rust
        // Subscribe to URL input changes: a pasted `curl …` command imports the
        // whole request; anything else just re-parses query params.
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let value = this.url_input.read(cx).value().to_string();
                if value.trim_start().starts_with("curl ")
                    && let Some(request) = crate::curl_import::parse_curl(&value)
                {
                    // load_request rewrites the URL input, which re-fires
                    // Change — the new value no longer starts with "curl",
                    // so there is no loop.
                    this.load_request(&request, window, cx);
                    return;
                }
            }
            this.parse_url_to_params(window, cx);
        });
```

- [ ] **Step 2: Compile check + full suite**

Run: `cargo check` (WSL) — clean.
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` — 101 passed.

- [ ] **Step 3: Commit**

```bash
git add src/request_editor.rs
git commit -m "feat(editor): smart-paste cURL commands into the URL bar"
```

---

### Task 3: Final gates + PR

- [ ] **Step 1:** `cargo clippy --all-targets` (WSL) — 0 warnings; fix + `style:` commit if any.
- [ ] **Step 2:** Full Windows suite — `test result: ok. 101 passed; 0 failed`.
- [ ] **Step 3:** Push `feat/curl-import`, open PR titled `feat: import pasted cURL commands in the URL bar`, body summarizing parser rules + limitations (unknown value-flags, `--data-urlencode` not percent-encoded), with a visual checklist:
  1. Paste `curl -X POST -H 'Content-Type: application/json' -d '{"a":1}' https://httpbin.org/post` into the URL bar → method POST, URL filled, header present, JSON body in Body tab.
  2. Paste a multi-line curl command (with `\` continuations) → same result.
  3. Paste a normal URL → behaves exactly as before (params sync intact).
  4. Type (not paste) a URL character by character → no false triggers.

---

## Self-review notes

- Spec coverage: tokenizer ✅, all listed flags ✅, unknown-flag skip ✅, `RequestData` reuse (spec said `ParsedCurl`; using `RequestData` directly is strictly simpler — spec's "body reuses the existing BodyType model" intent preserved) ✅, UI hook on Change ✅, parse-failure leaves text ✅ (hook only acts on successful parse).
- The `\<whitespace>` = token-break deviation from POSIX is documented in module docs and pinned by `line_continuations_and_flattened_backslashes`.
- Type consistency: `FormDataRow { enabled, key, value }` and `FormDataValue::{Text, File{path}}` match `src/types.rs` as read on 2026-07-14; `HttpMethod::from_str` signature to be verified at implementation time (Result vs Option).
