# Tauri 2 多账户浏览器容器 MVP

> 当前源码版本：**0.5.1 / v13**。本轮重点修复空网址账号无法进入，并统一账号主页、URL 修复、保存失败恢复与分组拖拽一致性。详见 `CHANGES_V13.md`。

## v0.2.0 / 优化版新增

- 下载历史：支持搜索、成功/失败筛选、多选、全选、按账号分组/平铺切换、10/20/50/100 行分页。
- 批量操作：选中记录后可一次选择目标目录批量移动、仅移除选中历史，或批量删除文件与记录。
- 下载历史保留上限：100/200/500/1000 条可选，后端实际保护范围 50~2000。
- 更多设置：地址栏新增全局设置入口，下载设置仍保留右侧栏快捷入口。
- 主题系统：Claude 浅色/深色、午夜蓝、石墨黑、玻璃蓝、纸张奶油、紫晶、Win11、日落橙紫、薄荷浅色。
- 显示自由度：舒适/紧凑密度、90%/100%/110% 字号、动画开关、下载默认视图和每页行数。
- 体验修复：下载设置 Popover 也接入 WebView suppression，避免原生子 WebView 把浮层盖住。


一个基于 **Tauri 2.x + Rust + Vue 3 + TypeScript + Element Plus** 的多账户浏览器 Shell。它不保存用户名/密码，只保存账号元数据；登录态由每个 Profile 自己的 WebView Cookie / LocalStorage / IndexedDB / Cache 维护。

> 这个 MVP 采用“一个 Tauri 主窗口 + 多个原生 child WebView”的结构，**顶部标签栏里一个标签页对应一个账号**。标签栏、左侧栏和地址栏由 Vue 渲染，网站内容由 Rust 动态创建的原生 WebView 渲染。切换标签页只是 `hide()/show()`，不会销毁 WebView，因此会保留页面状态、滚动位置和 JS 运行状态。

## 1. 技术结论先说清楚

### Windows / WebView2

Tauri 2 的 `WebviewBuilder::data_directory(...)` 可以为每个 Profile 指定独立目录，本项目使用：

```text
{app_data_dir}/profiles/{profile_id}/
```

在 Windows 上这对应 WebView2 的独立 User Data Folder，Cookie、LocalStorage、IndexedDB、Cache 等会按 Profile 分开。这是本项目最稳妥的平台。

### macOS / WKWebView

WKWebView **不支持任意 `data_directory`**。Tauri 2.9+ 提供 `data_store_identifier([u8; 16])`，对应 `WKWebsiteDataStore(identifier:)`，可用 UUID 给每个 Profile 建立独立数据存储，但该能力只在 **macOS 14+** 可用。

因此本 MVP 的正式支持口径是：

- Windows 10/11 + WebView2：完整持久化隔离。
- macOS 14+：通过 `data_store_identifier` 完整持久化隔离。
- macOS 13 及更早：**不承诺多个持久 Profile 的完全物理隔离**。

旧 macOS 如果必须强隔离，建议二选一：

1. 为每个 Profile 启动独立 helper app / 独立应用进程，让每个进程拥有不同 bundle/container 数据域；这是最可靠但工程量最大的办法。
2. 降级为“同一时间只打开一个 Profile，切换前清理 `WKWebsiteDataStore`”，只能做到顺序使用，不适合同时保留多个账号会话。

不要在旧 macOS 上把“不同目录字符串”包装成已经隔离——WKWebView 不会按这个目录工作。

## 2. 整体架构

```text
┌──────────────────────────────────────────────────────────┐
│ Tauri 主 Window: main                                    │
│ ┌────────────┐ ┌───────────────────────────────────────┐ │
│ │ Vue Sidebar│ │ Vue TabStrip                          │ │
│ │ Profile A  │ ├───────────────────────────────────────┤ │
│ │ Profile B  │ │ 原生 child WebView (当前 Profile)    │ │
│ │ Profile C  │ │                                       │ │
│ └────────────┘ └───────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
        │ invoke / event                    │
        ▼                                   ▼
┌──────────────────────────────────────────────────────────┐
│ Rust                                                     │
│ - Profile 元数据 CRUD                                    │
│ - tauri-plugin-store                                     │
│ - 创建/隐藏/显示/销毁 WebView                            │
│ - Windows data_directory                                 │
│ - macOS 14+ data_store_identifier                        │
│ - navigate / back / forward / reload                     │
└──────────────────────────────────────────────────────────┘
```

职责划分：

- Vue：账号列表、创建/重命名/删除、拖拽排序、地址栏、浏览区占位矩形。
- Rust：真正的 WebView 生命周期、浏览数据隔离、导航、清数据、元数据持久化。
- `tauri-plugin-store`：只保存名称、URL、排序、创建时间等元数据，不保存密码。

