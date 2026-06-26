# History Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an always-visible search box to the History panel that filters stored requests by URL or HTTP method across the entire history.

**Architecture:** A new `Database::search_history` runs a `LIKE` over `url` and `method` for all rows (newest first); the `HistoryPanel` gains a gpui-component `Input`, and on every keystroke re-queries the DB (recent when empty, search when not) and re-renders. The row→`HistoryItem` mapping is extracted into one shared helper so the two queries can't drift.

**Tech Stack:** Rust, rusqlite 0.40 (SQLite), GPUI 0.2.2, gpui-component 0.4.

---

## File Structure

- `src/db.rs` — add `search_history`, extract `row_to_history_item` helper + `escape_like`; add tests. (CSP DB thread; tests run headless, no GPU.)
- `src/history_panel.rs` — add search `InputState`, query state, subscription, `refresh_list` helper, search-row UI, query-aware empty state.
- `assets/icons/search.svg` — new magnifier icon for the input prefix (embedded via rust-embed like the existing `code.svg`).

`src/app.rs` is **not** touched: it calls `HistoryPanel::reload(window, cx)`, whose signature is unchanged.

> **WSL2 note:** the GPUI app cannot run here (no GPU). DB tests run normally with `cargo test`. For UI changes, verify with `cargo check`/`cargo build`; visual confirmation requires a native Windows/Linux run.

---

## Task 1: DB search + shared row mapping

**Files:**
- Modify: `src/db.rs` (extract helper from `load_recent_history` at `src/db.rs:143-184`; add new fn + free fns; add tests in the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add these three tests inside `mod tests` in `src/db.rs` (after the existing `history_roundtrip` test). They reference `HttpMethod`, already imported in the test module via `use super::*`:

```rust
    #[test]
    fn search_history_matches_url_and_method_newest_first() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/users", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("POST", "https://api.test/login", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("DELETE", "https://api.test/orders/1", "[]", &crate::types::BodyType::None)
            .unwrap();

        // URL substring
        let r = db.search_history("login", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.url, "https://api.test/login");

        // method match, case-insensitive
        let r = db.search_history("post", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.method, HttpMethod::POST);

        // shared substring across all three, newest (last inserted) first
        let r = db.search_history("api.test", 10).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].request.url, "https://api.test/orders/1");
    }

    #[test]
    fn search_history_escapes_wildcards() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/a%b", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("GET", "https://api.test/axb", "[]", &crate::types::BodyType::None)
            .unwrap();

        // '%' must be treated literally: matches only the URL with a literal '%'
        let r = db.search_history("a%b", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.url, "https://api.test/a%b");
    }

    #[test]
    fn search_history_empty_query_matches_all() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/users", "[]", &crate::types::BodyType::None)
            .unwrap();
        let r = db.search_history("", 10).unwrap();
        assert_eq!(r.len(), 1);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib search_history 2>&1 | tail -20`
Expected: compile error — `no method named 'search_history' found for struct 'Database'`.

- [ ] **Step 3: Add the shared free functions**

At the top of `src/db.rs`, just below the `type Job = ...` line (around `src/db.rs:19`), add the row-mapping helper and the LIKE escaper as private free functions:

```rust
/// Map a `history` row (id, timestamp, method, url, request_headers, request_body)
/// into a `HistoryItem`. Shared by `load_recent_history` and `search_history` so
/// the two queries can never drift in how they decode a row.
fn row_to_history_item(row: &rusqlite::Row) -> rusqlite::Result<HistoryItem> {
    let id: i64 = row.get(0)?;
    let timestamp: String = row.get(1)?;
    let method: String = row.get(2)?;
    let url: String = row.get(3)?;
    let request_headers: String = row.get(4)?;
    let request_body: String = row.get(5)?;

    let headers: Vec<(String, String)> =
        serde_json::from_str(&request_headers).unwrap_or_default();
    let body: crate::types::BodyType =
        serde_json::from_str(&request_body).unwrap_or_default();

    let request = RequestData {
        method: HttpMethod::from_str(&method).unwrap_or(HttpMethod::GET),
        url,
        headers,
        body,
    };
    Ok(HistoryItem::new(id, timestamp, request, None))
}

/// Escape a user query so SQLite `LIKE` treats `%`, `_`, and `\` literally.
/// Paired with `ESCAPE '\'` in the SQL. Backslash must be escaped first.
fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
```

- [ ] **Step 4: Use the helper in `load_recent_history`**

Replace the closure body in `load_recent_history` (`src/db.rs:153-176`) so the whole `query_map` call reads:

```rust
            let items = stmt.query_map([limit as i64], row_to_history_item)?;

            let mut result = Vec::new();
            for item in items {
                result.push(item?);
            }
            Ok(result)
```

(Delete the old inline `|row| { ... Ok(HistoryItem::new(...)) }` closure and its now-duplicated decoding.)

- [ ] **Step 5: Add `search_history`**

Add this method inside `impl Database`, directly after `load_recent_history` (after `src/db.rs:184`):

```rust
    /// Search history by URL or method (case-insensitive substring), all rows,
    /// newest first. An empty query matches everything.
    pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryItem>> {
        let pattern = format!("%{}%", escape_like(query));
        self.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, method, url, request_headers, request_body
                 FROM history
                 WHERE url LIKE ?1 ESCAPE '\\' OR method LIKE ?1 ESCAPE '\\'
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?2",
            )?;
            let items = stmt.query_map(params![pattern, limit as i64], row_to_history_item)?;
            let mut result = Vec::new();
            for item in items {
                result.push(item?);
            }
            Ok(result)
        })
    }
