# CHANGES_V19 — 拆分 `src-tauri/src/lib.rs`

对应 `OPTIMIZATION_REPORT_V14.md` 第 5 条：**把 4326 行的 lib.rs 按领域拆成模块**。

纯重构，无功能变更、无数据格式变更。

---

## 1. 背景

v14 的 `src-tauri/src/lib.rs` 有 4326 行，常量、类型、store 读写、校验、
WebView 生命周期、下载、诊断、40 个 Tauri 命令全挤在一个文件里。
问题不是"文件长"本身，而是：

- 改一个下载逻辑要在 4000 行里翻找；
- git blame 全是同一文件，冲突概率高；
- 新人（和 AI）很难判断某段逻辑该在哪儿加。

## 2. 拆分结果

| 文件 | 行数 | 职责 |
|---|---:|---|
| `lib.rs` | 1085 | 常量、struct/enum 定义、serde 默认值、`run()` 入口、`#[cfg(test)] mod tests`（约 600 行） |
| `webviews.rs` | 1335 | WebView 创建/布局/注入脚本、**账号起始 URL 计算** |
| `downloads.rs` | 558 | 下载去重账本、历史记录、移动/删除/打开 |
| `profiles.rs` | 456 | 账号 CRUD 命令、账号模板 |
| `validation.rs` | 347 | 昵称/网址/代理/时区/语言校验，CSV 解析与生成 |
| `import_export.rs` | 219 | 导入导出：文件对话框、冲突策略、逐行错误报告 |
| `status.rs` | 202 | 账号状态追踪（登录态/封禁/回答就绪） |
| `store.rs` | 172 | store 读写封装 + 结构版本迁移 |
| `diagnostics.rs` | 161 | 启动诊断、自救修复、代理连通性测试 |
| `settings.rs` | 108 | 应用设置读写、下载配置下发 |

（`lib.rs` 的 1085 行里有 ~600 行是测试，实际非测试代码约 490 行，全是类型定义。）
后续可考虑把 `mod tests` 也挪到独立文件，进一步瘦身。

## 3. 关键技术决策

### 3.1 类型留在 lib.rs，只搬函数

拆分的第一个念头是"类型也一起搬"，那就要把 `Profile` / `AppSettings` 等
struct 的字段改成 `pub(crate)`，改动面大、容易漏。

**利用 Rust 的模块可见性规则：子模块可以访问父模块的私有项。**
所以只搬函数、类型留在 `lib.rs`，子模块 `use crate::*;` 就能直接用
`Profile`、`AppSettings`、`STORE_FILE` 等，**一个字段可见性都不用改**。

这也是为什么"类型定义"和"函数"分家看起来奇怪，实际是本次能低成本完成的关键。

### 3.2 `#[tauri::command]` 的宏作用域

`#[tauri::command]` 会在**定义它的模块**里生成 `__cmd__xxx` /
`__tauri_command_name_xxx` 两个 `macro_rules!`。搬走函数后踩了三个坑：

1. `cannot find macro __cmd__xxx in this scope`（131 个）
   —— 宏留在了子模块，但 `generate_handler!` 在 `lib.rs` 展开。
2. 试过 `pub(crate) use {__cmd__x, __tauri_command_name_x};` 再导出
   → `E0252: name __cmd__xxx defined multiple times`（和模块自己的定义撞名）。
3. **正确解法：`generate_handler!` 直接写模块路径**：

   ```rust
   .invoke_handler(tauri::generate_handler![
       profiles::list_profiles,
       profiles::create_profiles_bulk,
       downloads::get_profile_icon,
       // ...
   ])
   ```

   Tauri 源码注释里明确写了 "Rely on rust 2018 edition to allow importing a
   macro from a path"，即支持这种写法。好处是调用点自带归属信息，
   一眼能看出命令在哪个文件。

### 3.3 `pub(crate) use module::*` 不能导出私有项

子模块里的函数是 `fn`（私有）时，`lib.rs` 写 `pub(crate) use webviews::*;`
**再导出不了任何东西**（E0425: cannot find function）。
解决办法：给搬走的所有顶层 `fn` 加 `pub(crate)`（本次加了 103 处）。

`lib.rs` 里的再导出保留一份，并加注释说明用途：

```rust
// 函数按领域拆到各子模块后，这里做一次 crate 内扁平再导出：
// 文件末尾的 `mod tests` 用扁平名字调用它们，不必关心函数落在哪个文件。
// 生产代码调用子模块函数时请直接写模块路径（如 `profiles::list_profiles`），
#[allow(unused_imports)]
pub(crate) use {
    diagnostics::*, downloads::*, import_export::*, profiles::*, settings::*, status::*,
    store::*, validation::*, webviews::*,
};
```

### 3.4 拆分脚本

拆分是机械操作，用 `scripts-dev/split_lib.py` 完成（用完即弃，未入库）。
要点：

- **括号配对 + raw string 感知**：`r#"..."#` 里嵌的 JS 含大量 `{}` 和空行，
  按空行切块的朴素做法必然出错（第一次尝试就炸在
  `unexpected closing delimiter`）。
- 块从**文档注释**开始算，而不是从 `fn` 开始，保证注释跟着函数走。
- 每个子模块顶部写 `//!` 一行职责说明 + `use crate::*;`。

## 4. 清理 warning

`cargo fix --lib` 自动剔除了各模块 header 里 62 处未使用的 import。
有 4 处它没动，手工处理：

- `validation.rs`：`Manager`、`serde::Deserialize` 未使用 → 删。
- `downloads.rs`：`base64::Engine as _` 未使用 → 删。
- `lib.rs`：`tauri::Manager`、`tauri_plugin_store::StoreExt` 只有**测试**用得到
  → `cargo check --lib`（非 test）判定未使用，但删了测试就编译不过。
  最终把 `StoreExt` 的导入**移进 `mod tests` 内部**，非 test 编译不再报 warning。

最后 `cargo fmt` 统一格式（把 `cargo fix` 改乱的 import 块重新排好）。

## 5. 验证

| 项目 | 结果 |
|---|---|
| `cargo check --lib` | 0 error 0 warning |
| `cargo check --lib --tests` | 0 error 0 warning |
| `cargo check --bins` | 0 error 0 warning |
| `bash scripts/cargo-test.sh --lib` | **30 passed / 0 failed** |
| `npm run build`（`vue-tsc --noEmit` + vite） | 通过 |
| `cargo build --release` | 通过 |

版本号未 bump，仍是 **0.6.0**。

## 6. 注意事项（后续改动请遵守）

1. **新命令**加到对应子模块，并在 `run()` 的 `generate_handler!` 里用
   `模块名::函数名` 形式注册。
2. **新类型**（struct/enum）放 `lib.rs`，除非它只在单个模块内使用。
3. 跨模块调用写**模块路径**，不要依赖 `lib.rs` 的扁平再导出
   （那一份只是给测试用的）。
4. URL 规则的唯一真值在 `webviews.rs`（`effective_profile_home_url` 等），
   前端 `src/lib/profileUrl.ts` 只做即时反馈 —— 已在文件头注释里写明。