## 3. 项目目录

```text
tauri-multi-account-browser/
├─ package.json
├─ vite.config.ts
├─ tsconfig.json
├─ index.html
├─ src/
│  ├─ main.ts
│  ├─ App.vue
│  ├─ style.css
│  ├─ types.ts
│  └─ components/
│     ├─ ProfileSidebar.vue
│     ├─ TabStrip.vue
│     ├─ AddressBar.vue
│     ├─ BrowserViewport.vue
│     └─ ProfileSettingsDialog.vue
└─ src-tauri/
   ├─ Cargo.toml
   ├─ build.rs
   ├─ tauri.conf.json
   ├─ capabilities/default.json
   └─ src/
      ├─ main.rs
      └─ lib.rs
```

## 4. 运行

前置环境：Node.js 20+、Rust stable、Tauri 2 系统依赖。

Windows 需要 WebView2 Runtime（Windows 10/11 通常已安装）。macOS 建议 14+。

```bash
npm install
npm run tauri:dev
```

打包：

```bash
npm run tauri:build
```

## 5. 核心隔离代码

核心实现在 `src-tauri/src/lib.rs`。

Windows：

```rust
#[cfg(target_os = "windows")]
{
    let data_dir = profile_data_dir(app, &profile.id)?;
    std::fs::create_dir_all(&data_dir)?;
    builder = builder.data_directory(data_dir);
}
```

macOS 14+：

```rust
#[cfg(target_os = "macos")]
{
    let uuid = Uuid::parse_str(&profile.id)?;
    builder = builder.data_store_identifier(*uuid.as_bytes());
}
```

切换账号：

```text
activate_profile(A)
  ├─ A 尚未创建 -> 创建 child WebView
  ├─ hide(B/C/...)
  ├─ set_position / set_size(A)
  └─ show(A) + focus(A)
```

因此账号切换不触发 reload。

## 6. 每个标签页一个账号：隔离与防关联

### 6.1 UI 模型

- 顶部标签栏（TabStrip）：**一个标签页 = 一个账号容器**，标签上有账号专属颜色圆点，临时账号带“临时”角标。`+` 打开未打开的账号或新建账号；`×` 或中键关闭标签页。
- 关闭标签页只销毁 WebView，磁盘上的会话数据保留，**重新打开即恢复登录态**；隐身账号例外，关闭即清空。
- 左侧栏是账号库：新建、隔离设置、清除数据、删除、拖拽排序；绿点表示该账号已作为标签页打开。
- 切换标签页 = `hide()/show()`，不触发 reload。

### 6.2 隔离层次

| 层次 | 机制 | 说明 |
| --- | --- | --- |
| 存储 | 每账号独立 UserDataFolder（Windows）/ `WKWebsiteDataStore`（macOS 14+） | Cookie、LocalStorage、IndexedDB、Cache 物理分目录，互不可见 |
| 网络 | 每账号独立代理（http / https / socks5，Windows/Linux） | 代理参数挂在各自的 WebView2 环境上，只影响该账号的浏览器进程 |
| WebRTC | 配置代理的账号自动 `--force-webrtc-ip-handling-policy=disable_non_proxied_udp` | 禁止 WebRTC 绕过代理直连，防止真实 IP 泄漏形成关联 |
| 身份 | 每账号可选 User-Agent / 时区 / 语言区域 | 时区、语言经注入脚本接管 `Intl`/`navigator.language`；语言同时通过 `--lang` 下发 |
| 指纹 | 按账号确定性噪声 | 画布 `toDataURL/toBlob/getImageData` 加 ±1 噪声；GPU 字符串掩码；`hardwareConcurrency/deviceMemory` 按账号固定 |

指纹噪声由账号 UUID 派生种子（FNV-1a）生成：同账号跨页面、跨会话读到同一噪声（一致性），不同账号对同一页面读到不同噪声（区分度）。画布噪声只作用于“读出结果”，不改用户实际看到的画布。

### 6.3 防封号实践建议（诚实版）

代码能做的是**隔离与去关联**，没有任何浏览器能保证账号不被封。平台风控通常综合以下信号，按重要程度排序：

1. **IP**：同一出口 IP 登录多个账号是最强关联信号。给每个账号配置固定独立代理，并保持“账号—IP”长期绑定，不要频繁更换。
2. **环境与 IP 一致性**：代理是香港 IP 就配 `Asia/Hong_Kong` 时区；UA 一般留空跟随系统，除非明确需要。
3. **行为特征**：多账号同一时间高频做同样操作（注册、发帖、点赞）比指纹更容易触发风控。
4. **深层指纹**：WebGL 渲染结果、音频指纹、字体枚举只做了轻量处理；对抗高等级风控需要真机/多设备隔离或更完整的反指纹方案。
5. **不要在同一个账号容器里登录另一个账号**：一次交叉登录就可能把两个账号的存储与行为关联起来。

