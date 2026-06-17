# UI 重设计（暖调浅色 / 布局 B）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Poopman 重设计为 Claude/Anthropic 暖调浅色观感（纸感背景 + 珊瑚橘强调色 + 紧凑布局），覆盖全部面板。

**Architecture:** 两层。① 新增 `src/theme.rs`，在 `gpui_component::init` 后用 `Theme::global_mut(cx)` 覆盖颜色/字体/圆角 token——因所有组件已读 `cx.theme()`，这一层即统一全局配色;② 逐面板结构微调(历史单行、内层 tab 下划线、状态栏 pill 等)。`theme.rs` 同时集中尺寸常量,替换散落的 `px()`。

**Tech Stack:** Rust, GPUI 0.2.2, gpui-component 0.5.1。关键 API 已核实:`gpui_component::Theme` 在 crate 根导出;`Theme: DerefMut<Target=ThemeColor>`;`Theme::global_mut(cx) -> &mut Theme`;字段 `mode/radius/radius_lg/font_family/mono_font_family`;`gpui::rgb(0xRRGGBB).into()` → `Hsla`。

**Spec:** `docs/superpowers/specs/2026-06-17-ui-redesign-design.md`
**视觉基准:** `docs/superpowers/specs/2026-06-17-ui-redesign-mockup.html`(实现时逐面板对照)

**验证约束:** WSL2 无法运行/链接 GUI(缺 `libxkbcommon`),也跑不了 `cargo test`。每个任务的自动门禁是 **`cargo check`(无新增 warning)**。视觉正确性由开发者在 **Windows** 真机 `cargo build --release`(需设 `GPUI_FXC_PATH`)后对照 mockup 目视验收;建议在 Task 1 后(确认全局配色)和 Task 7 后(完整)各做一次真机验收。

---

## File Structure

- **新增** `src/theme.rs` — 调色板常量、尺寸常量、`apply_theme(cx)`、`method_color(method, theme)`。
- **改** `src/main.rs` — 声明 `mod theme`;`gpui_component::init(cx)` 后调用 `theme::apply_theme(cx)`。
- **改** `src/app.rs` — `px()` 字面量换尺寸常量;侧栏背景用 `sidebar` token。
- **改** `src/request_editor.rs` — `px()` 换常量;内层 tab 改下划线式。
- **改** `src/body_editor.rs` — `px()` 换常量;次要按钮统一暖调。
- **改** `src/history_panel.rs` — 去 logo 圆点、单行紧凑、方法文字标签、发丝分隔、浅珊瑚选中。
- **改** `src/tab_bar.rs` — 方法实心徽章改文字标签;复用 `method_color`。
- **改** `src/response_viewer.rs` — 状态栏 pill + 元信息;响应体卡片化。

---

## Task 1: 主题模块 + 全局应用（配色层）

**Files:**
- Create: `src/theme.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: 创建 `src/theme.rs`**

写入完整内容:

```rust
//! Warm-light ("Claude paper") theme: palette, layout dimensions, and the
//! global theme application applied once at startup.

use gpui::{px, App, Hsla};
use gpui_component::{Theme, ThemeMode};

use crate::types::HttpMethod;

// ===== Palette (warm paper light), as 0xRRGGBB =====
const BACKGROUND: u32 = 0xFAF9F5; // main canvas
const SIDEBAR: u32 = 0xF0EEE6; // sidebar / muted surfaces
const SURFACE: u32 = 0xFFFFFF; // white surfaces (input / popover)
const FOREGROUND: u32 = 0x1A1915; // primary text
const MUTED_FG: u32 = 0x73706A; // secondary text
const BORDER: u32 = 0xE7E4DA; // hairline border
const HOVER: u32 = 0xEBE8DE; // subtle warm hover bg
const PRIMARY: u32 = 0xC15F3C; // coral / terracotta accent
const PRIMARY_HOVER: u32 = 0xAD5435;
const WASH: u32 = 0xF3E7E0; // light coral selection wash
const WASH_BORDER: u32 = 0xE8D3C8;
const SUCCESS: u32 = 0x4F8A5B; // 2xx / GET
const DANGER: u32 = 0xC0503F; // 4xx-5xx / DELETE
const WARNING: u32 = 0xC98A3C; // amber / POST,PUT
const SCROLLBAR: u32 = 0xD8D4C8;

