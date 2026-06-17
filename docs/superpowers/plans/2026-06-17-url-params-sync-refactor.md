# URL↔Params 焦点仲裁式同步重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用焦点仲裁取代三个重入脏标志,根治 `RequestEditor` 的 URL↔Params 同步死循环 bug,并修复 `load_request` 误清订阅导致的 body/header 静默失效。

**Architecture:** 谁持有键盘焦点谁就是同步的"驱动方",另一侧的 `Change` 回调因未持有焦点而短路 —— 程序化 `set_value` 仍会发 `Change`,但目标控件无焦点,循环被天然切断。订阅按生命周期分桶:URL/body 订阅常驻,行(header/param)订阅可重建。

**Tech Stack:** Rust, GPUI 0.2.2, gpui-component 0.4.0(`InputState` 实现 `Focusable`,提供公开 `focus_handle(cx).is_focused(window)`)。

**验证约束:** 重构集中在 `src/request_editor.rs`,该文件依赖 GPUI runtime,**WSL2 无法运行 GUI**。每个任务的自动化门禁是 `cargo test`(`url_params.rs` 纯函数单测保持全绿)+ `cargo build`(编译通过);行为正确性由开发者在真机按任务末尾的手动验收清单确认。

**Spec:** `docs/superpowers/specs/2026-06-17-url-params-sync-refactor-design.md`

---

## File Structure

仅改动一个文件:

- **Modify:** `src/request_editor.rs` —— `RequestEditor` 的字段、`new`、`load_request`、`load_params_state`、`load_headers_state`、`add_custom_header_row`、`add_param_row`、`add_param_row_with_values`、`parse_url_to_params`、`sync_params_to_url`、`toggle_param`、`remove_param`,并新增 `rebuild_url_from_params`。
- **不改:** `src/url_params.rs`(纯函数及其单测保持不变,作为回归基线)。

