# CHANGES_V27 — 全局键盘快捷键：焦点在网页里也能切标签 / 刷新 / 关标签

## 1. 需求

> 让用户体验更好。

本轮选择补上"浏览器外壳"最基本的一块体验：**键盘快捷键**。此前没有任何快捷键，
切账号只能用鼠标点标签，刷新要点工具栏按钮；而这类工具的高频操作（切标签、
关标签、刷新、回地址栏）全部有成熟的键盘惯例可用。

## 2. 为什么之前做不了：焦点在原生子 WebView 里

应用的结构是"主窗口 + 多个原生子 WebView"，用户绝大部分时间键盘焦点都落在
账号网页里，宿主 Vue UI 的 `window.addEventListener('keydown')` 一个键也收不到。

更麻烦的是 WebView2 的"浏览器保留键"（Ctrl+T/W/L、Ctrl+数字、Ctrl+Tab、
F5/Ctrl+R、Alt+←/→……）：默认由 WebView2 的浏览器层直接吞掉处理，**页面 JS
同样收不到 keydown**。也就是说无论怎么写监听脚本，这批键都传不出来。

### 2.1 解法：关掉浏览器加速键 + 假导航回传

两步组合（都在 Windows/WebView2 上验证过 API 存在性，运行时要求 ≥ 92）：

1. **创建 WebView 后立刻调用 `ICoreWebView2Settings3::SetAreBrowserAcceleratorKeysEnabled(false)`**
   （经 `tauri::Webview::with_webview` 拿到 WebView2 COM 控制器）。微软文档明确：
   该设置只禁用"浏览器专属功能键"（查找/打印/重载/缩放/前进后退等），**不影响
   Ctrl+C/V/X/A/Z、Home/End、PgUp/PgDn 等文本编辑键**。禁用后这批键作为普通
   keydown 到达页面。
2. 页面里的 `KEY_FORWARD_JS` 对白名单内的组合 `preventDefault` 后发起
   `mbkey://press?k=<组合键>` 假导航 —— 与 `mbstatus://` / `mbimage://` 完全相同
   的套路：宿主在 `on_navigation` 拦截、取消导航（页面不动），把组合键
   `emit('profile:hotkey')` 给主界面执行。远程页面始终拿不到任何 IPC 权限。

主界面 WebView 也同样关闭加速键：焦点在侧栏/地址栏时按 F5/Ctrl+R，
旧行为是**重载整个 Vue 外壳**（丢界面状态），现在统一转成对当前账号页的刷新。

### 2.2 被禁掉的原生功能去了哪

| 原生行为 | 现状 |
| --- | --- |
| F5 / Ctrl+R 刷新 | 宿主接管，遵守"刷新后恢复位置"偏好（比原生更好） |
| Alt+←/→ 前进后退 | 宿主接管（history.back/forward） |
| Ctrl+T / Ctrl+W / Ctrl+Tab / Ctrl+1-9 | 宿主接管（新建账号 / 关标签 / 切标签） |
| Ctrl+L 聚焦地址栏 | 宿主接管（先 `focus_main_ui` 把焦点切回主 WebView） |
| Ctrl+J 下载历史 | 宿主接管 |
| Ctrl+±/0 缩放 | 暂无替代，v29 计划做每账号缩放 |
| Ctrl+F 页内查找、Ctrl+P 打印 | 暂无替代（WebView2 自带 UI 被禁），已知取舍 |

## 3. 快捷键清单（与 Chrome 对齐的部分惯例）

| 按键 | 动作 |
| --- | --- |
| Ctrl+Tab / Ctrl+Shift+Tab | 下一个 / 上一个标签页 |
| Ctrl+1..8 / Ctrl+9 | 第 N 个 / 最后一个标签页 |
| Ctrl+W | 关闭当前标签页 |
| Ctrl+T | 新建账号 |
| Ctrl+L | 聚焦地址栏（全选便于直接输入） |
| F5 / Ctrl+R（/Ctrl+F5） | 刷新（遵守"刷新后恢复位置"） |
| Alt+← / Alt+→ | 后退 / 前进 |
| Ctrl+J | 打开下载历史 |