// ===== Layout dimensions (px) =====
pub const SIDEBAR_WIDTH: f32 = 264.;
pub const SIDEBAR_MIN: f32 = 200.;
pub const SIDEBAR_MAX: f32 = 420.;
pub const REQUEST_INITIAL_HEIGHT: f32 = 350.;
pub const REQUEST_MIN: f32 = 150.;
pub const REQUEST_MAX: f32 = 700.;
pub const METHOD_SELECT_WIDTH: f32 = 92.;
pub const RAW_SUBTYPE_WIDTH: f32 = 120.;

/// Convert a 0xRRGGBB literal into an Hsla theme color.
fn c(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Semantic color for an HTTP method label (used by tab bar + history).
pub fn method_color(method: HttpMethod, theme: &Theme) -> Hsla {
    match method {
        HttpMethod::GET => theme.success,
        HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH => theme.warning,
        HttpMethod::DELETE => theme.danger,
        HttpMethod::HEAD | HttpMethod::OPTIONS => theme.muted_foreground,
    }
}

/// Apply the warm-light theme to the global Theme. Call once after
/// `gpui_component::init(cx)`.
pub fn apply_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.mode = ThemeMode::Light;
    theme.radius = px(6.);
    theme.radius_lg = px(8.);

    // Surfaces & text
    theme.background = c(BACKGROUND);
    theme.foreground = c(FOREGROUND);
    theme.muted = c(SIDEBAR);
    theme.muted_foreground = c(MUTED_FG);
    theme.border = c(BORDER);
    theme.input = c(SURFACE);
    theme.popover = c(SURFACE);
    theme.popover_foreground = c(FOREGROUND);
    theme.secondary = c(SIDEBAR);
    theme.secondary_foreground = c(FOREGROUND);
    theme.secondary_hover = c(HOVER);
    theme.secondary_active = c(HOVER);

    // Sidebar
    theme.sidebar = c(SIDEBAR);
    theme.sidebar_foreground = c(FOREGROUND);
    theme.sidebar_border = c(BORDER);

    // Accent / primary (coral)
    theme.primary = c(PRIMARY);
    theme.primary_hover = c(PRIMARY_HOVER);
    theme.primary_active = c(PRIMARY_HOVER);
    theme.primary_foreground = c(SURFACE);
    theme.ring = c(PRIMARY);
    theme.caret = c(PRIMARY);
    theme.accent = c(WASH);
    theme.accent_foreground = c(FOREGROUND);
    theme.selection = c(WASH);

    // Lists (history rows)
    theme.list = c(SIDEBAR);
    theme.list_hover = c(HOVER);
    theme.list_active = c(WASH);
    theme.list_active_border = c(WASH_BORDER);

    // Tabs
    theme.tab = c(BACKGROUND);
    theme.tab_active = c(SURFACE);

    // Status semantics
    theme.success = c(SUCCESS);
    theme.success_foreground = c(SURFACE);
    theme.danger = c(DANGER);
    theme.danger_foreground = c(SURFACE);
    theme.danger_hover = c(DANGER);
    theme.danger_active = c(DANGER);
    theme.warning = c(WARNING);
    theme.warning_foreground = c(SURFACE);

    // Scrollbar
    theme.scrollbar_thumb = c(SCROLLBAR);
    theme.scrollbar_thumb_hover = c(MUTED_FG);
}
```

- [ ] **Step 2: 在 `main.rs` 声明模块并调用**

在 `src/main.rs` 的模块声明区(`mod app;` 那一组)加入,保持字母序附近即可:

```rust
mod theme;
```

然后把 `app.run` 闭包里的:

```rust
        gpui_component::init(cx);
