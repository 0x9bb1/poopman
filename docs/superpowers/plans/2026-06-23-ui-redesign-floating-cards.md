# Poopman UI Redesign (Floating Cards + Edit Menu + Env Dialog) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reimplement Poopman's GPUI UI to match the Figma "floating-card" design, move environment switching into an Edit menu in the title bar, and beautify the environment dialog.

**Architecture:** Add two small modules — `src/ui.rs` (shared `card_panel` + segmented-pill-tab helpers) and `src/menu_bar.rs` (the Edit dropdown menu) — then restyle each panel in place to consume the helpers. Environment switching becomes a `PopupMenu` built from a captured `Entity<PoopmanApp>` handle whose item `on_click` closures call back into the app. The existing warm coral theme is retained.

**Tech Stack:** Rust, gpui 0.2.2, gpui-component 0.5.x (`Button::dropdown_menu`, `PopupMenu`, `TitleBar`, `Dialog`, `resizable`), rusqlite.

**Verification note (important):** This is a GPU app that **cannot run in WSL2** and the styling cannot be meaningfully unit-tested headlessly. The automated gate for every task is therefore `cargo check` (compiles fine without a GPU). Final visual confirmation is a manual step performed by the user on Windows/macOS via `cargo run` (Task 9). Each task still ends with a compile gate + commit.

**Conventions:** Match existing code style. Commit messages use `type(scope): Subject` and end with:
`Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`

---

## File Structure

- **Create `src/ui.rs`** — shared, callback-free styling helpers: `card_panel`, `segmented_bar`, `segment_pill`. One responsibility: visual primitives reused across panels.
- **Create `src/menu_bar.rs`** — `edit_menu(...)`: builds the Edit `Button` + `dropdown_menu` (environment list + "Manage Environments…"). One responsibility: the title-bar menu.
- **Modify `src/main.rs`** — register the two new modules.
- **Modify `src/app.rs`** — floating-card layout; embed Edit menu in `TitleBar`; remove the old Env toolbar button; add `set_active_environment`; restyle the env dialog wrapper (title + subtitle).
- **Modify `src/tab_bar.rs`** — pill-style request tabs.
- **Modify `src/response_viewer.rs`** — segmented pill tabs + status pill.
- **Modify `src/request_editor.rs`** — segmented pill tabs.
- **Modify `src/history_panel.rs`** — keep current card rows (already close); minor radius/border polish only.
- **Modify `src/environment_manager.rs`** — dialog redesign: dashed "New environment" button, dot-activation, remove "Set active" button, card-style variable table, bottom Save footer.

---

## Task 1: Shared UI helpers (`src/ui.rs`)

**Files:**
- Create: `src/ui.rs`
- Modify: `src/main.rs:3-17` (module list)

- [ ] **Step 1: Create `src/ui.rs`**

```rust
//! Shared visual primitives for the floating-card UI: panel cards and
//! segmented pill tab strips. All helpers are callback-free — callers attach
//! `.id(...)`/`.on_click(...)` themselves so the helpers stay generic.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{h_flex, theme::Theme};

/// A floating panel card: white-ish surface, hairline border, large radius,
/// soft shadow, clipped contents. Wrap a panel's content in this.
pub fn card_panel(theme: &Theme) -> Div {
    div()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius_lg)
        .shadow_sm()
        .overflow_hidden()
}

/// The container for a segmented pill tab strip (muted rounded track).
pub fn segmented_bar(theme: &Theme) -> Div {
    h_flex()
        .gap_1()
        .p_0p5()
        .rounded(theme.radius_lg)
        .bg(theme.muted)
}

/// A single segment pill. Caller adds `.id(...)`, `.on_click(...)`, `.child(label)`.
/// Active pills sit on the card surface with a soft shadow; inactive are muted.
pub fn segment_pill(theme: &Theme, active: bool) -> Div {
    div()
        .px_3()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(theme.background)
                .text_color(theme.foreground)
                .font_weight(FontWeight::SEMIBOLD)
                .shadow_sm()
        })
        .when(!active, |d| d.text_color(theme.muted_foreground))
}
```

- [ ] **Step 2: Register the module in `src/main.rs`**

In the `mod` block (currently `src/main.rs:3-17`), add `mod ui;` in alphabetical position (after `mod tab_bar;` / before `mod theme;`):

