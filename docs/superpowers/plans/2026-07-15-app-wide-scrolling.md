# App-Wide Scrolling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the tab bar and response headers scroll, and bring all six scrollable surfaces onto one convention with hover scrollbars.

**Architecture:** A global `theme.scrollbar_show = Hover` gives every scrollbar its visibility policy from one place. Each surface is then rebuilt as three parts — a viewport that owns the size constraint, a scroller that owns content layout and `overflow_scroll`, and a scrollbar that is the scroller's *sibling* inside the viewport, sharing its `ScrollHandle`.

**Tech Stack:** Rust, gpui 0.2.2, gpui-component 0.5.1.

**Spec:** `docs/superpowers/specs/2026-07-15-app-wide-scrolling-design.md`

**Branch:** `feat/app-wide-scrolling` (already checked out; the spec commits are on it).

---

## Background the engineer needs

**This project CANNOT be built or run under WSL2**, which is where you are. `cargo build` and `cargo test` fail on a missing `libxkbcommon`. `cargo check` and `cargo clippy` DO work locally. Tests run on the Windows side, invoked from WSL:

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

**Test baseline is 123 passed, 0 failed.** This change adds no tests and must break none.

**There are no unit tests in this plan, and that is correct, not an oversight.** This is a pure layout/rendering change; there is no extractable pure logic, and the behavior needs a real window. Do not invent tests that assert nothing just to have tests. The automated gate is `cargo check` + `cargo clippy` + the unchanged suite. Correctness rests on the user's manual pass (Task 6).

**You cannot verify any of this works.** Do not claim the scrolling works. You can honestly claim it compiles clean and breaks no tests.

### The two rules everything here follows

**Rule A — the three-part idiom.** Every scrollable surface looks like this:

```rust
div()                              // viewport: owns the size constraint
    .flex_1()
    .min_h_0()
    .child(
        v_flex()                   // scroller: owns content layout and scrolling
            .id("some-scroll-container")
            .size_full()
            .track_scroll(&self.some_handle)
            .overflow_scroll()
            .children(…),
    )
    .vertical_scrollbar(&self.some_handle)   // sibling of the scroller, same handle
```

**The scrollbar goes on the scroller's parent, never on the scroller itself.** `.vertical_scrollbar(&h)` expands to `self.child(ScrollbarLayer{…})`, and that layer renders as `div().absolute().top_0().left_0().right_0().bottom_0()`. Put it inside the scroller and it becomes part of the scrolling content and scrolls away with it. gpui-component places it on the parent at every one of its own call sites (`menu/popup_menu.rs:1309`, `setting/page.rs:189`, `tree.rs:439`).

No `.relative()` is needed on the viewport: gpui defaults `position` to `Position::Relative`, unlike CSS (`gpui-0.2.2/src/style.rs:1193`).

**Do not "simplify" any of this to `.overflow_y_scrollbar()`.** That wrapper moves the element's style onto an outer div and resets the element to a bare one, which relocates `gap`/`padding` away from the content. `history_panel.rs:244` uses it and works — leave it alone — but do not spread it.

**Rule B — `min_h_0` on flex ancestors.** A flex child defaults to `min-height: auto` and refuses to shrink below its content size, so the parent grows to fit instead of constraining the child, and `overflow_scroll` never engages. Every flex ancestor between a scroller and the nearest fixed-height boundary needs `min_h_0()` (`min_w_0()` horizontally). Worked example already in the codebase: `src/body_editor.rs:542`.

