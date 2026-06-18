# 环境变量 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 Postman 式环境变量:多命名环境 + 当前激活,请求中的 `{{var}}` 在发送时按激活环境替换,带管理 UI,持久化到 SQLite。

**Architecture:** 四层、自底向上、可逐步验证:① 纯函数替换引擎(`variables.rs`)② 数据模型(`types.rs`)+ 持久化(`db.rs` 新表/方法)③ 发送集成(`RequestEditor` 持 `env_vars`,发送时替换)④ UI(标题栏环境选择器 + 管理 Dialog,由 `PoopmanApp` 协调)。

**Tech Stack:** Rust, GPUI 0.2.2, gpui-component 0.5.1, rusqlite 0.40。已核实:`WindowExt::open_dialog(cx, |Dialog,&mut Window,&mut App|->Dialog)`;`Dialog: ParentElement`(`.child`);`Dialog::new(window, cx).title(..).child(..).footer(..)`。

**Spec:** `docs/superpowers/specs/2026-06-18-environment-variables-design.md`

**验证约束:** WSL2 无法链接/运行 GUI(`cargo test` 也因链接失败跑不了)。门禁是 `cargo check` 与 `cargo check --tests`(编译 + 测试编译,无新增 warning;既有 2 个 dead-code warning 除外:`toggle_formdata_type`、`from_request`)。纯逻辑单测(`variables`、`db`)在 **Windows 真机** `cargo test` 跑;UI 在真机对照 spec 验收清单目视。

---

## File Structure

- **新增** `src/variables.rs` — 纯函数 `substitute` + 单测。
- **新增** `src/environment_manager.rs` — 管理 UI 组件(`EnvironmentManager` Entity:环境列表 + 变量表)。
- **改** `src/types.rs` — `Environment`、`EnvVar` 模型。
- **改** `src/db.rs` — 新表(environments / env_variables / app_meta)、`PRAGMA foreign_keys`、环境 CRUD + 激活环境读写 + 单测。
- **改** `src/main.rs` — 声明 `mod variables; mod environment_manager;`。
- **改** `src/request_editor.rs` — `env_vars` 字段 + `set_env_vars` + 发送时替换。
- **改** `src/app.rs` — 持有环境状态 + `EnvironmentManager`,标题栏环境选择器,切换/编辑后推送 `env_vars`,打开管理 Dialog。

---

## Task 1: 替换引擎 `variables.rs`

**Files:**
- Create: `src/variables.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: 写 `src/variables.rs`(实现 + 单测)**

```rust
//! Pure `{{variable}}` substitution used at request-send time.

use std::collections::HashMap;

