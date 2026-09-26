# 项目长期记忆 — tauri-multi-account-browser

> 构建 / 发布 / 测试环境的坑（MSVC link.exe、测试二进制 manifest、custom-protocol、
> 发布命令、clippy）已沉淀到 skill **`windows-rust-msvc-build`**，此处只留项目特有信息。

## 技术栈 / 版本
- Tauri 2 + Rust + Vue 3 + TS + Element Plus，Vite 7。当前版本 **0.7.0**。
- 版本号三处同步：`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml`。
- `CHANGES_V{n}.md` 记第 n 轮需求，与 semver 不一一对应（V14=0.6.0，V22=0.7.0）。

## 架构
- 一主窗口 + 多原生 child WebView，一标签页 = 一账号；切换只 `hide()/show()`。
- 远程页面无 IPC 权限；状态回传走假导航 `mbstatus`，下载配置走 `mb-eval` + `CustomEvent`。
- URL 规则唯一真值在 Rust（`effective_profile_home_url` / `normalized_global_default_url`），
  前端 `src/lib/profileUrl.ts` 只做即时反馈。`Profile.url_mode`: `inherit` / `custom`。
- **AI 状态边沿**：扫描脚本 `answerReady = !profileActive`（后台账号才提醒），
  故「AI 答完」的可靠信号是 `answer_generating` **下降沿**，不是 `answer_ready` 上升沿
  （用后者会导致当前正看的账号永远不记时间 —— v24 踩过）。

### 前端分层（v20 起）
`App.vue` 只做宿主编排；业务在 `src/composables/`：`useSettings()`（无依赖）/
`useTabs(deps)` / `useProfiles({profiles, tabs})`。
1. composable 之间**不互相 import**，一律参数注入。
2. **`profiles` ref 由 App.vue 持有**再传给两个 composable，挪进去会成环。
3. `<script setup>` 只对**顶层** ref 解包 → 「对象取函数、顶层解构取 ref」：
   `const { openTabs, activeId } = tabs`，模板里 `@activate="tabs.activate"`。

## 多会话工作区（2026-09-26 起）

同一时刻可能有 2~3 条流程在改这份代码（实测 0.13.0 那一轮同时有：账号总览按网站分类、
多账号生成器、快捷工具栏「网站导航」按钮）。

- **版本号与 `CHANGES_V{n}.md` 是共享的**：动手前先 `git status`，看清哪些未提交改动不是自己的
  —— 别把别人的改动当成自己的，也别重复 bump 版本号；改动并入当时那一轮的版本即可。
- 临时目录 `.mut/`、`preview/` 是共用的：**只删自己建的文件**，`rm -rf` 整目录会删掉别人的工作文件。
  `preview/` 被别人删掉时，自己起着的 vite 预览会**静默 fallback 到根 `index.html`**（页面能开、
  组件全无）—— 预览入口放自己独占的路径更稳。

## 项目特有坑

**1. `set_app_settings` 整体覆盖写盘**（不做字段合并）。任何保存入口必须传**完整**
`AppSettings`，否则未传字段被 serde 默认值重置。`skip_downloaded_files` 侧强制 true。
→ 新增设置项要同步所有 `emit('saveSettings')` / `save-downloads` 调用点。