```rust
mod tab_bar;
mod theme;
mod types;
mod ui;
```

(Keep the other `mod` lines unchanged.)

- [ ] **Step 3: Compile gate**

Run: `cargo check`
Expected: PASS (warnings about unused helpers are fine at this stage).

- [ ] **Step 4: Commit**

```bash
git add src/ui.rs src/main.rs
git commit -m "feat(ui): Add shared card_panel + segmented pill tab helpers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Edit menu in the title bar (`src/menu_bar.rs`, `src/app.rs`)

This adds the functional Edit menu (environment list + manage entry) and removes the old Env toolbar button. After this task, environment switching happens through the menu.

**Files:**
- Create: `src/menu_bar.rs`
- Modify: `src/main.rs` (add `mod menu_bar;`)
- Modify: `src/app.rs` — add `set_active_environment`; embed menu in `TitleBar`; delete the old env selector button block (`src/app.rs:459-485`).

- [ ] **Step 1: Add `set_active_environment` to `PoopmanApp`**

In `src/app.rs`, directly after `open_env_manager` (ends at `src/app.rs:212`), add:

```rust
    /// Switch the active environment (or clear it) from the Edit menu, then
    /// reload + refresh the request editor's variable map.
    fn set_active_environment(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        if let Err(e) = self.db.set_active_environment_id(id) {
            log::error!("Failed to set active environment: {}", e);
            return;
        }
        self.reload_environments(cx);
    }
```

(`reload_environments` already calls `cx.notify()` and pushes vars to the editor.)

- [ ] **Step 2: Create `src/menu_bar.rs`**

```rust
//! The Edit menu shown in the title bar. Houses environment switching (with a
//! check mark on the active one) and an entry to open the environment dialog.
//! Item handlers call back into `PoopmanApp` via a captured entity handle.

use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    Sizable as _,
};

use crate::app::PoopmanApp;
use crate::types::Environment;

/// Build the "Edit" dropdown button for the title bar.
///
/// `environments` / `active_id` are a snapshot taken in `PoopmanApp::render`;
/// `app` is the app's own entity handle used to dispatch actions from menu items.
pub fn edit_menu(
    app: Entity<PoopmanApp>,
    environments: Vec<Environment>,
    active_id: Option<i64>,
) -> impl IntoElement {
    Button::new("edit-menu")
        .ghost()
        .small()
        .label("Edit")
        .dropdown_menu(move |mut menu, _window, _cx| {
            menu = menu.label("Environment");

            // One checkable item per environment.
            for env in &environments {
                let id = env.id;
                let app = app.clone();
                let is_active = active_id == Some(id);
                menu = menu.item(
                    PopupMenuItem::new(env.name.clone())
                        .checked(is_active)
                        .on_click(move |_, _window, cx| {
                            app.update(cx, |app, cx| {
                                app.set_active_environment(Some(id), cx);
                            });
                        }),
                );
            }

            // Clear-active entry.
            {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new("No Environment")
                        .checked(active_id.is_none())
                        .on_click(move |_, _window, cx| {
                            app.update(cx, |app, cx| {
                                app.set_active_environment(None, cx);
                            });
                        }),
                );
            }

            menu = menu.separator();

            // Open the management dialog.
            {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new("Manage Environments…").on_click(
                        move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.open_env_manager(window, cx);
                            });
                        },
                    ),
                );
            }

            menu
        })
}
```

NOTE on imports: confirm the exact public paths while implementing — they are re-exported from `gpui_component`. If `menu::DropdownMenu` / `menu::PopupMenuItem` don't resolve, check `gpui_component::{DropdownMenu, PopupMenuItem}` (top-level re-exports) by grepping `gpui_component-0.5.1/src/lib.rs` for `DropdownMenu` and `PopupMenuItem`. `Button` ghost/small come from `button::ButtonVariants` and `Sizable` (already used in `app.rs`). Also make `PoopmanApp::open_env_manager` and `set_active_environment` callable from this module: change both from `fn` to `pub(crate) fn` in `src/app.rs`.

- [ ] **Step 3: Register module in `src/main.rs`**

Insert the single line `mod menu_bar;` into the `mod` block so it sits alphabetically between `mod http_client;` and `mod request_editor;`. The block should read:

```rust
mod history_panel;
mod http_client;
mod menu_bar;
mod request_editor;
```

- [ ] **Step 4: Embed the menu in the `TitleBar` and remove the old env selector**

In `src/app.rs render`, replace the current title-bar child (`src/app.rs:425-433`):

```rust
                // Custom warm title bar (replaces the white native title bar)
                TitleBar::new().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("Poopman"),
                ),
