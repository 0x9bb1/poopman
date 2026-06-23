# Poopman UI Redesign — Floating Cards, Edit Menu, Env Dialog

**Date:** 2026-06-23
**Status:** Approved (design)
**Source design:** Figma export "GUI app design" (React/Tailwind) — coral-orange "Claude Code" inspired API client.

## Goal

Reimplement Poopman's GPUI UI to match the provided Figma design, with three user-driven changes:

1. Adopt the Figma **floating-card layout** (padded canvas; each panel is an independent rounded, bordered, shadowed card with gaps between them).
2. Move the **Environment control** out of the toolbar and into an **Edit menu** integrated at the top-left of the title bar (like VS Code / modern desktop apps), cross-platform on macOS and Windows.
3. **Beautify the Environment dialog** in the same floating-card / coral style, with a refined activation interaction.

The existing warm coral "Claude paper" theme in `theme.rs` already matches the Figma palette and is largely retained — this is primarily a structural + styling pass, not a rewrite of behavior or data flow.

## Non-Goals (YAGNI)

- No new View/Help menus (Edit only for now; structure leaves room to add later).
- No changes to HTTP logic, DB schema, request/response data flow, or URL/params sync.
- No dark mode work.
- No pixel-for-pixel cloning — match the design language (rounded cards, segmented pill tabs, card-style rows, coral accents), not exact Tailwind values.

## Constraints

- **WSL2 cannot run GPUI** (GPU requirement). Verification in this environment is limited to `cargo check` / `cargo build`. Visual confirmation is done by the user running `cargo run` on Windows/macOS.
- Cross-platform title bar: rely on gpui-component `TitleBar`'s platform-aware control placement (macOS traffic-light inset on the left; Windows window controls on the right). The menu sits left-aligned, immediately after the "Poopman" brand, after any platform inset.
- Stack: gpui 0.2.2, gpui-component 0.5.x. Menu uses `Button::dropdown_menu(...)` + `PopupMenu` (`.label`, `.menu_with_check`, `.separator`, item `on_click`), all confirmed present in 0.5.1.

## Approach

