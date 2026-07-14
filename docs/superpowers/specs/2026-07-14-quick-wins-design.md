# Quick-Win Improvements Design

Date: 2026-07-14
Status: Approved

Four small user-facing features, delivered as **one PR each**, in this order.
Each PR follows the project gate: TDD for pure logic → `cargo clippy
--all-targets` 0 warnings (WSL) → `cargo test` all green (Windows via
`pwsh.exe`) → PR → user visual verification on Windows → merge.

Response-body search/copy was considered and dropped: the response editor
already supports it.

---

## PR 1 — Request cancellation (Send ⇄ Cancel)

**Decision:** the Send button turns into a red Cancel button while a request
is in flight (Postman behavior, no extra layout space).

**Mechanism.** `HttpClient::send` currently does `runtime.spawn(...).await`
on the shared tokio runtime. Split it in two:

- `start_send(...) -> InFlightRequest` — spawns onto the runtime and returns
  a wrapper holding the `tokio::task::JoinHandle`, exposing `abort_handle()`.
- `InFlightRequest::wait().await -> Result<HttpResponse>` — awaits the join
  handle; a `JoinError` with `is_cancelled()` maps to a dedicated `Canceled`
  error variant.

Cancel = `AbortHandle::abort()`: the tokio-side task is genuinely aborted
(an in-progress large download stops; we don't merely ignore the result).
Rejected alternative: drop-the-future-only cancellation — simpler but leaves
the transfer running in the background.

**RequestEditor.** Stores `abort_handle: Option<AbortHandle>` and a
`send_generation: u64` counter. Every send increments the generation and
captures it in the spawned future; completion handlers compare generations so
a stale (canceled-then-resent) task can never clobber newer state.

**On cancel:** emit a `RequestCancelled` event → `ResponseViewer` shows
"Request canceled"; the canceled request is **not** written to history (same
as Postman). `loading` resets to false.

**Tests (TDD):** `start_send` + immediate `abort()` → `wait()` returns the
`Canceled` error. Runs on the real tokio runtime, no network needed.

---

## PR 2 — cURL import (smart paste into the URL bar)

**Decision:** pasting text starting with `curl ` into the URL input parses it
and populates method/URL/headers/body (Insomnia behavior, zero extra UI).
Rejected alternatives: menu + dialog import (Postman style, needs new modal
UI); doing both (most work, later if wanted).

**Parser.** New pure module `src/curl_import.rs`:

- Shell-style tokenizer: single quotes, double quotes, backslash escapes,
  line continuations (`\` + newline).
- Supported flags: `-X`/`--request`, `-H`/`--header`, `-d`/`--data`/
  `--data-raw`/`--data-binary` (implies POST when no explicit method),
  `--data-urlencode`, `-F`/`--form`, `-u`/`--user` (becomes a
  `Authorization: Basic <base64>` header), `--url`, bare URL argument.
- Unknown flags are skipped silently (with their value if they take one).
- Output: `ParsedCurl { method, url, headers, body }` where body reuses the
  existing `BodyType` model (`Raw` for `-d`, `FormData` for `-F`).

**UI hook.** In the URL input change handler: if the new value starts with
`curl ` and parses successfully, populate the request fields; on parse
failure leave the pasted text as-is.

**Tests (TDD):** the bulk of the work — quoting/escaping, multiple `-H`,
`-d` implying POST, `-u` basic auth, `-F` form fields, flag order, junk
input returning `None`.

---

## PR 3 — Keyboard shortcuts

**Decision (scope):** `ctrl-enter` = send request, `ctrl-t` = new tab,
`ctrl-w` = close current tab. (Ctrl+Tab switching and Ctrl+L focus-URL were
considered and deferred.)

**Mechanism.** First use of gpui's native action system in this project:
`actions!(poopman, [SendRequest, NewTab, CloseTab])`, `cx.bind_keys` in
main.rs, `.key_context("Poopman")` + `.on_action(...)` on the app root,
dispatching to `RequestEditor` / tab bar.

**Risk:** keystrokes must bubble up from focused text inputs to the app-level
context. gpui dispatches deepest-first, and the input widgets don't bind
these combos, so bubbling is expected — but this needs visual verification
on Windows before merge.

**Tests:** pure UI wiring; nothing meaningfully unit-testable. Relies on the
user's visual check.

---

## PR 4 — Inline image preview for binary responses

**Decision:** when a binary response's Content-Type maps to a gpui-supported
image format (png/jpeg/webp/gif/svg/bmp/tiff), the binary panel renders the
image inline from memory (`img()` with `Arc<gpui::Image>` — no temp file),
scaled to fit with a max height. The existing "type · size + Save to file"
row stays below the image. Unsupported binary types keep the current info
panel.

**Tests (TDD):** pure `content_type -> Option<ImageFormat>` mapping function
(parameter/charset stripping, case-insensitivity, unknown types → None).

---

## Cross-cutting constraints

- No default request timeout (previously rejected; Postman has none).
- `HttpMethod` variant names are serialized into the history DB — don't
  rename.
- Every GUI change requires user visual verification on Windows.
- WSL can only `cargo check`/`clippy`; `cargo test` runs on Windows via
  `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`.
