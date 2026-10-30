# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Poopman is a Postman-like desktop API client in Rust, built on GPUI 0.2 (Zed's UI layer) and gpui-component 0.5. Layout: sidebar (Collections / History) on the left, request tabs + request editor top-right, response viewer bottom-right. README.md has the user-facing feature list, data-storage paths, and a per-file module map.

## Commands

```bash
cargo run                                         # debug build + run
cargo build --release
cargo test                                        # all tests (unit + gpui TestAppContext UI tests)
cargo test <name_substring>                       # single test / module, e.g. cargo test url_params
cargo test http_client::connection_pool_tests -- --nocapture   # real keep-alive server, prints connection reuse/timings
cargo clippy
cargo run --profile profiling --features profile  # Tracy spans (app + GPUI's own instrumentation)
uv run tests/verify_curl_fixtures.py              # oracle: replays tests/fixtures/import/curl.json through the real curl binary
```

### WSL2 environment

The repo lives on `/mnt/e` and is usually driven from WSL2. In WSL only `cargo check` / `cargo clippy` work — `cargo build`/`cargo test` fail to link (no libxkbcommon), and the GUI cannot run there at all. Build and test on the Windows side instead:

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

Windows builds need `GPUI_FXC_PATH` pointing at the Windows SDK `fxc.exe`, and any running `poopman.exe` must be killed first (file lock). GUI behavior can't be verified by tests alone — visual/interaction changes need the user to check the running app.

### Releases

`.github/workflows/release.yml` is `workflow_dispatch`-only: the typed version is written into **both** `[package]` and `[package.metadata.bundle]` versions in Cargo.toml, committed, then built for Linux/macOS (x64+arm)/Windows. Don't hand-bump versions or push tags.

## Architecture

### One editor, many tabs

There is a **single** `RequestEditor` and a single `ResponseViewer` entity. Tabs (`RequestTab`, src/request_tab.rs) are plain data: on tab switch `PoopmanApp::save_current_tab_state` copies the editor's request/body draft/params/headers/response into the outgoing tab, and `load_tab_into_editor` / `load_tab_response` push the incoming tab back in. Consequences:

- Any new piece of editor state must be captured and restored through this path, or it leaks between tabs.
- In-flight requests are bound to their originating tab via `active_request_id`; `RequestCompleted`/`RequestCancelled` events must match it before mutating a tab (the user may have switched tabs mid-request).
- Dirty tracking: each tab holds a `baseline: RequestSnapshot`; `RequestSnapshot::matches` deliberately ignores blank rows and transport-owned headers so they don't make a tab dirty. Saves to collections are explicit (Ctrl/Cmd+S, Save As) and serialized per tab (`saving_tabs`, src/app/request_saving.rs).

### Event wiring

`PoopmanApp` (src/app.rs) owns every panel and wires them with `cx.subscribe_in`. Main events: `RequestStarted` / `RequestCompleted` / `RequestCancelled` / `SaveRequestRequested` / `OpenCodeSnippet` (RequestEditor), `HistoryItemClicked` (HistoryPanel), `SavedRequestClicked` / `NewRequestRequested` / `CollectionsChanged` (CollectionsPanel), tab-bar click/close/new events, and environment changes from `EnvironmentManager`. Keyboard actions (`SendRequest`, `SaveRequest`, `NewTab`, `CloseTab`, `NextTab`, `PrevTab`, `FocusUrl`, `Quit`) are declared with `actions!` in app.rs and handled with `on_action` on the root element.

### Database: single owner thread, never block the UI

`Database` (src/db.rs) owns the rusqlite `Connection` on a dedicated thread; callers send closures over a channel and block on a reply channel (`Database::call`). `register_ui_thread()` makes `call` **return an error if invoked on the GPUI thread** — all DB access from UI code must go through `cx.background_spawn(...)`. Startup loading happens in `AppInitialState::load()` before the window opens. Environment edits are persisted asynchronously in order via `EnvironmentPersistence` and flushed on shutdown (main.rs).

Schema is created in `init_schema` (shared by real and in-memory test DBs); column additions use idempotent `PRAGMA table_info` checks (see `migrate_add_request_auth`). DB path resolution and the one-time legacy `~/.poopman/history.db` migration live in src/data_directory.rs (behavior documented in README "Data Storage"). Auth secrets are intentionally kept out of history rows.

### HTTP

src/http_client.rs builds requests natively with reqwest on a shared multi-threaded tokio runtime, bridged into GPUI async. `HttpClientPool` keeps a `reqwest::Client` across tabs/sends and rebuilds it only when connection-affecting settings change; per-request timeouts and response size limits are captured per send. Redirects are followed manually (credential stripping across origins, scheme checks). Sends are cancellable (`InFlightRequest` aborts the tokio task). Settings (src/settings.rs) are shared as `Arc<RwLock<AppSettings>>` so changes apply to the next send before the disk write finishes. Do not add a default request timeout — Postman has none; timeouts must stay opt-in.

### Pure modules

`code_gen`, `variables`, `url_params`, `code_formatter`, `curl_import`, `postman`, `format`, `header_names` are side-effect-free and carry most of the unit tests. Import tests share JSON fixtures in `tests/fixtures/import/` via `include_str!`. Put new parsing/transformation logic in a pure module and keep the gpui components thin.

### URL ↔ Params sync

Bidirectional sync uses **focus arbitration**, not reentrancy flags: `parse_url_to_params` only acts when the URL input is focused, `sync_params_to_url` only when a param input is focused, so programmatic `set_value` changes can't loop. Use the un-gated `rebuild_*` helpers when syncing programmatically.

## GPUI gotchas (all compile cleanly and fail at runtime)

- **`PoopmanApp.focus_handle` is load-bearing.** If window focus is ever `None`, every keyboard shortcut silently dies. It is focused in `new()` and `track_focus`ed on the content div *below* the title bar — tracking it on the root kills the Windows minimize/maximize/close buttons.
- Never call `window.request_animation_frame()` from an action listener (panics outside paint).
- Scrolling convention: viewport (`flex_1().min_h_0()`) → scroller (`.id().size_full().track_scroll(&h).overflow_scroll()`) → scrollbar as the scroller's **sibling** (`.vertical_scrollbar` on the parent). Don't use `.overflow_y_scrollbar()` on elements whose own gap/padding matter.
- `flex_shrink` defaults to 1.0 — children that must overflow a strip need `.flex_shrink_0()`.
- Size a `ResizablePanel`'s child with `flex_1()` + explicit cross-axis size, never `size_full()`.
- gpui-component completion menus need MoveUp/MoveDown bridged manually on single-line inputs (see header_completion / request_editor).
- In files with `use gpui::*`, test modules import names explicitly instead of `use super::*`, because gpui's `test` attribute macro shadows `#[test]`.
- `Input` is editable by default; read-only views need `.disabled(true)`.

Design write-ups with the full reasoning behind these (and past features) are in `docs/superpowers/specs/`; implementation plans in `docs/superpowers/plans/`.

## Assets

SVG icons in `assets/icons/**` are embedded via `rust-embed` (`Assets` in main.rs). The Windows exe icon is embedded by build.rs (`winresource`). Theme colors/dimensions live in src/theme.rs; shared visual primitives in src/ui.rs.
