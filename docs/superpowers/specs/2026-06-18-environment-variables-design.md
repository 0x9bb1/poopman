# 环境变量（Environment Variables）设计

**日期**: 2026-06-18
**范围**: 新功能 —— 多命名环境 + 当前激活 + `{{var}}` 替换 + 管理 UI
**类型**: 功能新增(对标 Postman 环境变量)

## 背景与目标

对标 Postman:用户定义多个命名环境(dev/staging/prod 等),每个环境是一组 `key=value` 变量;顶部可切换"当前激活环境";请求中的 `{{var}}` 在**发送时**用当前环境的变量替换。提升日常多环境调试效率。

已确认决策:
- **组织模型**:多个命名环境 + 一个当前激活环境(可为"No Environment")。
- **持久化**:复用现有 SQLite(`~/.poopman/history.db`)。

## 架构(三层)

1. **数据/持久化** —— `src/types.rs`(模型)+ `src/db.rs`(表与方法)
2. **替换引擎** —— 新 `src/variables.rs`(纯函数,可单测)
3. **UI** —— 标题栏环境选择器 + 管理 Dialog(`window.open_dialog`),由 `PoopmanApp` 协调

## 数据模型(`types.rs`)

```rust
#[derive(Debug, Clone)]
pub struct Environment {
    pub id: i64,
    pub name: String,
    pub variables: Vec<EnvVar>,
}

#[derive(Debug, Clone)]
pub struct EnvVar {
    pub enabled: bool,
    pub key: String,
    pub value: String,
}
```

## 持久化(`db.rs`,新增表 + 方法)

建表(随 `Database::new` 的 schema 初始化):
- `environments(id INTEGER PK AUTOINCREMENT, name TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0)`
- `env_variables(id INTEGER PK AUTOINCREMENT, environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE, enabled INTEGER NOT NULL DEFAULT 1, key TEXT NOT NULL, value TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0)`
- `app_meta(key TEXT PRIMARY KEY, value TEXT)` —— 通用键值;本功能用键 `active_environment_id`(值为环境 id 字符串;无此行 = No Environment)

> 注:`env_variables` 用了外键级联,`Database::new` 需执行 `PRAGMA foreign_keys = ON;`(rusqlite 默认每连接关闭外键)。

方法(`Database`):
- `load_environments(&self) -> Result<Vec<Environment>>` —— 连同各自变量,按 position 排序
- `create_environment(&self, name: &str) -> Result<i64>`
- `rename_environment(&self, id: i64, name: &str) -> Result<()>`
- `delete_environment(&self, id: i64) -> Result<()>`(级联删除其变量)
- `replace_variables(&self, environment_id: i64, vars: &[EnvVar]) -> Result<()>` —— 事务内删旧插新
- `get_active_environment_id(&self) -> Result<Option<i64>>`
- `set_active_environment_id(&self, id: Option<i64>) -> Result<()>`

## 替换引擎(`src/variables.rs`,纯函数)

```rust
use std::collections::HashMap;

/// Replace `{{key}}` / `{{ key }}` (key trimmed) occurrences with values from `vars`.
/// Unknown variables are left literal (so a typo / missing var is visible).
pub fn substitute(input: &str, vars: &HashMap<String, String>) -> String;
```

- 匹配 `{{` ... `}}`,内部 trim 后查表;命中替换,未命中**原样保留** `{{...}}`。
- 不递归(替换结果中的 `{{}}` 不再二次替换),避免无限循环。
- **作用范围**(发送时):URL、每个 enabled header 的 key 与 value、body 的 Raw 内容、form-data 各行的文本值(File 路径与 key 也替换 key/文本值)。
- 编辑器/输入框始终显示原始 `{{var}}`;仅在发送构造请求时解析。

`PoopmanApp` 维护"当前激活环境变量表":取激活环境中 `enabled` 的变量构造 `HashMap<String,String>`(同名后者覆盖前者)。

## 发送集成

- `RequestEditor` 新增字段 `env_vars: HashMap<String, String>` 与方法 `set_env_vars(&mut self, map)`。**不直接依赖 `Database`**。
- `send_request` 在拿到 url/headers/body 后、构造发送前,用 `variables::substitute` 以 `self.env_vars` 替换 url、headers(key+value)、body。
- `PoopmanApp` 在启动、切换激活环境、保存环境编辑后,重新计算激活变量表并 `request_editor.update(|e| e.set_env_vars(map))`。
- 历史中保存的是用户输入的原始(含 `{{var}}`)请求(与 Postman 一致:存模板,不存解析后的值)。

## UI

### 环境选择器(标题栏)
- 在 `TitleBar` 内 "Poopman" 旁放一个下拉(`Select` 或按钮 + popover),显示当前环境名,默认 "No Environment"。
- 列表项:每个环境(点击即切换激活)+ 末尾 "Manage Environments…"(打开管理 Dialog)。
- 切换激活 → `db.set_active_environment_id` + 重算并推送 env_vars。

### 管理 Dialog(`window.open_dialog`)
- 左侧:环境列表 + "新建"按钮;每项可改名、删除。
- 右侧:选中环境的变量表 —— 每行 `启用勾选 + key 输入 + value 输入 + 删除`,末尾自动空行(复用 headers 表的交互/样式)。
- 底部:保存(写 `db.replace_variables` / 名称变更)/ 关闭。
- 保存后:重载环境;若激活环境被改动,刷新 editor 的 env_vars。
- 管理逻辑放入新组件 `src/environment_manager.rs`(`Entity`),由 `PoopmanApp` 持有并在打开 Dialog 时渲染其内容。

## 验证策略

- `variables::substitute` 与 `db` 的环境 CRUD 逻辑可在纯逻辑层单测(`substitute` 命中/未命中/trim/不递归/多变量;db 方法若需 runtime 则靠 `cargo check` + 真机)。WSL2 仅 `cargo check`/纯函数 `cargo test`。
- Windows 真机验收:
  1. 新建环境 dev,加 `baseUrl=https://postman-echo.com`;URL 写 `{{baseUrl}}/get` 发送 → 命中、200。
  2. 切到另一个环境(不同 baseUrl)→ 同一请求打到不同地址。
  3. 切 "No Environment" → `{{baseUrl}}` 原样发出(未解析)。
  4. header / body 里的 `{{var}}` 同样被替换。
  5. 重启应用 → 环境、变量、当前激活环境都持久化保留。

## 范围红线(YAGNI)

不做:全局变量层(Postman 的 globals)、密钥/掩码变量类型、动态变量(`{{$guid}}` 等)、变量悬停预览解析值、集合作用域变量、导入/导出。其余功能逻辑不动。
