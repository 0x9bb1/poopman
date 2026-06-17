# URL↔Params 焦点仲裁式同步重构

**日期**: 2026-06-17
**范围**: `src/request_editor.rs`(同步逻辑),关联修复 `load_request` 的订阅清空 bug
**类型**: 重构(根治状态同步缺陷),非新功能

## 背景与问题

`RequestEditor` 用 GPUI 的 `subscribe` 事件做 URL 输入框与 Params 列表的双向同步:

- URL `Change` → `parse_url_to_params`(URL → Params)
- 每个 param 输入框 `Change` → `sync_params_to_url`(Params → URL)

由于程序化 `set_value` **也会触发 `InputEvent::Change`**(经 gpui-component 0.4.0 源码确认:`set_value` → `replace_text` → `replace_text_in_range_silent`,其中 `silent_replace_text` 仅屏蔽 LSP 补全,`cx.emit(InputEvent::Change)` 在该判断之外照常发出),两个方向会互相触发形成无限循环。

现状用三个布尔/字符串脏标志压制循环:`updating_url`、`parsing_url`、`last_parsed_url`。这是"打补丁防循环"的反模式——每加一个交互就要新增/调整 flag 判断,是该组件"修不完的 bug"的根源。

## 目标

1. 用**真实焦点状态**取代脏标志,彻底删除 `updating_url`、`parsing_url`、`last_parsed_url`。
2. 保留 Postman 式的实时双向联动体验。
3. 顺手修复同源 bug:`load_request` 中 `_subscriptions.clear()` 误清 body/method/custom-header 订阅,导致切 tab / 点历史后这些功能静默失效。

## 设计

### 核心不变量

> 任一输入框的 `Change` 回调,只有当**该输入框当前持有键盘焦点**时,才执行其同步动作;否则立即 `return`。

理由:用户编辑某个输入框时,该框持有焦点,它是"驱动方",可以把变更推送到另一侧。程序化 `set_value` 写入的那一侧此刻**没有焦点**,其 `Change` 回调因焦点检查而短路,循环被天然切断,无需任何重入标志。

焦点查询通过公开 API:`InputState` 实现了 `gpui::Focusable`,可用
`input.read(cx).focus_handle(cx).is_focused(window)` 查询。

### 状态变更

`RequestEditor` 结构体:

- **删除字段**:`updating_url: bool`、`parsing_url: bool`、`last_parsed_url: String`。
- **不新增字段**:焦点状态实时查询,不缓存。

### 函数级改动

#### `parse_url_to_params`(URL → Params)

- 开头改为:`if !self.url_input.read(cx).focus_handle(cx).is_focused(window) { return; }`
- 删除所有 `updating_url` / `parsing_url` 判断与 `last_parsed_url` 读写。
- **保留**内容 diff 检查:用 `url_params::params_equal` 比较新解析结果与当前 params,相等则不重建,避免无谓重建打断用户输入。
- 不相等时重建 params 行(沿用现有 `add_param_row_with_values` + 末尾空行逻辑)。

#### `rebuild_url_from_params`(新增,Params → URL 的纯执行版)

- 从 `self.params` 读出 `Vec<QueryParam>`,取当前 URL 的 base,调用 `url_params::build_url_with_params` 得到新 URL,`set_value` 写回 `url_input`。
- **不含**任何焦点判断——供按钮类回调(无文本框焦点的场景)直接调用。
- 复用现有 `rebuild_url_with_params` 纯函数构造逻辑。

#### `sync_params_to_url`(Params → URL 的带门禁包装)

- 开头改为:仅当**存在任意 param 输入框持有焦点**时才继续,否则 `return`。
- 通过后调用 `rebuild_url_from_params`。
- 删除 flag 写入。
- 供 param 输入框的 `Change` 订阅调用。

#### `toggle_param` / `remove_param`(按钮回调)

- 这些由点击触发,焦点不在文本框上,因此**直接调用 `rebuild_url_from_params`**(绕过焦点门禁),而非 `sync_params_to_url`。

### 关联修复:`load_request` 订阅管理(bug #2)

当前 `load_request` 调用 `self._subscriptions.clear()`(request_editor.rs:167),把 body 的 `BodyTypeChanged` 订阅、custom-header 的 auto-add 订阅一并清除,之后只重建了 URL 与 params 订阅,导致:

- 切 tab / 点历史后 body 编辑器的 Content-Type 联动失效;
- custom header 的"输入即自动追加新行"失效。

**修复方向**:按来源分组管理订阅,不再一刀切 `clear()`。

- URL 订阅、body 订阅在 `RequestEditor::new` 中建立后**常驻**,`load_request` 不清除。
- params 订阅在每次重建 params 列表时重建。
- custom-header auto-add 订阅随各 header 行的生命周期管理。

具体实现细节(用单独的 `Vec<Subscription>` 分桶,还是用 `params_subscriptions` 独立字段)留待实现计划阶段确定,以最小改动为准。

## 测试策略

- `src/url_params.rs` 既有纯函数单测必须保持全绿(`cargo test`)。
- 焦点仲裁逻辑依赖 GPUI runtime,WSL2 环境无法运行 GUI,这部分由开发者在真机按下方验收清单手动验证。

### 真机验收清单

1. 在 URL 框输入 `http://x.com?a=1&b=2`,切到 Params 标签,应看到 a=1、b=2 两行。
2. 在 Params 标签新增/修改一行,URL 框应实时同步更新查询串。
3. 勾选/取消勾选某个 param(toggle),URL 应即时增删该参数。
4. 删除某个 param 行(×),URL 应即时移除该参数。
5. 在 URL 框连续打字,不出现字符回弹、光标跳动、输入卡顿。
6. 从历史打开一条请求,或在多个 tab 间切换后:body 编辑器仍可编辑、切换 body 类型时 Content-Type header 仍自动联动、custom header 输入仍会自动追加新行。

## 范围红线(YAGNI)

本次**仅**重构同步机制并修复 bug #2。**不**包含:form-data multipart 真实发送、gzip 解压、请求超时、二进制响应处理、Collections、环境变量、认证。这些为后续独立任务,各自单独 spec。
