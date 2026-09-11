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
