# Auth Helper Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Postman-style **Auth** sub-tab (None / Bearer / Basic / API-Key) whose configuration persists on the request and is computed into a wire header at send time, overriding a manual same-name header.

**Architecture:** Auth is a flat `AuthConfig` struct persisted on `RequestData` (not a header generator). A pure `compute_header()` derives the wire header; a pure `effective_wire_headers()` merges it over the manual headers (auth wins on same name). That single merge function feeds both the send path and code generation. A new `AuthEditor` GPUI entity (mirroring `BodyEditor`) is embedded as a request sub-tab **immediately after Headers** (order: Headers · Auth · Params · Body). History gains a `request_auth` column via an idempotent migration.

**Tech Stack:** Rust 2024, GPUI 0.2 + gpui-component 0.5, rusqlite 0.40, serde/serde_json, `base64 = "0.22"` (already a dependency), tokio.

**Spec:** `docs/superpowers/specs/2026-07-22-auth-helper-tab-design.md`

---

## Conventions

- **Tests run on the Windows toolchain.** WSL2 cannot link GPUI, so *every* `cargo` invocation in this plan runs through Windows PowerShell from the repo root (`/mnt/e` is the Windows `E:` drive):

  ```bash
  pwsh.exe -NoProfile -Command "cargo test <filter>"
  pwsh.exe -NoProfile -Command "cargo build"
  ```

  When a step says "Run: `cargo test x`", execute `pwsh.exe -NoProfile -Command "cargo test x"`.
- **GUI behavior is user-verified, not test-verified** (Tasks 8–9). Those tasks end with a manual verification checklist the user runs on the Windows build; do not claim the GUI works from a green `cargo build`.
- `base64 = "0.22"` is **already** in `Cargo.toml` — no dependency edit is needed.

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `src/types.rs` | `AuthType`, `AuthConfig`, `compute_header`, `effective_wire_headers`, `RequestData.auth` | Modify |
| `src/variables.rs` | `substitute_auth` + auth in `substitute_request` | Modify |
| `src/db.rs` | `request_auth` column migration, insert, read | Modify |
| `src/curl_import.rs` | `-u/--user` → Basic auth config (was: header) | Modify |
| `src/code_gen.rs` | `generate()` folds computed auth header in | Modify |
| `src/auth_editor.rs` | `AuthEditor` GPUI component | **Create** |
| `src/request_editor.rs` | Embed AuthEditor as sub-tab 1 (after Headers); inject at send; restore on load | Modify |
| `src/request_tab.rs` | `new_empty` sets `auth: default` | Modify |
| `src/app.rs` | `insert_history` call passes auth | Modify |
| `src/main.rs` | `mod auth_editor;` | Modify |

---

## Task 1: `AuthType` + `AuthConfig` + `compute_header`

Pure data + logic. `RequestData` is **not** touched yet, so nothing else breaks.

**Files:**
- Modify: `src/types.rs` (add types near the other request types, e.g. after `BodyType`'s `impl Default` around line 209; add `use` for base64 at top)
- Test: `src/types.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `src/types.rs`:

```rust
#[test]
fn compute_header_none_and_empty_fields_emit_nothing() {
    assert_eq!(AuthConfig::default().compute_header(), None);
    // Bearer with empty token → nothing (don't send a dangling "Bearer ")
    let a = AuthConfig { auth_type: AuthType::Bearer, ..Default::default() };
    assert_eq!(a.compute_header(), None);
    // Basic with both fields empty → nothing
    let a = AuthConfig { auth_type: AuthType::Basic, ..Default::default() };
    assert_eq!(a.compute_header(), None);
    // ApiKey with empty name → nothing
    let a = AuthConfig { auth_type: AuthType::ApiKey, api_key_value: "v".into(), ..Default::default() };
    assert_eq!(a.compute_header(), None);
}

#[test]
fn compute_header_bearer() {
    let a = AuthConfig { auth_type: AuthType::Bearer, bearer_token: "t0ken".into(), ..Default::default() };
    assert_eq!(a.compute_header(), Some(("Authorization".into(), "Bearer t0ken".into())));
}

#[test]
fn compute_header_basic_base64() {
    let a = AuthConfig {
        auth_type: AuthType::Basic,
        basic_username: "user".into(),
        basic_password: "pass".into(),
        ..Default::default()
    };
    // base64("user:pass") == "dXNlcjpwYXNz"
    assert_eq!(a.compute_header(), Some(("Authorization".into(), "Basic dXNlcjpwYXNz".into())));
}

#[test]
fn compute_header_basic_username_only() {
    let a = AuthConfig { auth_type: AuthType::Basic, basic_username: "user".into(), ..Default::default() };
    // base64("user:") == "dXNlcjo="
    assert_eq!(a.compute_header(), Some(("Authorization".into(), "Basic dXNlcjo=".into())));
}

#[test]
fn compute_header_api_key_uses_custom_name() {
    let a = AuthConfig {
        auth_type: AuthType::ApiKey,
        api_key_name: "X-API-Key".into(),
        api_key_value: "secret".into(),
        ..Default::default()
    };
    assert_eq!(a.compute_header(), Some(("X-API-Key".into(), "secret".into())));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test compute_header`
Expected: FAIL to compile ("cannot find type `AuthConfig`").

- [ ] **Step 3: Implement the types and `compute_header`**

At the top of `src/types.rs`, add the base64 import below the existing `use` lines:

```rust
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
```

Then add the types (place them right after `impl Default for BodyType { ... }`, before `RequestData`):

```rust
/// Authentication scheme selected in the Auth sub-tab.
///
/// Variant names are serialized by name into the history database, so renaming
/// them would break previously saved requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthType {
    #[default]
    None,
    Bearer,
    Basic,
    ApiKey,
}

/// Config-based auth: a flat struct (all fields always present) so switching
/// type in the UI preserves each type's previously-typed values, matching
/// Postman. The wire header is *computed* from this — auth is never stored as a
/// header row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    pub auth_type: AuthType,
    pub bearer_token: String,
    pub basic_username: String,
    pub basic_password: String,
    /// Header name for API-Key auth, e.g. "X-API-Key".
    pub api_key_name: String,
    pub api_key_value: String,
}

