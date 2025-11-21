# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Poopman is a Postman-like API client built with the GPUI framework and gpui-component library. It provides a desktop application for making HTTP requests with a classic Postman-style layout: history panel on the left, request editor on top-right, and response viewer on bottom-right.

## Build and Run

```bash
# Run the application
cargo run

# Build release version
cargo build --release
```

## Architecture

### Component Structure

The application uses GPUI's entity-component system with a pub-sub event model:

- **`PoopmanApp` (src/app.rs)**: Root component that composes all panels and manages event subscriptions between components
  - Subscribes to `RequestCompleted` events from `RequestEditor` to save to database and update `ResponseViewer`
  - Subscribes to `HistoryItemClicked` events from `HistoryPanel` to load requests into `RequestEditor`
  - Uses `v_resizable` for vertical resizing between request editor and response viewer

- **`RequestEditor` (src/request_editor.rs)**: Top-right panel for configuring HTTP requests
  - Contains `BodyEditor`, method selector, URL input, and headers management
  - Emits `RequestCompleted` event with both request and response data when send completes
  - Manages predefined headers (mandatory/toggleable) and custom headers separately via `HeaderType` enum

- **`ResponseViewer` (src/response_viewer.rs)**: Bottom-right panel for displaying responses
  - Shows status code, duration, response size in status bar
  - Tab interface for Body and Headers views
  - JSON syntax highlighting and formatting

- **`HistoryPanel` (src/history_panel.rs)**: Left panel showing request history
  - Displays recent requests from SQLite database
  - Emits `HistoryItemClicked` event when user clicks a history item
  - Reload functionality to refresh after new requests

- **`BodyEditor` (src/body_editor.rs)**: Request body configuration component
  - Supports Raw (JSON/XML/Text/JavaScript) and Form-data body types
  - Embedded in `RequestEditor`

### Data Flow

1. User configures request in `RequestEditor` and clicks Send
2. `RequestEditor` makes HTTP call using custom `HttpClient` (wraps reqwest with tokio runtime)
3. `RequestEditor` emits `RequestCompleted` event with request+response
4. `PoopmanApp` receives event and:
   - Saves to SQLite via `Database` (src/db.rs)
   - Updates `ResponseViewer` with response
   - Triggers `HistoryPanel` reload
5. User can click history item to reload request into editor

### Database (src/db.rs)

- SQLite database stored at `~/.poopman/history.db` (Linux/macOS) or `%USERPROFILE%\.poopman\history.db` (Windows)
- Stores request method, URL, headers (JSON), body, response status, duration, headers, body
- Thread-safe via `Arc<Mutex<Connection>>`

### Type System (src/types.rs)

Key types:
- `HttpMethod`: Enum for GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS
- `BodyType`: Enum for None, Raw (with RawSubtype), FormData
- `RequestData`: Complete request configuration (method, url, headers, body)
- `ResponseData`: Complete response (status, duration, headers, body)
- `HistoryItem`: Database record combining request + response
- `HeaderType`: Mandatory (always enabled) vs Predefined (toggleable) vs Custom (deletable)
- `PredefinedHeader`: Common headers like Content-Type, Cache-Control, User-Agent

### HTTP Client (src/http_client.rs)

Custom `HttpClient` that bridges GPUI's async HTTP interface with reqwest:
- Uses `OnceLock<Runtime>` for a shared tokio runtime (2 worker threads)
- Converts between GPUI's `AsyncBody` and reqwest's body types
- Necessary because GPUI's built-in HTTP client may have limitations for this use case

## Key Technologies

- **GPUI 0.2.2**: GPU-accelerated UI framework (Zed editor's UI layer)
- **gpui-component 0.4**: Component library providing buttons, inputs, selects, tabs, etc.
- **rusqlite 0.32**: SQLite database with bundled feature
- **reqwest 0.12**: HTTP client with json, multipart, and stream features
- **tokio**: Async runtime for HTTP operations
- **rust-embed**: Embeds assets (SVG icons) into binary
- **chrono**: Timestamp handling for history
- **serde/serde_json**: Serialization for database and type conversion

## Development Notes

### Asset Management

- SVG icons stored in `./assets/icons/**/*.svg`
- Embedded at compile time via `rust-embed` (see `Assets` struct in main.rs)
- Windows: Icon embedded via `build.rs` using `winresource`

### GPUI Patterns

- Use `cx.new(|cx| ...)` to create entities
- Use `entity.update(cx, |instance, cx| ...)` to mutate entity state
- Use `cx.subscribe_in(&entity, window, callback)` for event handling
- GPUI rendering uses declarative `div()` builders with method chaining
- Colors/spacing from `cx.theme()` for consistency