```

with (capture the app handle + snapshot first; `cx.entity()` gives `Entity<PoopmanApp>`):

```rust
                // Custom warm title bar: brand + Edit menu (window controls are
                // added by TitleBar itself, platform-aware: macOS left inset,
                // Windows controls on the right).
                TitleBar::new()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("Poopman"),
                    )
                    .child(crate::menu_bar::edit_menu(
                        cx.entity(),
                        self.environments.clone(),
                        self.active_environment_id,
                    )),
```

Then delete the now-obsolete environment selector. Remove the whole `.child(...)` that builds the env button — currently the second child of the tab-bar `h_flex` at `src/app.rs:464-484` (the `div().flex_shrink_0()...Button::new("env-selector")...` block). After removal the tab-bar row is just:

```rust
                            .child(
                                // Tab bar row (env selector moved to the Edit menu)
                                h_flex()
                                    .w_full()
                                    .child(div().flex_1().min_w_0().child(self.tab_bar.clone())),
                            )
```

Also delete the now-unused `env_label` local (`src/app.rs:413-419`).

- [ ] **Step 5: Compile gate**

Run: `cargo check`
Expected: PASS. If `cx.entity()` type inference complains, annotate: `let app_handle: Entity<PoopmanApp> = cx.entity();` and pass `app_handle`.

- [ ] **Step 6: Commit**

```bash
git add src/menu_bar.rs src/main.rs src/app.rs
git commit -m "feat(env): Move environment switching into a title-bar Edit menu

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Floating-card layout (`src/app.rs`)

**Files:**
- Modify: `src/app.rs render` (root container + panel wrappers + resize handles)

- [ ] **Step 1: Pad the canvas and separate it from the cards**

In `src/app.rs render`, change the root `v_flex()` so the canvas uses the warmer muted tone and gains padding/gap. Replace:

```rust
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
```

with:

```rust
        v_flex()
            .size_full()
            .bg(theme.muted)
            .child(
```

The `TitleBar` stays as the first child (flush, no padding). Wrap the **content area** (the `div().flex_1().min_h_0()...` at `src/app.rs:434-435`) so the cards get outer padding + gaps. Change that wrapper to:

```rust
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .p_3()
                    .child(
```

(Remember to add the matching extra closing `)` for this added `div` at the end of the content block — keep brackets balanced.)

- [ ] **Step 2: Wrap the tab bar in a card**

The tab-bar row child (now `h_flex().w_full().child(...tab_bar...)`) should become its own card. Wrap it:

```rust
                            .child(
                                // Tab bar card (its own floating row)
                                crate::ui::card_panel(theme).child(
                                    h_flex()
                                        .w_full()
                                        .child(div().flex_1().min_w_0().child(self.tab_bar.clone())),
                                ),
                            )
```

Add `use crate::ui;` is unnecessary — call via `crate::ui::card_panel`. `theme` is already bound at the top of `render`.

- [ ] **Step 3: Wrap the sidebar, request, and response panels in cards**

Sidebar panel — change `src/app.rs:442-447` from:

```rust
                            .child(
                                div()
                                    .size_full()
                                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                    .child(self.history_panel.clone()),
                            ),
```

to:

```rust
                            .child(
                                crate::ui::card_panel(theme)
                                    .size_full()
                                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                    .child(self.history_panel.clone()),
                            ),
```

Request panel — wrap `self.request_editor.clone()` (`src/app.rs:494`):

```rust
                                            resizable_panel()
                                                .size(px(REQUEST_INITIAL_HEIGHT))
                                                .size_range(px(REQUEST_MIN)..px(REQUEST_MAX))
                                                .child(
                                                    crate::ui::card_panel(theme)
                                                        .size_full()
                                                        .child(self.request_editor.clone()),
                                                ),
```

