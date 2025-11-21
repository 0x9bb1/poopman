# Poopman - API Client

A Postman-like API client built with GPUI Component library.

## Features

- ✅ **HTTP Requests**: Send GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS requests
- ✅ **Request Configuration**:
  - URL input with autocomplete
  - HTTP method selection (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS)
  - Query parameters editor with bidirectional URL synchronization
  - Headers management (predefined + custom headers)
  - Request body editor supporting multiple formats (JSON, XML, Text, JavaScript, Form-data)
- ✅ **Response Viewer**:
  - Status code, duration, and size display
  - JSON response with syntax highlighting and formatting
  - Response headers view
- ✅ **History**:
  - SQLite database storage
  - Click to reload previous requests
  - Clear history option
- ✅ **Postman Classic Layout**:
  - Left panel: History list
  - Right panel: Request editor (top) + Response viewer (bottom)
  - Resizable panels

## Requirements

- Rust 1.70+ (2021 edition)
- GPU-capable environment (GPUI requires GPU acceleration)
- **Note**: Not supported in WSL2 environments

## Running

```bash
cargo run
```

Build release version:

```bash
cargo build --release
```

## Usage

1. **Make a Request**:
   - Select HTTP method (default: GET)
   - Enter URL (e.g., `https://api.github.com/zen`)
   - (Optional) Configure query parameters in Params tab (automatically syncs with URL)
   - (Optional) Add headers in Headers tab
   - (Optional) Configure request body in Body tab (supports JSON, XML, Text, JavaScript, Form-data)
   - Click "Send" button or press Ctrl/Cmd+Enter

2. **View Response**:
   - See status code, duration, and size in the status bar
   - View formatted JSON response in Body tab
   - Check response headers in Headers tab

3. **History**:
   - All requests are automatically saved
   - Click any history item to reload it in the editor
   - Click "Clear" to delete all history

## Data Storage

History is stored in SQLite database at:
- Linux/macOS: `~/.poopman/history.db`
- Windows: `%USERPROFILE%\.poopman\history.db`

## Architecture

```
src/
├── main.rs              # Application entry point
├── app.rs               # Main layout and component composition
├── types.rs             # Data structures (Request, Response, etc.)
├── db.rs                # SQLite database manager
├── request_editor.rs    # Request editing panel
├── response_viewer.rs   # Response display panel
└── history_panel.rs     # History list panel
```

## Technologies

- **GPUI 0.2.2**: GPU-accelerated UI framework (Zed editor's UI layer)
- **gpui-component 0.4**: UI component library providing buttons, inputs, selects, tabs, etc.
- **rusqlite 0.32**: SQLite database with bundled feature
- **reqwest 0.12**: HTTP client with json, multipart, and stream features
- **tokio**: Async runtime for HTTP operations
- **Tree Sitter**: Syntax highlighting for JSON, XML, JavaScript

## Future Enhancements

- [ ] Environment variables support
- [ ] Request collections/folders
- [ ] Authentication presets (Bearer token, Basic auth, OAuth)
- [ ] Request/response body format options (HTML, etc.)
- [ ] Export/import collections (Postman format)
- [ ] Dark/Light theme toggle
- [ ] Search/filter in history
- [ ] Request duplication
- [ ] Response download/save to file