### 6.4 UI 弹层与原生 WebView 的层叠关系

Vue 渲染的标签栏“+”菜单、账号操作菜单、设置对话框、确认弹窗都绘制在主 UI WebView 上，而承载网页的原生子 WebView 永远位于其上层。因此弹层打开期间应用会临时隐藏当前账号的 WebView（引用计数式管理，多个弹层叠加时全部关闭才恢复），关闭后自动恢复显示——这是 `src/lib/suppress.ts` + `set_profile_webview_visible` 命令的职责。

### 6.5 谷歌登录防踢

谷歌对嵌入式浏览器环境的登录拦截主要来自 UA 中的 `Edg/` 标识与“全新环境 + IP 漂移”组合。本项目的做法：

1. 设置对话框提供“填入 Chrome 同源 UA”：取当前 WebView2（常青 Chromium）真实 UA 去掉 `Edg/` 标识，版本与引擎一致，不是伪造旧版本。
2. 账号固定一套“UA + 独立代理 IP + 时区 + 语言”，且不要频繁清除数据——每次清数据都是一次“新设备登录”。
3. 仍无法保证 100% 通过：谷歌风控是黑盒，极端情况会要求验证。这是所有内嵌 WebView 浏览器（包括 Electron 应用）的共同限制。

### 6.6 隔离设置的生效时机

代理 / UA / 时区 / 指纹防护 / 隐身开关都挂在 WebView 创建参数上。保存设置时应用会自动销毁并重开该账号的标签页（提示“隔离设置已生效”）；账号未打开时则下次打开生效。重启应用不会丢失这些设置（保存在 `profiles.json`）。

## 7. Google OAuth：必须按真实限制设计

### 7.1 站点内嵌 Google 登录

Google OAuth 政策不支持开发者控制的 embedded user-agent 进行 OAuth 授权。Tauri 在桌面端使用系统 WebView2 / WKWebView，而不是 Electron 自带 Chromium，但它仍然是嵌入式浏览环境，所以**不能保证 Google 登录长期可用**。

本项目的处理原则：

- 保留 WebView 默认 User-Agent。
- 不伪装 Chrome UA，不使用 UA 欺骗规避风控。
- 普通站点账号密码登录照常使用。
- Google 登录按钮可以 best-effort 尝试，但如果 Google 或目标站拒绝，视为平台限制，而不是通过伪造 UA 绕过。

### 7.2 外部浏览器 OAuth 的正确兜底

只有当**你控制 OAuth Client ID、redirect URI 和应用后端**时，才能做可靠、合规的系统浏览器流程：

```text
Profile 点击“Google 登录”
        │
        ▼
Rust 生成 state + PKCE
        │
        ▼
系统默认浏览器打开 Google Authorization URL
        │
        ▼
Google 登录 / 同意
        │
        ▼
https://127.0.0.1:{random_port}/callback?code=...&state=...
或应用自定义 Deep Link
        │
        ▼
Rust 校验 state，交换 authorization code
        │
        ▼
你的服务端建立该 Profile 对应的站点会话
        │
        ▼
把“你自己站点”的 session cookie 设置给对应 WebView
```

伪代码：

```rust
async fn login_with_google(profile_id: String) {
    let pkce = create_pkce();
    let state = random_state_bound_to(profile_id);
    let callback = start_loopback_http_server();

    shell_open(build_google_authorize_url(pkce, state, callback));

    let code = callback.wait_for_code();
    verify_state(code.state, profile_id);
    let tokens = exchange_code_with_google(code, pkce);

    // 关键：只能让你控制的后端把 Google identity 换成你自己站点的 session。
    let app_session = my_backend.create_session(tokens.id_token);

    let webview = get_profile_webview(profile_id);
    webview.set_cookie(app_session.cookie);
    webview.navigate("https://your-site.example/app");
}
```

### 7.3 为什么不能对任意第三方网站“外部 Google 登录后注入 token”

如果 `chat01.ai` 或其他第三方 SaaS 自己持有 Google OAuth Client，我们既不控制它的 redirect URI，也不控制它服务器如何把 Google code/token 换成自己站点的 session。因此一个通用桌面壳无法合法、可靠地截获 Google token 后自行制造该第三方网站的 Cookie。

所以推荐顺序是：

1. **普通账号密码 / 邮箱登录：完全使用 Profile WebView。**体验最好，也是 MVP 主路径。
2. **第三方网站的 Google 登录：在 WebView 中 best-effort；被拦截就提示用户该站点不支持嵌入式 OAuth。**
3. **你自己控制的网站：实现系统浏览器 + PKCE + loopback/deep-link 回调，再由自己后端建立站点 session。**这是合规推荐方案。

