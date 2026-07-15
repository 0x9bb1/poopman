# Spec: app-wide scrolling convention + tab-bar and response-header scroll fixes

Date: 2026-07-15
Status: Approved (design confirmed by user)

## Goal

Two reported defects, plus the app-wide convention that prevents them recurring:

1. **Response headers** — long values are truncated with no way to read them, and the
   header list does not scroll.
2. **Tab bar** — once tabs fill the row they overflow with no way to reach them, and
   the "+" button gets pushed off-screen with them.

The user explicitly asked for the whole app's scrolling to be considered rather than
two isolated patches, because the root cause is an absent convention: three different
scroll idioms coexist today and `min_h_0` is applied inconsistently.

## Audit (the fact base)

**Six surfaces, not eight.** The response body (`response_viewer.rs:365`) and the code
snippet panel (`code_snippet_panel.rs:149`) both render `Input::new(&self.body_display)`
— a gpui-component editor that owns its internal scrolling. They are out of scope.

| Surface | Current state | Verdict |
|---|---|---|
| Tab bar (`tab_bar.rs:70-152`) | No scroll at all; container has no `id`; "+" is a sibling of the tab strip | ✗ defect 2 |
| Response headers (`response_viewer.rs:242-276`) | Self has `overflow_scroll` + `min_h_0`; **parent (`:437-444`) lacks `min_h_0`**; values are `text_ellipsis` + `whitespace_nowrap` | ✗ defect 1 — two independent problems (vertical + horizontal) |
| Request headers (`request_editor.rs:1203-1206`) | `track_scroll` + `overflow_scroll`, **no `min_h_0`** | ✗ same suspected cause |
| Request params (`request_editor.rs:1278`) | Same as above | ✗ same suspected cause |
| Form-data (`body_editor.rs:659-667`) | Has `min_h_0`; scrolls | ~ missing scrollbar only |
| Environment manager (`environment_manager.rs:314, 442`) | `overflow_scroll` | ~ suspected + missing scrollbar |
| History (`history_panel.rs:244`) | `overflow_y_scrollbar` | ✓ the only surface written correctly |

The `min_h_0` lesson was already learned once and left un-generalized:
`body_editor.rs:542` carries the comment `.min_h_0()  // Critical for scrolling to work
in form-data`.

## Scope decisions

- **Tab overflow: horizontal scroll with a pinned "+".** Tab widths stay fixed; the
  strip scrolls; "+" moves outside the scroll area and stays at the right. Chrome-style
  shrink-to-fit was rejected (titles become unreadable fast, and the flex-shrink logic
  is fussier); an overflow dropdown was rejected as scope creep.
- **Scrollbars appear on hover**, set once globally rather than per-call-site.
- **Long header values wrap** rather than scrolling horizontally or hiding behind a
  tooltip. Reading the value is the whole point of looking at headers; a tooltip can't
  be selected or copied.
