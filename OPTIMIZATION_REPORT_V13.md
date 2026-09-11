# v13 代码结构与下一版本建议

## 本轮已经落地的结构优化

1. **URL 规则收口**
   - 前端新增 `src/lib/profileUrl.ts`，避免 `App.vue`、账号设置、工具栏各自写一套 URL 规则。
   - Rust 侧统一 `normalize_url / repair_profile_urls / profile_start_url`，WebView 启动不再直接依赖脏字段。

2. **失败状态一致性**
   - `activateProfile` 改为“后端成功后再写前端标签状态”。
   - 账号设置保存失败保留表单，不再先关闭再报错。
   - 分组移动改为单个 Rust command，一次 store 保存。

3. **启动性能**
   - Profile 必须先加载；其后的状态、设置、下载历史并行加载。

4. **本地状态容错**
   - 折叠分组 localStorage 加安全解析。
   - 拖拽取消补充状态清理。

## 目前代码结构最值得继续优化的地方

### 1. `src-tauri/src/lib.rs` 过大

当前约 2900+ 行，同时承担：

- Profile CRUD；
- WebView 生命周期；
- 代理/指纹注入；
- 状态抓取脚本；
- 下载保护/下载历史；
- 文件操作；
- Tauri command 注册。

建议拆为：

```text
src-tauri/src/
  profiles.rs
  webviews.rs
  downloads.rs
  status.rs
  settings.rs
  scripts.rs
  commands.rs
  lib.rs
```

优先把大段注入 JS 移到独立 `.js` 文件，用 `include_str!()` 引入，降低 Rust 文件噪音并让 JS 可单独做语法测试。

### 2. `src/App.vue` 仍然是“总控制器”

当前约 700+ 行，账号、标签、下载、状态、自动刷新、WebView suppression、设置都在一个组件里。

建议拆成 composables：

```text
src/composables/
  useProfiles.ts
  useProfileTabs.ts
  useDownloads.ts
  useProfileStatus.ts
  useBrowserActions.ts
```

`App.vue` 最终只负责组合布局与绑定事件。

### 3. Tauri invoke 错误处理应统一

目前不同函数有的 `catch(() => undefined)`，有的 toast，有的直接抛出。建议封装：

```text
invokeResult / invokeOrToast / invokeQuietly
```

这样能统一错误文案、日志、可恢复/不可恢复等级，并减少重复 try/catch。

### 4. Profile 数据需要显式 schema version

本轮已经做 URL 字段兼容，但随着版本继续增加字段，只靠 `#[serde(default)]` 会越来越难追踪迁移行为。

建议 store 增加：

```json
{
  "schemaVersion": 2,
  "profiles": []
}
```

启动时按版本顺序执行 migration，并记录迁移结果，避免未来出现“某个旧账号突然打不开”的不可诊断问题。

### 5. `last_url` 写盘可以进一步降频

当前页面导航会写入账号 metadata。对于路由变化频繁的 SPA，建议后续做 300~800ms debounce，或者只在：

- 标签切换；
- WebView 休眠；
- 应用退出；
- 主路径真正变化；

时持久化，减少 store I/O。

### 6. 建议补自动化测试

至少覆盖：

- URL 修复迁移；
- Profile 创建/更新；
- 分组移动排序；
- session restore；
- download guard rule；
- UI preference migration。

Rust 侧优先给纯函数加 `#[cfg(test)]`，前端给 `profileUrl.ts`、session/preferences normalization 做 Vitest。

## 下一版本建议：v0.6.0「账号模板 + 恢复中心」

我建议不要马上继续堆工具栏小功能，而是把“账号可批量维护、出问题可恢复”做成完整能力。

### A. 全局默认账号网址 + 继承/覆盖

- 更多设置里增加“新账号默认主页”。
- 账号设置可选择：`继承全局` / `自定义`。
- 修改全局主页时可以选择是否同步到所有“继承全局”的账号。
- 这会彻底消除源码中对某个网站常量的业务依赖。

### B. 账号模板

保存一套：

- 默认网址；
- 分组；
- 时区/语言；
- User-Agent 策略；
- 指纹防护；
- 常用工具栏链接。

新建账号时一键从模板创建，减少重复填写。

### C. 批量创建 / 导入 / 导出

支持 CSV/JSON：

- 批量创建账号；
- 批量改分组/主页；
- 导出账号元数据备份（不导出密码）；
- 导入前预览和冲突检查。

### D. 恢复中心

当账号打不开时，不只弹一个错误：

- 显示实际尝试的 URL；
- 检查 URL / 代理 / WebView 是否可用；
- 提供“回到默认主页”“重建 WebView”“清除当前错误 last_url”按钮；
- 记录最近一次失败原因。

这会比继续加零散功能明显提升日常可靠性。

### E. 性能面板

账号较多时显示：

- 当前活跃 WebView 数；
- 休眠数量；
- 每个账号最近活跃时间；
- 可选“一键休眠全部后台账号”。

这样用户能理解应用为什么占内存，也能主动控制资源。
