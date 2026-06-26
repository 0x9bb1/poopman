# Spec: History search (URL + method, DB-backed LIKE)

Date: 2026-06-26
Status: Approved

## Goal

Add a search box to the History panel that filters stored requests by **URL or
HTTP method** across the **entire** history (not just the loaded page), matching
Postman's always-visible history search.

## Scope decisions (settled during brainstorming)

- **Match fields:** URL + method only. Not request body / headers.
- **Backend:** DB-backed `LIKE` over all rows. Rejected client-side filtering
  (only sees the loaded 100) and rejected FTS5 / inverted index as YAGNI:
  local single-user data is tiny, `LIKE` on two short fields is sub-millisecond,
  and FTS5 is token-based so it would not even do the substring matching a
  filter box implies. Revisit FTS5 + trigram only if we later add full-text
  body/response search over millions of rows with relevance ranking.
- **UI:** an always-visible search row under the panel header.
- **Out of scope (v1):** match-source badges (`· body`), substring highlighting,
  date grouping, retention/pruning, dedup.

## Storage — `src/db.rs`

New method:

```rust
pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryItem>>
```

SQL:

```sql
SELECT id, timestamp, method, url, request_headers, request_body
FROM history
WHERE url LIKE ?1 ESCAPE '\' OR method LIKE ?1 ESCAPE '\'
ORDER BY timestamp DESC
LIMIT ?2
```

- Bind `?1` = `%{query}%`, escaping the user's `%`, `_`, and `\` so literal
  wildcards in the query are matched literally (paired with `ESCAPE '\'`).
- SQLite `LIKE` is case-insensitive for ASCII, so `LOGIN` matches `login`.
- `limit` binds as `i64` (rusqlite 0.40 dropped `ToSql` for `usize`), matching
  the existing `load_recent_history`.

**Refactor to avoid drift:** extract the row → `HistoryItem` mapping currently
inline in `load_recent_history` into one private helper (e.g.
`fn row_to_history_item(row) -> rusqlite::Result<HistoryItem>`), and have both
`load_recent_history` and `search_history` use it. This mirrors the existing
shared-`init_schema` discipline so the two queries can never diverge.

No new index: substring `%query%` cannot use a B-tree index anyway, and full
scans over this data set are negligible.

## Interaction — `src/history_panel.rs`

State additions:
- `search: Entity<InputState>` — created with `InputState::new(window, cx)` and
  `.placeholder("Search history")`.
- `query: String` — the current trimmed query, kept so empty-state text and
  `reload()` can read it.

Wiring:
- In `new(...)`, create the input and `cx.subscribe(&search, Self::on_search_change)`
  for `InputEvent::Change` (same pattern as `body_editor`).
- `on_search_change` reads the input value into `self.query`, then refreshes the
  list (see below) and `cx.notify()`.

List refresh (single helper, e.g. `refresh_list`):
- If `self.query` is empty → `self.db.load_recent_history(100)`.
- Else → `self.db.search_history(&self.query, 100)`.
- Store into `self.history`.

`reload()` (called by `app.rs` after each send) calls the same `refresh_list`
helper so it respects the active query: a newly sent request appears only if it
matches the current filter; an empty query behaves exactly as today.

## UI

A new always-visible search row between the existing `History [Clear]` header and
the list:

```
History                     [Clear]
------------------------------------
[search-icon]  Search history…   (x)
------------------------------------
GET    /api/users           2m ago
POST   /api/login           5m ago
```

- `Input::new(&self.search).small().cleanable(true).prefix(<search icon>)`.
- `.cleanable(true)` provides the built-in clear (✕) button when non-empty;
  clearing resets the list to recent history via the same `Change` path.

## Empty states

- `query` non-empty and zero results → `No history matches "{query}"`.
- `query` empty and history empty → existing `No history yet …` text, unchanged.

## Testing

Extend `db.rs` tests:
- Insert a few rows (varied method/URL), assert `search_history` returns only
  matching rows, newest-first.
- Case-insensitivity: a lowercase query matches an uppercase method.
- Wildcard escaping: a query containing `%` matches a URL with a literal `%`
  and does not match everything.
- Empty/`limit` behaviour consistent with `load_recent_history`.