任务顺序:先做订阅分桶(独立修复 bug #2,可单独验证),再做焦点仲裁(删除脏标志)。两步各自能编译、能提交。

---

## Task 1: 订阅分桶 —— 修复 `load_request` 误清订阅(bug #2)

**问题:** `load_request` 调用 `self._subscriptions.clear()` 会把 body 的 `BodyTypeChanged` 订阅一并清掉,之后只重建了 URL 与行订阅,导致切 tab / 点历史后 body 类型切换不再联动 Content-Type。

**方案:** 拆出第二个订阅桶 `_row_subscriptions` 专放 header/param 行的订阅;URL、body 订阅留在常驻的 `_subscriptions` 里,`load_request` 不再清除它们,因此也不需要在 `load_request` / `load_params_state` 里重新订阅 URL。

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: 给结构体加 `_row_subscriptions` 字段**

在 `src/request_editor.rs` 的 `RequestEditor` 结构体里,把:

```rust
    _subscriptions: Vec<Subscription>,
```

改为:

```rust
    _subscriptions: Vec<Subscription>,       // 常驻订阅:URL 输入、body editor
    _row_subscriptions: Vec<Subscription>,   // 行订阅:header/param 行,load 时可重建
```

- [ ] **Step 2: 在 `new()` 初始化新字段**

在 `new()` 的结构体字面量里,把:

```rust
            _subscriptions: vec![],
```

改为:

```rust
            _subscriptions: vec![],
            _row_subscriptions: vec![],
```

- [ ] **Step 3: 把所有"行订阅"的 push 改到新桶**

在 `add_custom_header_row` 中,把:

```rust
        self._subscriptions.push(sub);
```
改为:
```rust
        self._row_subscriptions.push(sub);
```

在 `load_headers_state` 的循环内,把:
```rust
                self._subscriptions.push(sub);
```
改为:
```rust
                self._row_subscriptions.push(sub);
```

在 `add_param_row_with_values` 中,把:
```rust
        self._subscriptions.push(sub1);
        self._subscriptions.push(sub2);
```
改为:
```rust
        self._row_subscriptions.push(sub1);
        self._row_subscriptions.push(sub2);
```

在 `add_param_row` 中,把:
```rust
        self._subscriptions.push(sub_key);
        self._subscriptions.push(sub_value);
```
改为:
```rust
        self._row_subscriptions.push(sub_key);
        self._row_subscriptions.push(sub_value);
```

在 `load_params_state` 的循环内,把:
```rust
            self._subscriptions.push(sub1);
            self._subscriptions.push(sub2);
```
改为:
```rust
            self._row_subscriptions.push(sub1);
            self._row_subscriptions.push(sub2);
```

注意:`new()` 里 `editor._subscriptions.push(url_sub);` 和 `editor._subscriptions.push(body_sub);` **保持不变**(它们是常驻订阅)。

- [ ] **Step 4: `load_request` 只清行订阅,且不再重订阅 URL**

在 `load_request` 中,把:

```rust
        // Set headers - reinitialize with predefined headers
        self.headers.clear();
        self._subscriptions.clear();

        // Clear params to force rebuild with fresh subscriptions
        // This is critical because _subscriptions.clear() removes all params subscriptions
        // but self.params still holds old ParamRow entities with dead subscriptions
        self.params.clear();
        self.last_parsed_url.clear(); // Reset to force URL parsing

        // Re-subscribe to URL input for URL → Params sync (cleared above)
        let url_input = self.url_input.clone();
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.parse_url_to_params(window, cx);
        });
        self._subscriptions.push(url_sub);

        // First, add all predefined headers
        self.init_predefined_headers(window, cx);
```

改为:

```rust
        // Set headers - reinitialize with predefined headers
        self.headers.clear();
        // Only clear ROW subscriptions (header/param rows). The permanent URL and body
        // subscriptions in self._subscriptions must survive, otherwise body Content-Type
        // sync and header auto-add silently break after switching tabs / loading history.
        self._row_subscriptions.clear();

        // Clear params to force rebuild with fresh subscriptions
        self.params.clear();
        self.last_parsed_url.clear(); // Reset to force URL parsing

        // First, add all predefined headers
        self.init_predefined_headers(window, cx);
```

(即:`_subscriptions.clear()` → `_row_subscriptions.clear()`;删除整段 URL 重订阅。)

- [ ] **Step 5: `load_params_state` 删除多余的 URL 重订阅**

在 `load_params_state` 末尾,删除这一整段:

```rust
        // Re-subscribe to URL input for URL → Params sync
        // This is needed because load_request() may have cleared it
        let url_input = self.url_input.clone();
        let url_sub = cx.subscribe_in(&url_input, window, |this, _, _event: &InputEvent, window, cx| {
            this.parse_url_to_params(window, cx);
        });
        self._subscriptions.push(url_sub);

        cx.notify();
```

替换为:

```rust
        cx.notify();
```

(URL 订阅现在常驻、永不被清,这里再订阅会造成重复订阅 → 一次输入触发多次解析。)

- [ ] **Step 6: 确认 `url_params` 纯函数单测仍全绿**

Run: `cargo test --lib url_params`
Expected: PASS(所有 `extract_base_url` / `parse_query_params` / `build_url_with_params` / `params_equal` 测试通过,本任务未触及该模块,作回归基线)。

- [ ] **Step 7: 确认编译通过**

Run: `cargo build`
Expected: 编译成功,无 error。可能出现的 warning 在 Task 2 删除脏标志后消除。

- [ ] **Step 8: Commit**

```bash
git add src/request_editor.rs
git commit -m "fix: Preserve URL/body subscriptions across load_request

Split row (header/param) subscriptions into a separate bucket so
load_request no longer wipes the permanent URL and body-editor
subscriptions. Fixes Content-Type sync and header auto-add silently
breaking after tab switch / history load.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

**真机验收(Task 1):** 启动后切换 body 类型(None/Raw/Form-data),Headers 里 Content-Type 应随之变化;从历史打开一条请求或新建/切换 tab 后,再切 body 类型,Content-Type **仍**应联动;custom header 输入框输入内容时应自动追加新空行。

---

## Task 2: 焦点仲裁 —— 删除三个脏标志

**方案:** 每个 `Change` 回调先判断"自己是否持有焦点",不持有则 `return`。新增无门禁的 `rebuild_url_from_params` 供按钮回调直接调用。一次性删除 `updating_url`、`parsing_url`、`last_parsed_url`(此三者跨多个函数耦合,必须在同一改动内完成才能编译)。

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: 从结构体删除三个标志字段**

在 `RequestEditor` 结构体里删除这三行:

```rust
    updating_url: bool, // Flag to prevent infinite loop between URL and params updates
    parsing_url: bool, // Flag to prevent syncing back to URL while parsing URL to params
    last_parsed_url: String, // Last URL that was parsed to params
```

- [ ] **Step 2: 从 `new()` 删除三个标志的初始化**

在 `new()` 结构体字面量里删除这三行:

```rust
            updating_url: false,
            parsing_url: false,
            last_parsed_url: String::new(),
```

- [ ] **Step 3: 从 `load_request` 删除 `last_parsed_url.clear()`**

删除这一行(Task 1 已保留它,现在随字段一起移除):

```rust
        self.last_parsed_url.clear(); // Reset to force URL parsing
```

- [ ] **Step 4: 从 `load_params_state` 删除 `parsing_url` 的读写**

删除这一行:
```rust
        // Set flag to prevent syncing back to URL while we're building params
        self.parsing_url = true;
```
以及这一行:
```rust
        // Reset flag after params are built
        self.parsing_url = false;
```
(连同其上方注释一并删除。加载是程序化操作,新建的 param 输入框不持有焦点,Step 7 的 `sync_params_to_url` 焦点门禁会自动短路,无需此标志。)

- [ ] **Step 5: 重写 `parse_url_to_params`(URL → Params,加焦点门禁)**

把整个 `parse_url_to_params` 函数体替换为:

```rust
    fn parse_url_to_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Focus arbitration: only parse when the URL input is the focused widget.
        // sync_params_to_url's programmatic set_value also emits InputEvent::Change,
        // but the URL input is not focused then, so this returns early and the
        // bidirectional loop is broken without any reentrancy flags.
        if !self.url_input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }

        let url_str = self.url_input.read(cx).value().to_string();
        let new_params = url_params::parse_query_params(&url_str);

        // URL is non-empty but has no query string (user still typing the base URL):
        // keep existing params instead of wiping them.
        if new_params.is_empty()
            && !url_str.is_empty()
            && !url_str.contains('?')
            && !self.params.is_empty()
        {
            return;
        }

        // Skip rebuild if the parsed params match current params (avoids disrupting
        // the user mid-edit and avoids needless entity churn).
        let current_params: Vec<(String, String)> = self
            .params
            .iter()
            .map(|p| {
                (
                    p.key_input.read(cx).value().to_string(),
                    p.value_input.read(cx).value().to_string(),
                )
            })
            .filter(|(k, v)| !k.is_empty() || !v.is_empty())
            .collect();
        if url_params::params_equal(&new_params, &current_params) && !self.params.is_empty() {
            return;
        }

        // Rebuild params list from the URL query string.
        self.params.clear();
        for (key_str, value_str) in new_params {
            self.add_param_row_with_values(&key_str, &value_str, true, window, cx);
        }
        // Always keep one trailing empty row for adding new params.
        self.add_param_row(window, cx);

        cx.notify();
    }