/// Replace `{{key}}` / `{{ key }}` (key trimmed) with values from `vars`.
///
/// - Unknown variables are left literal (so a typo / missing var is visible).
/// - Non-recursive: substituted values are not themselves re-scanned.
/// - An unclosed `{{` is emitted literally.
pub fn substitute(input: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        if let Some(close) = after.find("}}") {
            let key = after[..close].trim();
            match vars.get(key) {
                Some(val) => out.push_str(val),
                None => {
                    // keep the original token literally
                    out.push_str("{{");
                    out.push_str(&after[..close]);
                    out.push_str("}}");
                }
            }
            rest = &after[close + 2..];
        } else {
            // unclosed "{{" — emit the rest literally and stop
            out.push_str("{{");
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn replaces_known_var() {
        assert_eq!(substitute("{{a}}", &vars(&[("a", "1")])), "1");
    }

    #[test]
    fn trims_inner_whitespace() {
        assert_eq!(substitute("{{ a }}", &vars(&[("a", "1")])), "1");
    }

    #[test]
    fn replaces_multiple_and_keeps_surrounding_text() {
        assert_eq!(
            substitute("x{{a}}y{{b}}z", &vars(&[("a", "1"), ("b", "2")])),
            "x1y2z"
        );
    }

    #[test]
    fn unknown_var_left_literal() {
        assert_eq!(substitute("{{missing}}", &vars(&[])), "{{missing}}");
    }

    #[test]
    fn no_vars_unchanged() {
        assert_eq!(substitute("plain text", &vars(&[])), "plain text");
    }

    #[test]
    fn non_recursive() {
        // value containing {{b}} must NOT be re-substituted
        assert_eq!(
            substitute("{{a}}", &vars(&[("a", "{{b}}"), ("b", "X")])),
            "{{b}}"
        );
    }

    #[test]
    fn unclosed_brace_is_literal() {
        assert_eq!(substitute("{{ unclosed", &vars(&[])), "{{ unclosed");
    }
}
```

- [ ] **Step 2: 在 `main.rs` 声明模块**

在 `src/main.rs` 模块声明区(`mod ...;` 那组,保持字母序附近)加入:

```rust
mod variables;
```

- [ ] **Step 3: 编译 + 测试编译**

Run: `cargo check --tests`
Expected: 退出 0,无新增 warning(除既有 2 个)。

- [ ] **Step 4: Commit**

```bash
git add src/variables.rs src/main.rs
git commit -m "feat(env): Add {{variable}} substitution engine

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: 数据模型 `types.rs`

**Files:**
- Modify: `src/types.rs`

- [ ] **Step 1: 在 `src/types.rs` 末尾(`HeaderState` 结构体之后)追加模型**

```rust
/// A named environment holding a set of variables.
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub variables: Vec<EnvVar>,
}

/// A single environment variable (key/value, toggleable).
#[derive(Debug, Clone)]
pub struct EnvVar {
    pub enabled: bool,
    pub key: String,
    pub value: String,
}
```

- [ ] **Step 2: 编译**

Run: `cargo check`
Expected: 退出 0(`Environment`/`EnvVar` 暂未使用会有 dead-code warning —— 这是预期的临时状态,Task 3 起即被使用;**本步不要加 `#[allow]`**,Task 3 完成后 warning 自然消失。若执行顺序导致中途红线,可临时 `#[allow(dead_code)]` 并在 Task 3 移除)。

- [ ] **Step 3: Commit**

```bash
git add src/types.rs
git commit -m "feat(env): Add Environment and EnvVar models

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: 持久化 `db.rs`

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: 启用外键 + 建表**

在 `Database::new` 中,`let conn = Connection::open(&db_path)?;` 之后、建 history 表之前,加入启用外键:

```rust
        // Foreign keys are off per-connection by default in SQLite.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
```

在 history 表与其索引创建之后(`Ok(Self { ... })` 之前),加入新表:

```rust
        conn.execute(
            "CREATE TABLE IF NOT EXISTS environments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS env_variables (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
                enabled INTEGER NOT NULL DEFAULT 1,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;
```

- [ ] **Step 2: 引入模型到 db.rs 的 use**

把 `src/db.rs` 顶部的 `use crate::types::{HistoryItem, HttpMethod, RequestData};` 改为:

```rust
use crate::types::{Environment, EnvVar, HistoryItem, HttpMethod, RequestData};
```

- [ ] **Step 3: 在 `impl Database` 内追加环境 CRUD + 激活环境方法**

```rust
    /// Load all environments (with their variables), ordered by position.
    pub fn load_environments(&self) -> Result<Vec<Environment>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt =
            conn.prepare("SELECT id, name FROM environments ORDER BY position, id")?;
        let env_rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut result = Vec::with_capacity(env_rows.len());
        for (id, name) in env_rows {
            let mut vstmt = conn.prepare(
                "SELECT enabled, key, value FROM env_variables
                 WHERE environment_id = ?1 ORDER BY position, id",
            )?;
            let variables = vstmt
                .query_map([id], |row| {
                    Ok(EnvVar {
                        enabled: row.get::<_, i64>(0)? != 0,
                        key: row.get(1)?,
                        value: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            result.push(Environment { id, name, variables });
        }
        Ok(result)
    }

    /// Create a new (empty) environment, returning its id.
    pub fn create_environment(&self, name: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO environments (name, position)
             VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM environments))",
            params![name],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn rename_environment(&self, id: i64, name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE environments SET name = ?1 WHERE id = ?2", params![name, id])?;
        Ok(())
    }

    pub fn delete_environment(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // env_variables rows are removed by ON DELETE CASCADE (foreign_keys = ON).
        conn.execute("DELETE FROM environments WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Replace all variables of an environment in a single transaction.
    pub fn replace_variables(&self, environment_id: i64, vars: &[EnvVar]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM env_variables WHERE environment_id = ?1",
            params![environment_id],
        )?;
        for (position, v) in vars.iter().enumerate() {
            tx.execute(
                "INSERT INTO env_variables (environment_id, enabled, key, value, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![environment_id, v.enabled as i64, v.key, v.value, position as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Active environment id, or None for "No Environment".
    pub fn get_active_environment_id(&self) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'active_environment_id'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|s| s.parse::<i64>().ok()))
    }

    pub fn set_active_environment_id(&self, id: Option<i64>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        match id {
            Some(id) => {
                conn.execute(
                    "INSERT INTO app_meta (key, value) VALUES ('active_environment_id', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![id.to_string()],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM app_meta WHERE key = 'active_environment_id'",
                    [],
                )?;
            }
        }
        Ok(())
    }
```

- [ ] **Step 4: 引入 `OptionalExtension`(`.optional()` 需要)**

把 `src/db.rs` 顶部的 `use rusqlite::{params, Connection};` 改为:

```rust
use rusqlite::{params, Connection, OptionalExtension};
```

- [ ] **Step 5: 追加 db 单测(真机 `cargo test` 验证;WSL2 仅编译)**

在 `src/db.rs` 末尾加入(用内存库,避免落盘):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute(
            "CREATE TABLE environments (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0)",
            [],
        ).unwrap();
        conn.execute(
            "CREATE TABLE env_variables (id INTEGER PRIMARY KEY AUTOINCREMENT, environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE, enabled INTEGER NOT NULL DEFAULT 1, key TEXT NOT NULL, value TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0)",
            [],
        ).unwrap();
        conn.execute(
            "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        ).unwrap();
        Database { conn: std::sync::Arc::new(std::sync::Mutex::new(conn)) }
    }

    #[test]
    fn crud_and_active() {
        let db = mem_db();
        let id = db.create_environment("dev").unwrap();
        db.replace_variables(id, &[
            EnvVar { enabled: true, key: "baseUrl".into(), value: "http://x".into() },
            EnvVar { enabled: false, key: "token".into(), value: "abc".into() },
        ]).unwrap();

        let envs = db.load_environments().unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "dev");
        assert_eq!(envs[0].variables.len(), 2);
        assert_eq!(envs[0].variables[0].key, "baseUrl");
        assert!(!envs[0].variables[1].enabled);

        db.rename_environment(id, "staging").unwrap();
        assert_eq!(db.load_environments().unwrap()[0].name, "staging");

        assert_eq!(db.get_active_environment_id().unwrap(), None);
        db.set_active_environment_id(Some(id)).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), Some(id));
        db.set_active_environment_id(None).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), None);

        db.delete_environment(id).unwrap();
        assert!(db.load_environments().unwrap().is_empty());
    }
}
```

> 注:测试用 `Database { conn: ... }` 直接构造,要求 `conn` 字段在同 crate 可见(它是私有字段,但测试在 `db.rs` 内的子模块,可访问私有字段)。

- [ ] **Step 6: 编译 + 测试编译**

Run: `cargo check --tests`
Expected: 退出 0,无新增 warning。

- [ ] **Step 7: Commit**

```bash
git add src/db.rs
git commit -m "feat(env): Persist environments and active selection in SQLite

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: 发送集成（`request_editor.rs`）

**Files:**
- Modify: `src/request_editor.rs`

- [ ] **Step 1: 加 `env_vars` 字段**

在 `RequestEditor` 结构体里(`_row_subscriptions` 之后)加:

```rust
    /// Active environment variables, pushed by PoopmanApp; used at send time.
    env_vars: std::collections::HashMap<String, String>,
```

- [ ] **Step 2: 在 `new()` 初始化字段**

在 `new()` 的结构体字面量里(`_row_subscriptions: vec![],` 之后)加:

```rust
            env_vars: std::collections::HashMap::new(),
```

- [ ] **Step 3: 加 `set_env_vars` 方法**

在 `impl RequestEditor` 内(`get_current_request_data` 附近)加:

```rust
    /// Replace the active environment variable map (called by PoopmanApp).
    pub fn set_env_vars(&mut self, vars: std::collections::HashMap<String, String>) {
        self.env_vars = vars;
    }
```

- [ ] **Step 4: 发送时替换 url / headers / body**

在 `send_request` 中,定位到 `cx.spawn_in(window, async move |this, cx| {` **之前**(此时 `url`、`headers`、`body`、`method` 局部变量已构造好,`request` 也已构造)。在构造 `request` 之后、`self.loading = true;` 之前插入替换逻辑,替换 `url`、`headers`、`body`,并用替换后的值重建 `request`。

把这一段:
```rust
        let request = RequestData {
            method,
            url: url.clone(),
            headers: headers.clone(),
            body: body.clone(),
        };

        self.loading = true;
```
替换为:
```rust
        // Substitute {{env vars}} into url / headers / body at send time.
        let env = &self.env_vars;
        let url = crate::variables::substitute(&url, env);
        let headers: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| {
                (
                    crate::variables::substitute(k, env),
                    crate::variables::substitute(v, env),
                )
            })
            .collect();
        let body = match body {
            crate::types::BodyType::Raw { content, subtype } => crate::types::BodyType::Raw {
                content: crate::variables::substitute(&content, env),
                subtype,
            },
            crate::types::BodyType::FormData(rows) => crate::types::BodyType::FormData(
                rows.into_iter()
                    .map(|mut row| {
                        row.key = crate::variables::substitute(&row.key, env);
                        row.value = match row.value {
                            crate::types::FormDataValue::Text(t) => {
                                crate::types::FormDataValue::Text(crate::variables::substitute(&t, env))
                            }
                            other => other, // file path left as-is
                        };
                        row
                    })
                    .collect(),
            ),
            crate::types::BodyType::None => crate::types::BodyType::None,
        };

        // Save the resolved request to history (Postman stores templates, but our
        // history is a sent-request log; storing the resolved one is acceptable —
        // keep the original template request for the in-tab editor state).
        let request = RequestData {
            method,
            url: url.clone(),
            headers: headers.clone(),
            body: body.clone(),
        };

        self.loading = true;
```

> 注:`url`/`headers`/`body` 在 `send_request` 前半段已是 `let` 绑定的 owned 值(`url: String`、`headers: Vec<_>`、`body: BodyType`),此处以 `let` 遮蔽(shadow)为替换后的值;后续 `cx.spawn_in` 闭包按既有逻辑 move 进去,无需改动闭包内部。`FormDataValue` 已在 crate::types。

- [ ] **Step 5: 编译**

Run: `cargo check`
Expected: 退出 0,无新增 warning。(`set_env_vars` 暂未被调用 → 可能 dead-code warning;Task 5 即调用。若中途红线,临时 `#[allow(dead_code)]` 于 `set_env_vars`,Task 5 移除。)

- [ ] **Step 6: Commit**

```bash
git add src/request_editor.rs
git commit -m "feat(env): Substitute {{vars}} into request at send time

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: App 环境状态 + 标题栏选择器（`app.rs`）

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: PoopmanApp 加环境状态字段**

在 `PoopmanApp` 结构体加:

```rust
    environments: Vec<crate::types::Environment>,
    active_environment_id: Option<i64>,
```

- [ ] **Step 2: 在 `new()` 加载环境并初始化 editor 的 env_vars**

在 `PoopmanApp::new` 中,`let db = Arc::new(...)` 之后加载:

```rust
        let environments = db.load_environments().unwrap_or_default();
        let active_environment_id = db.get_active_environment_id().unwrap_or(None);
```

在 `request_editor` 创建之后(`cx.new(|cx| RequestEditor::new(...))` 之后),把激活变量推给它。先在 `impl PoopmanApp` 加一个纯函数 helper:

```rust
    /// Build the active environment's enabled variables as a flat map.
    fn active_env_vars(
        environments: &[crate::types::Environment],
        active_id: Option<i64>,
    ) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if let Some(id) = active_id {
            if let Some(env) = environments.iter().find(|e| e.id == id) {
                for v in &env.variables {
                    if v.enabled && !v.key.is_empty() {
                        map.insert(v.key.clone(), v.value.clone());
                    }
                }
            }
        }
        map
    }
```

在 `new()` 里 request_editor 创建后:

```rust
        let initial_env_vars = Self::active_env_vars(&environments, active_environment_id);
        request_editor.update(cx, |editor, _| editor.set_env_vars(initial_env_vars));
```

并在 `Self { ... }` 字面量加入 `environments,` 和 `active_environment_id,`。

- [ ] **Step 3: 加切换激活环境的方法**

在 `impl PoopmanApp` 加:

```rust
    /// Switch the active environment, persist it, and push vars to the editor.
    fn set_active_environment(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        self.active_environment_id = id;
        if let Err(e) = self.db.set_active_environment_id(id) {
            log::error!("Failed to persist active environment: {}", e);
        }
        let vars = Self::active_env_vars(&self.environments, id);
        self.request_editor.update(cx, |editor, _| editor.set_env_vars(vars));
        cx.notify();
    }

    /// Reload environments from the DB and refresh the editor's active vars.
    /// Call after the management dialog edits environments.
    fn reload_environments(&mut self, cx: &mut Context<Self>) {
        self.environments = self.db.load_environments().unwrap_or_default();
        // Drop active selection if the environment no longer exists.
        if let Some(id) = self.active_environment_id {
            if !self.environments.iter().any(|e| e.id == id) {
                self.active_environment_id = None;
                let _ = self.db.set_active_environment_id(None);
            }
        }
        let vars = Self::active_env_vars(&self.environments, self.active_environment_id);
        self.request_editor.update(cx, |editor, _| editor.set_env_vars(vars));
        cx.notify();
    }
```

> `self.db` 已是 `Arc<Database>` 字段(`#[allow(dead_code)]` 标注的 `db`)。本任务起 `db` 被实际使用,可移除该 `#[allow(dead_code)]`。

- [ ] **Step 4: 标题栏渲染环境选择器**

在 `render` 中,当前 TitleBar 是:
```rust
                TitleBar::new().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("Poopman"),
                ),
```
改为在标题旁加环境选择器(用 gpui-component `PopupMenu` 弹出环境列表 + Manage)。把上面替换为:

```rust
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child("Poopman"),
                        )
                        .child(self.render_env_selector(cx)),
                ),
```

并在 `impl PoopmanApp` 加 `render_env_selector`,用一个按钮显示当前环境名,点击打开一个列出环境 + "Manage…" 的 `PopupMenu`(gpui-component `popup_menu`)。实现:

```rust
    fn render_env_selector(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active_name = self
            .active_environment_id
            .and_then(|id| self.environments.iter().find(|e| e.id == id))
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "No Environment".to_string());

        let environments = self.environments.clone();

        Button::new("env-selector")
            .ghost()
            .small()
            .label(active_name)
            .popup_menu(move |mut menu, _window, _cx| {
                menu = menu.menu(
                    "No Environment",
                    Box::new(SetActiveEnv { id: None }),
                );
                for env in &environments {
                    menu = menu.menu(
                        env.name.clone(),
                        Box::new(SetActiveEnv { id: Some(env.id) }),
                    );
                }
                menu.separator()
                    .menu("Manage Environments…", Box::new(OpenEnvManager))
            })
    }
```

> 此处需要两个 gpui Action 与监听:`SetActiveEnv { id: Option<i64> }` 与 `OpenEnvManager`,以及 `Button::popup_menu` 的可用性。**实现者须先核实 gpui-component `Button` 是否提供 `.popup_menu(..)` 以及 `PopupMenu::menu` 的确切签名(`src/popup_menu.rs`、`src/button/`),按其实际 API 调整**;若 `Button` 无 `popup_menu`,改用独立 `PopupMenu` 组件或一个点击直接打开管理 Dialog 的简化按钮(见下方降级方案)。Action 定义示例:
> ```rust
> use gpui::actions; // or define via #[derive(Action)]
> ```
> **降级方案(若 PopupMenu 接线复杂)**:环境选择器按钮点击直接打开管理 Dialog;在 Dialog 内提供"设为激活"。先保证可用,再美化为下拉。

- [ ] **Step 5: 处理选择器动作**(切换环境 / 打开管理)

为上面的 action 注册处理(在 render 的根元素上 `.on_action(cx.listener(...))`),或在降级方案里用 `Button::on_click` 直接 `self.open_env_manager(window, cx)`。切换环境调用 `self.set_active_environment(id, cx)`。打开管理调用 Task 6 的 `open_env_manager`。

- [ ] **Step 6: 编译**

Run: `cargo check`
Expected: 退出 0,无新增 warning。

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "feat(env): App-level environment state + title-bar selector

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: 管理组件 + Dialog（`environment_manager.rs` + `app.rs`）

**Files:**
- Create: `src/environment_manager.rs`
- Modify: `src/main.rs`, `src/app.rs`

- [ ] **Step 1: 声明模块**

`src/main.rs` 加:

```rust
mod environment_manager;
```

- [ ] **Step 2: 创建 `EnvironmentManager` 组件**

`src/environment_manager.rs`:一个 `Entity` 组件,持有 `Arc<Database>`、环境列表、当前编辑中的环境 id、变量行(每行 enabled + key `InputState` + value `InputState`)。发出一个 `EnvironmentsChanged` 事件,供 `PoopmanApp` 订阅后 `reload_environments`。

结构与交互**复用 `request_editor.rs` 的 headers 表模式**(`InputState` 行 + 启用勾选 + 删除 + 末尾自动空行;参见 `request_editor.rs` 的 `add_custom_header_row` / 渲染 headers 那段)。关键点:

```rust
use gpui::*;
use gpui_component::{button::*, checkbox::Checkbox, input::*, h_flex, v_flex, ActiveTheme as _, Sizable as _};
use std::sync::Arc;
use crate::db::Database;
use crate::types::EnvVar;

/// Emitted when environments are created/renamed/deleted/saved so the app reloads.
#[derive(Clone)]
pub struct EnvironmentsChanged;

pub struct EnvironmentManager {
    db: Arc<Database>,
    environments: Vec<crate::types::Environment>,
    selected_env_id: Option<i64>,
    name_input: Entity<InputState>,         // rename of selected
    var_rows: Vec<VarRow>,                  // editable rows for selected env
    _subscriptions: Vec<Subscription>,
}

struct VarRow {
    enabled: bool,
    key_input: Entity<InputState>,
    value_input: Entity<InputState>,
}

impl EventEmitter<EnvironmentsChanged> for EnvironmentManager {}

impl EnvironmentManager {
    pub fn new(db: Arc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self { /* load envs, select first, build rows */ }

    fn select_env(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) { /* load that env's vars into var_rows + name_input */ }

    fn add_env(&mut self, window: &mut Window, cx: &mut Context<Self>) { /* db.create_environment("New Environment"); reload; select; emit */ }

    fn delete_env(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) { /* db.delete_environment; reload; emit */ }

    fn save_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_env_id {
            let name = self.name_input.read(cx).value().to_string();
            let _ = self.db.rename_environment(id, &name);
            let vars: Vec<EnvVar> = self.var_rows.iter().map(|r| EnvVar {
                enabled: r.enabled,
                key: r.key_input.read(cx).value().to_string(),
                value: r.value_input.read(cx).value().to_string(),
            }).filter(|v| !v.key.is_empty() || !v.value.is_empty()).collect();
            let _ = self.db.replace_variables(id, &vars);
            cx.emit(EnvironmentsChanged);
        }
    }
}

impl Render for EnvironmentManager {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Left: environment list (each selectable; add button; delete).
        // Right: name input + variable table (enabled checkbox + key + value + delete + trailing empty row).
        // Reuse the warm theme tokens + the headers-table row layout from request_editor.rs.
        // h_flex().gap_4()... (left list) ... (right editor)
        v_flex() /* ...full layout per spec... */
    }
}
```

> 实现者**逐方法补全函数体**(上面给了签名与 `save_selected` 完整实现作范式),变量行的增删/自动空行/渲染直接照搬 `request_editor.rs` 的 header 行做法。`db` CRUD 方法签名见 Task 3。

- [ ] **Step 3: PoopmanApp 持有 manager 并打开 Dialog**

`app.rs`:
- import:`use gpui_component::{... WindowExt, dialog::Dialog ...};`(`WindowExt` 提供 `open_dialog`;确认 `Dialog` 导出路径,核实 `gpui_component::Dialog` 或 `gpui_component::dialog::Dialog`)。
- `PoopmanApp` 加字段 `env_manager: Entity<EnvironmentManager>`,在 `new()` 创建:`let env_manager = cx.new(|cx| EnvironmentManager::new(db.clone(), window, cx));`,并订阅其 `EnvironmentsChanged`:

```rust
        let env_changed_sub = cx.subscribe_in(
            &env_manager,
            window,
            move |this, _, _e: &crate::environment_manager::EnvironmentsChanged, _window, cx| {
                this.reload_environments(cx);
            },
        );
```
把 `env_changed_sub` 加进 `_subscriptions`,把 `env_manager` 加进 `Self { ... }`。

- 加打开 Dialog 的方法:

```rust
    fn open_env_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let manager = self.env_manager.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title("Environments")
                .w(px(640.))
                .child(manager.clone())
        });
    }
```

> 核实 `Dialog` 是否 `impl ParentElement`(本计划已确认)与 `.w(px)` 可用(dialog.rs 有 `w`/`width`)。`open_dialog` 的 build 闭包是 `Fn(Dialog,&mut Window,&mut App)->Dialog + 'static`,捕获 `manager` clone 合法。

- 把 Task 5 选择器里 "Manage…" 的处理接到 `self.open_env_manager(window, cx)`。

- [ ] **Step 4: 编译 + 测试编译**

Run: `cargo check --tests`
Expected: 退出 0,无新增 warning。

- [ ] **Step 5: Commit**

```bash
git add src/environment_manager.rs src/main.rs src/app.rs
git commit -m "feat(env): Environment management dialog

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 6: (检查点)Windows 真机完整验收**

```powershell
$env:GPUI_FXC_PATH = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\fxc.exe"
cargo test            # 运行 variables / db 单测(真机可链接)
cargo build --release
```
运行 `target\release\poopman.exe`,按 spec「验证策略」5 项核对:新建环境+变量、`{{baseUrl}}/get` 命中、切换环境打到不同地址、No Environment 时原样发出、header/body 内 `{{var}}` 替换、重启后持久化。

---

## Self-Review

**Spec 覆盖:**
- 替换引擎(纯函数、trim、未知保留、不递归、作用范围)→ Task 1(引擎)+ Task 4(应用到 url/headers/body/form-data)✓
- 数据模型 → Task 2 ✓
- 持久化(三表 + 外键 PRAGMA + CRUD + 激活读写)→ Task 3 ✓
- 发送集成(RequestEditor `env_vars` + `set_env_vars`,不依赖 Database)→ Task 4 + Task 5 推送 ✓
- 多环境 + 激活切换 + 持久化激活 → Task 3(存)+ Task 5(切换/加载)✓
- UI 选择器 + 管理 Dialog → Task 5(选择器)+ Task 6(Dialog + 组件)✓
- 验收清单 → Task 6 Step 6 ✓
- YAGNI(不做 globals/密钥/动态变量/预览)→ 全程未涉及 ✓

**占位符扫描:** Task 1-4 为完整可粘贴代码。Task 5-6 的 UI 含完整关键代码(选择器、set_active、open_env_manager、save_selected、订阅接线),并对 `Button::popup_menu` / `PopupMenu::menu` / `Dialog` 导出路径标注"实现者先核实实际 API 再接线",且给了降级方案 —— 这些是 GPUI UI 必要的实地核实点,非占位符。变量行渲染明确指向复用 `request_editor.rs` 既有 header 行模式(现存代码)。

**类型/命名一致:** `Environment`/`EnvVar`(Task 2)在 Task 3/5/6 引用一致;`substitute`(Task 1)在 Task 4 调用签名一致;`set_env_vars`(Task 4)在 Task 5 调用一致;`active_env_vars`/`set_active_environment`/`reload_environments`/`open_env_manager`(Task 5/6)互相引用一致;db 方法名(Task 3)在 Task 5/6 调用一致;`EnvironmentsChanged` 事件(Task 6)与 Task 6 Step 3 订阅一致。
