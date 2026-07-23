# Auth Helper Tab — Design

**Date:** 2026-07-22
**Branch:** `feat/auth-helper-tab`
**Status:** Approved design, pending implementation plan.

## Goal

Add an **Auth** sub-tab to the request editor, giving Postman-style authentication
helpers: **None**, **Bearer**, **Basic**, and **API-Key**. The user picks an auth
type and fills its fields; at send time the helper computes the appropriate header
and injects it, overriding any manually-typed header of the same name.

This is the first of the two remaining Postman-core blocks (auth helper + Collections)
from the roadmap. Environment variables already ship; auth fields participate in the
same `{{variable}}` substitution.

## Decisions (from brainstorming)

1. **Config-based data model** (Postman parity), not a header generator. Auth is a
   persisted configuration on `RequestData`; the wire header is computed from it.
   Reopening a history item restores the auth *type* and fields, and the type can be
   switched without losing what was typed.
2. **Types:** None + Bearer + Basic + API-Key.
3. **API-Key placement:** Header only (custom key name). No query-param mode in v1 —
   avoids entangling with the existing URL↔Params synchronization.
4. **Auth wins over manual headers:** when auth is not None and produces a header, any
   manually-typed header with the same name is dropped and the computed one is used.
   The computed header is injected **only at send time** — it is not shown as a row in
   the Headers tab. The Auth tab carries a one-line note explaining this.
5. **Env-var interpolation:** all auth fields support `{{variable}}`, resolved at send
   time exactly like URL / headers / body.
6. **Export/import (balanced tier):** `code_gen` emits the computed auth header as a
   normal header across all six targets (Basic included, as `Authorization: Basic
   <base64>` — not `curl -u`). `curl_import` recognizes `-u/--user user:pass` → Basic
   auth; `Authorization` headers stay plain headers.

## Architecture

### 1. Types (`src/types.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthType {
    #[default]
    None,
    Bearer,
    Basic,
    ApiKey,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    pub auth_type: AuthType,
    pub bearer_token: String,
    pub basic_username: String,
    pub basic_password: String,
    pub api_key_name: String,   // e.g. "X-API-Key"
    pub api_key_value: String,
}
```

A **flat struct** (all fields always present) rather than an enum-with-data, so
switching type in the UI preserves each type's previously-typed values — matching
Postman. `Default` = `None` with empty strings.

```rust
impl AuthConfig {
    /// The header this auth would put on the wire, or None.
    /// Emitted only when the relevant field(s) are non-empty, so an
    /// in-progress edit does not send a placeholder header.
    pub fn compute_header(&self) -> Option<(String, String)> { ... }
}
```

Per-type rule for `compute_header`:

| Type   | Condition           | Header                                             |
|--------|---------------------|----------------------------------------------------|
| None   | —                   | `None`                                             |
| Bearer | `bearer_token` ≠ "" | `("Authorization", "Bearer <token>")`              |
| Basic  | user ≠ "" or pass ≠ "" | `("Authorization", "Basic " + base64(user:pass))` |
| ApiKey | `api_key_name` ≠ "" | `(<api_key_name>, <api_key_value>)`                |

> **Semantics note:** emitting only when non-empty differs slightly from Postman
> (which emits once a type is selected). Chosen to avoid sending a dangling
> `Authorization: Bearer ` while the user is still typing. One-line change if we ever
> want Postman-exact behavior.

Requires a **`base64`** dependency (`base64 = "0.22"`) for Basic encoding.

### 2. `RequestData` (`src/types.rs`)

Add `pub auth: AuthConfig` with `#[serde(default)]` so bodies/rows serialized before
this feature still deserialize (missing → `AuthConfig::default()`). `RequestData.headers`
continues to hold **manual headers only** — auth is never materialized into it.

`RequestData::new` and the `RequestTab::new_empty` / `from_history` constructors set
`auth: AuthConfig::default()` (the latter copies it from the history item).

### 3. Effective-headers helper (`src/types.rs` or a small module)

```rust
/// Manual headers with the computed auth header merged in.
/// Any manual header whose name case-insensitively matches the auth
/// header's name is removed first (auth wins), then the auth header is appended.
pub fn effective_wire_headers(
    headers: &[(String, String)],
    auth: &AuthConfig,
) -> Vec<(String, String)>;
```

Single source of truth for the wire header set, reused by both the send path and
`code_gen`. Pure and unit-tested.

### 4. `AuthEditor` component (`src/auth_editor.rs`)

Mirrors `BodyEditor` (its own entity, embedded in `RequestEditor`):