**Import paths** (verified; don't guess alternatives):
- `gpui_component::scroll::ScrollableElement` — the trait providing `.vertical_scrollbar(&h)` / `.horizontal_scrollbar(&h)`. Import as `scroll::ScrollableElement as _`. Precedent: `src/history_panel.rs:6`.
- `gpui_component::scroll::ScrollbarShow` — the global policy enum.
- `ScrollHandle` comes from `gpui::*`, already glob-imported in every file you touch.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/theme.rs` | Modify | The single global scrollbar-visibility lever |
| `src/tab_bar.rs` | Modify | Scrolling tab strip + pinned "+" + scroll-active-into-view |
| `src/response_viewer.rs` | Modify | Header list scroll + wrapping values + scrollbar |
| `src/request_editor.rs` | Modify | Headers/params viewports + scrollbars |
| `src/body_editor.rs` | Modify | Form-data viewport + scrollbar |
| `src/environment_manager.rs` | Modify | Two scrollers get handles, viewports, scrollbars |

No new files. `src/history_panel.rs` is deliberately untouched.

---

### Task 1: Global scrollbar policy

Foundational — every later task's scrollbar reads this. Do it first.

**Files:**
- Modify: `src/theme.rs`

- [ ] **Step 1: Add the import**

`src/theme.rs:5` currently reads:

```rust
use gpui_component::{Theme, ThemeMode};
```

Change to:

```rust
use gpui_component::{scroll::ScrollbarShow, Theme, ThemeMode};
```

- [ ] **Step 2: Set the policy**

At the end of `apply_theme` there is already a `// Scrollbar` section:

```rust
    // Scrollbar
    theme.scrollbar_thumb = c(SCROLLBAR);
    theme.scrollbar_thumb_hover = c(MUTED_FG);
```

Replace it with:

```rust
    // Scrollbar. `Hover` rather than the `Scrolling` default: a scrollbar that only
    // appears once you are already scrolling cannot tell you scrolling is possible,
    // which is exactly how a cut-off list reads as broken.
    // Safe to set here — `Theme::sync_scrollbar_appearance` would overwrite it, but
    // neither this app nor `gpui_component::init` calls it.
    theme.scrollbar_show = ScrollbarShow::Hover;
    theme.scrollbar_thumb = c(SCROLLBAR);
    theme.scrollbar_thumb_hover = c(MUTED_FG);
```

- [ ] **Step 3: Verify**

```bash
cargo check && cargo clippy
```

Expected: both clean, no warnings.

- [ ] **Step 4: Commit**

```bash
git add src/theme.rs
git commit -m "feat(theme): show scrollbars on hover app-wide"
```

---

### Task 2: Tab bar — horizontal scroll with a pinned "+"

**Files:**
- Modify: `src/tab_bar.rs`

- [ ] **Step 1: Add the import**

`src/tab_bar.rs:4` currently reads:

```rust
use gpui_component::{h_flex, ActiveTheme as _};
```

Change to:

```rust
use gpui_component::{h_flex, scroll::ScrollableElement as _, ActiveTheme as _};
```

- [ ] **Step 2: Add the scroll handle field**

`src/tab_bar.rs:26-29` currently reads:

```rust
pub struct TabBar {
    tabs: Vec<RequestTab>,
    active_tab_index: usize,
}
```

Change to:

```rust
pub struct TabBar {
    tabs: Vec<RequestTab>,
    active_tab_index: usize,
    scroll_handle: ScrollHandle,
}
```

`src/tab_bar.rs:32-37` currently reads:

```rust
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            tabs: vec![],
            active_tab_index: 0,
        }
    }
```

Change to:

```rust
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            tabs: vec![],
            active_tab_index: 0,
            scroll_handle: ScrollHandle::new(),
        }
    }
```

- [ ] **Step 3: Scroll the active tab into view when it changes**

`src/tab_bar.rs:40-43` currently reads:

```rust
    pub fn update_tabs(&mut self, tabs: Vec<RequestTab>, active_index: usize, _cx: &mut Context<Self>) {
        self.tabs = tabs;
        self.active_tab_index = active_index;
    }
```

Change to:

```rust
    pub fn update_tabs(&mut self, tabs: Vec<RequestTab>, active_index: usize, _cx: &mut Context<Self>) {
        let active_changed = self.active_tab_index != active_index;
        self.tabs = tabs;
        self.active_tab_index = active_index;

        // Ctrl+Tab can select a tab that is scrolled out of view. Indices line up
        // because the tabs are the scroller's direct children and "+" is not —
        // `scroll_to_item` indexes `child_bounds`.
        if active_changed {
            self.scroll_handle.scroll_to_item(active_index);
        }
    }
```

- [ ] **Step 4: Wrap the tab strip in a scrolling viewport**

In `impl Render for TabBar`, `src/tab_bar.rs:75-80` currently reads:

```rust
            .child(
                // Render all tabs
                h_flex()
                    .gap_1()
                    .items_center()
                    .children(self.tabs.iter().enumerate().map(|(index, tab)| {
```

Change to:

```rust
            .child(
                // Viewport for the scrolling tab strip. The "+" button is deliberately
                // outside it (it stays a child of the outer row below), so a full row
                // can never push it off-screen.
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .id("tab-strip")
                            .gap_1()
                            .items_center()
                            .size_full()
                            .track_scroll(&self.scroll_handle)
                            .overflow_x_scroll()
                            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
```

Then find the end of that `.children(...)` closure — currently `src/tab_bar.rs:131-132`:

```rust
                    }))
            )
```

Change to:

```rust
                            })),
                    )
                    .horizontal_scrollbar(&self.scroll_handle),
            )
```

Everything inside the `.children(...)` closure (the per-tab rendering, `src/tab_bar.rs:81-131`) is unchanged — only its indentation shifts. The "+" button block (`src/tab_bar.rs:133-152`) is unchanged and needs no edit: it is already a sibling of this `.child(...)` in the outer `h_flex`, and giving the viewport `flex_1` is what pins it to the right.

- [ ] **Step 5: Verify**

```bash
cargo check && cargo clippy
```

Expected: both clean, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/tab_bar.rs
git commit -m "feat(tabs): scroll the tab strip horizontally, pin the new-tab button"
```

---

### Task 3: Response headers — scroll, wrap, scrollbar

**Files:**
- Modify: `src/response_viewer.rs`

- [ ] **Step 1: Add the import**

`src/response_viewer.rs:3` currently reads:

```rust
use gpui_component::{button::*, h_flex, input::*, v_flex, ActiveTheme as _};
```

Change to:

```rust
use gpui_component::{button::*, h_flex, input::*, scroll::ScrollableElement as _, v_flex, ActiveTheme as _};
```

- [ ] **Step 2: Turn the header list's parent into a proper viewport**

`src/response_viewer.rs:436-446` currently reads:

```rust
                        .when(self.active_tab == 1, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .w_full()
                                    .overflow_hidden()
                                    .child(self.render_headers(cx)),
                            )
                        }),