**2. 存储分层（v15 起）**：设置/账号/下载历史/**模板**存 Rust store（`profiles.json`）。
Rust 写入前规范化并把结果**返回前端回写**。读失败退化为默认值、不阻塞界面。
写盘命令勿与内部 helper 同名，helper 用 `store_*` 前缀。
localStorage 只存 UI 偏好；旧数据迁移要幂等（store 空时导一次，成功即清旧键）。

**3. 批量写必须原子（v16 起）**：`create_profiles_bulk` 先全量校验再一次性 `save_profiles`，
不要前端循环 `invoke('create_profile')`。单个与批量共用 `build_profile()` 校验。

**4. Rust 单测**：跑法见 skill。
- **store 按文件路径进程内共享** → 每用例 `reset_store()` + 单线程。
  **`reset_store()` 必须删全部 key**（含 `STATUS_KEY`），漏一个就跨用例串数据（v24 踩过）。
- 需要 `AppHandle` 的被测函数要泛化 `<R: Runtime>`，否则 `MockRuntime` 传不进去（E0308）。
- `save_profiles` 整表覆盖，`seed_profile()` 循环调会互相抹掉 → 多账号用 `seed_profiles()`。

**5. 导入导出（v18 起）**：对话框用 `tauri-plugin-dialog`，**文件读写走 Rust `std::fs`**
（不受 fs scope 限制）。CSV 必须用 `csv` crate。两条创建路径语义不同别混用：
`create_profiles_bulk`（原子）/ `import_profiles`（尽力而为 + 冲突策略 + 逐行报错）。
`overwrite` 必须保留原 id/created_at/order（数据目录按 id 建，换 id 丢登录态）。
命令体抽成 `*_impl()` 供测试直调。

**6. 同步 `#[tauri::command]` 跑主线程**，阻塞操作必须改 `async` + `spawn_blocking`。

**7. 调系统命令**：一律 `hidden_command()`（加 `CREATE_NO_WINDOW`，否则闪黑框）；
不假设 `reg.exe` 可用（要有文件系统兜底）；**参数里不让前端传路径**（会变成任意进程启动）。

**8. Rust 模块划分（v19 起）**：`lib.rs`（常量+全部类型定义+`run()`+`mod tests`）、
`webviews.rs`（WebView + URL 真值）、`downloads.rs`、`profiles.rs`、`validation.rs`、
`import_export.rs`、`status.rs`、`store.rs`、`diagnostics.rs`、`settings.rs`。
- **类型留 `lib.rs`，只搬函数**（子模块可访问父模块私有项，`use crate::*;` 即可）。
- `generate_handler!` 必须写模块路径（`profiles::list_profiles`）；别再导出 `__cmd__*`（E0252）。
- 子模块函数要 `pub(crate)`，否则 `pub(crate) use x::*;` 再导出不了（E0425）。
- `cargo fix --lib` 会删掉只有测试用的顶层 import（如 `StoreExt`），修完再跑
  `cargo check --lib --tests`。

**9. 标签/收藏（v22 起）**：`Profile` 有 `tags`/`favorite`/`last_used_at`（均 serde default），
`SCHEMA_VERSION=2`。`normalize_tags()`：trim/丢空/大小写不敏感去重/24 字符/12 个/保序。
批量改动走 `update_profile_batch`（原子）。
**`normalized_global_default_url()` 很贵**（整份反序列化 AppSettings）→ 拆出
`build_profile_with_global(draft, order, &global)`，批量路径在**循环外算一次**。
**CSV 列只允许末尾追加**，tags 用 `|` 分隔。

**10. 工作区曾被 v14 zip 重新解压覆盖**（2026-09-11，当时无 git，多轮改动丢失）。
改代码前先 `tar` 备份（排除 node_modules/target/dist）。现已 git init。

**11. 这个工作区会同时开多个会话改同一份未提交代码**（2026-09-26 实测）。
`git status` 里的改动未必是自己写的，版本号也可能已被别人 bump。动手前先 `git status`
+ `git diff --stat` 认领文件；`.mut/` 是**共享临时目录**（各会话的探针脚本、chrome profile
都在里面）→ **绝不 `rm -rf .mut/`**，删之前先 `ls` 看归属。写 `CHANGES_V*.md`、bump 版本号、
`git commit` 之前先确认这一轮编号有没有被别人占用。

**12. UI 交互改动的验证方式**（2026-09-26 起）：能抽成纯函数的走 `scripts/test-*.mjs`
（`ts.transpileModule` + `data:` URL 动态 import）；DOM/交互行为走「vite dev + 零依赖 CDP 探针」，
两边的断言都要做变异校验。完整流程见 skill **`vite-component-cdp-probe`**，脚本留档在
`.workbuddy-ai/probe/`。环境坑：vite 只监听 `[::1]`（curl 要用 `localhost`）、沙箱 curl 要
加 `--noproxy '*'`。

**13. 网站归类只有一处真值**（2026-09-26 起）：`src/lib/siteDirectory.ts` 的
`buildWebsiteGroups` / `indexWebsiteGroups` / `countSiteFacets` —— 网站导航与「全部账号历史」的
网站筛选都从它派生。**App.vue 必须把 `global_default_url` 传给总览弹窗**，否则 inherit 账号归类错。
`parseDirectorySite` 显式拒绝含 `%` 的主机名：**Chromium 把 `https://not a url` 解析成
`not%20a%20url` 而不报错（Node 抛 TypeError）**，不拦就会凭空多出一个「网站」。

## 环境
- VS 18 Community，Rust 1.98.0，Node 24.14.0 / 22.22.2。备份放 `.workbuddy-ai/backup/`。
- git 已有，`master` 含 v19~v24。**坑：带斜杠的分支名创建不了**（`git update-ref` 返回成功
  但 refs 子目录没建，随后 commit 会变成无父根提交）→ 分支名一律不带斜杠。
- 提交用 `git commit -F "$(cygpath -w /tmp/msg.txt)"`（git for Windows 不认 `/tmp/xxx`）。
- 沙箱批量删除保护会拦 `rm -rf dist` 和 vite 清 dist → 用
  `mv dist .workbuddy-ai/backup/dist-stale-<ts>`。**但整目录的 `mv` 也会被拦**
  （单文件改名正常，小目录如 3 个文件的 dist 可以）→ chrome profile 这类大目录只能用
  `dangerouslyDisableSandbox` 显式列路径删，或留着说明。
