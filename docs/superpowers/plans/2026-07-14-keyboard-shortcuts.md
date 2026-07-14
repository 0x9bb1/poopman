# Keyboard Shortcuts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `ctrl-enter` sends the request from anywhere (including while typing in any input), `ctrl-t` opens a new tab, `ctrl-w` closes the current tab.

**Architecture:** First use of gpui's action system in this project. `actions!(poopman, [SendRequest, NewTab, CloseTab])` in app.rs; `cx.bind_keys` in main.rs **after** `gpui_component::init` — gpui gives later-added bindings precedence (keymap.rs: "reverse of the order they were added"), so re-binding `ctrl-enter` in the `"Input"` context shadows the component library's own `secondary-enter` input binding (whose `PressEnter{secondary:true}` event nothing in this app consumes). Handlers live on the app root via `.on_action`, dispatching to the existing `create_new_tab`/`close_tab` and a new public `RequestEditor::send`.

**Tech Stack:** gpui 0.2.2 `actions!`/`KeyBinding`/`on_action`; no new dependencies.

**Testing note:** this PR is pure UI wiring — no unit-testable surface (the TDD gate applies to the existing 101 tests staying green; behavior is verified visually on Windows per project convention).

---

### Task 0: Branch

- [ ] **Step 1:**

```bash
cd /mnt/e/code/poopman && git checkout -b feat/keyboard-shortcuts
```

---

### Task 1: `RequestEditor::send` public entry + double-send guard

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: Split the click handler**

Replace the `send_request` signature block:

```rust
    fn send_request(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
```

with:

```rust
    fn send_request(
        &mut self,
        _event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send(window, cx);
    }

    /// Send the current request. Public so the ctrl-enter action can trigger
    /// it from PoopmanApp; no-op while a request is already in flight (the
    /// button is swapped to Cancel then, but the keyboard path isn't).
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
```

(The rest of the old `send_request` body becomes `send`'s body unchanged.)

- [ ] **Step 2: Compile check**

Run: `cargo check` — clean.

- [ ] **Step 3: Commit**

```bash
git add src/request_editor.rs
git commit -m "refactor(editor): expose send() for keyboard dispatch; guard double-send"
```

---

### Task 2: Actions, bindings, root handlers

**Files:**
- Modify: `src/app.rs` (actions + root `.on_action`s)
- Modify: `src/main.rs` (bind_keys)

- [ ] **Step 1: Define actions in app.rs** (below the `use` block, before `pub struct PoopmanApp`)

```rust
actions!(poopman, [SendRequest, NewTab, CloseTab]);
```

(`actions!` comes via `use gpui::*`, already imported.)

- [ ] **Step 2: Bind keys in main.rs**

After `gpui_component::init(cx);` (order matters — later bindings win) in `main()`:

```rust
        // Late binding on purpose: gpui gives later-added bindings precedence,
        // so the "Input"-context ctrl-enter shadows gpui-component's own
        // secondary-enter input binding (its PressEnter{secondary:true} event
        // is unused in this app).
        cx.bind_keys([
            KeyBinding::new("ctrl-enter", crate::app::SendRequest, None),
            KeyBinding::new("ctrl-enter", crate::app::SendRequest, Some("Input")),
            KeyBinding::new("ctrl-t", crate::app::NewTab, None),
            KeyBinding::new("ctrl-w", crate::app::CloseTab, None),
        ]);
```

- [ ] **Step 3: Handle actions on the app root**

In `impl Render for PoopmanApp` (`src/app.rs:496`), the root element chain starts with:

```rust
        v_flex()
            .size_full()
            .bg(theme.muted)
```

Insert the key context and handlers right after `v_flex()`:

```rust
        v_flex()
            .key_context("Poopman")
            .on_action(cx.listener(|this, _: &SendRequest, window, cx| {
                this.request_editor.update(cx, |editor, cx| editor.send(window, cx));
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.create_new_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let index = this.active_tab_index;
                this.close_tab(index, window, cx);
            }))
            .size_full()
            .bg(theme.muted)
```

Notes: `create_new_tab`/`close_tab` already exist and handle everything (state save, last-tab reset). No pub changes needed — same module.

- [ ] **Step 4: Compile + full suite**

Run: `cargo check` (WSL) — clean.
Run: `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` — 101 passed.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat(app): ctrl-enter send, ctrl-t new tab, ctrl-w close tab"
```

---

### Task 3: Final gates + PR

- [ ] **Step 1:** `cargo clippy --all-targets` — 0 warnings.
- [ ] **Step 2:** Full Windows suite — 101 passed.
- [ ] **Step 3:** Push, open PR `feat: keyboard shortcuts (ctrl-enter send, ctrl-t/-w tabs)` with visual checklist:
  1. Focus the URL input, press Ctrl+Enter → request sends
  2. Focus the body editor, press Ctrl+Enter → request sends (no newline inserted)
  3. Click empty area (no focus), Ctrl+Enter → request sends
  4. Ctrl+Enter while a request is in flight → nothing (no double-send)
  5. Ctrl+T → new tab; Ctrl+W → closes current tab; Ctrl+W on last tab → resets it
  6. Plain Enter in the body editor still inserts a newline

---

## Self-review notes

- Spec coverage: the three approved shortcuts ✅; deferred Ctrl+Tab/Ctrl+L untouched ✅.
- The binding-precedence claim is verified against gpui 0.2.2 source (keymap.rs:117 "reverse of the order they were added"), and shadowing safety against a grep: no `PressEnter` consumer in poopman.
- Risk (accepted, flagged in PR): if action dispatch doesn't reach root handlers in some focus state, visual check item 3 catches it.