Response panel — replace the bordered wrapper (`src/app.rs:496-505`) with a card (drop the old `.border_t_1()`):

```rust
                                        .child(
                                            crate::ui::card_panel(theme)
                                                .flex_1()
                                                .min_h(px(200.))
                                                .child(self.response_viewer.clone())
                                                .into_any_element(),
                                        ),
```

- [ ] **Step 4: Restyle the resize handles**

The `h_resizable`/`v_resizable` handles are styled by gpui-component defaults; to match Figma's thin rounded grabber, set the handle size subtly via the panels' gap (already provided by `.gap_2p5()` between cards). Leave the resizable handle internals as-is (gpui-component renders its own handle). No code change required here beyond the gap; if the handle is visually too wide, this is a polish item for Task 9, not a blocker.

- [ ] **Step 5: Compile gate**

Run: `cargo check`
Expected: PASS. Watch for unbalanced parentheses from the added wrapper `div` (Step 1) — the compiler will point at the mismatched delimiter; fix by adding/removing one `)` at the end of the content block.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat(ui): Float panels as padded cards on a warm canvas

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Pill-style request tabs (`src/tab_bar.rs`)

**Files:**
- Modify: `src/tab_bar.rs render` (`src/tab_bar.rs:64-156`)

- [ ] **Step 1: Restyle the outer row and tab pills**

Replace the outer `h_flex()` styling (`src/tab_bar.rs:69-76`) — drop the bottom border (the card already frames it) and tighten padding:

```rust
        h_flex()
            .gap_1()
            .items_center()
            .px_1p5()
            .py_1()
            .bg(theme.background)
```

Replace each tab's container styling (`src/tab_bar.rs:89-98`) so the active tab is a filled pill (no border) and inactive tabs are muted with hover:

```rust
                        h_flex()
                            .id(("tab", tab.id))
                            .gap_1p5()
                            .items_center()
                            .px_3()
                            .py_1()
                            .rounded(theme.radius)
                            .bg(if is_active { theme.muted } else { gpui::transparent_black() })
                            .when(!is_active, |s| s.hover(|s| s.bg(theme.list_hover)))
                            .cursor_pointer()
```

(Keep the `.on_click(...)` and the method-label / title / close-button children unchanged. The method label keeps `verb_color`; the title keeps its color logic.)

- [ ] **Step 2: Restyle the "new tab" button to a subtle pill**

Already close; just ensure rounding matches (`src/tab_bar.rs:138-143`):

```rust
                div()
                    .id("new-tab-button")
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.list_hover).text_color(theme.foreground))
```

- [ ] **Step 3: Compile gate**

Run: `cargo check`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/tab_bar.rs
git commit -m "refactor(ui): Pill-style request tabs in the tab-bar card

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Segmented tabs + status pill (`src/response_viewer.rs`)

**Files:**
- Modify: `src/response_viewer.rs render` (`src/response_viewer.rs:278-335`), and `render_status_bar` (`src/response_viewer.rs:142-200`).

- [ ] **Step 1: Replace the underline tab row with a segmented pill bar**

Replace the tab-row block (`src/response_viewer.rs:278-335`, the `div().flex().flex_row().gap_5().border_b_1()...` containing `resp-tab-body` and `resp-tab-headers`) with:

```rust
                        .child(
                            crate::ui::segmented_bar(theme)
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 0)
                                        .id("resp-tab-body")
                                        .when(self.active_tab != 0, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Body"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 1)
                                        .id("resp-tab-headers")
                                        .when(self.active_tab != 1, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Headers"),
                                ),
                        )
```

`when` requires `use gpui::prelude::FluentBuilder as _;` — confirm it's imported in this file (add it if missing).

- [ ] **Step 2: Make the status code a coral/semantic pill**

In `render_status_bar` the status text is shown around `src/response_viewer.rs:164-190`. Ensure the status code chip is a rounded filled pill using the existing semantic colors. Read the current block, then wrap the status code element as:

```rust
                div()
                    .px_2p5()
                    .py_0p5()
                    .rounded(theme.radius)
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .bg(status_color.opacity(0.12))
                    .text_color(status_color)
                    .child(format!("{} {}", status, status_text)),
```