```

Change to:

```rust
                        .when(self.active_tab == 1, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h_0() // Let the list shrink so its overflow_scroll engages
                                    .w_full()
                                    .overflow_hidden()
                                    .child(self.render_headers(cx))
                                    .vertical_scrollbar(&self.headers_scroll_handle),
                            )
                        }),
```

This div is the viewport; `render_headers` returns the scroller. The scrollbar belongs here, as the scroller's sibling — not inside `render_headers`.

- [ ] **Step 3: Make long values wrap**

`src/response_viewer.rs:253-275` — the `.child(...)` inside the scroller — currently reads:

```rust
                .child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .children(response.headers.iter().map(|(key, value)| {
                            h_flex()
                                .gap_2()
                                .w_full()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_sm()
                                        .flex_shrink_0()
                                        .child(format!("{}:", key)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(value.clone()),
                                )
                        })),
                )
```

Change to:

```rust
                .child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .children(response.headers.iter().map(|(key, value)| {
                            h_flex()
                                .gap_2()
                                .w_full()
                                .items_start()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_sm()
                                        .flex_shrink_0()
                                        .child(format!("{}:", key)),
                                )
                                .child(
                                    // Wraps rather than ellipsizing — reading the whole
                                    // value is the point of looking at headers. min_w_0
                                    // is load-bearing: a value with no break
                                    // opportunities (a JWT, a long set-cookie) otherwise
                                    // has an automatic minimum width of the entire
                                    // string and blows the row out horizontally.
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .min_w_0()
                                        .child(value.clone()),
                                )
                        })),
                )
```

Three changes: the row gains `.items_start()` so the key stays top-aligned against a now-multi-line value; the value div drops `.overflow_hidden()`, `.text_ellipsis()`, `.whitespace_nowrap()`; and it gains `.min_w_0()`.

Leave the scroller itself (`src/response_viewer.rs:244-251`) exactly as it is — it already has `.id()`, `.min_h_0()`, `.track_scroll()`, `.overflow_scroll()`. Do **not** add a scrollbar there; Step 2 put it on the viewport.

- [ ] **Step 4: Verify**

```bash
cargo check && cargo clippy
```

Expected: both clean, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/response_viewer.rs
git commit -m "fix(viewer): scroll the response header list, wrap long values"
```

---

### Task 4: Request headers and params — viewports + scrollbars

Both scrollers are `this.child(v_flex()…)` with no viewport of their own, so each needs one interposed. Do **not** hang the scrollbar on the enclosing `this` — that is the whole tab-content area, so the scrollbar would span more than the list.

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: Add the import**

`src/request_editor.rs:4-7` currently reads:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, input::*,
    select::*, v_flex, ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _,
};
```

Change to:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, input::*,
    scroll::ScrollableElement as _,
    select::*, v_flex, ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _,
};
```

- [ ] **Step 2: Interpose a viewport around the headers list**

`src/request_editor.rs:1197-1206` currently reads:

