# UI 重设计：暖调浅色（Claude 观感）/ 布局 B

**日期**: 2026-06-17
**范围**: 全量 UI 重设计（视觉 + 布局），覆盖所有面板组件 + 新增主题模块
**类型**: 重设计（视觉/布局），非功能性改动
**参考预览**: `docs/superpowers/specs/2026-06-17-ui-redesign-mockup.html`（已确认观感方向）

## 背景与目标

当前 UI「丑」的核心问题(基于真机截图诊断):

1. **配色不统一** —— 同屏混用绿(GET 徽章)、黑(Send / 内层 tab 选中)、蓝(tab 栏/历史选中)三种强调色,无统一主色。
2. **内层 tab 黑色实心药丸** —— 在浅色界面中突兀笨重。
3. **历史卡片过大** —— 每条 ~95px、重边框,密度低。
4. **竖向空间浪费** —— 灰色大写区块标题、过大 padding;状态栏是糙灰块。
5. **输入框灰底偏旧**;小瑕疵(History 旁孤立 ● 圆点、POST 徽章发灰)。

**目标**:重做为 Claude/Anthropic 暖调观感 —— 纸感暖背景、单一珊瑚橘强调色、克制排版、柔和圆角、发丝边框、紧凑而呼吸感的密度。

## 已确认决策

- **风格方向**: Claude/Anthropic 暖调
- **主题范围**: 仅**暖调浅色**(深色为后续 Phase 2,本次不做)
- **重设计程度**: 全量重做布局(非仅换肤)
- **整体布局**: **方案 B** —— 侧边栏(历史)+ 主区(请求在上 / 响应在下,竖向分割)

## 架构

两层实现,关注点分离:

1. **全局主题层** —— 新增 `src/theme.rs`,定义暖调调色板并在 `gpui_component::init` 之后通过 `Theme::global_mut(cx)` 覆盖颜色 token、字体、圆角。`Theme` 通过 `DerefMut` 暴露 `ThemeColor` 全部字段。由于所有组件已读 `cx.theme()`,改 token 即全局生效——这一层就治掉配色不统一(内层 tab 的 `.primary()` 自动变珊瑚、蓝色选中态走 `list_active`/`tab_active` 也跟着统一)。
2. **组件结构层** —— 逐面板调整布局与密度(历史单行化、内层 tab 下划线式、状态栏重做、响应卡片化等)。

另**新增** `src/ui.rs`(或 `theme.rs` 内的子模块)集中存放尺寸常量,替换散落的 `px()` 魔法数字(顺带解决"hardcode"问题)。

### 文件清单

- **新增** `src/theme.rs` —— 调色板常量 + `apply_theme(cx)`(设置颜色/字体/圆角)+ 布局尺寸常量。
- **改** `src/main.rs` —— `gpui_component::init(cx)` 后调用 `crate::theme::apply_theme(cx)`。
- **改** `src/history_panel.rs` —— 紧凑单行、方法文字标签、发丝分隔、浅珊瑚选中、去掉 ● 圆点。
- **改** `src/tab_bar.rs` —— 卡片式 tab(选中态纸白 + 边框,非蓝色块)。
- **改** `src/request_editor.rs` —— URL 栏、内层 tab 下划线式、headers/params 表格留白收紧。
- **改** `src/body_editor.rs` —— 内层(单选/Format/Validate)按钮风格统一为暖调。
- **改** `src/response_viewer.rs` —— 状态栏重做(色块 pill + 元信息)、响应体卡片化、JSON 着色沿用。
- **改** `src/app.rs` —— 复用 `theme.rs` 的尺寸常量替换 `px()` 字面量;侧边栏背景用 `sidebar` token。

## 调色板（warm paper light）

| 用途 | token | 色值 (hex) | 说明 |
|---|---|---|---|
| 主画布 | `background` | `#FAF9F5` | 暖米白纸感 |
| 侧边栏 | `sidebar` | `#F0EEE6` | 略深一档暖纸 |
| 输入框/卡片浮层 | `input` / `popover` | `#FFFFFF` | 纯白浮于纸面(gpui-component 无 `card` token,白色表面用 `input`/`popover`) |
| 主文字 | `foreground` | `#1A1915` | 暖近黑 |
| 次要文字 | `muted_foreground` | `#73706A` | 暖灰 |
| 发丝边框 | `border` / `sidebar_border` | `#E7E4DA` | 极淡暖灰 |
| **主强调色** | `primary` / `ring` / `tab_active` | **`#C15F3C`** | Claude 珊瑚/陶土橘 |
| 强调色 hover | `primary_hover` | `#AD5435` | |
| 强调色上文字 | `primary_foreground` | `#FFFFFF` | |
| 选中/高亮 | `list_active` / `selection` / `accent` | `#F3E7E0` | 浅珊瑚晕染 |
| 选中边框 | `list_active_border` | `#E8D3C8` | |
| 2xx 状态 | `success` | `#4F8A5B` | 收敛暖绿 |
| 4xx/5xx 状态 | `danger` | `#C0503F` | 暖红 |
| 警告/3xx | `warning` | `#C98A3C` | 暖琥珀 |
| 滚动条 thumb | `scrollbar_thumb` | `#D8D4C8` | 暖灰 |