impl AuthConfig {
    /// The header this auth would put on the wire, or `None`.
    ///
    /// Emitted only when the relevant field(s) are non-empty, so an in-progress
    /// edit never sends a placeholder header (e.g. a dangling `Bearer `). This
    /// differs slightly from Postman, which emits once a type is selected.
    pub fn compute_header(&self) -> Option<(String, String)> {
        match self.auth_type {
            AuthType::None => None,
            AuthType::Bearer => {
                if self.bearer_token.is_empty() {
                    None
                } else {
                    Some(("Authorization".to_string(), format!("Bearer {}", self.bearer_token)))
                }
            }
            AuthType::Basic => {
                if self.basic_username.is_empty() && self.basic_password.is_empty() {
                    None
                } else {
                    let encoded = BASE64.encode(format!("{}:{}", self.basic_username, self.basic_password));
                    Some(("Authorization".to_string(), format!("Basic {}", encoded)))
                }
            }
            AuthType::ApiKey => {
                if self.api_key_name.is_empty() {
                    None
                } else {
                    Some((self.api_key_name.clone(), self.api_key_value.clone()))
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test compute_header`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "feat(auth): add AuthType/AuthConfig with computed header"
```

---

## Task 2: `effective_wire_headers`

The single source of truth for the wire header set (auth merged over manual headers), reused by the send path and code generation.

**Files:**
- Modify: `src/types.rs` (free function after the `AuthConfig` impl)
- Test: `src/types.rs` `mod tests`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn effective_headers_none_leaves_manual_untouched() {
    let manual = vec![("Accept".to_string(), "*/*".to_string())];
    let out = effective_wire_headers(&manual, &AuthConfig::default());
    assert_eq!(out, manual);
}

#[test]
fn effective_headers_appends_auth() {
    let manual = vec![("Accept".to_string(), "*/*".to_string())];
    let auth = AuthConfig { auth_type: AuthType::Bearer, bearer_token: "t".into(), ..Default::default() };
    let out = effective_wire_headers(&manual, &auth);
    assert_eq!(
        out,
        vec![
            ("Accept".to_string(), "*/*".to_string()),
            ("Authorization".to_string(), "Bearer t".to_string()),
        ]
    );
}

#[test]
fn effective_headers_auth_wins_over_same_name_manual_case_insensitive() {
    // A manually-typed "authorization" is dropped in favor of the computed one.
    let manual = vec![
        ("Accept".to_string(), "*/*".to_string()),
        ("authorization".to_string(), "Bearer OLD".to_string()),
    ];
    let auth = AuthConfig { auth_type: AuthType::Bearer, bearer_token: "NEW".into(), ..Default::default() };
    let out = effective_wire_headers(&manual, &auth);
    assert_eq!(
        out,
        vec![
            ("Accept".to_string(), "*/*".to_string()),
            ("Authorization".to_string(), "Bearer NEW".to_string()),
        ]
    );
}

#[test]
fn effective_headers_api_key_custom_name_dedupes() {
    let manual = vec![("X-API-Key".to_string(), "old".to_string())];
    let auth = AuthConfig {
        auth_type: AuthType::ApiKey,
        api_key_name: "X-API-Key".into(),
        api_key_value: "new".into(),
        ..Default::default()
    };
    let out = effective_wire_headers(&manual, &auth);
    assert_eq!(out, vec![("X-API-Key".to_string(), "new".to_string())]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test effective_headers`
Expected: FAIL to compile ("cannot find function `effective_wire_headers`").

- [ ] **Step 3: Implement**

Add after the `impl AuthConfig` block in `src/types.rs`:

```rust
/// Manual headers with the computed auth header merged in.
///
/// Any manual header whose name case-insensitively matches the auth header's
/// name is removed first (auth wins), then the auth header is appended. When the
/// auth produces no header, the manual headers are returned unchanged.
pub fn effective_wire_headers(
    headers: &[(String, String)],
    auth: &AuthConfig,
) -> Vec<(String, String)> {
    match auth.compute_header() {
        None => headers.to_vec(),
        Some((name, value)) => {
            let mut out: Vec<(String, String)> = headers
                .iter()
                .filter(|(k, _)| !k.eq_ignore_ascii_case(&name))
                .cloned()
                .collect();
            out.push((name, value));
            out
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test effective_headers`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "feat(auth): add effective_wire_headers merge (auth wins)"
```

---

## Task 3: Add `auth` field to `RequestData` and make the crate compile

Adding the field breaks every `RequestData { .. }` literal. Fix them all with a default (or pass-through) value. This task changes no behavior; the existing suite must stay green.

**Files (all Modify):**
- `src/types.rs:212` (`RequestData` struct) and `:222` (`RequestData::new`)
- `src/variables.rs:71` (`substitute_request` return) and `:132` (test literal)
- `src/db.rs:36` (`row_to_history_item`)
- `src/curl_import.rs:233` (`parse_curl` return)
- `src/code_gen.rs` tests `:538`, `:547`, `:561` (`get_req`, `post_json_req`, `form_req`)
- `src/request_tab.rs:26` (`new_empty`) and `:105` (test `empty_request`)
- `src/request_editor.rs:329` (`get_current_request_data`) and `:1055` (`send`)

- [ ] **Step 1: Add the field with `#[serde(default)]`**

In `src/types.rs`, change the `RequestData` struct:

```rust
/// Request data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestData {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: BodyType,
    /// Config-based auth. `#[serde(default)]` so requests serialized before this
    /// feature (history rows / saved tabs) still deserialize — missing → `None`.
    #[serde(default)]
    pub auth: AuthConfig,
}
```

Update `RequestData::new`:

```rust
    pub fn new(method: HttpMethod, url: String) -> Self {
        Self {
            method,
            url,
            headers: vec![],
            body: BodyType::default(),
            auth: AuthConfig::default(),
        }
    }
```

- [ ] **Step 2: Fix the remaining literals (default), except `substitute_request` (pass-through)**

`src/db.rs` — in `row_to_history_item`, the `RequestData { .. }` around line 36. The DB column does not exist yet (added in Task 5), so default it for now:

```rust
    let request = RequestData {
        method: HttpMethod::from_str(&method).unwrap_or(HttpMethod::GET),
        url,
        headers,
        body,
        auth: crate::types::AuthConfig::default(),
    };
```

`src/curl_import.rs` — the return near line 233:

```rust
    Some(RequestData { method, url, headers, body, auth: crate::types::AuthConfig::default() })
```

(Also add `AuthConfig` to imports if you prefer a shorter path; `crate::types::AuthConfig::default()` works without an import.)

`src/request_tab.rs` — `new_empty` (line 26) request literal, and the test `empty_request` (line 105) literal, each gain `auth: BodyType`-style default:

```rust
            request: RequestData {
                method: HttpMethod::GET,
                url: String::new(),
                headers: vec![],
                body: BodyType::default(),
                auth: crate::types::AuthConfig::default(),
            },
```

(In the test module, use `crate::types::AuthConfig::default()` as well.)
`from_history` needs **no change** — it does `request: item.request.clone()`, which already carries `auth`.

`src/code_gen.rs` tests — `get_req`, `post_json_req`, `form_req` each end their `RequestData { .. }` with:

```rust
            auth: crate::types::AuthConfig::default(),
```

`src/request_editor.rs` — `get_current_request_data` (line 329) and `send` (line 1055) each build `RequestData { .. }`; add `auth: crate::types::AuthConfig::default(),` to both for now (Task 9 replaces these with real values).

`src/variables.rs` — `substitute_request` return (line 71): use **pass-through** so Task 4 can wire real substitution:

```rust
    RequestData {
        method: req.method,
        url: substitute(&req.url, vars),
        headers,
        body,
        auth: req.auth.clone(),
    }
```

And its test literal (line 132, `let req = RequestData { .. }`): add `auth: crate::types::AuthConfig::default(),`.

- [ ] **Step 3: Build and run the full suite**

Run: `cargo build`
Expected: compiles clean (fix any literal the compiler flags — the error points to the exact file:line).

Run: `cargo test`
Expected: PASS — the entire existing suite is green, unchanged.

- [ ] **Step 4: Commit**

```bash
git add src/types.rs src/variables.rs src/db.rs src/curl_import.rs src/code_gen.rs src/request_tab.rs src/request_editor.rs
git commit -m "feat(auth): add auth field to RequestData (serde default)"
```

---

## Task 4: `substitute_auth` + auth in `substitute_request`

Auth fields participate in `{{variable}}` substitution, resolved at send/preview time exactly like URL/headers/body.

**Files:**
- Modify: `src/variables.rs`
- Test: `src/variables.rs` `mod tests`

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test substitute_auth substitute_request_resolves_auth`
Expected: FAIL to compile ("cannot find function `substitute_auth`").

- [ ] **Step 3: Implement**

In `src/variables.rs`, add the import at the top (extend the existing `use crate::types::...`):

```rust
use crate::types::{AuthConfig, BodyType, FormDataRow, FormDataValue, RequestData};
```

Add the helper (after `substitute`, before `substitute_request`):

```rust
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
```

In `substitute_request`, replace the `auth: req.auth.clone(),` pass-through (from Task 3) with:

```rust
        auth: substitute_auth(&req.auth, vars),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test substitute`
Expected: PASS (existing substitute tests + the 2 new ones).

- [ ] **Step 5: Commit**

```bash
git add src/variables.rs
git commit -m "feat(auth): substitute {{vars}} in auth fields"
```

---

## Task 5: Persistence — `request_auth` column, migration, insert, read

Add the column via an idempotent migration; store auth JSON on insert; read it back (NULL → default). Update every `insert_history` call site so the crate compiles.

**Files:**
- Modify: `src/db.rs` (`init_schema`, new `migrate_add_request_auth`, `row_to_history_item`, the two SELECTs, `insert_history`, db tests)
- Modify: `src/app.rs:100` (the `insert_history` call)

- [ ] **Step 1: Write the failing tests**

Add to `src/db.rs` `mod tests`. Add the type import at the top of the test module (it already has `use super::*;`, so also):

```rust
    use crate::types::{AuthConfig, AuthType};

    #[test]
    fn migration_adds_request_auth_and_old_rows_default() {
        // Simulate a pre-feature database: history table WITHOUT request_auth.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp TEXT NOT NULL, method TEXT NOT NULL, url TEXT NOT NULL,
                 request_headers TEXT, request_body TEXT,
                 status_code INTEGER, duration_ms INTEGER,
                 response_headers TEXT, response_body TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO history (timestamp, method, url, request_headers, request_body)
             VALUES ('t','GET','https://x','[]','null')",
            [],
        )
        .unwrap();

        // Migration is idempotent and adds the column.
        Database::migrate_add_request_auth(&conn).unwrap();
        Database::migrate_add_request_auth(&conn).unwrap(); // second run is a no-op

        let db = Database::spawn(conn);
        let items = db.load_recent_history(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].request.auth.auth_type, AuthType::None);
    }

    #[test]
    fn history_roundtrips_auth() {
        let db = mem_db();
        let auth = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "abc".into(),
            ..Default::default()
        };
        db.insert_history("GET", "https://x", "[]", &BodyType::None, &auth).unwrap();
        let items = db.load_recent_history(10).unwrap();
        assert_eq!(items[0].request.auth.auth_type, AuthType::Bearer);
        assert_eq!(items[0].request.auth.bearer_token, "abc");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test migration_adds_request_auth history_roundtrips_auth`
Expected: FAIL to compile (`migrate_add_request_auth` missing; `insert_history` takes 4 args, not 5).

- [ ] **Step 3: Implement the migration + column read/write**

In `src/db.rs`, at the end of `init_schema` (after the `execute_batch(...)?;`, before `Ok(())`):

```rust
        Self::migrate_add_request_auth(conn)?;
        Ok(())
```

Add the migration function (below `init_schema`):

```rust
    /// Idempotently add the `request_auth` column. SQLite has no
    /// `ADD COLUMN IF NOT EXISTS`, so check `PRAGMA table_info` first. Old rows
    /// read back as NULL → `AuthConfig::default()`.
    fn migrate_add_request_auth(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(history)")?;
        let has_column = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "request_auth");
        drop(stmt);
        if !has_column {
            conn.execute("ALTER TABLE history ADD COLUMN request_auth TEXT", [])?;
        }
        Ok(())
    }
```

Update `row_to_history_item` to read column index 6 and deserialize:

```rust
    let request_body: String = row.get(5)?;
    let request_auth: Option<String> = row.get(6)?;

    let headers: Vec<(String, String)> =
        serde_json::from_str(&request_headers).unwrap_or_default();
    let body: BodyType = serde_json::from_str(&request_body).unwrap_or_default();
    let auth: crate::types::AuthConfig = request_auth
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let request = RequestData {
        method: HttpMethod::from_str(&method).unwrap_or(HttpMethod::GET),
        url,
        headers,
        body,
        auth,
    };
```

Add `request_auth` to **both** SELECT column lists (so index 6 exists) — in `load_recent_history` (line 179) and `search_history` (line 202):

```sql
SELECT id, timestamp, method, url, request_headers, request_body, request_auth
```

Update `insert_history` to accept and store auth. Extend the `AuthConfig` import at the top (`use crate::types::{..., AuthConfig, ...}`), then:

```rust
    pub fn insert_history(
        &self,
        method: &str,
        url: &str,
        request_headers: &str,
        request_body: &BodyType,
        auth: &AuthConfig,
    ) -> Result<i64> {
        let method = method.to_string();
        let url = url.to_string();
        let request_headers = request_headers.to_string();
        let body_json = serde_json::to_string(request_body).unwrap_or_default();
        let auth_json = serde_json::to_string(auth).unwrap_or_default();

        self.call(move |conn| {
            let timestamp = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO history (timestamp, method, url, request_headers, request_body, request_auth)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![timestamp, method, url, request_headers, body_json, auth_json],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }
```

- [ ] **Step 4: Fix every existing `insert_history` call site (add the 5th arg)**

In `src/db.rs` `mod tests`, the existing calls (`history_roundtrip`, `search_history_*` — lines ~405, 417, 419, 421, 443, 445, 447, 465) each need `&crate::types::AuthConfig::default()` appended, e.g.:

```rust
db.insert_history("GET", "https://api.test/x", "[]", &crate::types::BodyType::None, &crate::types::AuthConfig::default())
    .unwrap();
```

In `src/app.rs` (the call at line 100), pass the request's auth:

```rust
                    if let Err(e) = db_clone.insert_history(
                        event.request.method.as_str(),
                        &event.request.url,
                        &request_headers,
                        &event.request.body,
                        &event.request.auth,
                    ) {
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib` (db module — and confirm nothing else regressed)
Expected: PASS, including the two new tests.

- [ ] **Step 6: Commit**

```bash
git add src/db.rs src/app.rs
git commit -m "feat(auth): persist request_auth column with migration"
```

---

## Task 6: `curl_import` — `-u/--user` populates Basic auth config

**Behavior change.** Today `-u user:pass` produces an `Authorization: Basic <base64>` *header*. The spec wants it to set `auth_type = Basic` with raw username/password and **no** header. Rewrite the branch and its test; drop the now-unused base64 import.

**Files:**
- Modify: `src/curl_import.rs`
- Test: `src/curl_import.rs` `mod tests`

- [ ] **Step 1: Rewrite the existing test + add cases (they will fail)**

Replace the existing `user_flag_becomes_basic_auth_header` test with:

```rust
    #[test]
    fn user_flag_becomes_basic_auth_config() {
        let r = parse("curl -u user:pass https://example.com");
        assert_eq!(r.auth.auth_type, crate::types::AuthType::Basic);
        assert_eq!(r.auth.basic_username, "user");
        assert_eq!(r.auth.basic_password, "pass");
        // No Authorization header is synthesized — the config computes it at send time.
        assert!(r.headers.iter().all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
    }

    #[test]
    fn user_flag_long_and_attached_forms() {
        assert_eq!(parse("curl --user u:p https://example.com").auth.basic_username, "u");
        assert_eq!(parse("curl --user=u:p https://example.com").auth.basic_password, "p");
        let r = parse("curl -uadmin:s3cret https://example.com");
        assert_eq!(r.auth.basic_username, "admin");
        assert_eq!(r.auth.basic_password, "s3cret");
    }

    #[test]
    fn user_flag_without_colon_is_username_only() {
        let r = parse("curl -u alice https://example.com");
        assert_eq!(r.auth.auth_type, crate::types::AuthType::Basic);
        assert_eq!(r.auth.basic_username, "alice");
        assert_eq!(r.auth.basic_password, "");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test user_flag`
Expected: FAIL (current code pushes an Authorization header; `r.auth` is default `None`).

- [ ] **Step 3: Implement the change**

Remove the now-unused base64 import (line 8):

```rust
// DELETE: use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
```

Add `AuthConfig, AuthType` to the types import (line 10):

```rust
use crate::types::{AuthConfig, AuthType, BodyType, FormDataRow, FormDataValue, HttpMethod, RawSubtype, RequestData};
```

In `parse_curl`, declare an auth accumulator alongside the others (near line 128–130):

```rust
    let mut auth = AuthConfig::default();
```

Replace the `-u/--user` branch (lines 184–187) with:

```rust
        } else if matches_flag(&tok, "-u", "--user") {
            if let Some(v) = flag_value(&tokens, &mut i, "-u", "--user") {
                // Split on the first ':' into user/pass; a value with no ':' is a
                // username with an empty password (curl then prompts, we don't).
                let (user, pass) = match v.split_once(':') {
                    Some((u, p)) => (u.to_string(), p.to_string()),
                    None => (v, String::new()),
                };
                auth = AuthConfig {
                    auth_type: AuthType::Basic,
                    basic_username: user,
                    basic_password: pass,
                    ..AuthConfig::default()
                };
            }
        }
```

Update the return (from Task 3's `auth: AuthConfig::default()`) to carry the parsed auth:

```rust
    Some(RequestData { method, url, headers, body, auth })
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test-threads=1 -p poopman curl` *(or)* `cargo test user_flag cookie simple_get`
Expected: PASS — new `user_flag_*` tests pass; all other curl_import tests (cookies, data, forms) unaffected.

- [ ] **Step 5: Commit**

```bash
git add src/curl_import.rs
git commit -m "feat(auth): map curl -u/--user to Basic auth config"
```

---

## Task 7: `code_gen` emits the computed auth header across all targets

Fold the auth header into the header list once, at the `generate()` entry, so all six generators emit it as a normal header with no per-target special-casing.

**Files:**
- Modify: `src/code_gen.rs` (`generate`)
- Test: `src/code_gen.rs` `mod tests`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn code_gen_emits_bearer_auth_header() {
        use crate::types::{AuthConfig, AuthType};
        let mut req = get_req();
        req.auth = AuthConfig { auth_type: AuthType::Bearer, bearer_token: "t0ken".into(), ..Default::default() };
        assert!(generate(CodeTarget::Curl, &req).contains("--header 'Authorization: Bearer t0ken'"));
        assert!(generate(CodeTarget::PythonRequests, &req).contains("\"Authorization\": \"Bearer t0ken\""));
        assert!(generate(CodeTarget::GoNetHttp, &req).contains("req.Header.Add(\"Authorization\", \"Bearer t0ken\")"));
    }

    #[test]
    fn code_gen_basic_and_api_key_headers() {
        use crate::types::{AuthConfig, AuthType};
        let mut req = get_req();
        req.auth = AuthConfig {
            auth_type: AuthType::Basic,
            basic_username: "user".into(),
            basic_password: "pass".into(),
            ..Default::default()
        };
        assert!(generate(CodeTarget::Curl, &req).contains("Authorization: Basic dXNlcjpwYXNz"));

        req.auth = AuthConfig {
            auth_type: AuthType::ApiKey,
            api_key_name: "X-API-Key".into(),
            api_key_value: "secret".into(),
            ..Default::default()
        };
        assert!(generate(CodeTarget::Curl, &req).contains("--header 'X-API-Key: secret'"));
    }

    #[test]
    fn code_gen_auth_overrides_manual_same_name_header() {
        use crate::types::{AuthConfig, AuthType};
        let mut req = get_req();
        req.headers = vec![("Authorization".into(), "Bearer OLD".into())];
        req.auth = AuthConfig { auth_type: AuthType::Bearer, bearer_token: "NEW".into(), ..Default::default() };
        let out = generate(CodeTarget::Curl, &req);
        assert!(out.contains("Bearer NEW"));
        assert!(!out.contains("Bearer OLD"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test code_gen_emits_bearer code_gen_basic code_gen_auth_overrides`
Expected: FAIL (no Authorization header in output; auth is ignored).

- [ ] **Step 3: Implement**

In `src/code_gen.rs`, change `generate` to merge auth into the headers before dispatch:

```rust
/// Top-level dispatch: generate source code for `target` from `req`.
pub fn generate(target: CodeTarget, req: &RequestData) -> String {
    // Fold the computed auth header into the header list so every target emits it
    // as a normal header. `effective_wire_headers` is the single source of truth,
    // shared with the send path. When auth is None this is a no-op copy.
    let mut merged = req.clone();
    merged.headers = crate::types::effective_wire_headers(&req.headers, &req.auth);
    let req = &merged;

    match target {
        CodeTarget::Curl => gen_curl(req),
        CodeTarget::RustReqwest => gen_rust(req),
        CodeTarget::PythonRequests => gen_python(req),
        CodeTarget::JavaScriptFetch => gen_fetch(req),
        CodeTarget::NodeAxios => gen_axios(req),
        CodeTarget::GoNetHttp => gen_go(req),
    }
}
```

No other function changes: `export_headers(req)` now reads the merged list, and its form-data Content-Type skip still applies (an auth header is never Content-Type).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test` (code_gen module — new tests pass, all existing generator tests unaffected because default auth leaves headers unchanged)
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/code_gen.rs
git commit -m "feat(auth): emit computed auth header in generated code"
```

---

## Task 8: `AuthEditor` GPUI component

New entity mirroring `BodyEditor`. **No unit tests** — GUI state needs a `Window`/`Context` and cannot link on WSL. Verify by `cargo build` here; behavior is user-verified in Task 9.

**Files:**
- Create: `src/auth_editor.rs`
- Modify: `src/main.rs` (add `mod auth_editor;`)

- [ ] **Step 1: Register the module**

In `src/main.rs`, add after `mod app;` (line 3), keeping alphabetical order:

```rust
mod auth_editor;
```

- [ ] **Step 2: Create `src/auth_editor.rs`**

```rust
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui::px;
use gpui_component::{
    input::{Input, InputState},
    v_flex, h_flex, ActiveTheme as _,
};

use crate::types::{AuthConfig, AuthType};

/// Auth sub-tab editor. A flat set of input fields (one per auth field) plus a
/// type selector; only the active type's fields render. Values persist across
/// type switches because each field is its own always-alive `InputState`.
pub struct AuthEditor {
    /// 0 = None, 1 = Bearer, 2 = Basic, 3 = ApiKey.
    auth_type_index: usize,
    bearer_token: Entity<InputState>,
    basic_username: Entity<InputState>,
    basic_password: Entity<InputState>,
    api_key_name: Entity<InputState>,
    api_key_value: Entity<InputState>,
}

impl AuthEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            auth_type_index: 0,
            bearer_token: cx.new(|cx| InputState::new(window, cx).placeholder("Token")),
            basic_username: cx.new(|cx| InputState::new(window, cx).placeholder("Username")),
            basic_password: cx.new(|cx| InputState::new(window, cx).placeholder("Password")),
            api_key_name: cx.new(|cx| InputState::new(window, cx).placeholder("Key (e.g. X-API-Key)")),
            api_key_value: cx.new(|cx| InputState::new(window, cx).placeholder("Value")),
        }
    }

    /// Read the current auth configuration from the UI fields.
    pub fn get_auth(&self, cx: &App) -> AuthConfig {
        AuthConfig {
            auth_type: match self.auth_type_index {
                1 => AuthType::Bearer,
                2 => AuthType::Basic,
                3 => AuthType::ApiKey,
                _ => AuthType::None,
            },
            bearer_token: self.bearer_token.read(cx).value().to_string(),
            basic_username: self.basic_username.read(cx).value().to_string(),
            basic_password: self.basic_password.read(cx).value().to_string(),
            api_key_name: self.api_key_name.read(cx).value().to_string(),
            api_key_value: self.api_key_value.read(cx).value().to_string(),
        }
    }

    /// Load an auth configuration into the UI (used by `load_request`).
    pub fn set_auth(&mut self, auth: &AuthConfig, window: &mut Window, cx: &mut Context<Self>) {
        self.auth_type_index = match auth.auth_type {
            AuthType::None => 0,
            AuthType::Bearer => 1,
            AuthType::Basic => 2,
            AuthType::ApiKey => 3,
        };
        self.bearer_token.update(cx, |i, cx| i.set_value(&auth.bearer_token, window, cx));
        self.basic_username.update(cx, |i, cx| i.set_value(&auth.basic_username, window, cx));
        self.basic_password.update(cx, |i, cx| i.set_value(&auth.basic_password, window, cx));
        self.api_key_name.update(cx, |i, cx| i.set_value(&auth.api_key_name, window, cx));
        self.api_key_value.update(cx, |i, cx| i.set_value(&auth.api_key_value, window, cx));
        cx.notify();
    }

    /// A labelled input row (label on the left, field filling the rest).
    fn field_row(label: &'static str, input: &Entity<InputState>, theme: &gpui_component::Theme) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .w_full()
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(div().flex_1().child(Input::new(input)))
    }
}

impl Render for AuthEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .gap_3()
            .w_full()
            .flex_1()
            .min_h_0()
            // Type selector — muted radios, matching BodyEditor's body-type row.
            .child(
                h_flex().gap_4().items_center().children(
                    ["None", "Bearer", "Basic", "API Key"].into_iter().enumerate().map(|(i, label)| {
                        let selected = self.auth_type_index == i;
                        h_flex()
                            .id(("auth-type", i))
                            .gap_1p5()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.auth_type_index = i;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if selected { theme.primary } else { theme.border })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |d| {
                                        d.child(div().size(px(6.)).rounded_full().bg(theme.primary))
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if selected { theme.foreground } else { theme.muted_foreground })
                                    .child(label),
                            )
                    }),
                ),
            )
            // A one-line note: auth is injected at send time and wins over a manual header.
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("The auth header is added when the request is sent and overrides a manually-typed header of the same name."),
            )
            // Active type's fields.
            .when(self.auth_type_index == 0, |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .child("This request does not use authorization"),
                )
            })
            .when(self.auth_type_index == 1, |this| {
                this.child(Self::field_row("Token", &self.bearer_token, theme))
            })
            .when(self.auth_type_index == 2, |this| {
                this.child(Self::field_row("Username", &self.basic_username, theme))
                    .child(Self::field_row("Password", &self.basic_password, theme))
            })
            .when(self.auth_type_index == 3, |this| {
                this.child(Self::field_row("Key", &self.api_key_name, theme))
                    .child(Self::field_row("Value", &self.api_key_value, theme))
            })
    }
}
```

> **Note (masking):** gpui-component's single-line `Input` has no password-mask option in 0.5 (only `otp_input` does), so the Basic password renders as plain text in v1. Acceptable — not a spec requirement.

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles clean. Fix any API drift the compiler reports (e.g. `h_flex`/`Theme` import path) against the patterns already used in `src/body_editor.rs`.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/auth_editor.rs
git commit -m "feat(auth): add AuthEditor component"
```

---

## Task 9: Wire `AuthEditor` into `RequestEditor` (tab, send, load)

Embed AuthEditor as sub-tab 3, read it into `get_current_request_data`, restore it in `load_request`, and inject the computed header at send time. GUI-integrated — ends with a **manual verification checklist**.

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: Add the field, import, and construction**

Import at the top:

```rust
use crate::auth_editor::AuthEditor;
```

Add to the `RequestEditor` struct (near `body_editor`):

```rust
    auth_editor: Entity<AuthEditor>,
```

In `new`, create it beside `body_editor` (after line 113):

```rust
        let auth_editor = cx.new(|cx| AuthEditor::new(window, cx));
```

Add it to the struct initializer (near `body_editor,`):

```rust
            auth_editor,
```

- [ ] **Step 2: Include auth in `get_current_request_data`**

Replace the Task-3 placeholder `auth: crate::types::AuthConfig::default(),` at the end of the `RequestData { .. }` in `get_current_request_data` (line ~329) with:

```rust
            auth: self.auth_editor.read(cx).get_auth(cx),
```

- [ ] **Step 3: Restore auth in `load_request`**

In `load_request`, after the body is set via `self.body_editor.update(...)` (around line 219–221), add:

```rust
        // Set auth via AuthEditor
        self.auth_editor.update(cx, |editor, cx| {
            editor.set_auth(&request.auth, window, cx);
        });
```

- [ ] **Step 4: Inject the computed header at send time**

In `send`, the manual `headers` vec is built and substituted by line ~1032. Immediately after that substitution block (before `let body = match body { ... }` or right before building `request`), read + resolve auth:

```rust
        // Resolve auth {{vars}} and compute the wire header. The saved request
        // keeps manual headers + the auth config; only the wire gets the merged
        // header set (auth wins over a manual same-name header).
        let resolved_auth = crate::variables::substitute_auth(&self.auth_editor.read(cx).get_auth(cx), env);
```

Change the `request` literal (line ~1055) — replace the Task-3 placeholder `auth: ...default()` with the resolved config, keeping `headers: headers.clone()` (manual only):

```rust
        let request = RequestData {
            method,
            url: url.clone(),
            headers: headers.clone(),
            body: body.clone(),
            auth: resolved_auth.clone(),
        };
```

Change the wire call (line ~1072) to send the *merged* headers:

```rust
        let wire_headers = crate::types::effective_wire_headers(&headers, &resolved_auth);
        let inflight = client.start_send(method, url, wire_headers, body);
```

(`headers` is still owned here — it was only `.clone()`d into `request` above, and `effective_wire_headers` borrows it; `body` is moved into `start_send` as before.)

- [ ] **Step 5: Insert the Auth sub-tab after Headers, renumbering Params→2 and Body→3**

Target tab order: **Headers(0) · Auth(1) · Params(2) · Body(3)**. The current `render` has Headers(0), Params(1), Body(2); make the six edits below inside the tab section. Anchor on the pill ids (`"tab-headers"`, `"tab-params"`, `"tab-body"`), which are stable.

**5a — Insert the Auth pill** as a new `.child(...)` on `segmented_bar(theme)`, immediately after the Headers pill child (the `segment_pill` whose `.id("tab-headers")` ends `.child("Headers")`) and before the Params pill child:

```rust
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 1)
                                        .id("tab-auth")
                                        .when(self.active_tab != 1, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Auth"),
                                )
```

**5b — Renumber the Params pill** (`.id("tab-params")`) from index `1` to `2` — all three occurrences (`== 1`, `!= 1`, `= 1`):

```rust
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 2)
                                        .id("tab-params")
                                        .when(self.active_tab != 2, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 2;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Params"),
                                )
```

**5c — Renumber the Body pill** (`.id("tab-body")`) from index `2` to `3` — all three occurrences (`== 2`, `!= 2`, `= 2`):

```rust
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 3)
                                        .id("tab-body")
                                        .when(self.active_tab != 3, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 3;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Body"),
                                ),
```

**5d — Insert the Auth panel** as a new `.when(...)` on the tabs container, immediately after the Headers panel (`.when(self.active_tab == 0, ...)`, the block rendering the headers list) and before the Params panel:

```rust
                        .when(self.active_tab == 1, |this| {
                            this.child(
                                div()
                                    .p_2()
                                    .w_full()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .min_h_0()
                                    .child(self.auth_editor.clone()),
                            )
                        })
```

**5e — Renumber the Params panel** guard: the `.when(self.active_tab == 1, |this| { ... })` that renders `"params-scroll-container"` becomes `.when(self.active_tab == 2, |this| { ... })`.

**5f — Renumber the Body panel** guard: the `.when(self.active_tab == 2, |this| { ... })` that renders `self.body_editor.clone()` becomes `.when(self.active_tab == 3, |this| { ... })`.

> The panels are guarded purely by the `active_tab` value, so their source order does not matter — but inserting 5d between the Headers and Params panels keeps the code order matching the visual order (0,1,2,3), which is easier to maintain.

- [ ] **Step 6: Build**

Run: `cargo build`
Expected: compiles clean.

- [ ] **Step 7: Run the full test suite (regression guard)**

Run: `cargo test`
Expected: PASS — all unit tests from Tasks 1–7 plus the pre-existing suite are green.

- [ ] **Step 8: Manual verification on the Windows build**

Run the app: `pwsh.exe -NoProfile -Command "cargo run"`. Confirm each — this is the part tests do **not** cover:

1. The request editor shows a 4th sub-tab **Auth**; clicking it renders the type radios + note.
2. Selecting **Bearer**, typing a token, then switching to **Basic** and back to **Bearer** preserves the typed token (flat-struct persistence).
3. Send a request to an echo endpoint (e.g. `https://httpbin.org/headers`) with Bearer set; the response shows `Authorization: Bearer <token>` — i.e. the computed header reached the wire.
4. Add a manual `Authorization` header **and** set Bearer auth; the echoed request shows the auth value, not the manual one (auth wins).
5. With auth **None**, no `Authorization` header is sent.
6. Open the code snippet (`</>`) with Bearer set — the generated code includes the Authorization header.
7. Send a request with auth set, then reopen it from the History panel — the Auth tab restores the type and fields.
8. Paste `curl -u user:pass https://httpbin.org/headers` into the URL bar — it imports as **Basic** in the Auth tab (username/password populated), with no manual Authorization header row.

- [ ] **Step 9: Commit**

```bash
git add src/request_editor.rs
git commit -m "feat(auth): wire Auth sub-tab into request editor and send path"
```

---

## Self-Review

**Spec coverage** (each spec section → task):

- §Decisions 1 (config-based model) → Tasks 1, 3. ✓
- §Decisions 2 / §1 Types (None/Bearer/Basic/ApiKey, flat struct, `compute_header` + empty-skip + base64) → Task 1. ✓
- §Decisions 3 (API-Key header-only, custom name) → Task 1 (`ApiKey` emits `(name, value)`; no query-param mode). ✓
- §Decisions 4 / §3 (auth wins over manual same-name, injected only at send) → Task 2 (`effective_wire_headers`), Task 9 (send uses merged, save keeps manual). ✓
- §Decisions 5 / §7 (env-var interpolation of auth fields) → Task 4. ✓
- §Decisions 6 / §8 (code_gen emits computed header on all 6 targets; curl `-u` → Basic) → Tasks 7, 6. ✓
- §2 (`RequestData.auth`, `#[serde(default)]`; `new_empty`/`from_history` set auth) → Task 3 (`from_history` inherits via `request.clone()`). ✓
- §4 (`AuthEditor`, sub-tab **index 1 — immediately after Headers**, `get_auth`/`set_auth`) → Tasks 8, 9. ✓
- §5 (send-time injection point) → Task 9. ✓
- §6 (db column, idempotent migration, insert param, read index 6, NULL→default) → Task 5. ✓
- §Testing (types, effective_wire_headers, variables, curl_import, db) → Tasks 1,2,4,5,6; GUI items → Task 9 manual checklist. ✓
- §Out of scope — honored: no OAuth, no API-Key query mode, no idiomatic `curl -u`/`auth=` export (uniform computed header), no reverse-engineering `Authorization` headers on import, no greyed auth row in Headers tab.

**Deviations from the spec, and why:**

- **Auth tab position:** placed at index 1 (immediately after Headers), with Params→2 and Body→3 renumbered — per the user's UX preference, not the spec's original §4 index 3. Spec §4 updated to match.
- Real method names used: the spec's `build_request_data` is actually `send()` + `get_current_request_data()`; `row_to_request` is `row_to_history_item`. Plan targets the real code.
- `base64` dependency already present in `Cargo.toml` — the spec's "add dependency" step is dropped.
- `curl_import` **already** had a `-u/--user` branch (producing a header); Task 6 *rewrites* it and its test rather than adding fresh, and removes the now-unused base64 import.
- Added `PartialEq, Eq` to `AuthConfig` (spec omits them) — costless (all fields are `String`/`enum: Eq`) and simplifies test assertions.
- Basic password field is unmasked (library limitation, noted in Task 8) — not a spec requirement.

**Placeholder scan:** none — every code step carries full code; every run step names a concrete command and expected result.

**Type consistency:** `AuthType`, `AuthConfig`, `compute_header`, `effective_wire_headers`, `substitute_auth`, `get_auth`/`set_auth`, `auth_type_index`, and the `request_auth` column are named identically across all tasks.
