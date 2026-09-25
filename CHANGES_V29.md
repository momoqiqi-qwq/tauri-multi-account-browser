# CHANGES_V29 — 代码优化（第一轮）：修真缺陷 + 把阻塞从主线程挪走

## 1. 背景与范围

用户要求"优化代码"。先对全仓（Rust 11 个模块 + 前端 20 个文件）做了两轮审计，
再把范围收敛到**只修缺陷与主线程阻塞**，不动模块划分与文件结构。性能热点与结构
拆分留在后面几轮（见第 6 节）。

版本号按 `agent.md` 规则：纯修复 → patch 位，0.10.0 → 0.10.1。

## 2. 修掉的 4 个真缺陷

### 2.1 测代理会污染 v28 的 AI 对话计时

`test_profile_proxy` 拿到探测结果后，把结果塞进一个 `ProfileStatus::default()`
再调用 `save_profile_status`。而 `save_profile_status` 的语义是"页面状态上报"，
它会拿传入值和上一条做**边沿比较**：

- 默认值的 `answer_generating = false`，若上一条是 `true` → 被判成一次真实的
  「生成中 → 空闲」下降沿 → 凭空写下一个 `answer_finished_at`；
- 默认值的 `answer_ready = false` → 未读红点被清掉。

也就是说：**AI 正在回答时点一下"检测出口 IP"，这一轮对话的时间就打脏了。**

现在代理结果走新增的 `status::save_profile_proxy_result`，它经由
`patch_profile_status` **就地改 4 个代理字段**，完全不参与边沿判定。
顺带去掉了那次只为"确认账号存在"的全量 `load_profiles`（命令开头已经查过）。

新增回归单测：`proxy_result_neither_ends_a_round_nor_clears_unread`。

### 2.2 两个弹窗会被账号网页盖住

`suppress.ts` 的机制是：原生子 WebView 永远画在主 WebView（Vue UI）之上，
弹层打开期间必须临时隐藏账号页。四个全屏对话框都接了这个开关，
**`AccountToolsDialog`（860px 的账号管理工具）和 `ProfileDiagnosisDialog`（启动
诊断）漏了** —— 表现是"点了没反应 / 弹窗被网页盖住"，只有在账号页铺满视口时才
复现，所以一直没被发现。

根因是"每个对话框都得自己记得复制那 14 行"。现在 `suppress.ts` 提供
`useDialogSuppression(() => visible.value)`（含 `onBeforeUnmount` 释放），两个漏网
弹窗改用它。已有的 5 处保持原样，不再制造第二种写法之外的改动。

### 2.3 一个事件监听失败会让后面全部不注册

`App.vue` 的 `onMounted` 里六个 `await listen(...)` 是串行的，而 `keydown` 监听、
自动刷新定时器、标签生命周期定时器、`ResizeObserver` 都排在它们**之后**。
任意一个 `listen` 失败或被卡住，键盘快捷键（v27 的全部入口）、自动刷新、
原生视图边界同步就静默失效。

改为 `Promise.all` 并行注册，单个失败只让出该事件本身（新增 `listenOrNone`）。

### 2.4 "移动下载文件"会冻结整个界面

`pick_move_directory` 是**同步 command**（跑在主线程），内部用
`powershell.exe -STA` 弹 `FolderBrowserDialog` 并 `.output()` 阻塞等用户点完 ——
从打开选择器到关闭期间，主窗口完全不响应，看起来像程序卡死。而
`tauri-plugin-dialog` 早就接好了（`import_export.rs` 的导入/导出就是这么写的）。

现在改为 async command + `app.dialog().file().blocking_pick_folder()`。附带收益：
原本为 Windows / macOS / Linux 各写一份外部进程调用（含一份和 `hidden_command`
重复的 `CREATE_NO_WINDOW`），现在约 70 行平台分支变成 20 行，且不再有弹出
PowerShell 的开销。

## 3. 主线程阻塞：剩余三处

同类的同步大 IO 一并挪进阻塞线程池（`tauri::async_runtime::spawn_blocking`），
JS 侧 `invoke` 契约不变（本来就在 await）：

| command | 原本 | 现在 |
| --- | --- | --- |
| `move_download` | 主线程 rename/复制整个文件（跨卷时几个 G） | `move_file_on_disk` 进 spawn_blocking |
| `clear_profile_data` | 主线程 `remove_dir_all` 删账号数据目录（上万小文件） | 进 spawn_blocking |
| `read_import_file` / `write_export_file` | 主线程读写用户选中的文件（网盘/U 盘时冻住） | 进 spawn_blocking |

## 4. 热路径上的重复反序列化

`load_profiles` 每次导航、每次切标签、每个图标请求都会跑，而它的修复循环
（`repair_profile_urls`）对**每个账号**都要读一次 store 并反序列化整个
`AppSettings` 拿全局主页 —— N 个账号就是 2~3N 次全量解析 + 同样次数的全局写锁。