```

Note: the `ORDER BY timestamp DESC, id DESC` tie-breaker makes "newest first" deterministic even when several inserts share a timestamp (the test relies on this).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all tests pass, including the existing `history_roundtrip` and `crud_and_active` (confirms the `load_recent_history` refactor didn't regress).

- [ ] **Step 7: Commit**

```bash
git add src/db.rs
git commit -m "feat(db): add search_history (LIKE over url+method), share row mapping

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Search icon asset

**Files:**
- Create: `assets/icons/search.svg`

- [ ] **Step 1: Create the icon**

Write `assets/icons/search.svg` (lucide "search", `currentColor` so it inherits theme color, matching the existing icon style):

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
```

- [ ] **Step 2: Verify it is picked up by the embed**

Run: `ls assets/icons/search.svg`
Expected: the path prints (rust-embed embeds `./assets` at compile time, so a rebuild will include it; no code registration needed — same as `code.svg`).

- [ ] **Step 3: Commit**

```bash
git add assets/icons/search.svg
git commit -m "feat(assets): add search icon for history search input

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: History panel search UI + wiring

**Files:**
- Modify: `src/history_panel.rs` (imports, struct, `new`, add `on_search_change` + `refresh_list`, `reload`, `render`)

This task has no unit test (it is GPUI render wiring); verification is `cargo check` plus a manual native run. Build the whole change, then verify it compiles.

- [ ] **Step 1: Update imports**

Replace the `gpui_component` import block at `src/history_panel.rs:3-6` with:

```rust
use gpui_component::{
    button::*, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex, ActiveTheme as _, Icon, Sizable as _,
};
```

- [ ] **Step 2: Add fields to the struct**

In `struct HistoryPanel` (`src/history_panel.rs:19-23`), add the search input and query string:

```rust
pub struct HistoryPanel {
    db: Arc<Database>,
    history: Vec<HistoryItem>,
    selected_id: Option<i64>,
    search: Entity<InputState>,
    query: String,
}
```

- [ ] **Step 3: Build the input and subscribe in `new`**

Replace `new` (`src/history_panel.rs:26-35`) with (note `window`/`cx` are now used, no longer `_`-prefixed):

```rust
    pub fn new(db: Arc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Load initial history from database
        let history = db.load_recent_history(100).unwrap_or_default();

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search history"));
        cx.subscribe(&search, Self::on_search_change).detach();

        Self {
            db,
            history,
            selected_id: None,
            search,
            query: String::new(),
        }
    }
```

- [ ] **Step 4: Add `on_search_change` and `refresh_list`; update `reload`**

Add these two methods inside `impl HistoryPanel` (e.g. right after `new`), and replace the existing `reload` (`src/history_panel.rs:38-41`) with the version below:

```rust
    /// Re-query the list to honor the current query: recent when empty,
    /// search otherwise. Shared by typing and by `reload`.
    fn refresh_list(&mut self) {
        let q = self.query.trim();
        self.history = if q.is_empty() {
            self.db.load_recent_history(100).unwrap_or_default()
        } else {
            self.db.search_history(q, 100).unwrap_or_default()
        };
    }

    fn on_search_change(
        &mut self,
        _state: Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Change = event {
            self.query = self.search.read(cx).value().to_string();
            self.refresh_list();
            cx.notify();
        }
    }

    /// Reload history from database, honoring the active search query.
    pub fn reload(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_list();
        cx.notify();
    }
```

- [ ] **Step 5: Render the search row**

In `render`, insert a search row immediately after the header `.child(...)` block (after `src/history_panel.rs:114`, before the `.when(self.history.is_empty(), ...)`):

```rust
            .child(
                // Search row (always visible, like Postman)
                div()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Input::new(&self.search)
                            .small()
                            .cleanable(true)
                            .prefix(Icon::empty().path("icons/search.svg")),
                    ),
            )
```

- [ ] **Step 6: Make the empty state query-aware**

Replace the empty-state child text in the `.when(self.history.is_empty(), ...)` block (`src/history_panel.rs:115-127`). Keep the same container styling; only the text becomes dynamic:

```rust
            .when(self.history.is_empty(), |this| {
                let msg = if self.query.trim().is_empty() {
                    "No history yet\n\nSend a request to get started".to_string()
                } else {
                    format!("No history matches \"{}\"", self.query.trim())
                };
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .text_sm()
                        .child(msg),
                )
            })
```

- [ ] **Step 7: Verify it compiles**

Run: `cargo check 2>&1 | tail -20`
Expected: `Finished` with no errors. (Cannot run the GPUI app under WSL2; compile is the automated gate.)

- [ ] **Step 8: Commit**

```bash
git add src/history_panel.rs
git commit -m "feat(history): add always-visible search box filtering url+method

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Final verification

- [ ] Run the full test suite: `cargo test 2>&1 | tail -20` — expected: all pass.
- [ ] Run `cargo check` — expected: clean.
- [ ] Manual (native host only, optional): launch the app, type in the History search box, confirm the list filters by URL and method, the ✕ clears it, and an unmatched query shows `No history matches "…"`.
