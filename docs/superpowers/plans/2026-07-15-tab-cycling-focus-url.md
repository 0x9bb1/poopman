# Tab Cycling & Focus-URL Shortcuts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Ctrl+Tab / Ctrl+Shift+Tab (linear tab cycling with wrap-around) and Ctrl+L (focus the URL input and select all its text).

**Architecture:** Follows the shortcut pattern already established by ctrl-enter/ctrl-t/ctrl-w in three steps: declare a gpui action in `src/app.rs`, bind a keystroke to it in `src/main.rs`, handle it with `.on_action` on `PoopmanApp`'s root element. Tab index math is extracted into a pure free function so it can be unit-tested without a GPUI window. Ctrl+L delegates to a new `pub fn focus_url` on `RequestEditor`, mirroring the existing `pub fn send`.

**Tech Stack:** Rust, gpui 0.2.2, gpui-component 0.5.1.

**Spec:** `docs/superpowers/specs/2026-07-15-tab-cycling-focus-url-design.md`

**Branch:** `feat/keyboard-shortcuts-cycling` (already checked out; the spec commit is on it).

---

## Background the engineer needs

**This project cannot be built or run under WSL2.** `cargo check` and `cargo clippy` work locally; `cargo build`/`cargo test` fail on a missing `libxkbcommon`. Tests run on the Windows side, invoked from WSL:

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

That command is the test gate. Use it wherever this plan says "run the tests".

**Critical gotcha — test modules in files that `use gpui::*`:** `src/app.rs` starts with `use gpui::*;`. gpui exports a `test` attribute macro that **shadows the standard `#[test]`**. A test module in such a file must NOT `use super::*`; import the items under test by name instead. `src/response_viewer.rs:467` is the working precedent — copy its shape.

**GUI behavior cannot be verified from WSL.** Tasks 1–2 are unit-testable. Tasks 3–5 change GUI wiring and are verified by `cargo check` + `cargo clippy` plus the user's manual checklist at the end.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/app.rs` | Modify | Declare the three new actions; add `cycle_index` free function + its tests; add three `.on_action` handlers |
| `src/main.rs` | Modify | Bind `ctrl-tab` / `ctrl-shift-tab` / `ctrl-l` to those actions |
| `src/request_editor.rs` | Modify | New `pub fn focus_url` — focus URL input + dispatch select-all |

No new files. All three changes are small and belong with their existing owners.

---

### Task 1: `cycle_index` pure function + tests

The only unit-testable logic in this feature. Do it first, TDD.

**Files:**
- Modify: `src/app.rs` (add function near the bottom, before any `#[cfg(test)]` block)
- Test: `src/app.rs` (new `#[cfg(test)] mod tests` at end of file)

- [ ] **Step 1: Write the failing test**

Append to the very end of `src/app.rs`:

```rust
#[cfg(test)]
mod tests {
    // NOT `use super::*`: that would pull in `gpui::*`, whose `test` attribute
    // macro shadows the standard `#[test]`.
    use super::cycle_index;

    #[test]
    fn steps_forward_through_the_middle_of_the_list() {
        assert_eq!(cycle_index(0, 3, true), 1);
        assert_eq!(cycle_index(1, 3, true), 2);
    }

    #[test]
    fn wraps_forward_past_the_last_tab() {
        assert_eq!(cycle_index(2, 3, true), 0);
    }

    #[test]
    fn steps_backward_through_the_middle_of_the_list() {
        assert_eq!(cycle_index(2, 3, false), 1);
        assert_eq!(cycle_index(1, 3, false), 0);
    }

    #[test]
    fn wraps_backward_past_the_first_tab() {
        assert_eq!(cycle_index(0, 3, false), 2);
    }

    #[test]
    fn single_tab_stays_put_in_both_directions() {
        assert_eq!(cycle_index(0, 1, true), 0);
        assert_eq!(cycle_index(0, 1, false), 0);
    }

    #[test]
    fn empty_list_returns_current_without_panicking() {
        assert_eq!(cycle_index(0, 0, true), 0);
        assert_eq!(cycle_index(0, 0, false), 0);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test cycle_index"
```

Expected: compile error — `cannot find function 'cycle_index' in module 'super'`.

- [ ] **Step 3: Write the implementation**

Add to `src/app.rs`, immediately after the `impl Render for PoopmanApp { ... }` block (top-level, not inside an impl):

```rust
/// Next (`forward`) or previous tab index, wrapping at both ends.
///
/// Returns `current` unchanged when `len` is 0 or 1 — callers then hit
/// `switch_to_tab`'s `index == self.active_tab_index` early-return and no-op.
fn cycle_index(current: usize, len: usize, forward: bool) -> usize {
    if len <= 1 {
        return current;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}
```

Note the backward case adds `len - 1` rather than subtracting 1: `usize` subtraction underflows and panics when `current == 0`.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test cycle_index"
```

Expected: `test result: ok. 6 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(tabs): cycle_index helper for wrap-around tab navigation"
```

---

### Task 2: Declare the actions

**Files:**
- Modify: `src/app.rs:21`

- [ ] **Step 1: Extend the actions! macro**

Replace line 21 of `src/app.rs`:

```rust
actions!(poopman, [SendRequest, NewTab, CloseTab]);
```

with:

```rust
actions!(poopman, [SendRequest, NewTab, CloseTab, NextTab, PrevTab, FocusUrl]);
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check
```

Expected: success. Warnings about `NextTab`/`PrevTab`/`FocusUrl` being unused are expected at this point — the handlers land in Task 4.

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat(actions): declare NextTab, PrevTab, FocusUrl actions"
```

---

### Task 3: `RequestEditor::focus_url`

**Files:**
- Modify: `src/request_editor.rs` (add the method inside `impl RequestEditor`, next to `pub fn send` at ~line 864)

No unit test: this needs a live `Window` and a rendered frame. It is covered by the manual checklist.

- [ ] **Step 1: Write the method**

Add inside `impl RequestEditor`, directly above `pub fn send`:

```rust
/// Focus the URL input and select all of its text. Public so the ctrl-l
/// action can trigger it from PoopmanApp.
///
/// Select-all goes through action dispatch because `InputState::select_all`
/// is `pub(super)` in gpui-component and unreachable from this crate; the
/// `SelectAll` action itself is public. The dispatch must wait for the next
/// frame: `Window::dispatch_action` resolves its target from the *last
/// rendered* frame's focus, so dispatching in this same tick would route to
/// whatever was focused before. `request_animation_frame` guarantees that
/// frame happens even when the URL input already had focus — `Window::focus`
/// early-returns without scheduling a redraw in that case, which would
/// otherwise strand the callback and make a second Ctrl+L do nothing.
pub fn focus_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.url_input.update(cx, |input, cx| input.focus(window, cx));
    window.request_animation_frame();
    window.on_next_frame(|window, cx| {
        window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
    });
}
```

`SelectAll` is spelled fully-qualified on purpose. `request_editor.rs` already globs `gpui_component::input::*`, so the bare name would resolve, but the file also globs `gpui::*` — writing the full path removes any doubt for the reader. No new `use` line is needed.

- [ ] **Step 2: Verify it compiles and is clean**

```bash
cargo check && cargo clippy
```

Expected: both succeed. A dead-code warning for `focus_url` is expected here — the caller lands in Task 4.

- [ ] **Step 3: Commit**

```bash
git add src/request_editor.rs
git commit -m "feat(editor): expose focus_url() for keyboard dispatch"
```

---

### Task 4: Wire the action handlers

**Files:**
- Modify: `src/app.rs` (the `.on_action` chain in `impl Render for PoopmanApp`, ~line 503-513)

- [ ] **Step 1: Add the three handlers**

In `src/app.rs`, find the existing chain that starts at `.key_context("Poopman")`. After the existing `CloseTab` handler:

```rust
.on_action(cx.listener(|this, _: &CloseTab, window, cx| {
    let index = this.active_tab_index;
    this.close_tab(index, window, cx);
}))
```

add:

```rust
.on_action(cx.listener(|this, _: &NextTab, window, cx| {
    let next = cycle_index(this.active_tab_index, this.request_tabs.len(), true);
    this.switch_to_tab(next, window, cx);
}))
.on_action(cx.listener(|this, _: &PrevTab, window, cx| {
    let prev = cycle_index(this.active_tab_index, this.request_tabs.len(), false);
    this.switch_to_tab(prev, window, cx);
}))
.on_action(cx.listener(|this, _: &FocusUrl, window, cx| {
    this.request_editor.update(cx, |editor, cx| editor.focus_url(window, cx));
}))
```

No bounds-checking needed at the call site: `cycle_index` returns `current` for `len <= 1`, and `switch_to_tab` (`src/app.rs:320`) already early-returns on `index >= self.request_tabs.len() || index == self.active_tab_index`.

- [ ] **Step 2: Verify it compiles and is clean**

```bash
cargo check && cargo clippy
```

Expected: both succeed, and the dead-code warnings from Tasks 2 and 3 are now gone.

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): handle NextTab, PrevTab and FocusUrl actions"
```

---

### Task 5: Bind the keystrokes

**Files:**
- Modify: `src/main.rs:118-123`

- [ ] **Step 1: Add the bindings**

In `src/main.rs`, extend the existing `cx.bind_keys([...])` call so it reads:

```rust
cx.bind_keys([
    KeyBinding::new("ctrl-enter", crate::app::SendRequest, None),
    KeyBinding::new("ctrl-enter", crate::app::SendRequest, Some("Input")),
    KeyBinding::new("ctrl-t", crate::app::NewTab, None),
    KeyBinding::new("ctrl-w", crate::app::CloseTab, None),
    KeyBinding::new("ctrl-tab", crate::app::NextTab, None),
    KeyBinding::new("ctrl-shift-tab", crate::app::PrevTab, None),
    KeyBinding::new("ctrl-l", crate::app::FocusUrl, None),
]);
```

Keep them inside this same call — the existing comment above it explains that binding late lets these win over gpui-component's own bindings, and that reasoning still applies.

Context is `None` for all three new bindings; do **not** add `Some("Input")` shadow bindings. That was needed only for ctrl-enter, to out-register gpui-component's secondary-enter binding. gpui-component 0.5.1 binds neither `ctrl-tab` nor `ctrl-l` anywhere. Its bare `tab` → `IndentInline` binding (`input/state.rs:127`) does not collide, because gpui matches modifiers exactly.

- [ ] **Step 2: Verify the whole thing compiles and is clean**

```bash
cargo check && cargo clippy
```

Expected: both succeed with no warnings.

- [ ] **Step 3: Run the full test suite**

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

Expected: all tests pass, including the 6 new `cycle_index` tests.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(keys): bind ctrl-tab, ctrl-shift-tab and ctrl-l"
```

---

### Task 6: User manual verification

**This feature cannot be signed off from WSL.** Ask the user to build and run on Windows and walk the checklist. Do not claim the feature works before they report back.

- [ ] **Step 1: Ask the user to run through the checklist**

From the spec's manual verification section:

- [ ] Ctrl+Tab past the last tab wraps to the first
- [ ] Ctrl+Shift+Tab past the first tab wraps to the last
- [ ] Both still fire while typing in the URL input and in the body editor
- [ ] Ctrl+Tab with only one tab open does nothing (no flicker, no reload)
- [ ] Ctrl+L from the body editor focuses the URL input and selects the whole URL
- [ ] Typing right after Ctrl+L replaces the URL rather than appending
- [ ] Ctrl+L pressed twice in a row (URL input already focused) still re-selects
- [ ] Ctrl+L with an empty URL does not crash
- [ ] Tab-switching still saves/restores per-tab state (no regression in `switch_to_tab`)

Build note for the user: run via `pwsh.exe` with `GPUI_FXC_PATH` set to the SDK's `fxc.exe`, and kill any running `poopman.exe` first (it holds a file lock).

- [ ] **Step 2: If select-all misbehaves, fall back to focus-only**

The user pre-approved this retreat. If the `SelectAll` dispatch proves unreliable, cut it down to:

```rust
pub fn focus_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.url_input.update(cx, |input, cx| input.focus(window, cx));
}
```

and update the spec's scope decision to record that focus-only shipped.

---

## Definition of done

- `cargo check` and `cargo clippy` clean under WSL.
- `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` fully green.
- User has walked the Task 6 checklist and confirmed.
- Then, and only then, open the PR against `main` for rebase-merge (matching PRs #13–#19).