```

- [ ] **Step 6: 新增 `rebuild_url_from_params`(无门禁执行版)**

在 `sync_params_to_url` 函数**之前**插入这个新函数:

```rust
    /// Rebuild the URL input from the current params list. No focus gating.
    ///
    /// Used both by `sync_params_to_url` (the focus-gated wrapper for text edits)
    /// and directly by button callbacks (toggle/remove), where no text input holds
    /// focus. The resulting `set_value` emits InputEvent::Change, but the URL input
    /// is not focused, so `parse_url_to_params` short-circuits — no loop.
    fn rebuild_url_from_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_url = self.url_input.read(cx).value().to_string();
        let new_url = self.rebuild_url_with_params(&current_url, cx);
        self.url_input.update(cx, |input, cx| {
            input.set_value(&new_url, window, cx);
        });
    }
```

(复用既有的纯构造函数 `rebuild_url_with_params(&self, url_str, cx) -> String`,该函数保持不变。)

- [ ] **Step 7: 重写 `sync_params_to_url`(Params → URL,加焦点门禁)**

把整个 `sync_params_to_url` 函数体替换为:

```rust
    fn sync_params_to_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Focus arbitration: only sync when a param input is the focused widget.
        // Otherwise this Change was triggered by a programmatic set_value (e.g. from
        // parse_url_to_params rebuilding rows), and syncing back would loop.
        let param_focused = self.params.iter().any(|p| {
            p.key_input.read(cx).focus_handle(cx).is_focused(window)
                || p.value_input.read(cx).focus_handle(cx).is_focused(window)
        });
        if !param_focused {
            return;
        }

        self.rebuild_url_from_params(window, cx);
    }