```rust
                            this.child(
                                // Scrollable headers list
                                v_flex()
                                    .id("headers-scroll-container")
                                    .gap_2()
                                    .p_2()
                                    .pb_4()  // Bottom padding to prevent last row from being obscured
                                    .flex_1()
                                    .track_scroll(&self.headers_scroll_handle)
                                    .overflow_scroll()
                                    .children(self.headers.iter().enumerate().map(
```

Change to:

```rust
                            this.child(
                                // Viewport: owns the size constraint so the list can
                                // shrink and actually scroll; also hosts the scrollbar,
                                // which must be the scroller's sibling rather than its
                                // child (an absolute layer inside the scroller scrolls
                                // away with the content).
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        // Scrollable headers list
                                        v_flex()
                                            .id("headers-scroll-container")
                                            .gap_2()
                                            .p_2()
                                            .pb_4()  // Bottom padding to prevent last row from being obscured
                                            .size_full()
                                            .track_scroll(&self.headers_scroll_handle)
                                            .overflow_scroll()
                                            .children(self.headers.iter().enumerate().map(
```

Note `.flex_1()` moved off the `v_flex` and became `.size_full()` — the viewport owns the flex sizing now.

Then find the end of that `.children(...)` closure and the close of the `v_flex`, and close the new viewport around it, adding the scrollbar:

```rust
                                    )
                                    .vertical_scrollbar(&self.headers_scroll_handle),
                            )
```

- [ ] **Step 3: Interpose a viewport around the params list**

`src/request_editor.rs:1272-1279` currently reads:

```rust
                                v_flex()
                                    .id("params-scroll-container")
                                    .gap_2()
                                    .p_2()
                                    .pb_4()
                                    .flex_1()
                                    .track_scroll(&self.params_scroll_handle)
                                    .overflow_scroll()
                                    .children(self.params.iter().enumerate().map(
```

Change to:

```rust
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        v_flex()
                                            .id("params-scroll-container")
                                            .gap_2()
                                            .p_2()
                                            .pb_4()
                                            .size_full()
                                            .track_scroll(&self.params_scroll_handle)
                                            .overflow_scroll()
                                            .children(self.params.iter().enumerate().map(
```

and close the viewport after the `v_flex` closes:

```rust
                                    )
                                    .vertical_scrollbar(&self.params_scroll_handle),
```

- [ ] **Step 4: Verify**

```bash
cargo check && cargo clippy
```

Expected: both clean, no warnings. Watch for brace/paren mismatches — this task re-nests two long builder chains, and that is where it will go wrong. If the errors are confusing, re-read the whole `.when(...)` block rather than patching braces one at a time.

Both lists keep their `ScrollHandle`: `request_editor.rs:468-495` uses them to auto-scroll to the newly added row. Do not remove the handles.

- [ ] **Step 5: Commit**

```bash
git add src/request_editor.rs
git commit -m "fix(editor): scroll headers and params lists, add scrollbars"
```

---

### Task 5: Form-data — viewport + scrollbar

**Files:**
- Modify: `src/body_editor.rs`

- [ ] **Step 1: Add the import**

`src/body_editor.rs:4-7` currently reads:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::{Input, InputState, InputEvent as InputChangeEvent, TabSize},
    select::*, v_flex, ActiveTheme as _, IndexPath, Sizable as _,
};
```

Change to:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::{Input, InputState, InputEvent as InputChangeEvent, TabSize},
    scroll::ScrollableElement as _,
    select::*, v_flex, ActiveTheme as _, IndexPath, Sizable as _,
};
```

- [ ] **Step 2: Interpose the viewport**

`src/body_editor.rs:657-667` currently reads:

```rust
                this.child(
                    v_flex()
                        .id("formdata-scroll-container")
                        .gap_2()
                        .p_2()
                        .pb_4()  // Bottom padding to prevent last row from being obscured
                        .flex_1()
                        .min_h_0()  // Allow scrolling to work
                        .w_full()
                        .track_scroll(&self.formdata_scroll_handle)
                        .overflow_scroll()
```

Change to:

```rust
                this.child(
                    div()
                        .flex_1()
                        .min_h_0()  // Allow scrolling to work
                        .child(
                            v_flex()
                                .id("formdata-scroll-container")
                                .gap_2()
                                .p_2()
                                .pb_4()  // Bottom padding to prevent last row from being obscured
                                .size_full()
                                .track_scroll(&self.formdata_scroll_handle)
                                .overflow_scroll()
```

