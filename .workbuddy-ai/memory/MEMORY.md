# 项目长期记忆 — tauri-multi-account-browser

## 技术栈与版本
- Tauri 2 + Rust + Vue 3 + TypeScript + Element Plus，Vite 7。
- 版本号需三处同步：`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml`。
- 当前版本 **0.6.0**（对应 CHANGES_V14）。
- 版本命名规律：CHANGES_V{n}.md 记录第 n 轮需求，不一定与 semver 一一对应（V10=0.5.0，V13=0.5.1，V14=0.6.0）。

## 架构约定
- 一个 Tauri 主窗口 + 多个原生 child WebView，一个标签页 = 一个账号。
- Vue 渲染标签栏/侧栏/地址栏；Rust 动态创建原生 WebView，切换标签只 `hide()/show()`。
- 远程页面**不允许**持有 IPC 权限；状态回传走假导航 scheme（`mbstatus`），下载配置走 `mb-eval` 注入 + `CustomEvent`。
- URL 规则唯一真值在 Rust：`effective_profile_home_url()` / `normalized_global_default_url()`。前端 `src/lib/profileUrl.ts` 只做即时反馈。
- `Profile.url_mode`: `inherit` 跟随全局默认主页 / `custom` 独立网址。旧账号 serde 默认 `custom`。

## 关键坑（务必记住）

### 1. Rust `set_app_settings` 是整体覆盖写盘
`set_app_settings(app, settings: AppSettings)` 不做字段级合并；`load_app_settings` 用 serde 默认值兜底。
**任何前端保存设置的入口都必须传完整的 `AppSettings`**，否则未传字段会被重置为 serde 默认值并落盘。
已知 `skip_downloaded_files` 在 Rust 侧被强制 `= true`。
→ 新增设置项时，必须同步所有 `emit('saveSettings')` / `save-downloads` 调用点。

### 2. 构建环境：Git Bash 的 link.exe 会遮蔽 MSVC 链接器
报错 `link.exe returned an unexpected error` 或 `LNK1181: 无法打开输入文件"kernel32.lib"`，
`cargo check` / `cargo build` 会卡在 tauri-build 构建脚本。
**解法：用 `scripts/cargo-msvc.sh` 封装环境**（已在仓库里，可直接复用）：
- 把 `C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.50.35717\bin\Hostx64\x64`
  前置到 PATH，让 `link.exe` 解析到 MSVC 版本而非 `/usr/bin/link.exe`；
- `INCLUDE` / `LIB` / `LIBPATH` **必须是 Windows 反斜杠绝对路径**，POSIX 路径不生效；
- SDK 版本 10.0.26100.0；
- 用法：`cd src-tauri && bash ../scripts/cargo-msvc.sh build --release`
- 注意 Bash/PowerShell 工具禁止调用 cmd.exe，不要试图用 .bat + vcvars 包装。

### 3. v14 官方包未做完整构建验证
CHANGES_V11/V12/V13 均自述"当前环境没有 Rust toolchain / node_modules 不完整，未完成最终编译验证"。
实测确实存在阻塞 `npm run build` 的 TS 错误。**升级官方包后必须自己跑一次构建**。

### 4. 设置/数据存储的分层约定（v15 起）
- 应用设置、账号、下载历史、**账号模板**都存 Rust store（`profiles.json`）。
- Rust 侧写入前做规范化，并把规范化结果**返回给前端回写**，避免前后端状态漂移。
- 读取类命令失败时不阻塞界面，退化为默认值 + 提示。
- 需要写盘的 `#[tauri::command]` 不要与内部 helper 同名（`save_profile_templates` 命令 vs
  helper 曾冲突），helper 统一用 `store_*` 前缀命名。
- localStorage 仅存纯前端 UI 偏好（见 `src/lib/uiPreferences.ts`）。迁移旧 localStorage 数据时
  要幂等：只在 Rust store 为空时导入一次，成功后立即清掉旧键。

### 5. 批量写操作必须原子（v16 起）
- 批量创建/导入走 `create_profiles_bulk`：**先全部校验，通过后才 `save_profiles`**，
  任一条不合格则整批不落盘。不要用前端循环 `invoke('create_profile')` —— 中途失败会留下部分结果。