```

改为:

```rust
        gpui_component::init(cx);
        crate::theme::apply_theme(cx);
```

- [ ] **Step 3: `cargo check`**

Run: `cargo check`
Expected: 退出 0。除既有的 2 个 dead-code warning(`toggle_formdata_type`、`from_request`)外无新增 error/warning。
(若报 `method_color` 未使用的 warning,属正常——Task 3/4 才会用到;先加 `#[allow(dead_code)]` 于 `method_color` 上,在 Task 4 移除。)

- [ ] **Step 4: Commit**

```bash
git add src/theme.rs src/main.rs
git commit -m "feat(ui): Add warm-light theme module and apply globally

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 5: (检查点)Windows 真机验收全局配色**

在 Windows 终端:
```powershell
$env:GPUI_FXC_PATH = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\fxc.exe"
cargo run --release
```
Expected: 背景变暖纸色、Send 按钮/内层 tab 选中/历史选中均为珊瑚色系,无黑/亮蓝/亮绿杂色(结构尚未调整属正常)。

---

## Task 2: 尺寸常量替换 `px()` 字面量

**Files:**
- Modify: `src/app.rs`, `src/request_editor.rs`, `src/body_editor.rs`

- [ ] **Step 1: `app.rs` 顶部引入常量**

在 `src/app.rs` 现有 `use crate::...` 区加入:

```rust
use crate::theme::{
    REQUEST_INITIAL_HEIGHT, REQUEST_MAX, REQUEST_MIN, SIDEBAR_MAX, SIDEBAR_MIN, SIDEBAR_WIDTH,
};
```

- [ ] **Step 2: `app.rs` 替换历史面板与请求区尺寸**

把:
```rust
                        resizable_panel()
                            .size(px(280.)) // Initial width
                            .size_range(px(200.)..px(500.)) // Can resize between 200px-500px
```
改为:
```rust
                        resizable_panel()
                            .size(px(SIDEBAR_WIDTH))
                            .size_range(px(SIDEBAR_MIN)..px(SIDEBAR_MAX))
```

把:
```rust
                                            resizable_panel()
                                                .size(px(350.)) // Request editor initial size
                                                .size_range(px(150.)..px(700.)) // Can resize between 150px-700px
```
改为:
```rust
                                            resizable_panel()
                                                .size(px(REQUEST_INITIAL_HEIGHT))
                                                .size_range(px(REQUEST_MIN)..px(REQUEST_MAX))
```

- [ ] **Step 3: `request_editor.rs` 替换 method 选择器宽度**

在 `use crate::...` 区加入 `use crate::theme::METHOD_SELECT_WIDTH;`,然后把 URL 栏里的:
```rust
                                .w(px(100.))
                                .child(Select::new(&self.method_select)),
```
改为:
```rust
                                .w(px(METHOD_SELECT_WIDTH))
                                .child(Select::new(&self.method_select)),
```

- [ ] **Step 4: `body_editor.rs` 替换 Raw 子类型下拉宽度**

加入 `use crate::theme::RAW_SUBTYPE_WIDTH;`,把:
```rust
                            div()
                                .w(px(120.))
                                .child(Select::new(&self.raw_subtype_select))
```
改为:
```rust
                            div()
                                .w(px(RAW_SUBTYPE_WIDTH))
                                .child(Select::new(&self.raw_subtype_select))
```

- [ ] **Step 5: `cargo check`**

Run: `cargo check`
Expected: 退出 0,无新增 warning。(`px(0.)`/`px(0.1)` 滚动偏移保持不变,不提取。)

- [ ] **Step 6: Commit**

```bash
git add src/app.rs src/request_editor.rs src/body_editor.rs
git commit -m "refactor(ui): Replace hardcoded layout px() with theme dimension constants

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: History 面板 — 单行紧凑

**Files:**
- Modify: `src/history_panel.rs`

