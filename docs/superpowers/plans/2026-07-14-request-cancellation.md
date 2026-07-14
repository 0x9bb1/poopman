# Request Cancellation (Send ⇄ Cancel) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** While a request is in flight the Send button becomes a red Cancel button; clicking it genuinely aborts the tokio-side transfer, shows "Request canceled" in the response area, and never writes to history.

**Architecture:** `HttpClient::send` is split into `start_send()` (spawns onto the shared tokio runtime, returns an `InFlightRequest` wrapper exposing `abort_handle()`) and `InFlightRequest::wait()` (awaits the join handle, mapping a cancelled `JoinError` to a `RequestCanceled` marker error). `RequestEditor` stores the abort handle plus a `send_generation` counter so a stale task can never clobber newer state. A new `RequestCancelled` event flows to `PoopmanApp`, which tells `ResponseViewer` to show a canceled notice.

**Tech Stack:** Rust, gpui 0.2.2, gpui-component 0.5.1, tokio (`JoinHandle::abort_handle`), reqwest, anyhow.

**Test gate (every "run tests" step):** WSL cannot link the binary. Run tests on Windows:

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

Compile-only checks in WSL: `cargo check --tests` / `cargo clippy --all-targets`.

---

### Task 0: Branch setup

**Files:** none

- [ ] **Step 1: Create the feature branch off local main** (local main already carries the spec + plan docs commits, so they ride along in this PR)

```bash
cd /mnt/e/code/poopman && git checkout -b feat/request-cancel
```

---

### Task 1: HttpClient cancellable API

**Files:**
- Modify: `src/http_client.rs` (split `send` → `start_send` + `InFlightRequest::wait`; `send` is deleted — its only caller was `RequestEditor`, and keeping it would be dead code)
- Test: same file, new `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests**

Append to `src/http_client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, HttpMethod};
    use std::io::{Read as _, Write as _};

    /// Block on a future using the same runtime `start_send` spawned onto.
    /// (Awaiting a JoinHandle from outside the runtime is exactly what the
    /// gpui side does in production.)
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        RUNTIME
            .get()
            .expect("start_send initializes the runtime")
            .block_on(fut)
    }

    #[test]
    fn abort_maps_to_request_canceled_error() {
        // A listener that accepts but never responds: the request hangs
        // until aborted, no matter how fast or slow the test thread is.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());

        let client = HttpClient::new();
        let inflight = client.start_send(HttpMethod::GET, url, vec![], BodyType::None);
        inflight.abort_handle().abort();

        let err = block_on(inflight.wait()).expect_err("aborted request must fail");
        assert!(
            err.downcast_ref::<RequestCanceled>().is_some(),
            "expected RequestCanceled, got: {err:#}"
        );
    }

    #[test]
    fn start_send_completes_normally_without_abort() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());

        // Minimal one-shot HTTP server on a plain thread.
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf); // consume the request
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
                )
                .unwrap();
        });

        let client = HttpClient::new();
        let inflight = client.start_send(HttpMethod::GET, url, vec![], BodyType::None);

        let response = block_on(inflight.wait()).expect("request should succeed");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hi");
    }
}
```

- [ ] **Step 2: Verify RED**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test http_client"`
Expected: FAIL to compile — `no method named 'start_send'`, `cannot find type 'RequestCanceled'`.

- [ ] **Step 3: Implement the API**

In `src/http_client.rs`, add above `impl HttpClient`:

```rust
/// Marker error: the in-flight request was aborted by the user.
/// Callers detect it with `err.downcast_ref::<RequestCanceled>()`.
#[derive(Debug)]
pub struct RequestCanceled;

impl std::fmt::Display for RequestCanceled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request canceled")
    }
}

impl std::error::Error for RequestCanceled {}

/// A request already running on the tokio runtime. `abort_handle()` lets the
/// UI abort the underlying task — the transfer really stops, the result isn't
/// merely ignored. Await `wait()` for the outcome.
pub struct InFlightRequest {
    handle: tokio::task::JoinHandle<Result<HttpResponse>>,
}

impl InFlightRequest {
    pub fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.handle.abort_handle()
    }

    pub async fn wait(self) -> Result<HttpResponse> {
        match self.handle.await {
            Ok(result) => result,
            Err(e) if e.is_cancelled() => Err(anyhow::Error::new(RequestCanceled)),
            Err(e) => Err(e.into()),
        }
    }
}
```

Then rename `pub async fn send` to `pub fn start_send` with this signature and shell (the entire existing `async move { ... }` body inside `runtime.spawn(...)` is kept **unchanged**):

```rust
    /// Spawn the request onto the shared tokio runtime and return immediately.
    ///
    /// - `BodyType::Raw` is sent as a raw byte body.
    /// - `BodyType::FormData` is sent as real `multipart/form-data` via
    ///   reqwest's `multipart::Form` (it generates the boundary and the
    ///   `Content-Type` header; file parts are read from disk with their MIME
    ///   guessed from the extension).
    pub fn start_send(
        &self,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: BodyType,
    ) -> InFlightRequest {
        let client = self.client.clone();

        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to initialize tokio runtime")
        });

        let handle = runtime.spawn(async move {
            // ... existing body: build reqwest request, send, collect ...
        });

        InFlightRequest { handle }
    }
```

Key mechanical change at the end: the old code was `runtime.spawn(...).await?` — now it's `let handle = runtime.spawn(...); InFlightRequest { handle }` and the function is **not** `async`.

- [ ] **Step 4: Fix the one caller so the crate compiles**

`src/request_editor.rs:938` currently awaits `client.send(...)`. Minimal bridge (full rework comes in Task 2):

```rust
let response = match client.start_send(method, url.clone(), headers.clone(), body.clone()).wait().await {
```

- [ ] **Step 5: Verify GREEN**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test http_client"`
Expected: PASS — 2 new tests. Then the full suite:
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`
Expected: 85 passed, 0 failed.

- [ ] **Step 6: Commit**

```bash
git add src/http_client.rs src/request_editor.rs
git commit -m "feat(http): cancellable requests — start_send/InFlightRequest with abort"
```

---

### Task 2: RequestEditor — cancel wiring, generation guard, Send⇄Cancel button

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: Add the event and fields**

Below `OpenCodeSnippet` (near line 25):

```rust
/// Event emitted when the user cancels an in-flight request.
#[derive(Clone)]
pub struct RequestCancelled;
```

Next to the existing emitters (line ~997):

```rust
impl EventEmitter<RequestCancelled> for RequestEditor {}
```

In `struct RequestEditor`, after `loading: bool,`:

```rust
    /// Abort handle for the in-flight request (Some only while loading).
    abort_handle: Option<tokio::task::AbortHandle>,
    /// Incremented on every send *and* cancel; spawned tasks capture their
    /// generation and bail out if it no longer matches, so a stale task can
    /// never clobber state owned by a newer send.
    send_generation: u64,
```

In `RequestEditor::new` initializer, after `loading: false,`:

```rust
            abort_handle: None,
            send_generation: 0,
