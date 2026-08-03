# Poopman — API Client

A Postman-like desktop API client built in Rust with the [GPUI](https://www.gpui.rs/)
framework and the `gpui-component` library. The left sidebar switches between
collections and history; the request editor is on the top-right and the response
viewer is on the bottom-right.

## Features

- **HTTP requests** — GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS.
- **Request editor**
  - Method selector + URL input.
  - **Query params** with bidirectional URL ↔ params synchronization.
  - **Headers** — predefined (toggleable) and custom (deletable); `Content-Type`
    and `Content-Length` are kept in sync with the body automatically.
  - **Body** — Raw (JSON / XML / Text / JavaScript, with syntax highlighting and a
    Beautify action) and **multipart Form-data** (text and file fields).
- **Response viewer** — status / duration / size, pretty-printed JSON, response
  headers, and a **binary response** path that previews type/size and saves to disk.
- **Environments & variables** — define `{{variable}}` sets per environment, pick the
  active one from the Edit menu, and have them resolved at send time (and in code export).
- **Code snippet export** — turn the current request into runnable code via the `</>`
  button: **cURL, Rust (reqwest), Python (Requests), JavaScript (Fetch), NodeJS (Axios),
  Go (net/http)**, with Copy. Raw and multipart form-data bodies both export.
- **Tabs** — work on multiple requests at once.
- **History** — every sent request is stored in SQLite; click to reload, or clear all.
- **Collections** — explicitly save requests into collections or nested folders;
  create, rename, duplicate, search, and delete tree nodes with cascading deletion.
- **Postman interoperability** — import and export **Postman Collection v2.1** JSON.
  Request URLs, disabled query/header/form rows, raw/form-data/url-encoded bodies,
  and Bearer/Basic/API-key auth are mapped where supported; unsupported fields are
  reported as non-blocking import warnings.

## Requirements

- Rust (2024 edition).
- A GPU-capable environment — GPUI requires GPU acceleration.
- **Not supported under WSL2** (no GPU surface). Build/test there is fine; run the app
  on native Linux / macOS / Windows.

## Build & Run

```bash
cargo run                 # debug
cargo build --release     # optimized binary at target/release/poopman[.exe]
cargo test                # unit tests (pure modules: code_gen, variables, url_params, db, …)
```

## Usage

1. **Send a request** — pick a method, enter a URL (e.g. `https://api.github.com/zen`),
   optionally set Params / Headers / Body, then click **Send**.
2. **View the response** — status, time, and size in the status bar; formatted body and
   headers in their tabs; binary payloads can be saved to a file.
3. **Environments** — open **Edit → Manage Environments…** to create environments and
   variables, then select the active one from the Edit menu. Reference them anywhere as
   `{{name}}`.
4. **Export code** — click the `</>` button next to Send, choose a language, and Copy.
5. **History** — sent requests are saved automatically; click one to reload it, or Clear.
6. **Collections** — switch the sidebar to **Collections**, create a collection or
   folder, then click the bookmark button beside the request actions. New saves ask
   for a request name and destination; clicking the bookmark on an existing saved
   request updates it.
7. **Postman** — use **Import** in the Collections sidebar, or use a collection's
   context menu to export a v2.1 JSON file.

## Data Storage

History and environments live in a SQLite database at:

- Linux / macOS: `~/.poopman/history.db`
- Windows: `%USERPROFILE%\.poopman\history.db`

## Architecture

GPUI's entity-component system with a pub/sub event model. `PoopmanApp` composes the
panels and wires events (`RequestCompleted`, `HistoryItemClicked`, `EnvironmentsChanged`,
`OpenCodeSnippet`, tab events) between them.

```
src/
├── main.rs                # Entry point, embedded SVG assets, logging
├── app.rs                 # Root layout, tabs, dialogs, event wiring
├── types.rs               # Core data types (RequestData, ResponseData, BodyType, …)
├── db.rs                  # SQLite access — CSP style (see note below)
├── http_client.rs         # reqwest wrapper: shared client + shared tokio runtime
├── request_editor.rs      # Method / URL / params / headers, send logic
├── body_editor.rs         # Request body (Raw + multipart Form-data)
├── response_viewer.rs     # Response display (text / binary / headers)
├── history_panel.rs       # History list
├── collections_panel.rs    # Collection/folder/request tree and Postman file actions
├── environment_manager.rs # Environment CRUD dialog
├── variables.rs           # Pure {{variable}} substitution
├── code_gen.rs            # Pure code-snippet generation (6 targets)
├── code_snippet_panel.rs  # Code snippet dialog (language select + Copy)
├── tab_bar.rs             # Request tab strip
├── request_tab.rs         # Per-tab model
├── menu_bar.rs            # Edit menu (environment switching)
├── postman.rs              # Pure Postman Collection v2.1 conversion
├── url_params.rs           # Pure URL / query-param helpers
├── code_formatter.rs      # Pure JSON / XML formatting & validation
├── ui.rs                  # Shared visual primitives (cards, segmented pills)
└── theme.rs               # Warm-light theme + layout dimensions
```

### Concurrency notes

- **Database (CSP):** the SQLite `Connection` is owned by a single background thread.
  Callers don't share it behind a `Mutex`; they send jobs over a channel and receive
  results back over a per-call reply channel — "share memory by communicating." One
  owner means no data races and no lock to poison.
- **HTTP:** a single `reqwest::Client` (connection pool) is shared across requests, and
  all requests run on one shared multi-threaded tokio runtime, bridged to GPUI's async.
- **Pure modules** (`code_gen`, `variables`, `url_params`, `code_formatter`, parts of
  `types`) are side-effect-free and unit-tested directly.

## Key Technologies

- **GPUI 0.2** — GPU-accelerated UI framework (Zed's UI layer).
- **gpui-component 0.5** — buttons, inputs, selects, tabs, dialogs, code editor.
- **rusqlite** — bundled SQLite.
- **reqwest** — HTTP client (json, multipart, stream).
- **tokio** — async runtime for HTTP.
- **tree-sitter** — syntax highlighting (JSON, Rust, Python, Go, JavaScript, …).
- **rust-embed** — embeds SVG icons into the binary.

## Possible Future Enhancements

- [x] Request collections / folders.
- [x] Authentication presets (Bearer, Basic, API key).
- [x] Export / import collections (Postman Collection v2.1).
- [ ] Search / filter in history.