- [ ] **Step 1: 去掉 header 的 logo 圆点**

把:
```rust
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::default()
                                    .path("icons/logo.svg")
                                    .size_5()
                            )
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("History"))
```
改为:
```rust
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("History")
```
若 `Icon` 因此不再使用,删除其 `use` 导入以免 warning(检查 `history_panel.rs` 顶部 `use gpui_component::{... Icon ...}`,移除 `Icon`)。

- [ ] **Step 2: 列表容器去重边框、收紧**

把列表容器:
```rust
                    v_flex()
                        .size_full()
                        .gap_2()
                        .p_2()
                        .children(self.history.iter().map(|item| {
```
改为(改用 `theme.sidebar` 背景、小间距、内边距收紧):
```rust
                    v_flex()
                        .size_full()
                        .gap_0p5()
                        .px_2()
                        .py_1()
                        .children(self.history.iter().map(|item| {
```

- [ ] **Step 3: 重写单条历史项为紧凑单行**

定位到 item 渲染块。当前它读取 `let method = item.request.method.as_str();` 并用 `method_color = match method { "GET" => theme.success, ... }`、外层是带粗边框的卡片。替换整个 item 渲染(从 `let method = ...` 到该 item `div()...` 结束)为:

```rust
                            let item_id = item.id;
                            let is_selected = self.selected_id == Some(item_id);
                            let verb = item.request.method.as_str();
                            let verb_color = crate::theme::method_color(item.request.method, theme);
                            let url = item.request.url.clone();
                            let time = Self::format_relative_time(&item.timestamp);
                            let item_clone = item.clone();

                            h_flex()
                                .id(("history-item", item_id as u64))
                                .gap_2()
                                .items_start()
                                .w_full()
                                .px_2p5()
                                .py_1p5()
                                .rounded(theme.radius)
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
                                .hover(|s| s.bg(if is_selected { theme.list_active } else { theme.list_hover }))
                                .on_click(cx.listener(move |this, _event: &gpui::ClickEvent, window, cx| {
                                    this.on_item_click(&item_clone, window, cx);
                                }))
                                .child(
                                    // small mono method label, no filled pill
                                    div()
                                        .flex_shrink_0()
                                        .w(px(34.))
                                        .text_right()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(verb_color)
                                        .child(verb),
                                )
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .flex_1()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(theme.foreground)
                                                .overflow_x_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .child(url),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(time),
                                        ),
                                )
```

> 注意:此块**移除**了旧的 `status_color` 徽章逻辑(历史不存响应,徽章无意义)。若编译报 `status_color`/`method_color`(旧局部)未使用,一并删除其残留定义。

- [ ] **Step 4: 引入 `px` 与确认 `FontWeight` 可用**

确认 `history_panel.rs` 顶部已有 `use gpui::*;`(含 `px`、`FontWeight`、`transparent_black`)。当前文件已 `use gpui::prelude::FluentBuilder as _;` 和 `use gpui::*;`,无需新增。

- [ ] **Step 5: `cargo check`**

Run: `cargo check`
Expected: 退出 0,无新增 warning。

- [ ] **Step 6: Commit**

```bash
git add src/history_panel.rs
git commit -m "feat(ui): Compact single-line history rows with text method labels

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Tab 栏 — 方法文字标签

**Files:**
- Modify: `src/tab_bar.rs`

- [ ] **Step 1: 顶部引入 `method_color`**

把:
```rust
use gpui_component::{h_flex, ActiveTheme as _};
```
改为:
```rust
use gpui_component::{h_flex, ActiveTheme as _};