where `status_color` is the semantic color already computed in that function (GET-green for 2xx, etc. — reuse the existing color variable; if none exists, derive from `theme.success`/`theme.warning`/`theme.danger` by status range, mirroring the existing logic). `theme` must be in scope: `render_status_bar` takes `&App`, so add `let theme = cx.theme();` at its top if not present.

- [ ] **Step 3: Compile gate**

Run: `cargo check`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/response_viewer.rs
git commit -m "refactor(ui): Segmented tabs + status pill in response viewer

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Segmented tabs (`src/request_editor.rs`)

**Files:**
- Modify: `src/request_editor.rs render` tab strip (`src/request_editor.rs:1047-1129`).

- [ ] **Step 1: Replace the underline tab row with a segmented pill bar**

Replace the tab-row block (`src/request_editor.rs:1047-1129`, the `div().flex().flex_row().gap_5().border_b_1()...` containing `tab-headers`, `tab-params`, `tab-body`) with:

```rust
                        .child(
                            crate::ui::segmented_bar(theme)
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 0)
                                        .id("tab-headers")
                                        .when(self.active_tab != 0, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Headers"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 1)
                                        .id("tab-params")
                                        .when(self.active_tab != 1, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 1;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Params"),
                                )
                                .child(
                                    crate::ui::segment_pill(theme, self.active_tab == 2)
                                        .id("tab-body")
                                        .when(self.active_tab != 2, |s| {
                                            s.hover(|s| s.text_color(theme.foreground))
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 2;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Body"),
                                ),
                        )
```

Confirm `use gpui::prelude::FluentBuilder as _;` is imported in this file (it is, given existing `.when(...)` usage at lines 1062+).

- [ ] **Step 2: Compile gate**

Run: `cargo check`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/request_editor.rs
git commit -m "refactor(ui): Segmented tabs in request editor

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: History rows polish (`src/history_panel.rs`)

The history rows are already card-like. Apply only minor polish so they read as cards on the sidebar surface.

**Files:**
- Modify: `src/history_panel.rs render` (`src/history_panel.rs:146-166`).

- [ ] **Step 1: Give non-selected rows a subtle card surface on hover and rounded edges**

Replace the row container styling (`src/history_panel.rs:146-166`) so it uses `radius_lg` and a faint border that strengthens on hover:

```rust
                            h_flex()
                                .id(("history-item", item_id as u64))
                                .gap_2()
                                .items_start()
                                .w_full()
                                .px_2p5()
                                .py_2()
                                .rounded(theme.radius_lg)
                                .border_1()
                                .border_color(if is_selected {
                                    theme.list_active_border
                                } else {
                                    gpui::transparent_black()
                                })
                                .bg(if is_selected {
                                    theme.list_active
                                } else {
                                    gpui::transparent_black()
                                })
                                .cursor_pointer()
                                .hover(|s| {
                                    s.bg(if is_selected { theme.list_active } else { theme.list_hover })
                                })
                                .on_click(cx.listener(move |this, _event: &gpui::ClickEvent, window, cx| {
                                    this.on_item_click(&item_clone, window, cx);
                                }))
```

(Children unchanged.)

- [ ] **Step 2: Compile gate**

Run: `cargo check`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/history_panel.rs
git commit -m "refactor(ui): Polish history rows for the card sidebar

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 8: Environment dialog beautification (`src/environment_manager.rs`, `src/app.rs`)

**Files:**
- Modify: `src/app.rs open_env_manager` (`src/app.rs:207-212`) — title + subtitle.
- Modify: `src/environment_manager.rs render` — dashed New button, dot activation, remove "Set active", card variable table, bottom Save footer.

- [ ] **Step 1: Dialog title + subtitle**

Replace `open_env_manager` body (`src/app.rs:209-211`) with a richer title element:

```rust
        let manager = self.env_manager.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let theme = cx.theme();
            dialog
                .title(
                    gpui_component::v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(theme.foreground)
                                .child("Environments"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Define variables like {{base_url}} per environment"),
                        ),
                )
                .w(px(680.))
                .child(manager.clone())
        });
```

