# v18 / 导入导出：原生文件选择器、CSV、冲突策略、逐行错误报告

本版落实 `OPTIMIZATION_REPORT_V14.md` 第 2 条建议。

## 1. 原生文件选择器

- 新增 `pick_import_file` / `pick_export_file`，用系统原生打开/另存为对话框
  （`tauri-plugin-dialog`），不再需要手动复制粘贴大段文本。
- 文件读写走 **Rust 侧 `std::fs`**（`read_import_file` / `write_export_file`），
  不经过前端 fs 插件，因此**不受 fs 权限 scope 限制**——用户对话框选中的路径可直接读写。
- 只加了 `dialog:allow-open` / `dialog:allow-save` 两个权限，没有放开 fs。

## 2. CSV 支持

- 新增 `parse_accounts_csv` / `build_accounts_csv`。
- 用 `csv` crate 处理引号/逗号/换行的转义。昵称里带逗号是很常见的，
  手写解析必错，所以这里不自己造轮子。
- 表头可有可无：首行若是 `name,default_url,...` 就跳过，否则当数据行。
- 空行自动跳过。
- CSV 只存账号，不含模板（模板仍走 JSON）。

## 3. 冲突策略

导入时遇到同名账号，三种处理：

| 策略 | 行为 |
|---|---|
| `rename`（默认） | 自动改名，追加 ` (2)`、` (3)`…，双方都保留 |
| `skip` | 跳过该行，现有账号不动 |
| `overwrite` | 用导入内容覆盖同名账号，**保留原 id / created_at / order** |

`overwrite` 保留 id 很关键：账号的 WebView 数据目录是按 id 建的，
换 id 会让它丢失 Cookie / 登录态。

未知策略直接报错，不静默回退。

## 4. 逐行错误报告

- 新增 `import_profiles` 命令，返回 `ImportReport`：
  `created` / `updated` / `skipped` / `errors`，`errors` 与 `skipped` 都带**行号**。
- 导入默认**尽力而为**：单行校验失败只记进 errors，其余行照常导入。
  导入文件里个别行坏掉很常见，整批回滚反而更难受。
- 前端新增「导入结果明细」弹窗，逐行列出失败/跳过原因，用户能对着原文件定位。
- 支持 `dryRun`：只报告将要发生什么，不写盘。

**注意**：这与 v16 的 `create_profiles_bulk`（原子）是两条路径，
前者用于「批量创建」表单，后者用于「导入」，语义不同，不要混用。

## 5. 依赖变更

- Rust：`tauri-plugin-dialog` 2.7、`csv` 1.4。
- npm：`@tauri-apps/plugin-dialog`。
- capabilities：`dialog:allow-open`、`dialog:allow-save`。

## 6. 测试

`cargo test --lib` 从 22 增至 **30 passed; 0 failed**。新增 8 个用例：

- CSV 往返：昵称带逗号/引号必须原样保留
- CSV 跳过表头与空行、无表头输入也能读
- `unique_profile_name` 后缀递增
- 导入：坏行逐行报错、好的行照常导入（含行号断言）
- 三种冲突策略行为（rename 改名 / skip 不新增 / overwrite 保留 id 且字段更新）
- `dry_run` 不写盘
- 未知冲突策略报错

为可测性把命令体抽成 `import_profiles_impl()`，并让 `ProfileDraft` 支持 `Default`。

## 验证

- `npm run build`（vue-tsc --noEmit + vite build）通过，1629 模块。
- `bash scripts/cargo-test.sh --lib`：30 passed; 0 failed。
- `cargo check --lib --bins` 零警告。
- `cargo build --release` 通过。
- 前端 invoke 与后端命令交叉核对：无悬空调用。

## 未包含

`OPTIMIZATION_REPORT_V14.md` 第 5-8 条（lib.rs 拆分、App.vue composables 拆分、
诊断中心增强、标签/收藏/批量修改）留待后续。
