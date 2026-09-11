# V8 changes

## 本轮完成

- 在每个账号网页右下角加入悬浮刷新按钮；按钮直接注入账号子 WebView，因此不会被 Tauri 原生子 WebView 遮挡。
- 悬浮按钮使用 Shadow DOM 隔离样式，降低与目标网站 CSS 冲突的概率；包含 hover / focus / loading 反馈、键盘焦点和 `aria-label`。
- 悬浮刷新不直接调用 `location.reload()`，而是通过 `mbaction://reload` 请求宿主，再由 Vue 统一执行刷新，因此会继续遵守“刷新后恢复页面位置”设置。
- 把浏览器动作抽成 `browserActionFor(id, action)`，顶部刷新按钮和右下角悬浮按钮复用同一条逻辑，并增加统一错误提示。
- 新增 `profile:floating-reload` 事件监听，并在组件卸载时正确解除监听，避免热重载/窗口重建后的重复处理。
- 统一前端、Cargo 与 Tauri 配置版本号为 `0.4.0`，修复原项目 `package.json` 与 Rust/Tauri 版本不一致的问题。

## 结构检查后的优化建议

1. **拆分 Rust 单体文件**：`src-tauri/src/lib.rs` 已同时承担 profile、WebView、状态扫描、下载、代理、指纹等职责，建议下一版拆为 `profiles.rs / webviews.rs / downloads.rs / status.rs / isolation.rs / commands.rs`。
2. **前端 composables 化**：把 `App.vue` 的 profile/browser/status/download IPC 与状态拆成 `useProfiles`、`useBrowserTabs`、`useProfileStatus`、`useDownloads`，让 App 只负责布局和组件编排。
3. **统一 IPC 错误层**：目前仍有大量 `invoke(...).catch(() => ...)` 静默吞错。建议做 `safeInvoke`/错误码映射，区分“可忽略后台刷新失败”和“用户操作失败”。
4. **多账号资源治理**：账号数量大时，最大的 UX 风险是 WebView 内存/CPU。建议下一版支持“后台标签休眠 + 最大同时存活 WebView 数 + 最近使用淘汰”。
5. **状态中心升级**：增加“生成中 / 已完成 / 低积分 / 异常代理”筛选、排序和一键跳转，比继续增加侧栏信息密度更有价值。
6. **操作反馈一致化**：导航、切换账号、刷新、清数据、代理测试等动作增加统一 loading/disabled 状态，避免用户连续点击产生竞态。

## 建议下一个版本（V9）

优先做“**后台标签休眠 + 统一 IPC 错误与 loading 状态**”。这两项会直接改善多账号场景的速度、内存占用和操作确定性，收益高于继续添加新功能。之后再做快捷键中心与下载任务队列。