- **All surfaces change in one batch, verified in one manual pass** (user's call). See
  "Risk" below for why this is safe here.
- Out of scope: response body and code snippet panel (Input-internal scrolling);
  Collections; auth helper tab.

## The convention (three rules)

**Rule 1 — Scrollbar visibility is global.** `src/theme.rs`'s `apply_theme` sets:

```rust
theme.scrollbar_show = ScrollbarShow::Hover;
```

Every `Scrollbar` reads `cx.theme().scrollbar_show`, so this is the single lever.
Verified safe: `Theme::sync_scrollbar_appearance` (`theme/mod.rs:141`) would overwrite
this, but neither poopman nor `gpui_component::init` (`lib.rs:97-109`) calls it.
`Hover` is also what gpui-component itself picks for systems not set to auto-hide.

**Rule 2 — Pick the API by whether you need programmatic scroll control.**

- Need to drive the scroll position (scroll-to-bottom on row add, scroll-into-view):
  own a `ScrollHandle`, then `.track_scroll(&h).overflow_scroll()` plus
  `.vertical_scrollbar(&h)` / `.horizontal_scrollbar(&h)`.
- Don't need it: `.overflow_y_scrollbar()` / `.overflow_x_scrollbar()`, which wrap the
  element in `Scrollable` and manage their own scroll state via
  `ElementId::CodeLocation`.

Both come from `gpui_component::scroll::ScrollableElement`, already imported in
`history_panel.rs:6`.

**Rule 3 — `min_h_0` on every flex ancestor.** From the scroll container up to the
nearest fixed-height boundary, every intervening flex ancestor needs `min_h_0()`
(`min_w_0()` for horizontal). A flex child defaults to `min-height: auto`, which
refuses to shrink below content size — the parent grows to fit instead of constraining
the child, and the child's `overflow_scroll` never engages.

## Changes by surface

### Tab bar (`src/tab_bar.rs`)

Split the single `h_flex` into a scrolling strip plus a pinned button:

- The tab strip becomes the scroll container: `.id("tab-strip")`,
  `.track_scroll(&self.scroll_handle)`, `.overflow_x_scroll()`,
  `.horizontal_scrollbar(&self.scroll_handle)`, `.min_w_0()`.
- The "+" button moves **out** of the strip and becomes its sibling, so it can never be
  pushed off-screen. This is the actual fix for "点满了" — today "+" scrolls away with
  the tabs.
- `TabBar` gains a `scroll_handle: ScrollHandle` field.
- `update_tabs` calls `self.scroll_handle.scroll_to_item(active_index)` when
  `active_index` changes, so Ctrl+Tab to an off-screen tab brings it into view.

`scroll_to_item` genuinely handles the horizontal axis — verified at
`gpui-0.2.2/src/elements/div.rs`, where `scroll_to_active_item`'s
`if state.overflow.x == Overflow::Scroll` branch sits *outside* the `strategy` match and
runs unconditionally, performing a minimal scroll to make the item fully visible.

**Constraint this imposes:** `scroll_to_item(ix)` indexes `child_bounds`, i.e. the
scroll container's **direct children**. Tabs must therefore be direct children of the
strip, and "+" must not be. Both hold under this design, so the index equals the tab
index exactly.

No change needed in `src/app.rs`: `:589` already wraps the tab bar in
`div().flex_1().min_w_0()`, so the horizontal constraint is in place.

### Response headers (`src/response_viewer.rs`)

- Parent container (`:437-444`) gains `.min_h_0()`.
- The value `div` (`:266-273`) drops `.whitespace_nowrap()`, `.text_ellipsis()`, and
  `.overflow_hidden()` so long values wrap, and **gains `.min_w_0()`**.

  The `min_w_0()` is Rule 3's horizontal counterpart and is load-bearing, not
  decoration. The value div is a `flex_1` child of an `h_flex` row. Dropping
  `whitespace_nowrap` alone makes values with spaces (`content-security-policy`) wrap,
  but a value with no break opportunities — a JWT in `authorization`, a long
  `set-cookie` — has an automatic minimum width equal to the whole string, so the row
  would blow out horizontally instead. `min_w_0()` lets it shrink.

  A single unbreakable token longer than the pane still has nowhere to wrap; the
  checklist covers observing what actually happens rather than guessing.
- `render_headers` gains `.vertical_scrollbar(&self.headers_scroll_handle)`. It already
  owns the handle (`:63`), so Rule 2's first branch applies.

### Request headers and params (`src/request_editor.rs`)

Both scroll containers (`:1203-1206`, `:1278`) gain `.min_h_0()` and a
`.vertical_scrollbar(&…_scroll_handle)`. Both already own handles for the existing
scroll-to-bottom-on-add behavior, which must keep working.

### Form-data (`src/body_editor.rs`) and environment manager (`src/environment_manager.rs`)

Form-data (`:659-667`) already scrolls correctly; it gains only
`.vertical_scrollbar(&self.formdata_scroll_handle)`.

Environment manager (`:314`, `:442`) has no handle and needs no programmatic control, so
it takes Rule 2's second branch: `.overflow_scroll()` → `.overflow_y_scrollbar()`.

Rule 3 also applies to both containers, but the exact ancestors are **not enumerated
here** — unlike the other surfaces, this file's chains were not traced during design.
The implementer must read outward from each of `:314` and `:442` to the nearest
fixed-height boundary and add `min_h_0()` to each intervening flex ancestor that lacks
it. Enumerating them here from memory would be guessing.

### History (`src/history_panel.rs`)

No change. Already conforms.

## Risk: the `min_h_0` diagnosis is a hypothesis

The only established facts are that the user reports these surfaces don't scroll, and
that the codebase already fixed this exact symptom with `min_h_0` elsewhere. Whether
gpui's taffy layout implements CSS's automatic-minimum-size rule strictly enough for
this to be *the* cause has **not** been verified — it cannot be, without a GUI build.

Batching all surfaces before verifying is nonetheless safe here, and this is the reason:
**`min_h_0` is inert if the hypothesis is wrong.** It relaxes a minimum constraint; it
cannot break a layout that was working. This is categorically unlike the
`request_animation_frame` mistake in the 2026-07-15 shortcuts round, where the
speculative fix actively introduced a panic — that is what made incremental verification
mandatory there and optional here.

**If manual verification shows the surfaces still don't scroll:** stop. Return to root-
cause investigation rather than layering more fixes on a disproven hypothesis. The tab
bar and header-wrapping changes are independent of this hypothesis and should still hold.

## Error handling

No fallible operations. No new state beyond `TabBar`'s `ScrollHandle`, which is
infallible to construct (`ScrollHandle::new()`).

## Testing

This is a pure GUI/layout change. Unlike the previous round's `cycle_index`, there is no
extractable pure logic worth unit-testing — the entire behavior is layout and rendering,
which needs a real window.

- Local gate (WSL2): `cargo check` and `cargo clippy` must be clean.
- Test gate (Windows, from WSL):
  `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` — must stay at 123
  passed, 0 failed (this change adds no tests and must break none).
- Correctness rests entirely on the manual checklist below. Do not claim the feature
  works before the user confirms.

## Manual verification checklist (user, on Windows)

Tab bar:
- [ ] Open enough tabs to overflow the row — the strip scrolls horizontally
- [ ] The "+" button stays visible and clickable no matter how many tabs are open
- [ ] A hover scrollbar appears under the tab strip when the mouse is over it
- [ ] Ctrl+Tab to a tab scrolled out of view scrolls it back into view
- [ ] Ctrl+Shift+Tab likewise, in the other direction
- [ ] Clicking a partially visible tab still selects it (no misrouted hit-testing)

Response headers:
- [ ] A response with many headers scrolls vertically
- [ ] A long value with spaces (e.g. `content-security-policy`) wraps and is fully readable
- [ ] A long value with **no** break opportunities (e.g. a JWT in `authorization`, a long
      `set-cookie`) does not blow the row out horizontally — this is the `min_w_0()` case;
      note what it actually does, since a single unbreakable token has nowhere to wrap
- [ ] A hover scrollbar appears when the mouse is over the header list
- [ ] Header text can still be selected

Other surfaces:
- [ ] Request headers and params lists scroll, with a hover scrollbar
- [ ] Adding a header/param row still auto-scrolls to the new row (no regression)
- [ ] Form-data rows scroll, with a hover scrollbar; add-row auto-scroll still works
- [ ] Environment manager variable list scrolls, with a hover scrollbar
- [ ] History panel is unchanged

Global:
- [ ] Scrollbars are invisible until hovered, everywhere
- [ ] Scrolling the response area does not scroll the history panel or vice versa
      (`app.rs:566, 583` isolate scroll events — no regression)