设置（更多设置 → 快捷键）里加了完整按键表和与浏览器行为的差异说明。

转发脚本只放行白名单：输入法组合中（`isComposing`）、AltGr（Ctrl+Alt）、
Ctrl+Shift+Tab 之外的 Shift 组合一律不转发，避免干扰站点自己的快捷键和输入法。

## 4. 实现细节

- `webviews.rs::KEY_FORWARD_JS`：capture 阶段监听 keydown，组合键归一化为
  `ctrl → alt → shift → 键名`（如 `ctrl+shift+tab`），`encodeURIComponent` 后拼进
  假导航 URL；`+` 编码成 `%2B`，`query_map` 的 form 解码恰好还原。
- `webviews.rs::disable_browser_accelerator_keys`：Windows 上经
  `with_webview` → `ICoreWebView2Controller::CoreWebView2` → `Settings` → cast
  `ICoreWebView2Settings3` 下发；非 Windows 编译为空实现。任何一步失败只打日志，
  绝不影响 WebView 本身。
- 依赖新增 `webview2-com = 0.38` + `windows-core = 0.61`（仅 Windows target），
  版本与 tauri-runtime-wry 依赖的完全一致，COM 接口类型统一，无需引入 wry。
- `profiles.rs::focus_main_ui`：把原生焦点从账号 WebView 切回主界面，
  之后 Vue 侧 `AddressBar.focusUrl()`（defineExpose 出来的聚焦+全选）才能生效。
- `App.vue::runShortcut`：两路来源（宿主 keydown / `profile:hotkey` 事件）共用；
  `isSuppressed()` 时（弹层打开、WebView 隐藏）一律忽略，避免在对话框底下偷切标签。
- 设置页新增"快捷键"标签页（静态按键表 + 行为差异说明）。

## 5. 已知限制

- WebView2 Runtime < 92 时 `SetAreBrowserAcceleratorKeysEnabled` 无效果：
  保留键仍被吞，快捷键只在宿主 UI 获得焦点时可用（不影响其它功能）。
- Ctrl+F / Ctrl+P 原生 UI 被禁用，暂无替代；站点若自带搜索（如聊天记录搜索）不受影响。
- 快捷键在账号网页里生效依赖注入脚本，页面尚未加载完成的极端瞬间按快捷键无效。

## 6. 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/lib/shortcuts.ts` | 新增：组合键归一化、快捷键表、`comboFromEvent` |
| `src/App.vue` | `runShortcut` + 宿主 keydown + `profile:hotkey` 监听 + 地址栏聚焦 |
| `src/components/AddressBar.vue` | `defineExpose({ focusUrl })`，按钮提示带快捷键 |
| `src/components/AppPreferencesDialog.vue` | 新增"快捷键"标签页 |
| `src/style.css` | 快捷键表样式 |
| `src-tauri/src/webviews.rs` | `KEY_FORWARD_JS`、`disable_browser_accelerator_keys`、`mbkey://` 分支、注入脚本 |
| `src-tauri/src/profiles.rs` | `focus_main_ui` 命令 |
| `src-tauri/src/lib.rs` | `KEY_SCHEME`、`HotkeyEvent`、setup 里主 WebView 关加速键、命令注册、2 条单测 |
| `src-tauri/Cargo.toml` | Windows 下新增 `webview2-com`、`windows-core` |

## 7. 验证

```
vue-tsc --noEmit + vite build                 通过
cargo test --lib                              56 passed（新增 2 条）
clippy --lib --tests                          0 warning
bash scripts/tauri-msvc.sh build --no-bundle  通过
npm run verify:assets                         css / js / html 逐字节一致
```

新增单测：
- `key_scheme_script_and_host_agree_on_contract` —— 转发脚本与宿主解析必须对齐
  `mbkey://press?k=`、必须编码、必须 preventDefault；
- `key_scheme_navigation_parses_combo_from_query` —— `encodeURIComponent` 出来的
  `%2B` 经 `query_map` 解码必须还原成 `+`（脚本与宿主的编码契约）。
