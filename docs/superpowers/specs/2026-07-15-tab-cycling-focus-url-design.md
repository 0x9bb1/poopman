# Spec: Ctrl+Tab tab cycling and Ctrl+L focus-URL shortcuts

Date: 2026-07-15
Status: Approved (design confirmed by user)

## Goal

Ship the two shortcuts deferred from the 2026-07-14 quick-wins round (items B
and C):

- **Ctrl+Tab / Ctrl+Shift+Tab** — cycle the active request tab forward/backward
  in tab-bar order, wrapping at both ends.
- **Ctrl+L** — focus the URL input and select its whole contents.

This completes the keyboard-shortcut surface started in PR #15
(ctrl-enter / ctrl-t / ctrl-w). No other behavior changes.

## Scope decisions

- **Cycling is linear, not MRU.** Ctrl+Tab walks the tab bar left-to-right and
  wraps to the first tab past the last; Ctrl+Shift+Tab is the mirror. This
  matches Postman. MRU (browser/VSCode-style, hold-Ctrl-to-keep-flipping) was
  rejected: it needs an MRU stack plus "Ctrl released" detection, which gpui
  exposes no hook for.
- **Ctrl+L focuses *and* selects all**, like a browser address bar, so the next
  keystroke replaces the URL. Fallback if the select-all dispatch proves
  unreliable in practice: ship focus-only (user-approved retreat).
- **No new persistence, no new UI.** Both shortcuts drive existing state.
- Out of scope: Collections, auth helper tab (the two remaining roadmap blocks).

## Architecture

Follows the established three-step shortcut pattern verbatim.

### 1. Actions (`src/app.rs:21`)

```rust
actions!(poopman, [SendRequest, NewTab, CloseTab, NextTab, PrevTab, FocusUrl]);
```

### 2. Bindings (`src/main.rs:118`)

```rust
KeyBinding::new("ctrl-tab", crate::app::NextTab, None),
KeyBinding::new("ctrl-shift-tab", crate::app::PrevTab, None),
KeyBinding::new("ctrl-l", crate::app::FocusUrl, None),
```

Context is `None` for all three — **no `Some("Input")` shadow binding needed**,
unlike ctrl-enter. Verified against gpui-component 0.5.1 source: it binds
neither `ctrl-tab` nor `ctrl-l` anywhere. The only nearby binding is bare `tab`
→ `IndentInline` (`input/state.rs:127`) and bare `tab` → `Tab` (`root.rs:21`);
gpui matches modifiers exactly, so `ctrl-tab` does not collide with `tab`.
The ctrl-enter shadow binding existed only to out-register gpui-component's own
secondary-enter binding in the `Input` context; there is no equivalent here.

Bindings stay inside the existing late-`bind_keys` call, so the
later-bindings-win ordering note in that comment still holds.

### 3. Handlers (`src/app.rs`, root `v_flex().on_action(...)`)

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

### 4. Index math (`src/app.rs`, free function)

Extracted as a free function so it is unit-testable without a GPUI window:

```rust
/// Next/previous tab index, wrapping at both ends. Returns `current`
/// unchanged when `len` is 0 or 1 (callers then no-op).
fn cycle_index(current: usize, len: usize, forward: bool) -> usize
```

Single-tab and empty cases need no special-casing at the call site:
`switch_to_tab` already early-returns when `index >= len || index ==
self.active_tab_index` (`src/app.rs:320`), and `cycle_index` returns `current`
for `len <= 1`.

### 5. Focus URL (`src/request_editor.rs`)

New public method mirroring the existing `pub fn send()` (made public for
exactly this reason — see its doc comment at `src/request_editor.rs:864`):

```rust
/// Focus the URL input and select its contents. Public so the ctrl-l
/// action can trigger it from PoopmanApp.
pub fn focus_url(&mut self, window: &mut Window, cx: &mut Context<Self>)
```

Implementation:

1. `self.url_input.update(cx, |input, cx| input.focus(window, cx))` —
   `InputState::focus` is `pub` in gpui-component 0.5.1 (`input/state.rs:825`).
2. Dispatch select-all **one frame later** via the codebase's existing
   `cx.spawn_in(window, async move |...| { ... }).detach()` deferral pattern.
   The dispatch itself is `window.dispatch_action(Box::new(input::SelectAll),
   cx)` (`gpui-0.2.2 window.rs:1476`), reached from inside the async block
   through the async window context's `update(...)` — `window` is not directly
   in scope there.

**Why dispatch instead of a direct call:** `InputState::select_all` is
`pub(super)` (`input/state.rs:856`) and unreachable from this crate. The
`SelectAll` *action* is public — `input/mod.rs:29` re-exports it via
`pub use state::*`. Dispatching routes it to the focused input's own handler.

**Why deferred a frame:** gpui computes the action dispatch path from the
last rendered frame's focus, so dispatching in the same tick as `focus()`
can route to the previously focused element instead of the URL input.

## Error handling

No fallible operations. The only edge cases are structural (empty/single tab
list, empty URL string) and are handled by the early-returns described above;
an empty URL selects nothing rather than erroring.

## Testing

- **Unit-testable:** `cycle_index` only — forward wrap past last, backward wrap
  past first, mid-list steps both directions, `len == 1`, `len == 0`.
- **Not unit-testable:** action binding, dispatch routing, and focus/selection
  behavior all need a real window; these are covered by the manual checklist.
- Gotcha (from the 2026-07-14 round): in files that `use gpui::*`, the test
  module must NOT `use super::*` — gpui's `test` attribute macro shadows
  `#[test]`. Import the items under test by name.
- Local gate (WSL2): `cargo check` + `cargo clippy` must be clean.
- Test gate (Windows, from WSL):
  `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"`.

## Manual verification checklist (user, on Windows)

- [ ] Ctrl+Tab past the last tab wraps to the first
- [ ] Ctrl+Shift+Tab past the first tab wraps to the last
- [ ] Both still fire while typing in the URL input and in the body editor
- [ ] Ctrl+Tab with only one tab open does nothing (no flicker, no reload)
- [ ] Ctrl+L from the body editor focuses the URL input and selects the whole URL
- [ ] Typing right after Ctrl+L replaces the URL rather than appending
- [ ] Ctrl+L with an empty URL does not crash
- [ ] Tab-switching still saves/restores per-tab state (no regression in
      `switch_to_tab`'s save-then-load path)