Introduce small shared UI helpers rather than inlining styles per panel (consistent with the project's "reuse over reinvent" rule):

- **`src/ui.rs` (new):** shared building blocks —
  - `card_panel()` → a container with `bg(card) + border_1 + border_color(border) + rounded_lg + shadow_sm + overflow_hidden`, used to wrap each floating panel.
  - segmented pill tab strip helper → `bg(muted)` rounded container holding pills; active pill is `bg(card) + shadow_sm + foreground`, inactive is `muted_foreground` with hover. Reused by Request and Response tab bars.
- **`src/menu_bar.rs` (new):** the Edit menu (a `Button` with `dropdown_menu`) intended to be embedded as a child of `TitleBar`.
- Existing panels are restyled in place to consume these helpers.

Alternative considered: inline all styling directly into each panel. Rejected — more scattered, higher risk of visual drift between panels, and against the codebase's reuse guidance.

## Component-Level Design

### 1. Floating-card layout — `app.rs` (render)

- Root: `v_flex().size_full().bg(background)`, with `p_3` padding and `gap` between stacked sections (≈ Tailwind `gap-2.5`).
- `TitleBar` stays flush at the very top (brand + Edit menu + window controls); it is **not** wrapped in a card.
- Below the title bar, content is composed of card-wrapped panels:
  - **Tab bar card** — its own full-width row (`card_panel`), containing the request tabs + "new tab" button. The current Env selector at the row's end is **removed**.
  - **Main split** (`h_resizable`): left **Sidebar card** + right column.
  - Right column (`v_resizable`): **Request card** above, **Response card** below.
  - Each panel content is wrapped with `card_panel()`.
- **Resize handles** restyled to match Figma: a thin, short, rounded `bg(border)` bar centered in the handle track; on hover → `primary` at reduced opacity. Applies to both the horizontal (sidebar/main) and vertical (request/response) handles.

### 2. Edit menu — `menu_bar.rs` (new) + `app.rs` + actions

- A `Button::new("edit-menu").ghost()` labeled "Edit", placed as a `TitleBar` child immediately right of the "Poopman" brand label.
- `.dropdown_menu(|menu, _, _| ...)` builds a `PopupMenu`:
  - `.label("Environment")` — group header.
  - One entry per environment: `.menu_with_check(env.name, is_active, action_or_onclick)` — clicking switches the active environment (check mark indicates current).
  - A `"No Environment"` entry to clear the active selection (checked when none active).
  - `.separator()`.
  - `"Manage Environments…"` — opens the (beautified) environment dialog.
- Wiring: define gpui actions (e.g. `SwitchEnvironment { id: i64 }`, `ClearEnvironment`, `OpenEnvManager`) or equivalent item `on_click` closures dispatched to `PoopmanApp`. Handlers reuse existing `set_active(...)` and `open_env_manager(...)` logic. Switching active env continues to emit/refresh request-editor variables exactly as today.
- `PopupMenu::init(cx)` is invoked at startup if not already (alongside `gpui_component::init`).

### 3. Environment dialog beautification — `environment_manager.rs` + dialog wrapper in `app.rs`

Behavior is preserved (create / select / activate / delete environments; add / edit / toggle / delete variables; immediate DB writes; `EnvironmentsChanged` emission). Visual + one interaction change:

- **Dialog frame:** larger corner radius, soft shadow. Header shows title **"Environments"** plus a one-line subtitle: *"Define variables like `{{base_url}}` per environment."* Close (✕) affordance consistent with the style.
- **Left list:**
  - `"+ New environment"` rendered as a **dashed coral button**.
  - Environment rows: the **left dot is the activation control** — clicking the dot toggles active (filled coral = active; empty = inactive). Clicking the rest of the row selects it for editing (decoupled from activation, so a non-active env can be edited without switching). Active row gets a coral-wash background + an `ACTIVE` tag.
  - The detail pane's **"Set active" button is removed** (activation now lives on the dot).
- **Right detail pane:**
  - Name input + `Delete` (ghost/danger) on one row.
  - Variables rendered as a **card table**: a header row (Key / Value), zebra-striped rows, checkbox with coral fill when enabled, a thin vertical separator between key and value, and a delete affordance revealed on hover.
  - `"+ Add variable"` as a coral text button.
- **Footer:** the *"改动即时写入数据库"* hint is **removed**. Footer keeps only a right-aligned primary **`Save`** button.

### 4. Theme touch-ups — `theme.rs`

The existing coral palette is retained. Add/confirm only what the card style needs:

- A subtle `shadow_sm` value for cards (via gpui shadow on the `card_panel` helper, not necessarily a theme field).
- Segmented pill tabs: active pill on white (`card`) surface; segment container on `muted`.
- HTTP method colors keep the existing semantic mapping in `method_color(...)`.

## Data Flow (unchanged)

Request configuration → send → `RequestCompleted` → app saves to DB, updates `ResponseViewer`, reloads `HistoryPanel`. Environment switching (now via the Edit menu) → `set_active` → `EnvironmentsChanged` → app reloads environments and refreshes the request editor's variable map. No flow changes; only the *entry points* for environment switching/management move into the menu.

## Files Touched

- `src/ui.rs` — **new** (shared card + segmented-tab helpers).
- `src/menu_bar.rs` — **new** (Edit menu).
- `src/app.rs` — floating-card layout, title-bar menu embedding, action handling, dialog wrapper styling; remove old Env toolbar button.
- `src/environment_manager.rs` — dialog visual redesign + dot-activation interaction; remove "Set active" button.
- `src/request_editor.rs` — segmented pill tabs, card-style key/value rows, rounded inputs.
- `src/response_viewer.rs` — segmented pill tabs, status pill, card-style headers/body.
- `src/history_panel.rs` — card-style history rows matching Figma.
- `src/tab_bar.rs` — pill-style request tabs inside the tab-bar card.
- `src/theme.rs` — minor shadow / segment surface additions if needed.
- `src/main.rs` — register new modules; ensure `PopupMenu::init` if required.

## Testing / Verification

- `cargo check` and `cargo build` must pass in this environment (primary automated gate).
- Manual visual verification performed by the user on Windows/macOS via `cargo run`, checked against the Figma mockups in `.superpowers/brainstorm/`.
- Spot-check behavior parity: environment switch from menu updates active env + request variables; dialog create/edit/activate/delete still persist; request/response panels still send and render.

## Open Items

None blocking. View/Help menus and any further polish are deferred.