```

- [ ] **Step 2: Rework the send tail (lines ~924-992)**

Replace everything from `self.loading = true;` through the end of the spawned future with:

```rust
        self.send_generation = self.send_generation.wrapping_add(1);
        let generation = self.send_generation;
        self.loading = true;

        log::debug!("Starting {} request to: {}", method.as_str(), url);

        // Spawn the HTTP work onto the tokio runtime *now* so we can hold an
        // abort handle; the gpui task below only awaits the outcome.
        let start = std::time::Instant::now();
        let client = crate::http_client::HttpClient::new();
        let inflight = client.start_send(method, url, headers, body);
        self.abort_handle = Some(inflight.abort_handle());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let response = match inflight.wait().await {
                Ok(r) => r,
                Err(e) => {
                    if e.downcast_ref::<crate::http_client::RequestCanceled>().is_some() {
                        // cancel_request() already reset the UI and bumped the
                        // generation; nothing left to do.
                        return Ok(());
                    }
                    let duration = start.elapsed();
                    let error_message = format!("Request failed: {}", e);
                    log::error!("{}", error_message);

                    let error_response = ResponseData {
                        status: None, // None indicates a network error
                        duration_ms: duration.as_millis() as u64,
                        headers: vec![],
                        body: error_message.into_bytes(),
                        is_text: true,
                    };

                    this.update(cx, |this, cx| {
                        if this.send_generation != generation {
                            return; // superseded by a newer send/cancel
                        }
                        this.loading = false;
                        this.abort_handle = None;
                        cx.emit(RequestCompleted {
                            request,
                            response: std::sync::Arc::new(error_response),
                        });
                        cx.notify();
                    })?;
                    return Ok(());
                }
            };

            let duration = start.elapsed();
            let status = response.status;

            log::debug!("Request completed with status {} in {}ms", status, duration.as_millis());

            let is_text = crate::types::is_text_response(&response.headers, &response.body);
            log::debug!("Response body size: {} bytes (text={})", response.body.len(), is_text);

            let response_data = ResponseData {
                status: Some(status),
                duration_ms: duration.as_millis() as u64,
                headers: response.headers,
                body: response.body,
                is_text,
            };

            this.update(cx, |this, cx| {
                if this.send_generation != generation {
                    return; // superseded by a newer send/cancel
                }
                this.loading = false;
                this.abort_handle = None;
                cx.emit(RequestCompleted {
                    request,
                    response: std::sync::Arc::new(response_data),
                });
                cx.notify();
            })?;

            Ok::<_, anyhow::Error>(())
        })
```

Notes for the implementer:
- `let request = RequestData { method, url: url.clone(), headers: headers.clone(), body: body.clone() };` (existing, ~line 917) stays **above** this block — after it, `url`/`headers`/`body` are moved into `start_send`, no extra clones.
- The old `log::debug!("Sending HTTP request...")` line and the in-closure `HttpClient::new()` disappear.

- [ ] **Step 3: Add cancel_request next to send_request**

```rust
    /// Abort the in-flight request (Send button shows Cancel while loading).
    fn cancel_request(
        &mut self,
        _event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(handle) = self.abort_handle.take() {
            handle.abort();
        }
        // Invalidate the spawned task so its completion can't touch state.
        self.send_generation = self.send_generation.wrapping_add(1);
        self.loading = false;
        cx.emit(RequestCancelled);
        cx.notify();
    }
```

- [ ] **Step 4: Swap the button in render (lines ~1048-1057)**

```rust
                        .child(
                            // Send button - prevent it from shrinking.
                            // While loading it becomes a Cancel button.
                            div().flex_shrink_0().child(if self.loading {
                                Button::new("cancel-btn")
                                    .danger()
                                    .label("Cancel")
                                    .on_click(cx.listener(Self::cancel_request))
                            } else {
                                Button::new("send-btn")
                                    .primary()
                                    .label("Send")
                                    .on_click(cx.listener(Self::send_request))
                            }),
                        ),
```

- [ ] **Step 5: Compile check (WSL) then full tests (Windows)**

Run: `cargo check --tests` — expected: clean.
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`
Expected: 85 passed, 0 failed.

- [ ] **Step 6: Commit**

```bash
git add src/request_editor.rs
git commit -m "feat(editor): Send button becomes Cancel while in flight; generation-guarded completion"
```

---

### Task 3: ResponseViewer — "Request canceled" notice

**Files:**
- Modify: `src/response_viewer.rs`

- [ ] **Step 1: Add the flag**

In `struct ResponseViewer` after `response: Option<Arc<ResponseData>>,`:

```rust
    /// True right after the user cancels a request; shows a notice instead of
    /// the usual empty state. Reset by the next set_response/clear_response.
    canceled: bool,
```

Initialize `canceled: false,` in `new()`. Set `self.canceled = false;` at the top of **both** `set_response` and `clear_response`.

- [ ] **Step 2: Add show_canceled**

```rust
    /// Clear the panel and show a "Request canceled" notice.
    pub fn show_canceled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_response(window, cx);
        self.canceled = true;
        cx.notify();
    }
```

(Ordering matters: `clear_response` resets the flag, so set it **after** the call.)

- [ ] **Step 3: Render the notice**

In `render_status_bar`, else-branch (line ~196): replace `.child("No response yet")` with:

```rust
                .child(if self.canceled { "Request canceled" } else { "No response yet" })
```

In `render` empty state (line ~395): replace `.child("Send a request to see the response here")` with:

```rust
                        .child(if self.canceled {
                            "Request canceled"
                        } else {
                            "Send a request to see the response here"
                        }),
```

Known scope cut (documented, accepted): the canceled notice is not per-tab — switching tabs clears it.

- [ ] **Step 4: Compile check**

Run: `cargo check` — expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/response_viewer.rs
git commit -m "feat(viewer): Request-canceled notice state"
```

---

### Task 4: PoopmanApp — wire the event

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Import the event**

Line 13 becomes:

```rust
use crate::request_editor::{OpenCodeSnippet, RequestCancelled, RequestCompleted, RequestEditor};
```

- [ ] **Step 2: Subscribe (after `open_code_sub`, ~line 182)**

```rust
        // Show the canceled notice when the user aborts an in-flight request.
        // Canceled requests are never written to history (same as Postman).
        let response_viewer_for_cancel = response_viewer.clone();
        let cancel_sub = cx.subscribe_in(
            &request_editor,
            window,
            move |_this, _, _e: &RequestCancelled, window, cx| {
                response_viewer_for_cancel.update(cx, |viewer, cx| {
                    viewer.show_canceled(window, cx);
                });
            },
        );
```

- [ ] **Step 3: Register the subscription**

Add `cancel_sub,` to the `_subscriptions: vec![...]` list (~line 204).

- [ ] **Step 4: Compile check**

Run: `cargo check` — expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): route RequestCancelled to the response viewer"
```

---

### Task 5: Final gates + PR

**Files:** none new

- [ ] **Step 1: Clippy zero warnings (WSL)**

Run: `cargo clippy --all-targets`
Expected: 0 warnings. Fix anything that appears, commit as `style:`.

- [ ] **Step 2: Full test suite (Windows)**

Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`
Expected: `test result: ok. 85 passed; 0 failed`.

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin feat/request-cancel
gh pr create --title "feat: cancel in-flight requests (Send ⇄ Cancel)" --body "..."
```

PR body must include a visual-verification checklist for the user (Windows):
1. Send a request to a slow endpoint (e.g. `https://httpbin.org/delay/10`) → Send becomes red Cancel.
2. Click Cancel → response area shows "Request canceled", button returns to Send.
3. Canceled request does **not** appear in history.
4. Immediately re-send after cancel → new response lands normally (generation guard).
5. Normal fast requests behave exactly as before.

---

## Self-review notes

- Spec coverage: Send⇄Cancel ✅ (Task 2), real tokio abort ✅ (Task 1), generation guard ✅ (Task 2), canceled notice ✅ (Tasks 3-4), no history write ✅ (cancel path never emits `RequestCompleted`, and app.rs only writes history from that event), TDD test for abort→Canceled ✅ (Task 1).
- Type consistency: `RequestCanceled` (http error, one "l") vs `RequestCancelled` (gpui event, two "l"s) — intentional but easy to trip on; both spellings are exact in every snippet.
- `send()` is deleted rather than kept as a wrapper — its only caller is reworked, and an unused method would fail the clippy/dead-code gate.