```

- [ ] **Step 8: `toggle_param` / `remove_param` 改用无门禁版**

按钮点击时焦点不在任何 param 文本框上,必须绕过 `sync_params_to_url` 的门禁。

在 `toggle_param` 中,把:
```rust
            self.sync_params_to_url(window, cx);
```
改为:
```rust
            self.rebuild_url_from_params(window, cx);
```

在 `remove_param` 中,把:
```rust
            self.sync_params_to_url(window, cx);
```
改为:
```rust
            self.rebuild_url_from_params(window, cx);
```

- [ ] **Step 9: 确认三个标志已彻底移除**

Run: `grep -n "updating_url\|parsing_url\|last_parsed_url" src/request_editor.rs`
Expected: 无任何输出(空结果)。若有残留,按提示删除对应行。

- [ ] **Step 10: 确认 `url_params` 纯函数单测仍全绿**

Run: `cargo test --lib url_params`
Expected: PASS(回归基线,本任务未触及该模块)。

- [ ] **Step 11: 确认编译通过且无 dead-code warning**

Run: `cargo build 2>&1 | grep -i "warning\|error" || echo "clean"`
Expected: 无与本次改动相关的 error;不再出现 `updating_url`/`parsing_url`/`last_parsed_url` 相关 warning。

- [ ] **Step 12: Commit**

```bash
git add src/request_editor.rs
git commit -m "refactor: Replace URL/Params sync flags with focus arbitration

Drop updating_url/parsing_url/last_parsed_url. Each Change handler now
acts only when its own input holds keyboard focus; programmatic set_value
still emits Change but the target is unfocused, so the bidirectional loop
is broken structurally. Button callbacks (toggle/remove) call the new
unguarded rebuild_url_from_params directly.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

**真机验收(Task 2,完整清单):**
1. URL 框输入 `http://x.com?a=1&b=2` → Params 标签出现 a=1、b=2 两行。
2. Params 标签新增/修改一行 → URL 框查询串实时同步。
3. 勾选/取消某个 param(toggle)→ URL 即时增删该参数。
4. 删除某个 param 行(×)→ URL 即时移除该参数。
5. URL 框连续打字 → 无字符回弹、光标跳动、输入卡顿。
6. 从历史打开请求 / 多 tab 切换后:body 可编辑、切 body 类型时 Content-Type 仍联动、custom header 输入仍自动追加新行。

---

## Self-Review 结论

- **Spec 覆盖:** 焦点不变量(Task 2 Step 5/7)、删除三标志(Task 2 Step 1-4/9)、新增 `rebuild_url_from_params`(Task 2 Step 6)、toggle/remove 绕过门禁(Task 2 Step 8)、bug #2 订阅分桶(Task 1 全部)、纯函数单测保绿(两任务的测试步)、真机验收清单(两任务末尾)—— 均有对应步骤。
- **占位符:** 无 TBD/TODO,所有代码步骤含完整代码。
- **类型一致:** `rebuild_url_from_params`(新增,`&mut self`)与既有 `rebuild_url_with_params`(`&self -> String`)命名区分明确且全程一致;`_row_subscriptions` 在所有 push 点与 clear 点拼写一致。