现在拆成 `repair_all_profile_urls`：先扫一遍是否需要兜底值，
**数据都已规范化时一次 store 都不读**；需要时也只解析一次。规范化本身的口径
完全没变（仍对每个账号做 `normalize_url`）。

新增回归单测：`repair_all_profile_urls_is_noop_on_normalized_profiles`。

顺带清掉因此失去调用点的 `repair_profile_urls`（单账号版）和 `diagnostics.rs`
里三个变成未使用的 import。

## 5. 改动文件

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/status.rs` | 新增 `patch_profile_status` / `save_profile_proxy_result`；`clear_profile_answer_ready` 改走 patch 路径；`is_active_profile` 去掉一次无谓 clone |
| `src-tauri/src/diagnostics.rs` | 代理结果改调 `save_profile_proxy_result`；清理 3 个失效 import |
| `src-tauri/src/webviews.rs` | `repair_profile_urls` → `repair_all_profile_urls` + `_with`（全局主页只解析一次） |
| `src-tauri/src/store.rs` | `load_profiles` / `migrate_v0_to_v1` 改用批量修复 |
| `src-tauri/src/downloads.rs` | `pick_move_directory` 改 async + dialog 插件；`move_download` 拆出 `move_file_on_disk` 进阻塞线程池 |
| `src-tauri/src/profiles.rs` | `clear_profile_data` 的目录删除进阻塞线程池 |
| `src-tauri/src/import_export.rs` | 读/写导入导出文件进阻塞线程池 |
| `src-tauri/src/lib.rs` | 2 条单测 |
| `src/lib/suppress.ts` | 新增 `useDialogSuppression` |
| `src/components/AccountToolsDialog.vue` / `ProfileDiagnosisDialog.vue` | 接入抑制开关 |
| `src/App.vue` | 事件监听并行注册（`listenOrNone`） |
| `package.json` / `tauri.conf.json` / `Cargo.toml` / `Cargo.lock` | 版本 0.10.0 → 0.10.1 |

## 6. 明确没做的（下一轮候选）

审计里剩下的都不在这轮范围，按"改了能拿到什么"排：

**性能热点**（收益和账号数量成正比，需要用户实际账号数才能定优先级）
- `TabStrip.vue` 的 1 秒 `setInterval` 无条件整条标签栏重渲染，即使没有任何账号
  在生成、甚至 `showTabAiStatus` 关着；前端共有三个互相竞争的 1s 定时器。
- `ProfileSidebar.vue` 图标缓存整体替换（`iconCache.value = {...spread}`）导致
  N 个图标触发 N 次全列表重渲染；搜索未防抖，每次按键重跑一遍。
- `profile:status` 每次事件替换整个 `statuses` record；导航直接改深响应
  `Profile` 对象 → 连锁让 `filteredProfiles` / `visibleProfiles` / `allTags` 全量重建。
- 切标签会重新解析下载账本并把配置脚本 eval 给**每一个**账号 WebView
  （`push_download_cfg` / `layout_profile_webviews`）。
- 状态上报每次都把整个 `profiles.json` 全量写盘，而下载历史（上限 1000 条）
  就在同一个文件里；`refresh_profile_statuses` 会让 N 个标签串行写 N 次。
  这需要"内存态 + 防抖落盘"的设计，会动到持久化语义，不适合塞进本轮。
- `profiles.rs` / `webviews.rs` / `downloads.rs` 的读-改-写整表没有互斥：
  切标签与导航并发时有一方写入会丢（当前只丢时间戳，严重性低）。

**结构**
- `lib.rs` 从 v19 的 1085 行涨回 2119，其中 1457 行是一个 `#[cfg(test)] mod tests`
  —— 每轮新特性都往这里追加。拆回各模块即可回到约 500 行。
- `lib.rs:23-27` 的 glob re-export 让 `dead_code` 检查全 crate 失效，
  编译器不再帮忙找死代码。
- `webviews.rs` 1891 行里约 1100 行是内联 JS 字符串，可外置成 `include_str!` 文件
  （其中 `nativeFn` 反检测助手重复了 4 份）。
- `mbstatus://` / `mbimage://` / `mbkey://` 三条假导航分发可收成 `schemes.rs`。
- 前端：下载设置表单在两个组件里各写一份、4 个格式化函数重复、
  `App.vue` 还可抽 5 块 composables。

## 7. 验证

```
bash scripts/cargo-test.sh --lib        64 passed（新增 2 条）
bash scripts/cargo-msvc.sh clippy --lib --tests -- -D warnings   0 warning
npm run build（vue-tsc --noEmit + vite build）   通过
```

**需要人工点一遍的两处**（我跑不了 UI）：
1. 账号页打开时点「账号管理工具」和触发「启动诊断」，确认弹窗现在能盖在网页上面
   且按钮可点；关闭后账号页恢复显示。
2. 下载历史里点「移动」，确认弹出的是系统文件夹选择器、取消返回 null、
   选定后文件真的移动成功。