## 8. chat01.ai 隔离验收用例

建议至少做以下测试：

```text
1. 创建 Profile A -> 打开 chat01.ai -> 登录账号 A
2. 创建 Profile B -> 打开 chat01.ai -> 登录账号 B
3. A/B 标签页来回切换 20 次：页面不 reload，滚动位置应保持
4. 完全退出应用再启动：重新打开 A/B 标签页，各自仍保持登录态（非隐身）
5. 对 A 执行“清除数据”：A 应退出登录，B 不受影响
6. 删除 A：Windows 对应 profiles/{A_UUID}/ 目录被移除
7. 创建隐身 Profile C：关闭应用再启动后不应保留登录态
8. 关闭 A 的标签页再从左侧栏/「+」重新打开：登录态保留（非隐身）
9. 给 A、B 配置不同代理：两侧访问 IP 检测类站点（如 ip.sb）应显示不同出口
```

## 9. 已包含

- 顶部标签页：**一个标签页一个账号**，关闭标签页销毁 WebView 但保留磁盘会话，重开即恢复登录态。
- 账号新建、隔离设置、删除、拖拽排序；账号元数据 `tauri-plugin-store` 持久化。
- Windows 独立 `data_directory`；macOS 独立 `data_store_identifier`。
- 隐身账号：使用 WebView incognito/nonPersistent 模式。
- 多 WebView 常驻，标签页切换 `hide/show` 不 reload。
- 地址栏、前进、后退、刷新。
- **每账号独立代理**（http/https/socks5，Windows/Linux）。
- **每账号可选 UA 与时区**（时区经注入脚本伪装）。
- **指纹防护**：画布噪声、GPU 字符串掩码、硬件并发/内存按账号固定（默认开启，可关闭）。
- 清除账号浏览数据。
- 导航 URL 回传给 Vue 并持久化 `last_url`。

## 10. 已知限制 / 二期建议

1. **macOS 14 以下**：没有等价的持久化自定义 WKWebsiteDataStore 标识；需要 helper app / 多进程方案才能强隔离。macOS 上代理与 UA 参数不可用（WKWebView 无公开代理 API）。
2. **Google OAuth**：不能承诺任意第三方站点在 embedded WebView 中可用，也不应通过 UA spoofing 绕过。
3. **`window.open` / OAuth 弹窗**：MVP 未做完整弹窗路由。二期应使用 `on_new_window`，并确保新窗口继承当前 Profile 的相同数据存储。
4. **同一账号多标签页**：当前是“一标签页 = 一账号”。若未来支持同账号多 tab，key 需扩展成 `profile_id + tab_id`，且同一 Profile 的 tab 必须共享同一个 data directory / dataStoreIdentifier。
5. **登录状态角标**：绿点仅代表容器已作为标签页打开，不代表目标站已登录。真正登录状态需要站点适配器或用户脚本判断。
6. **未读红点**：需要站点级 DOM/消息适配器，不适合做成完全通用的首期能力。
7. **下载管理、文件上传、权限请求**：需要补 Tauri/Wry 对应事件和权限策略。
8. **代理**：已支持 http/https/socks5（Windows/Linux）。Chromium 的 `--proxy-server` 不支持内嵌用户名密码，带认证的代理需要本地转发工具（如 gost / clash 本地端口）转成无认证端口。另外代理在 WebView 创建时固化，不要期望运行中热切换（应用会自动重建 WebView）。
9. **指纹防护边界**：画布/GPU/硬件做了按账号一致性噪声，但 WebGL 渲染管线、音频指纹、字体枚举、屏幕参数等仍是真实值；防关联不是防封承诺，请配合独立代理与隔离的行为习惯使用。
10. **安全**：当前 MVP 为了快速开发把主 UI CSP 设为 `null`。生产版应恢复严格 CSP，并限制所有 Tauri command 的参数/来源。
11. **站点兼容性**：DRM、Passkey、某些硬件 WebAuthn、浏览器扩展、企业 SSO 可能和完整 Chrome/Edge 行为不同。

## 11. 为什么选 Vue 3 + Element Plus

需求本身偏桌面管理工具：侧栏、卡片、弹窗、下拉菜单、表单较多。Vue 3 + Element Plus 能用较少代码完成 MVP，同时和 Tauri 的 `invoke/event` 模型配合直接。浏览器内容本身不是 Vue iframe，而是 Rust 创建的原生 WebView，因此前端框架不会影响账号隔离强度。

## v14 / 0.6.0

新增全局默认账号网址与“继承全局 / 单独覆盖”主页模式；加入账号模板、批量创建、JSON 导入导出和账号启动诊断中心。账号打开失败时可直接清除错误 last_url、回主页、切换全局主页或禁用代理后重试。详见 `CHANGES_V14.md`。