Ensure `ActiveTheme` is in scope in `app.rs` (add `use gpui_component::ActiveTheme as _;` if `cx.theme()` doesn't resolve there).

- [ ] **Step 2: Dashed "New environment" button**

In `src/environment_manager.rs`, the add-row is `src/environment_manager.rs:225-259`. Give it a dashed coral border and pill shape — change its styling chain to:

```rust
                        h_flex()
                            .id("env-add")
                            .w_full()
                            .px_2()
                            .py_1p5()
                            .gap_2()
                            .items_center()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_dashed()
                            .border_color(theme.primary)
                            .text_color(theme.primary)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.list_active))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_environment(window, cx);
                            }))
```

(Keep its two children — the centered "+" indicator and the "New environment" label — unchanged.)

- [ ] **Step 3: Make the list dot the activation control + active highlight + ACTIVE tag**

In the env row (`src/environment_manager.rs:270-312`), the indicator-dot child currently only displays active state. Make the **dot clickable to toggle active**, and keep the rest-of-row click for selection. Replace the dot child (`src/environment_manager.rs:284-300`) with:

```rust
                                    .child(
                                        // Dot = activation toggle (stops row-select propagation)
                                        div()
                                            .id(("env-active-dot", id as u64))
                                            .w(px(16.))
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                cx.stop_propagation();
                                                let new = if this.active_id == Some(id) {
                                                    None
                                                } else {
                                                    Some(id)
                                                };
                                                this.set_active(new, cx);
                                            }))
                                            .child(
                                                div()
                                                    .w(px(7.))
                                                    .h(px(7.))
                                                    .rounded_full()
                                                    .when(is_active, |d| d.bg(theme.primary))
                                                    .when(!is_active, |d| {
                                                        d.border_1().border_color(theme.muted_foreground)
                                                    }),
                                            ),
                                    )
```

Add an `ACTIVE` tag at the end of the active row. After the name-label child (`src/environment_manager.rs:301-311`), append:

```rust
                                    .when(is_active, |row| {
                                        row.child(
                                            div()
                                                .flex_shrink_0()
                                                .px_1p5()
                                                .py_0()
                                                .rounded(theme.radius)
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .bg(theme.primary.opacity(0.12))
                                                .text_color(theme.primary)
                                                .child("ACTIVE"),
                                        )
                                    })
```

`set_active` already exists on `EnvironmentManager` (`src/environment_manager.rs:142`) and emits `EnvironmentsChanged`, so the menu and the request editor stay in sync.

- [ ] **Step 4: Remove the "Set active" button from the detail pane**

In the detail name row (`src/environment_manager.rs:322-347`), delete the `.when(active_id != Some(sel_id), |this| { this.child(Button::new("env-set-active")...) })` block entirely. The row keeps only the name `Input` and the `Delete` button.

- [ ] **Step 5: Card-style variable table**

Wrap the column-header row + the var rows in a single bordered card with a header strip and zebra striping. Replace the header-row block (`src/environment_manager.rs:348-360`) and the `v_flex().id("env-vars")...` block (`src/environment_manager.rs:361-395`) with one card:

```rust
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_color(theme.border)
                            .overflow_hidden()
                            .child(
                                // header strip
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .px_3()
                                    .py_1p5()
                                    .bg(theme.muted)
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(div().w(px(20.)).flex_shrink_0())
                                    .child(div().flex_1().child("KEY"))
                                    .child(div().flex_1().child("VALUE"))
                                    .child(div().w(px(24.)).flex_shrink_0()),
                            )
                            .child(
                                v_flex()
                                    .id("env-vars")
                                    .flex_1()
                                    .overflow_scroll()
                                    .children(self.var_rows.iter().enumerate().map(|(index, row)| {
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .items_center()
                                            .px_3()
                                            .py_1p5()
                                            .when(index % 2 == 1, |r| r.bg(theme.muted.opacity(0.4)))
                                            .border_t_1()
                                            .border_color(theme.border)
                                            .child(
                                                div().w(px(20.)).flex_shrink_0().flex().justify_center().child(
                                                    Checkbox::new(("var-check", index))
                                                        .checked(row.enabled)
                                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                                            this.toggle_var(index, cx);
                                                        })),
                                                ),
                                            )
                                            .child(div().flex_1().min_w_0().child(Input::new(&row.key_input)))
                                            .child(div().flex_1().min_w_0().child(Input::new(&row.value_input)))
                                            .child(
                                                div().w(px(24.)).flex_shrink_0().flex().justify_center().child(
                                                    Button::new(("var-del", index))
                                                        .ghost()
                                                        .xsmall()
                                                        .label("×")
                                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                                            this.remove_var_row(index, cx);
                                                        })),
                                                ),
                                            )
                                    })),
                            ),
                    )
```

- [ ] **Step 6: Bottom bar: "+ Add variable" (left) and a Save footer (right)**

Replace the final bottom row (`src/environment_manager.rs:396-420`, the `h_flex().justify_between()` with Add variable + Save) with an add-variable button directly under the table, plus a bordered footer holding only a right-aligned Save:

```rust
                    .child(
                        Button::new("env-add-var")
                            .small()
                            .ghost()
                            .label("+ Add variable")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_var_row(window, cx);
                            })),
                    )
                    .child(
                        // Footer: only a right-aligned Save (no hint text).
                        h_flex()
                            .w_full()
                            .justify_end()
                            .pt_2()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(
                                Button::new("env-save")
                                    .small()
                                    .primary()
                                    .label("Save")
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.save_and_notify(cx);
                                    })),
                            ),
                    )
```

- [ ] **Step 7: Compile gate**

Run: `cargo check`
Expected: PASS. If `theme.primary.opacity(0.12)` doesn't resolve, use `theme.primary.opacity(0.12)` from gpui's `Hsla::opacity`; it exists (used widely). If `border_dashed()` is missing, it's `gpui::Styled::border_dashed` — confirm by grepping gpui for `fn border_dashed`.

- [ ] **Step 8: Commit**

```bash
git add src/environment_manager.rs src/app.rs
git commit -m "feat(env): Beautify environment dialog (dot activation, card table, Save footer)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 9: Final build + manual visual verification

**Files:** none (verification only).

- [ ] **Step 1: Full release-mode compile gate**

Run: `cargo build`
Expected: PASS with no errors. Resolve any remaining warnings about unused imports (e.g. removed `Button`/`Sizable` usages in `app.rs` if the env button removal orphaned an import).

- [ ] **Step 2: Hand off to the user for visual verification on Windows/macOS**

The agent cannot run the GPU app in WSL2. Ask the user to run `cargo run` on their Windows/macOS machine and confirm against the Figma mockups in `.superpowers/brainstorm/`:
  - Panels float as padded cards with gaps; resize handles still work.
  - Title bar shows "Poopman" + an "Edit" menu at top-left; window controls render correctly on their platform.
  - Edit → environment list switches the active env (check mark moves; request variables update); "Manage Environments…" opens the dialog.
  - Request/Response tab strips are segmented pills; response status is a colored pill.
  - Env dialog: dashed New button, dot toggles active (+ ACTIVE tag), no "Set active" button, card variable table with header/zebra, Save in a bottom-right footer, no hint text.

- [ ] **Step 3: Address any visual fixes the user reports**, then final commit if changes were made:

```bash
git add -A
git commit -m "fix(ui): Visual polish from manual verification

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review Notes

- **Spec coverage:** floating-card layout → Task 3; Edit menu w/ env list + manage → Task 2; cross-platform title bar → relies on gpui-component `TitleBar` (Task 2 Step 4); env dialog (dashed new, dot activation, no "Set active", card table, Save footer, no hint) → Task 8; segmented pill tabs → Tasks 5–6; pill request tabs → Task 4; history card rows → Task 7; theme retained, shadows via `shadow_sm` helper → Task 1.
- **Sequencing:** helpers (1) precede consumers (3–8); menu (2) adds `set_active_environment`/`open_env_manager` visibility used nowhere else; dialog title change (8) and manager redesign (8) committed together.
- **Type consistency:** `card_panel`/`segmented_bar`/`segment_pill` signatures `(theme: &Theme[, active: bool]) -> Div` used identically everywhere; `set_active_environment(Option<i64>)` and `set_active(Option<i64>)` (existing manager method) both take `Option<i64>`; `PopupMenuItem::new(..).checked(..).on_click(..)` matches verified 0.5.1 API.
- **Known verify-at-implementation points (flagged inline):** exact re-export paths for `DropdownMenu`/`PopupMenuItem`; presence of `border_dashed`; `render_status_bar` semantic color variable name. These are grep-confirmable in seconds and noted at the relevant steps.
