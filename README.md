# Poopman - API Client

A Postman-like API client built with GPUI Component library.

## Features

- ✅ **HTTP Requests**: Send GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS requests
- ✅ **Request Configuration**:
  - URL input with autocomplete
  - HTTP method selection
  - Headers (Content-Type, Authorization)
  - JSON request body with syntax highlighting
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

## Running

```bash
cargo run --example poopman
```

Or from the examples directory:

```bash
cd examples/poopman
cargo run
```

## Usage

1. **Make a Request**:
   - Select HTTP method (default: GET)
   - Enter URL (e.g., `https://api.github.com/zen`)
   - (Optional) Add headers in Headers tab
   - (Optional) Add JSON body in Body tab
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

- **GPUI**: GPU-accelerated UI framework
- **GPUI Component**: UI component library (60+ components)
- **SQLite**: Persistent storage via rusqlite
- **ReqwestClient**: HTTP client
- **Tree Sitter**: JSON syntax highlighting

## Future Enhancements

- [ ] Environment variables support
- [ ] Request collections/folders
- [ ] More header presets
- [ ] Query parameters table editor
- [ ] Request/response body format options (XML, HTML, etc.)
- [ ] Export/import collections
- [ ] Dark/Light theme toggle
- [ ] Search in history