- A type selector (dropdown/segmented) + `InputState` fields for each type.
- Renders only the active type's fields; `None` shows an explanatory line.
- `get_auth(cx) -> AuthConfig` reads current field values.
- `set_auth(&AuthConfig, window, cx)` loads values (used by `load_request`).
- Registered as sub-tab **index 1 "Auth"** in `RequestEditor`, placed immediately
  after Headers(0), with Params and Body renumbered to (2) and (3). Auth sits next
  to Headers, where users look for it.

Keeping this in its own file follows the existing `BodyEditor` boundary and keeps
`request_editor.rs` (already ~1471 lines) from growing further.

### 5. Send-time injection (`request_editor.rs`, `build_request_data`)

After the existing manual-header collection + env-var substitution:

1. `let resolved_auth = substitute_auth(auth_editor.get_auth(cx), env);`
2. Wire: `start_send(method, url, effective_wire_headers(&resolved_headers, &resolved_auth), body)`.
3. Save: `RequestData { method, url, headers: resolved_headers /* manual only */, body, auth: resolved_auth }`.

`HttpClient::start_send` is unchanged — it still receives a plain
`Vec<(String, String)>`; the auth header is merged in by the caller. The saved
`RequestData` (emitted via `RequestCompleted`) stays clean: manual headers + auth
config, so history round-trips without a duplicate `Authorization` row.

### 6. Persistence (`src/db.rs`)

- Add column `request_auth TEXT` to the `history` table.
- Migration: after `CREATE TABLE IF NOT EXISTS history`, check
  `PRAGMA table_info(history)` for the column and `ALTER TABLE history ADD COLUMN
  request_auth TEXT` if absent (SQLite has no `ADD COLUMN IF NOT EXISTS`). Old rows
  read back as NULL → `AuthConfig::default()`.
- `insert_history` gains an `auth: &AuthConfig` param (serialized to JSON alongside
  `request_body`). Update the call site in `app.rs`.
- `row_to_request` reads the new column (index 6) and deserializes; NULL / parse
  failure → `AuthConfig::default()`.

### 7. Variable substitution (`src/variables.rs`)

Extend `substitute_request` to also substitute the auth fields (a `substitute_auth`
helper applied to `bearer_token`, `basic_username`, `basic_password`, `api_key_name`,
`api_key_value`). This keeps generated code / previews using resolved values.

### 8. Export / import

- **`code_gen`:** compute `effective_wire_headers(&req.headers, &req.auth)` (on the
  already-substituted request) and emit its rows as normal headers across all six
  targets. No per-type special-casing.
- **`curl_import`:** add a branch for `-u` / `--user` (separate, `--user=`, and
  attached `-u<v>` forms, consistent with the existing `-b/--cookie` handling). Split
  on the first `:` into `basic_username` / `basic_password` and set
  `auth_type = Basic`. A value with no `:` is treated as username, empty password.
  `Authorization` request headers are left as plain headers (no reverse-engineering
  into Bearer).
- **`load_request` (`request_editor.rs`):** call `auth_editor.set_auth(&request.auth, ...)`
  so importing a curl / opening a history item populates the Auth tab.

## Testing

Unit tests (run on the Windows side via `pwsh.exe … cargo test`; WSL cannot link the GUI):

- **types:** `compute_header` for all four types, including the empty-field skip and a
  concrete base64 assertion for Basic.
- **effective_wire_headers:** same-name dedupe (auth wins), API-Key custom name, None
  leaves headers untouched.
- **variables:** `substitute_request` resolves auth fields.
- **curl_import:** `-u user:pass` → Basic; `--user user:pass`; attached form; value
  without `:`.
- **db:** migration adds the column; a row written before the migration reads back as
  `None`; write→read round-trip of a Bearer/Basic/ApiKey config.

**GUI behavior requires user visual verification on the Windows build** and is *not*
claimed as verified by tests: the Auth tab renders and switches, switching type
preserves each type's fields, the computed header actually reaches the wire and
overrides a manual same-name header, and reopening a history item restores the tab.

## Out of scope (deferred)

- OAuth 2.0 / token acquisition & refresh flows.
- API-Key in query-param position (header only for v1).
- Type-aware idiomatic export (`curl -u`, `requests` `auth=`) — balanced tier emits
  the computed header uniformly instead.
- Reverse-engineering `Authorization: Bearer/Basic` headers on curl import into the
  Auth tab.
- Showing the computed auth header as a greyed read-only row in the Headers tab
  (Postman does; v1 injects at send only with a note in the Auth tab).
- Preserving `{{variable}}` templates in saved history (a pre-existing app-wide
  limitation — history stores resolved values today; auth follows the same path).