- 单个创建 `create_profile` 与批量共用 `build_profile()` 校验，避免两套口径分叉。
- 新增批量类命令时沿用这个模式，并带上序号化错误信息。

### 6. Rust 单元测试
- 测试模块 `#[cfg(test)] mod tests` 在 `src-tauri/src/lib.rs` 末尾。
- **跑法：`bash scripts/cargo-test.sh --lib`**（不是直接 `cargo test`，原因见坑 7）。
- 已覆盖：昵称校验、URL 规范化、代理校验、时区/语言注入防护、模板规范化、
  ProfileDraft 反序列化、schema 迁移（22 个用例）。
- **store 是进程内共享的**：tauri-plugin-store 按文件路径共享 store，多个 `mock_app()`
  拿到同一份数据。每个用例开头必须 `reset_store()`，且必须单线程跑（`RUST_TEST_THREADS=1`，
  已在脚本里设置）。否则用例间互相污染，症状是「空 store 应视为 v0 却读到 1」。
- 需要 AppHandle 的被测函数要泛化为 `<R: Runtime>`，否则 `&AppHandle<MockRuntime>` 传不进去
  （E0308）。已泛化：load_app_settings / load_profiles / save_profiles /
  normalized_global_default_url / effective_profile_home_url / repair_profile_urls /
  load_profile_templates / store_profile_templates / stored_schema_version /
  migrate_v0_to_v1 / migrate_store。`use tauri::{...}` 要带 `Runtime`。

### 7. Windows 测试二进制无法启动（STATUS_ENTRYPOINT_NOT_FOUND 0xc0000139）
打开 tauri 的 `test` feature 后，`cargo test` 产出的 exe 在 main 之前就挂掉。根因：
- `test` feature 链入 tray-icon/muda → 引用 `comctl32.dll!TaskDialogIndirect`。
- 该导出**只在 Common-Controls v6**（WinSxS 旁加载程序集）里；
  `System32\comctl32.dll` 是 v5.82，**不导出它**。没有 v6 manifest 就绑到 v5.82。
- 正式 app 由 tauri-build 注入 manifest 所以没事；`cargo test` 的 harness 不走那条路径。

排障时踩过的三个死胡同（别再试）：
- `build.rs` 里**拿不到** `CARGO_CFG_TEST`（那是给被编译 crate 用的），无法区分构建类型。
- `cargo:rustc-link-arg-tests` 只作用于 `tests/` 集成测试，**覆盖不到 `--lib` 的 harness**。
- 无作用域 `cargo:rustc-link-arg` 会让正式 app 出现两份 MANIFEST 资源 → `CVT1100 资源重复`。

**最终方案**：manifest 作为静态文件 `src-tauri/common-controls.manifest`，
只在跑测试时用 `cargo --config target.x86_64-pc-windows-msvc.rustflags=[...]` 注入，
不影响 release。见 `scripts/cargo-test.sh`。
另：`mt.exe` 必须在 PATH 上（否则 `LNK1158 无法运行 mt.exe`），
它随 Windows SDK 分发，已由 `scripts/cargo-msvc.sh` 加入 PATH。

### 8. 工作区可能被 v14 压缩包重新解压覆盖
2026-09-11 发生过一次：v14 zip 被重新解压到工作区，把 `src/`、`src-tauri/src/`、
`Cargo.toml` 全部还原成 v14 基线，多轮改动丢失（当时没有 git 仓库）。
zip 里**不含** `scripts/cargo-msvc.sh`、`src-tauri/tests/`、`CHANGES_V15+`、`.workbuddy-ai/`，
所以这些能幸存。
→ **改代码前先 `tar` 备份**（排除 node_modules / target / dist），
→ 并且该项目没有 git，考虑 `git init` 建立版本控制。

## 环境
- VS 18 Community，MSVC 14.50.35717，Rust 1.98.0，Node 24.14.0 / 22.22.2。
- 备份放 `.workbuddy-ai/backup/`，排除 node_modules / src-tauri/target / dist。
- 当前版本 0.6.0（v14）。v15 模板迁入 Rust store、v16 批量创建原子提交、
  v17 schema_version 显式迁移，均未 bump 版本号，变更见
  `CHANGES_V15.md` / `CHANGES_V16.md` / `CHANGES_V17.md`。