use crate::theme::method_color;
```

- [ ] **Step 2: 用语义色文字标签替换实心徽章**

把当前计算 `method_color` 的硬编码块:
```rust
                        // Method color badge
                        let method_color = match tab.request.method {
                            crate::types::HttpMethod::GET => gpui::rgb(0x61affe),
                            crate::types::HttpMethod::POST => gpui::rgb(0x49cc90),
                            crate::types::HttpMethod::PUT => gpui::rgb(0xfca130),
                            crate::types::HttpMethod::DELETE => gpui::rgb(0xf93e3e),
                            crate::types::HttpMethod::PATCH => gpui::rgb(0x50e3c2),
                            crate::types::HttpMethod::HEAD => gpui::rgb(0x9012fe),
                            crate::types::HttpMethod::OPTIONS => gpui::rgb(0x0d5aa7),
                        };
```
改为:
```rust
                        let verb_color = method_color(tab.request.method, theme);
```

然后把那段实心徽章 child:
```rust
                            .child(
                                // Method badge
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(method_color)
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(gpui::white())
                                            .child(method)
                                    )
                            )
```
改为(去掉实心底色,文字着语义色):
```rust
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(verb_color)
                                    .child(method)
                            )
```

- [ ] **Step 3: 选中态用纸白浮层而非晕染色块**

把 tab 容器的:
```rust
                            .bg(if is_active { theme.list_active } else { theme.background })
```
改为:
```rust
                            .bg(if is_active { theme.tab_active } else { theme.background })
```

- [ ] **Step 4: 移除 Task 1 给 `method_color` 加的 `#[allow(dead_code)]`**

在 `src/theme.rs` 中删除 `method_color` 上方的 `#[allow(dead_code)]`(现已被 tab_bar/history 使用)。

- [ ] **Step 5: `cargo check`**

Run: `cargo check`
Expected: 退出 0,无新增 warning。

- [ ] **Step 6: Commit**

```bash
git add src/tab_bar.rs src/theme.rs
git commit -m "feat(ui): Tab method badges become text labels; white active tab

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: 请求区 — 内层 tab 下划线式

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: 把三个内层 tab 按钮改为下划线式 div**

定位 render 中 Headers/Params/Body 三个 `Button::new("tab-headers"/"tab-params"/"tab-body")...when(self.active_tab == N, |btn| btn.primary())...` 的横排容器。把这三颗 Button 各自替换为下划线式可点击 div。以 Headers 为例,把:
```rust
                                .child(
                                    Button::new("tab-headers")
                                        .ghost()
                                        .label("Headers")
                                        .when(self.active_tab == 0, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        )),
                                )