> 颜色以 hex 给出;实现时通过 `gpui::rgb(0xRRGGBB).into()` 转为 `Hsla` 赋给对应 token。未单独列出的 token(如 `secondary`/`accordion`/`tab` 等)统一推导自同一暖色系(浅表面=background/`#FAF9F5`,文字=foreground,边框=border),避免遗漏处露出旧色。注:gpui-component 0.5.1 **无 `card` token**,凡需纯白卡片表面处用 `popover`(浮层)或 `input`(输入)token。

## 字体与形状

- **圆角**: 卡片/响应体 `8px`,输入/按钮/tab `6px`。设 `Theme.radius`。
- **正文字体**: 保留系统 UI sans(不打包字体,避免体积/授权)。
- **等宽字体**: Body/Response 代码区用 `mono_font_family`(保留系统等宽)。

## 各组件结构设计

### 侧边栏 / History (`history_panel.rs`)
- **去掉** "History" 左侧孤立 ● 圆点。
- 每条历史:**单行紧凑**布局 —— 左侧小号等宽**方法文字标签**(`GET` 绿 / `POST` 琥珀 / `DELETE` 红,无实心底色),右侧 URL 单行省略 + 下方小号时间。
- 行间用**发丝分隔/留白**取代重边框卡片;hover 浅灰、选中浅珊瑚晕染(`list_active` + `list_active_border`)。
- 背景使用 `sidebar` token。

### Tab 栏 (`tab_bar.rs`)
- 卡片式 tab:选中态为纸白底 + 上/左/右发丝边框 + 顶部圆角(贴合下方内容区),未选中为透明 + 暖灰文字。
- tab 内方法小标签用语义色;关闭 ✕ 暖灰、hover 转 danger。
- `＋` 新建按钮:暖灰、hover 珊瑚。

### 请求区 (`request_editor.rs`)
- **URL 栏**: method 选择器(纸白卡片 + 边框,方法名语义色)| URL 输入(纸白卡片)| Send 按钮(珊瑚实心 + hover 加深)。
- **内层 tab(Headers/Params/Body)**: **下划线式** —— 选中珊瑚文字 + 珊瑚下划线 + 半粗;未选中暖灰,hover 转主文字。取代当前黑色实心药丸。
- **表格行**: checkbox 选中态珊瑚;输入框纸白 + 发丝边框;只读/预定义项浅灰底 + 暖灰字;行距收紧。

### Body 编辑器 (`body_editor.rs`)
- None/Raw/Form-data 单选、子类型下拉、Format/Validate 按钮统一暖调(次要按钮:透明/浅底 + 暖灰字 + hover 浅珊瑚)。
- 代码编辑区随主题(纸白底、发丝边框、圆角)。

### 响应区 (`response_viewer.rs`)
- **状态栏重做**: `200 OK` 小**色块 pill**(2xx 绿 / 4xx-5xx 红 / 网络错误红),后接 `128 ms`、`1.2 KB` 元信息(数值主文字、单位暖灰),取代当前整条灰块。
- **响应体卡片化**: 纸白卡片 + 发丝边框 + 圆角 + 内边距;沿用 JSON 语法着色(key 珊瑚、字符串绿、数字赭),行号暖灰。
- Body/Headers 切换用下划线式内层 tab(与请求区一致)。
- 空态文案居中、暖灰。

## 尺寸常量（解决 hardcode）

`theme.rs` 中集中定义,替换 `app.rs` / `request_editor.rs` / `body_editor.rs` 的 `px()` 字面量:

| 常量 | 值 | 替换处 |
|---|---|---|
| `SIDEBAR_WIDTH` | 264px | app.rs 历史面板初始宽度(原 280) |
| `SIDEBAR_MIN` / `SIDEBAR_MAX` | 200 / 420 | app.rs 范围 |
| `REQUEST_INITIAL_HEIGHT` | 350px | app.rs |
| `REQUEST_MIN` / `REQUEST_MAX` | 150 / 700 | app.rs |
| `METHOD_SELECT_WIDTH` | 92px | request_editor.rs(原 100) |
| `RAW_SUBTYPE_WIDTH` | 120px | body_editor.rs |

> `px(0.)`/`px(0.1)` 为滚动偏移内部计算,**不**提取(非设计值)。

## 验证策略

- WSL2 无法运行 GUI,自动门禁为 `cargo check`(无新增 warning)。
- Windows 真机 `cargo build --release`(需 `GPUI_FXC_PATH`)产出可执行文件后,**对照参考 HTML 预览**逐面板目视验收:
  1. 全局暖纸背景 + 珊瑚橘强调色,无残留绿/黑/蓝杂色
  2. 历史单行紧凑、无 ● 圆点、选中浅珊瑚
  3. 内层 tab 下划线式(请求区与响应区一致)
  4. URL 栏 / Send 按钮 / 卡片观感符合预览
  5. 响应状态栏 pill + 元信息、响应体卡片化 + JSON 着色
  6. 切 tab / 加载历史后样式稳定,无错位

## 范围红线（YAGNI）

本次**仅**做暖调浅色的视觉 + 布局重设计。**不**包含:深色主题(Phase 2)、主题切换 UI、Collections、环境变量、图标体系重做、动效。功能逻辑(请求发送、同步、历史)一律不动。