and close the viewport after the `v_flex` closes, adding the scrollbar:

```rust
                        )
                        .vertical_scrollbar(&self.formdata_scroll_handle),
                )
```

This surface already scrolled correctly — it had `min_h_0` all along — so it is only gaining the scrollbar and the uniform shape. Keep the handle: `body_editor.rs:364-390` uses it for scroll-to-new-row.

- [ ] **Step 3: Verify**

```bash
cargo check && cargo clippy
```

Expected: both clean, no warnings.

- [ ] **Step 4: Commit**

```bash
git add src/body_editor.rs
git commit -m "fix(body): add a scrollbar to the form-data list"
```

---

### Task 6: Environment manager — handles, viewports, scrollbars

Neither scroller here owns a `ScrollHandle` today, so both need one. `.overflow_y_scrollbar()` is **not** an option: `env-list` carries `.gap_0p5()`, and that wrapper would relocate the gap onto an outer div, silently collapsing the spacing between environment rows.

**Files:**
- Modify: `src/environment_manager.rs`

- [ ] **Step 1: Add the import**

`src/environment_manager.rs:8-10` currently reads:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::*, v_flex, ActiveTheme as _, Sizable as _,
};
```

Change to:

```rust
use gpui_component::{
    button::*, checkbox::Checkbox, h_flex, input::*, scroll::ScrollableElement as _, v_flex,
    ActiveTheme as _, Sizable as _,
};
```

- [ ] **Step 2: Add two scroll handle fields**

`src/environment_manager.rs:27-39` currently reads:

```rust
pub struct EnvironmentManager {
    db: Arc<Database>,
    environments: Vec<Environment>,
    active_id: Option<i64>,
    selected_id: Option<i64>,
    name_input: Entity<InputState>,
    var_rows: Vec<VarRow>,
    /// True while programmatically loading inputs, so their `Change` events don't
    /// trigger an auto-save of values we just set.
    suspend_autosave: bool,
    /// Live input-change subscriptions (name + each var row), rewired on load.
    _subs: Vec<Subscription>,
}
```

Change to:

```rust
pub struct EnvironmentManager {
    db: Arc<Database>,
    environments: Vec<Environment>,
    active_id: Option<i64>,
    selected_id: Option<i64>,
    name_input: Entity<InputState>,
    var_rows: Vec<VarRow>,
    env_list_scroll_handle: ScrollHandle,
    var_list_scroll_handle: ScrollHandle,
    /// True while programmatically loading inputs, so their `Change` events don't
    /// trigger an auto-save of values we just set.
    suspend_autosave: bool,
    /// Live input-change subscriptions (name + each var row), rewired on load.
    _subs: Vec<Subscription>,
}
```

`src/environment_manager.rs:50-59` currently reads:

```rust
        let mut this = Self {
            db,
            environments,
            active_id,
            selected_id,
            name_input,
            var_rows: vec![],
            suspend_autosave: false,
            _subs: vec![],
        };
```

Change to:

```rust
        let mut this = Self {
            db,
            environments,
            active_id,
            selected_id,
            name_input,
            var_rows: vec![],
            env_list_scroll_handle: ScrollHandle::new(),
            var_list_scroll_handle: ScrollHandle::new(),
            suspend_autosave: false,
            _subs: vec![],
        };
