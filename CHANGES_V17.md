# v17 / store 结构版本化与显式迁移

本版落实 `OPTIMIZATION_REPORT_V14.md` 第 4 条建议：给 store 加 `schema_version`，
把此前散落在各处的「靠 serde default 隐式兜底」变成显式的、可验证的迁移步骤。

## 1. 为什么需要

此前所有结构兼容都依赖 `#[serde(default)]`：新增字段时旧数据能读出来，靠的是默认值。
这在短期内能用，但有两个隐患：

- **隐式**：没人知道旧数据到底长什么样，也无法验证「补齐」是否真的发生。
- **脆弱**：一旦将来要删掉某个 `serde(default)`（或改变字段语义），旧数据会静默损坏，
  而没有任何地方拦截。

## 2. 做法

- `SCHEMA_VERSION: u64 = 1`，写入 store 的 `schema_version` 键。
- `stored_schema_version()`：缺失或非法一律视为 0（即 v14 及之前的历史数据）。
- `migrations()`：按升版顺序排列的迁移函数数组，索引 i 表示「把版本 i 升到 i + 1」。
  新增版本时往尾部追加即可，**已发布的旧步骤不再改动**。
- `migrate_v0_to_v1()`：把历史数据里靠默认值才成立的字段补齐并落盘 ——
  修正空/坏网址的账号、规范化 `url_mode`、修复损坏的全局默认主页、
  清理无效模板。**不改变任何用户可见行为。**
- `migrate_store()` 在 `.setup()` 里、任何命令被调用前执行一次。

## 3. 版本超前时拒绝迁移

若 store 里的版本号**大于**当前程序支持的版本，说明用户用旧版程序打开了新版数据。

此时**不降级、不改数据**，直接返回错误并提示升级。否则用旧结构回填会用默认值
覆盖掉新字段，造成不可逆的数据丢失。

## 4. 防呆

`migrate_store` 会校验 `migrations().len() == SCHEMA_VERSION`。
新增版本时如果忘了补迁移函数，会直接报错而不是静默跳过。

## 5. 顺带解决：Windows 测试二进制无法启动

打开 tauri 的 `test` feature 后，测试可执行文件会以
`STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)` 启动失败。根因：

- `test` feature 会链入 tray-icon/muda，它们引用 `comctl32.dll!TaskDialogIndirect`。
- 该导出**只**存在于 Common-Controls **v6**（WinSxS 旁加载程序集）；
  `System32\comctl32.dll` 是 v5.82，并不导出它。
- 没有 v6 manifest，加载器就绑到 v5.82，进程在 main 之前就挂了。

正式 app 由 tauri-build 注入 manifest 所以没问题；`cargo test` 的 harness 不走那条路径。

**注意两个坑**：

- `build.rs` 里**拿不到** `CARGO_CFG_TEST`（那是给被编译 crate 用的），无法区分构建类型。
- `cargo:rustc-link-arg-tests` 只作用于 `tests/` 下的集成测试，
  **覆盖不到 `--lib` 的单元测试 harness**。
- 若改用无作用域的 `cargo:rustc-link-arg`，正式 app 会出现两份 MANIFEST 资源，
  链接时报 `CVT1100 资源重复`。

最终方案：manifest 作为静态文件 `src-tauri/common-controls.manifest`，
只在跑测试时通过 `cargo --config` 注入 rustflags（见 `scripts/cargo-test.sh`），
**不影响 release 构建**。同时 `mt.exe` 必须在 PATH 上（否则 `LNK1158`），
已由 `scripts/cargo-msvc.sh` 处理。

## 6. 测试

`cargo test --lib`：**22 passed; 0 failed**（15 个原有校验用例 + 7 个迁移用例）。
新增迁移用例：步骤数与版本号一致、全新安装、幂等、版本号真正落盘、
修复历史账号且不丢 id、修复损坏全局主页、超前版本被拒绝且不改数据。

**注意**：`tauri-plugin-store` 的 store 按文件路径在**进程内共享**，
多个 `mock_app()` 拿到的是同一份数据。因此每个用例开头调用 `reset_store()`，
并强制单线程运行（`RUST_TEST_THREADS=1`，见 `scripts/cargo-test.sh`）。

## 验证

- `npm run build`（vue-tsc --noEmit + vite build）通过，1629 模块。
- `bash scripts/cargo-test.sh --lib`：22 passed; 0 failed。
- `cargo check`（lib + bin）通过，零警告零错误。
- `cargo build --release` 通过，产出 `tauri-multi-account-browser.exe`。

## 未包含

`OPTIMIZATION_REPORT_V14.md` 第 2 条（原生文件选择器 / CSV / 冲突策略 / 逐行错误报告）留待后续。