```
改为:
```rust
                                .child(
                                    div()
                                        .id("tab-headers")
                                        .px_0p5()
                                        .pb_2()
                                        .text_sm()
                                        .cursor_pointer()
                                        .border_b_2()
                                        .when(self.active_tab == 0, |this| {
                                            this.border_color(theme.primary)
                                                .text_color(theme.primary)
                                                .font_weight(FontWeight::SEMIBOLD)
                                        })
                                        .when(self.active_tab != 0, |this| {
                                            this.border_color(gpui::transparent_black())
                                                .text_color(theme.muted_foreground)
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Headers"),
                                )
```
对 Params(`active_tab == 1`,id `"tab-params"`,文案 `"Params"`)、Body(`active_tab == 2`,id `"tab-body"`,文案 `"Body"`)做完全相同的替换(仅 id/索引/文案不同)。

- [ ] **Step 2: 给三个 tab 的横排容器加下边框作为基线**

把包裹这三个 tab 的 `div().flex().flex_row().gap_1()`(紧邻上面三 child 的父容器)改为:
```rust
                            div()
                                .flex()
                                .flex_row()
                                .gap_5()
                                .border_b_1()
                                .border_color(theme.border)
```
(原为 `.gap_1()` 无下边框;改为 `gap_5` 拉开间距 + 下边框作下划线基线。)

- [ ] **Step 3: 确认 `theme` 在 render 作用域可用**

`request_editor.rs` 的 `render` 开头已有 `let theme = cx.theme();`,上述 `theme.primary` 等可直接用。`FontWeight` 来自 `use gpui::*;`(已存在)。

- [ ] **Step 4: `cargo check`**

Run: `cargo check`
Expected: 退出 0,无新增 warning。

- [ ] **Step 5: Commit**

```bash
git add src/request_editor.rs
git commit -m "feat(ui): Underline-style Headers/Params/Body tabs

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Body 编辑器 — 次要按钮统一暖调

**Files:**
- Modify: `src/body_editor.rs`

- [ ] **Step 1: Format/Validate 按钮统一为次要(ghost)风格**

在 render 中找到 Format 与 Validate 按钮。Validate 已是 `.ghost()`;把 Format 也改为 `.ghost()` 以与整体克制风一致。把:
```rust
                            Button::new("format-button")
                                .small()
                                .label("Format")
```
改为:
```rust
                            Button::new("format-button")
                                .small()
                                .ghost()
                                .label("Format")
```

- [ ] **Step 2: `cargo check`**

Run: `cargo check`
Expected: 退出 0,无新增 warning。
(其余 body 控件——单选、下拉、代码区——均读 `cx.theme()`,已随 Task 1 主题自动变暖,无需逐一改。)

- [ ] **Step 3: Commit**

```bash
git add src/body_editor.rs
git commit -m "style(ui): Make Body Format button secondary to match tone

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: 响应区 — 状态 pill + 卡片化

**Files:**
- Modify: `src/response_viewer.rs`

- [ ] **Step 1: 状态栏改为 pill + 元信息(去整条灰块)**

`render_status_bar` 中,有响应时的 `h_flex()....bg(cx.theme().muted)....` 整条容器。把它的 `.bg(cx.theme().muted)` 去掉、加下边框,使状态栏与画布同色仅靠分隔线区分。把:
```rust
            h_flex()
                .gap_3()
                .items_center()
                .p_2()
                .bg(cx.theme().muted)
                .border_b_1()
                .border_color(cx.theme().border)
```
改为:
```rust
            h_flex()
                .gap_3()
                .items_center()
                .px_4()
                .py_2p5()
                .border_b_1()
                .border_color(cx.theme().border)
```
其中 `200 OK` 色块(`.bg(status_color).text_color(gpui::white())`)保留为 pill;把它的圆角统一:把那块的 `.rounded(cx.theme().radius)` 保留即可(主题已设 6px)。`Time: {} ms` / `Size: {} bytes` 文案保持。

- [ ] **Step 2: 无响应时的状态栏同样去灰块**

把无响应分支:
```rust
            h_flex()
                .p_2()
                .bg(cx.theme().muted)
                .border_b_1()
                .border_color(cx.theme().border)
                .child("No response yet")
```
改为:
```rust
            h_flex()
                .px_4()
                .py_2p5()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_color(cx.theme().muted_foreground)
                .child("No response yet")
```

- [ ] **Step 3: 响应体卡片化**

在 `render` 的 Body tab 分支(`when(self.active_tab == 0, ...)`),把包住 `Input::new(&self.body_display)` 的容器加上卡片样式。把:
```rust
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .w_full()
                                    .child(
                                        Input::new(&self.body_display)
                                            .disabled(is_error)
                                            .w_full()
                                            .h_full(),
                                    ),
                            )
```
改为:
```rust
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .w_full()
                                    .rounded(theme.radius_lg)
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.popover)
                                    .overflow_hidden()
                                    .child(
                                        Input::new(&self.body_display)
                                            .disabled(is_error)
                                            .w_full()
                                            .h_full(),
                                    ),
                            )
```
注:`render` 开头已有 `let theme = cx.theme();`;`radius_lg` 是 `Theme` 字段,可直接用。

- [ ] **Step 4: Body/Headers 内层 tab 改下划线式**

与 Task 5 同法,把响应区的 `Button::new("tab-body")` 与 `Button::new("tab-headers")`(各带 `.when(self.active_tab == N, |btn| btn.primary())`)替换为下划线式 div。以 Body 为例,把:
```rust
                                .child(
                                    Button::new("tab-body")
                                        .ghost()
                                        .label("Body")
                                        .when(self.active_tab == 0, |btn| btn.primary())
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        )),
                                )