```

- [ ] **Step 3: Rebuild the environment list as viewport / scroller / scrollbar**

`src/environment_manager.rs:310-315` currently reads:

```rust
                        v_flex()
                            .id("env-list")
                            .flex_1()
                            .gap_0p5()
                            .overflow_scroll()
                            .children(self.environments.iter().map(|env| {
```

Change to:

```rust
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(
                                v_flex()
                                    .id("env-list")
                                    .size_full()
                                    .gap_0p5()
                                    .track_scroll(&self.env_list_scroll_handle)
                                    .overflow_scroll()
                                    .children(self.environments.iter().map(|env| {
```

and close the viewport after the `v_flex` closes, adding the scrollbar:

```rust
                            )
                            .vertical_scrollbar(&self.env_list_scroll_handle),
```

- [ ] **Step 4: Rebuild the variables list the same way**

`src/environment_manager.rs:438-442` currently reads:

```rust
                            .child(
                                v_flex()
                                    .id("env-vars")
                                    .flex_1()
                                    .overflow_scroll()
                                    .children(self.var_rows.iter().enumerate().map(|(index, row)| {
```

Change to:

```rust
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        v_flex()
                                            .id("env-vars")
                                            .size_full()
                                            .track_scroll(&self.var_list_scroll_handle)
                                            .overflow_scroll()
                                            .children(self.var_rows.iter().enumerate().map(|(index, row)| {
```

and close the viewport after the `v_flex` closes, adding the scrollbar:

```rust
                                    )
                                    .vertical_scrollbar(&self.var_list_scroll_handle),
                            )
```

- [ ] **Step 5: Apply Rule B to this file's ancestor chains**

**This step requires you to read the code. The ancestors are deliberately not listed — they were not traced during design, and writing them from memory would be guessing.**

For each of the two viewports you just created, read outward to the nearest fixed-height boundary (a `.h(px(…))`, a `.max_h(…)`, a sized dialog container, or the window root). Add `.min_h_0()` to every intervening flex ancestor that lacks it. Rule B and its rationale are in the Background section; the worked example is `src/body_editor.rs:542`.

If a chain is ambiguous, report DONE_WITH_CONCERNS describing what you found rather than guessing. A superfluous `min_h_0` is inert; a missing one leaves the surface broken and the manual pass will blame the wrong change.

- [ ] **Step 6: Verify**

```bash
cargo check && cargo clippy
```

Expected: both clean, no warnings.

- [ ] **Step 7: Run the full suite**

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"
```

Expected: `123 passed; 0 failed` — unchanged.

- [ ] **Step 8: Commit**

```bash
git add src/environment_manager.rs
git commit -m "fix(env): scroll the environment and variable lists, add scrollbars"
```

---

### Task 7: User manual verification

**This cannot be signed off from WSL.** Every claim about whether scrolling works is the user's to make. Build a release exe and ask them to walk the checklist.

- [ ] **Step 1: Build the exe on Windows**

Kill any running `poopman.exe` first — it holds a file lock — then:

```bash
pwsh.exe -NoProfile -Command "cd E:\code\poopman; \$env:GPUI_FXC_PATH = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\fxc.exe'; cargo build --release"
```

Expected: `Finished release profile [optimized]`, roughly 2-3 minutes. Binary at `E:\code\poopman\target\release\poopman.exe`.

- [ ] **Step 2: Ask the user to walk the checklist**

Tab bar:
- [ ] Open enough tabs to overflow the row — the strip scrolls horizontally
- [ ] The "+" button stays visible and clickable no matter how many tabs are open
- [ ] A hover scrollbar appears under the tab strip when the mouse is over it
- [ ] Ctrl+Tab to a tab scrolled out of view scrolls it back into view
- [ ] Ctrl+Shift+Tab likewise, in the other direction
- [ ] Clicking a partially visible tab still selects it

Response headers:
- [ ] A response with many headers scrolls vertically
- [ ] A long value with spaces (e.g. `content-security-policy`) wraps and is fully readable
- [ ] A long value with no break opportunities (a JWT in `authorization`, a long `set-cookie`) does not blow the row out horizontally — note what it actually does
- [ ] A hover scrollbar appears when the mouse is over the header list
- [ ] The scrollbar stays put while scrolling (it must not scroll away with the content)
- [ ] Header text can still be selected

Other surfaces:
- [ ] Request headers and params lists scroll, with a hover scrollbar
- [ ] Adding a header/param row still auto-scrolls to the new row (no regression)
- [ ] Form-data rows scroll, with a hover scrollbar; add-row auto-scroll still works
- [ ] Environment manager: both the environment list and the variable list scroll, with hover scrollbars
- [ ] Environment rows still have visible spacing between them (the `gap_0p5` regression check)
- [ ] History panel is unchanged

Global:
- [ ] Scrollbars are invisible until hovered, everywhere
- [ ] Scrolling the response area does not scroll the history panel or vice versa

- [ ] **Step 3: If the lists still don't scroll, STOP**

The `min_h_0` diagnosis is a hypothesis (see the spec's "Risk" section). If the manual pass shows the lists still don't scroll, **do not layer more fixes on it** — return to root-cause investigation with the `superpowers:systematic-debugging` skill.

The tab-bar changes and the header-value wrapping are independent of that hypothesis and should hold regardless; report them separately rather than reverting everything.

---

## Definition of done

- `cargo check` and `cargo clippy` clean under WSL.
- `pwsh.exe -NoProfile -Command "cd E:\code\poopman; cargo test"` still 123 passed, 0 failed.
- User has walked the Task 7 checklist and confirmed.
- Then, and only then, open the PR against `main` for rebase-merge (matching PRs #13–#20).
