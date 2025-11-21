# Known Issues

This document tracks known issues and bugs in Poopman.

## UI Issues

### 1. Response Splitter Drift
- **Status:** Open
- **Description:** After the application window starts, the vertical resizable splitter between the Request Editor and Response Viewer drifts downward multiple times automatically.
- **Expected:** The splitter should remain at its initial position until manually adjusted by the user.
- **Workaround:** Manually drag the splitter back to the desired position.

## Build/CI Issues

### 2. Linux Build Not Supported
- **Status:** Open
- **Description:** Linux build target has been removed from GitHub Actions due to missing GPUI dependencies on GitHub Actions runners.
- **Notes:** Currently only Windows and macOS builds are supported in CI. Linux users need to build from source.

---

## Resolved Issues

### Response Header UI Deformation
- **Fixed in:** 2025-11-21
- **Solution:** Added scroll container with `v_flex()`, `track_scroll()`, and `overflow_scroll()` in `src/response_viewer.rs`. Added text truncation with ellipsis for long header values.

### Duplicate Tab Creation from History
- **Fixed in:** 2025-11-21
- **Solution:** Added `history_id` field to `RequestTab` struct. Modified `open_history_in_new_tab()` to check for existing tabs before creating new ones.

### Event Passthrough Between Panels
- **Fixed in:** 2025-11-21
- **Solution:** Added `on_scroll_wheel()` and `on_click()` with `stop_propagation()` to isolate scroll and click events between panels.

### Response Not Isolated Per Tab
- **Fixed in:** 2025-11-21
- **Solution:** Added `response: Option<ResponseData>` field to `RequestTab` struct. Updated tab switching logic to save and restore response state.

### History Record Duplication
- **Fixed in:** 2025-11-21
- **Solution:** Modified `RequestCompleted` handler to check `history_id` before creating new history entries. Requests from history no longer create duplicate records.

### CMD Window on Windows Startup
- **Fixed in:** 2025-11-21
- **Solution:** Added `#![windows_subsystem = "windows"]` attribute to `src/main.rs`

### History Panel Splitter Not Draggable
- **Fixed in:** 2025-11-21
- **Solution:** Replaced fixed width layout with `h_resizable` component in `src/app.rs`

### GitHub Actions Workflow Issues
- **Fixed in:** 2025-11-21
- **Solution:** Added `permissions: contents: write` for release job, updated `softprops/action-gh-release` to v2
