# Spec: form-data body support in code snippet export

Date: 2026-07-14
Status: Approved (design confirmed by user via Telegram; B/Ctrl-Tab and C/Ctrl-L deferred)

## Goal

`src/code_gen.rs` currently emits a `NOTE: form-data body is not yet supported`
comment for all six targets when the request body is multipart form-data.
Replace that with real, runnable multipart code in every target:
cURL, Rust (reqwest), Python (Requests), JavaScript (Fetch), NodeJS (Axios),
Go (net/http).

## Scope decisions

- **Rows exported:** only rows with `enabled == true` and a non-blank key.
  A form-data body whose rows are all disabled/blank exports like `BodyType::None`.
- **Content-Type header is skipped** (case-insensitive match) in ALL targets
  when a non-empty form-data body is present. The UI pins a read-only
  `multipart/form-data; boundary=<auto>` header on the request; exporting it
  verbatim would break the request because each library must generate its own
  boundary. Raw/None bodies keep the existing header pass-through.
- **Variables:** `generate()` receives an already-resolved `RequestData`
  (resolution happens upstream, unchanged).
- **File fields embed the local path** the user typed in the Form-data UI.
  That path is machine-specific by nature — same behavior as Postman's codegen.
- Out of scope: Ctrl+Tab/Ctrl+L shortcuts (deferred), snippet UI changes
  (panel already re-generates on demand), send-path changes (real multipart
  sending shipped earlier).

## Shared plumbing (`src/code_gen.rs`)

Replace `form_data_present()` with:

```rust
/// Enabled, non-blank-key form-data rows — the rows that export.
fn form_rows(req: &RequestData) -> Vec<&FormDataRow> // empty vec unless BodyType::FormData
```

Add a small helper for header emission:

```rust
/// Headers to export: all non-blank headers, minus Content-Type when a
/// non-empty form-data body is present (each target's library generates
/// its own multipart boundary).
fn export_headers(req: &RequestData) -> Vec<(&str, &str)>
```

All six generators switch from `headers(req)` to `export_headers(req)` and
drop their NOTE lines. Existing escaping helpers are reused: `shell_single`
for cURL, `dq` for double-quoted contexts (also correct for Windows paths —
`C:\x` → `C:\\x`).

## Per-target emission (file field key `f`, path `p`; text field key `k`, value `v`)

**cURL** — text fields use `--form-string` (immune to curl's `@`/`<`
leading-char file expansion); file fields use `--form` with `@`:

```
  --form-string 'k=v'
  --form 'f=@"p"'         (p shell-single-escaped inside the double quotes)
```

**Rust (reqwest, blocking — matches existing style)** — add
`use reqwest::blocking::multipart;`; build before the request:

```rust
let form = multipart::Form::new()
    .text("k", "v")
    .file("f", "p")?;
```

and attach with `.multipart(form)` instead of `.body(...)`.

**Python (Requests)** — a single `files` dict forces multipart even when all
fields are text (bare `data=` alone would send urlencoded):

```python
files = {
    "k": (None, "v"),
    "f": open("p", "rb"),
}
response = requests.request("POST", url, headers=headers, files=files)
```

**JavaScript (Fetch)** — browser JS cannot open a local path; file fields
emit a commented placeholder:

```js
const formdata = new FormData();
formdata.append("k", "v");
// Browsers can't read local paths — wire this to an <input type="file">:
// formdata.append("f", fileInput.files[0], "basename-of-p");
```

`body: formdata` in requestOptions (no Content-Type header appended).

**NodeJS (Axios)** — `form-data` package:

```js
const FormData = require("form-data");
const fs = require("fs");
const formdata = new FormData();
formdata.append("k", "v");
formdata.append("f", fs.createReadStream("p"));
```

config gets `data: formdata` and
`headers: { ...formdata.getHeaders(), /* other exported headers */ }`.

**Go (net/http)** — `bytes.Buffer` + `mime/multipart` (new imports: `bytes`,
`mime/multipart`, `os`, `path/filepath`; `io` already imported):

```go
payload := &bytes.Buffer{}
writer := multipart.NewWriter(payload)
_ = writer.WriteField("k", "v")
file, err := os.Open("p")
// err check
part, err := writer.CreateFormFile("f", filepath.Base("p"))
// err check
_, err = io.Copy(part, file)
file.Close()
writer.Close()
```

request body is `payload`, then after the exported headers:
`req.Header.Set("Content-Type", writer.FormDataContentType())`.

## Docs touched

- `src/code_gen.rs` module doc: drop the "v1 supports None and Raw" caveat.
- `README.md`: remove the "Form-data export in code snippets" item from
  Possible Future Enhancements; update the code-export feature bullet.

## Testing (pure unit tests in `code_gen::tests`)

New tests (per existing test style, one request fixture with a text row, a
file row, and a disabled row):

1. Each of the six targets: output contains the text field, the file
   field/path, and NOT the disabled row's key, and NOT the NOTE string.
2. Content-Type skipped: a request with a `Content-Type: multipart/...`
   header + form-data body → no `Content-Type` in output (all targets);
   control: raw-body request still exports its Content-Type.
3. cURL specifics: text field uses `--form-string`, file field uses
   `--form 'f=@...'`; a text value starting with `@` survives literally.
4. Python: all-text form still emits `files = {` with `(None, ...)` tuples
   (multipart forced), no `data=` payload.
5. Empty/disabled-only form-data exports like a body-less request.

Test gate (WSL cannot link the GUI crate):
`pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test code_gen"`.

## Branch

`feat/formdata-code-export` off `main`, merged via PR per repo convention.
