# CHANGES_V21 — 诊断中心增强

对应 `OPTIMIZATION_REPORT_V14.md` 第 7 条：**诊断中心继续增加 WebView2 Runtime 检测、
代理真实连通性检测、数据目录占用检测和错误码分类**。

---

## 1. 错误码分类（新）

以前"打开失败"只会把后端的原始错误串丢给用户，通常是一长串英文
（`failed to create webview ...`、`os error 32`），用户看不懂也不知道该怎么办。

新增纯函数 `classify_error(&str) -> DiagnosticCode`，把错误串归成 7 类，
每类配一句修复建议（`DiagnosticCode::hint()`）：

| code | 含义 | 建议方向 |
|---|---|---|
| `webview2_runtime` | 缺 WebView2 Runtime / 安装损坏 | 安装或修复 Edge WebView2 Runtime |
| `data_dir` | 数据目录不可读写（权限/占用/磁盘满） | 查磁盘、查安全软件锁定 |
| `proxy` | 代理格式错误或不可达 | 查代理地址端口，或先禁用代理 |
| `network` | 网址不可达 / 超时 / DNS 失败 | 查网络、DNS、目标站点 |
| `invalid_url` | 网址格式非法 | 改回 http/https 完整网址 |
| `bad_last_url` | 上次访问的网址导致启动失败 | 清除 last_url 回主页 |
| `unknown` | 未能归类 | 走下方修复操作 / 看日志 |

**判定顺序是设计的一部分**：`proxy` 的错误里几乎必然带 `connection`，
如果先判网络就会误导用户去查网络。所以"越具体越靠前"：
WebView2 → 代理 → 数据目录 → 网络 → 网址 → unknown。

`diagnose_profile` 新增 `error: Option<String>` 参数，前端把失败原因传进来，
Rust 侧归类后返回 `code` + `hint`，诊断对话框顶部直接显示建议。

## 2. WebView2 Runtime 检测（新）

没有 WebView2 Runtime，一个标签页都开不出来 —— 这是"打开失败"时第一件要确认的事。

`detect_webview2_runtime()` 返回 `{ installed, version, source }`：

1. **注册表**（最准，能拿到版本号）：查 EdgeUpdate 的 WebView2 客户端 ID
   `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` 的 `pv` 值，依次试
   `HKLM\SOFTWARE\WOW6432Node` → `HKLM\SOFTWARE` → `HKCU\Software`。
2. **文件系统兜底**：扫 `%LOCALAPPDATA%\Microsoft\EdgeWebView\Application`
   和 `Program Files (x86)\Microsoft\EdgeWebView\Application`，
   找含 `msedgewebview2.exe` 的版本号目录，取最大的。

`source` 记录版本来源（`registry-hklm` / `registry-hkcu` / `filesystem` / `none`），
方便远程排查。

**为什么需要兜底**：`reg.exe` 在部分受限环境（锁定策略、精简系统）里跑不起来，
此时注册表路径会静默失败。实测本机注册表查询被环境策略拦掉，
文件系统兜底仍能正确识别到 `152.0.4191.66`。
目录里还有 `SetupMetrics` 这类非版本目录，用"首位是数字"过滤掉。

顺带：把 `hidden_curl()` 里"不弹控制台黑框"的逻辑抽成通用的
`hidden_command(program)`，新命令（reg / explorer）都走它。

## 3. 数据目录占用检测（新）

`measure_dir(path, max_files)` 递归统计账号数据目录的**字节数和文件数**。

- 浏览数据目录动辄几万个小文件，全量遍历会让诊断对话框卡住几秒，
  所以设 `MAX_DIAG_FILES = 20_000` 上限，超了就停并把 `truncated` 置 true。
  用户只需要量级（几十 MB 还是几个 GB），不需要精确到字节。
- 目录遍历放在 `spawn_blocking` 里：`#[tauri::command]` 的**同步**命令跑在主线程，
  同步遍历会直接冻住界面。`diagnose_profile` 因此改成了 `async`。

配套新增命令 `open_profile_data_dir(id)`：在文件管理器里打开该账号的数据目录。
**路径由 Rust 侧按账号 id 拼，不接受前端传路径** —— 否则这个命令就退化成
"让前端指定任意路径去启动进程"了。

## 4. 代理检测增强

`test_profile_proxy` 的返回值新增三个字段：

- `latency_ms`：本次 curl 往返耗时。既能反映代理质量，也能区分"连不上"和"太慢"。
- `via_proxy`：请求是否真的走了代理（直连账号为 false）。
- `endpoint`：实际使用的代理地址，直连时为空串。

宿主提示语也跟着改了：`账号A 出口：1.2.3.4（中国 上海 · 电信） · 代理 socks5://... · 320 ms`。

## 5. 前端

- `src/types.ts` 补上 `ProfileDiagnostic` / `WebView2Status` / `DataDirUsage` /
  `ProxyTestResult` / `DiagnosticCode`，诊断对话框不再用 `any`。
- `ProfileDiagnosisDialog.vue` 重写：顶部多一条"原因归类 + 修复建议"提示条；
  描述表新增「错误归类」「WebView2 Runtime」「数据目录占用」（带"打开目录"按钮）。
- `App.vue` 的 `testProxy` 用上新的 `latency_ms` / `via_proxy` / `endpoint`。
- 新增 `src/composables/` 无改动；`ProfileDiagnosisDialog` 的样式加了 `.diag-muted`。

## 6. 测试

新增 7 个用例（**共 37 个，全过**）：

- `classify_error_maps_webview2_failures`
- `classify_error_prefers_proxy_over_network` —— 锁死"代理优先于网络"的判定顺序
- `classify_error_maps_io_and_network_failures`
- `classify_error_maps_url_failures_and_unknown`
- `every_diagnostic_code_has_a_non_empty_hint` —— 防某个枚举漏配建议导致前端空白
- `parse_reg_pv_reads_version_and_rejects_garbage`
- `measure_dir_sums_files_and_stops_at_limit`

## 7. 验证

| 项目 | 结果 |
|---|---|
| `cargo check --lib --tests` | 0 error 0 warning |
| `bash scripts/cargo-test.sh --lib` | **37 passed / 0 failed** |
| `npm run build`（vue-tsc + vite） | 通过 |
| `cargo build --release` | 通过 |

版本号未 bump，仍是 **0.6.0**。