```
改为:
```rust
                                .child(
                                    div()
                                        .id("resp-tab-body")
                                        .px_0p5()
                                        .pb_2()
                                        .text_sm()
                                        .cursor_pointer()
                                        .border_b_2()
                                        .when(self.active_tab == 0, |this| {
                                            this.border_color(theme.primary)
                                                .text_color(theme.primary)
                                                .font_weight(FontWeight::SEMIBOLD)
                                        })
                                        .when(self.active_tab != 0, |this| {
                                            this.border_color(gpui::transparent_black())
                                                .text_color(theme.muted_foreground)
                                        })
                                        .on_click(cx.listener(
                                            |this, _event: &gpui::ClickEvent, _window, cx| {
                                                this.active_tab = 0;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Body"),
                                )
```
对 Headers(`active_tab == 1`,id `"resp-tab-headers"`,文案 `"Headers"`)做相同替换。并把这两个 tab 的父横排容器 `div().flex().flex_row().gap_1()` 改为 `div().flex().flex_row().gap_5().border_b_1().border_color(theme.border)`(与 Task 5 Step 2 一致)。

- [ ] **Step 5: 确认 `FontWeight` 导入**

`response_viewer.rs` 顶部已 `use gpui::*;`(含 `FontWeight`、`transparent_black`)。无需新增。

- [ ] **Step 6: `cargo check`**

Run: `cargo check`
Expected: 退出 0,无新增 warning。

- [ ] **Step 7: Commit**

```bash
git add src/response_viewer.rs
git commit -m "feat(ui): Response status pill + card-framed body + underline tabs

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 8: (最终检查点)Windows 真机完整验收**

```powershell
$env:GPUI_FXC_PATH = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\fxc.exe"
cargo build --release
```
然后运行 `target\release\poopman.exe`,对照 `2026-06-17-ui-redesign-mockup.html` 逐项核对(见 spec「验证策略」6 点):全局暖纸 + 珊瑚橘、历史单行无圆点、内层 tab 下划线(请求区与响应区一致)、URL 栏/Send/卡片观感、响应 pill + 卡片化、切 tab/加载历史样式稳定。

---

## Self-Review

**Spec 覆盖:**
- 全局暖调主题 → Task 1 ✓
- 调色板各 token(含 `card` 用 `popover`/`input` 代替)→ Task 1 apply_theme ✓
- 字体/圆角 → Task 1(radius/radius_lg)✓
- 尺寸常量解决 hardcode → Task 1 定义 + Task 2 替换 ✓(`px(0.)`/`px(0.1)` 明确不动)
- 历史单行 + 去 ● 圆点 + 方法文字标签 + 浅珊瑚选中 → Task 3 ✓
- Tab 栏方法文字标签 + 卡片选中 → Task 4 ✓
- 请求区内层 tab 下划线式 → Task 5 ✓
- Body 次要按钮统一 → Task 6 ✓
- 响应状态 pill + 卡片化 + 下划线 tab → Task 7 ✓
- 范围红线(不做深色/Collections/逻辑)→ 全程未触碰功能逻辑 ✓

**占位符扫描:** 无 TBD/TODO;所有代码步骤含完整可粘贴代码。

**类型/命名一致:** `apply_theme(cx: &mut App)`、`method_color(method: HttpMethod, theme: &Theme) -> Hsla`、尺寸常量名(`SIDEBAR_WIDTH` 等)在 Task 1 定义,Task 2/3/4 引用一致;下划线 tab 模式(`border_b_2` + `when(active)` 着 `primary`)在 Task 5 与 Task 7 写法一致。

**验证现实性:** 已说明 WSL2 仅能 `cargo check`,真机 `cargo build --release` + 目视对照 mockup;`method_color` 的 dead-code 过渡在 Task 1 加 `#[allow]`、Task 4 移除,避免中途 warning。
