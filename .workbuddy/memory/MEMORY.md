# 项目长期记忆：tauri-multi-account-browser

## 构建便携 exe（本机唯一可用姿势）

Tauri 2 项目，前端 Vue 3 + Vite，产物路径 `src-tauri/target/release/tauri-multi-account-browser.exe`。

在 Git Bash 中构建**必须**先注入 MSVC 环境，否则必然失败：

```bash
export PATH="/c/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/MSVC/14.50.35717/bin/Hostx64/x64:$PATH"
export LIB='C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.50.35717\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64'
npm run tauri:build -- --no-bundle   # --no-bundle 只出单文件 exe，跳过 NSIS/MSI，约 3 分钟
```

原因：
- Git 的 `/usr/bin/link.exe`（GNU coreutils）会抢在 MSVC `link.exe` 前面 → `link: missing operand after '\377\376'`。
- 不设 `LIB` 则找不到 `OleAut32.lib` → `LNK1181`。
- 不要试图用 `cmd //c vcvars64.bat`（MSYS 会转换掉 `//c`），PowerShell 工具禁用 cmd.exe，且 `npm.ps1` 有 `$LASTEXITCODE` 报错。

## 交付习惯
- 便携 exe 复制到桌面的文件名固定为 `Multi Account Browser.exe`（productName）。
- 桌面另有清理脚本 `清理Yue后台并删除桌面exe.bat`，按 productName 杀进程并删桌面 exe。
- 桌面 exe 若正在运行，`cp` 会 Permission denied。不要杀进程，先 `mv` 改名（Windows 允许重命名
  运行中的 exe）再复制，旧文件留作 `.bak.exe`。

## 校验命令（每轮改完必跑）
- 前端类型：`npm run build`（内含 `vue-tsc --noEmit`）。
- Rust 静态检查：`bash scripts/cargo-msvc.sh clippy --manifest-path src-tauri/Cargo.toml --lib --tests`（必须 0 warning）。
- 单测：`bash scripts/cargo-test.sh --lib`（2026-09-12 起 54 条全过）。
- 注入脚本语法：把 `src-tauri/src/webviews.rs` 里的 `const XXX_JS: &str = r#"..."#` 抽出来 `node --check`。
- 前端资源嵌入：`npm run verify:assets`。

## 账号 WebView ↔ 宿主的桥（改注入脚本前必读）
远程页面拿不到任何 IPC 权限，**唯一的回传通道是假导航 scheme**：
脚本 `location.href = 'xxx://...'` → 宿主 `on_navigation` 里 `return false` 取消导航并处理。
- `mbstatus://report?...` —— 状态抓取回传（积分 / 登录账号 / 答题状态）。
- `mbimage://open?u=<图片地址>&t=<提示>` —— 点图片改为宿主另开预览小窗（V25）。

三条硬约束：
1. 注入的点击拦截**必须判 `e.isTrusted`**：宿主的「AI 文件自动下载」是靠
   `el.click()` 合成事件触发锚点的，若把合成点击也拦成预览窗，
   自动下载会被整条打断（默认 `ai_exts` 含 `png,jpg,jpeg,webp`，命中率极高）。
2. 建窗/改窗必须走 `app.run_on_main_thread(...)`：在 `on_navigation` 回调里
   同步建 WebView2 会和 WebView2 自身的消息处理打架。
3. WebView2 **不允许同一个 UserDataFolder 挂两套不同的
   `CoreWebView2EnvironmentOptions`**。想复用账号数据目录（blob:/cookie 需要同分区）
   就要准备「建失败 → 换 label → 退默认目录」的兜底。

## GitHub
- 仓库：`https://github.com/momoqiqi-qwq/tauri-multi-account-browser`（PUBLIC，默认分支 master）。
- gh CLI 已登录 `momoqiqi-qwq`，有 repo 权限。
- 本地 `.git/refs/remotes/origin/` 会被莫名删除 → 用 `git pack-refs --all` 写进 packed-refs 才留得住。
- 注意：`.workbuddy*/memory/` 被 git 跟踪，写记忆会让工作区变脏。
