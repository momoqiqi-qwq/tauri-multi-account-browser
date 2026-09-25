# CHANGES_V28 — 标签页时间改为「AI 对话计时」+ 图片新窗口兜底

## 1. 需求

> 标签页时间不是进去就直接开始，而是我使用 ai 发送后开始，到 ai 回答完毕结束。
> 当新的对话时重新从零计算。
> 确保点击图片链接只会弹出一个新窗口而不改变原网址。

## 2. AI 对话计时（替换原「运行时间」）

### 2.1 语义变化

| 场景 | 旧（运行时间） | 新（AI 对话计时） |
| --- | --- | --- |
| 打开标签页 | 立即开始计秒 | 显示 `00:00:00`（或上一轮实测耗时） |
| 向 AI 发送消息 | 继续计秒 | **从零开始走表** |
| AI 回答完毕 | 继续计秒 | **停在本轮耗时上** |
| 再次发送（新对话） | 继续累计 | 重新从零计算 |

标签上的暂停 / 重置按钮随之失去意义，已移除；「运行时间模式」（本次打开 /
应用累计）偏好一并移除。

### 2.2 测量链路

- **页面脚本**（`STATUS_REPORTER_JS`）：复用已有的「停止生成按钮」检测。
  - 上升沿（按钮出现 = 用户刚发送）：打点 `generatingStartedAt`；
  - 下降沿准备（按钮首次消失）：打点 `answerEndObservedAt`（**不含** 1.2s 防抖
    确认等待，更贴近真实完成时刻）；
  - 确认完成：算出 `d = answerEndObservedAt - generatingStartedAt`，只在完成的
    那次上报里携带 `&d=<毫秒>`。
- **宿主**（`save_profile_status`）：
  - generating 上升沿记 `answer_started_at`（前端生成期间用它实时走表）；
  - 下降沿照旧记 `answer_finished_at`，耗时优先取上报里的 `d`；
  - 脚本没带 `d`（典型：**生成中途刷新页面**，脚本状态复位后补报 g=0）就沿用
    上一轮实测值 —— 不会把刷新间隔误当成一轮对话。
- **前端**（`TabStrip`）：生成中显示 `now - answer_started_at`（1s 走表），
  其余时间显示 `answer_duration_ms`；两者都随账号状态持久化，重启后仍显示
  上一轮耗时。计时精度受停止按钮检测（约 ±1.5s）限制，属已知取舍。

## 3. 图片链接只弹小窗、原网址不变（兜底加固）

v25 的页面内补丁已把**左键点击**图片链接转成 `mbimage://` 假导航 → 独立预览
小窗，账号页不动。本轮补上绕过页面脚本的那个口子：

- **`on_new_window`**：此前对任何 http(s) 新窗口请求都「在本标签页打开」——
  中键点击图片链接时会把账号页顶成图片文档。现在凡是图片地址
  （http/https 按扩展名、`blob:` 一律算），先经 `run_on_main_thread` 另开
  预览小窗（与 mbimage 分支同一套数据目录复用逻辑），然后 `Deny` 新窗口请求；
  非图片地址保持原有「在本标签页打开」行为。
- 新增 `webviews::is_image_url`，判断口径与页面脚本 `isImageUrl` 完全一致；
  单测把两边钉死（正则原文 + 枚举表）。

现在四条路径全部收敛到同一个行为：左键点击（页面脚本）、`window.open`
（页面脚本）、`target=_blank` 兜底、**中键点击（本轮新增）**——都只弹一个
预览小窗，账号页网址永远不变。

## 4. 其他

- 顺手修了一个测试环境坑：直接 `cargo test` 会产出**不带 Common-Controls v6
  manifest** 的测试二进制，加载器绑到 comctl32 v5.82 后进程启动即报
  `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)`。必须用 `scripts/cargo-test.sh`
  （它会以 `--config` 注入 `/MANIFEST:EMBED`）。本轮曾因此误判构建损坏。

## 5. 改动文件

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/webviews.rs` | 脚本计时打点 + `d=` 上报；`is_image_url`；`on_new_window` 图片分支 |
| `src-tauri/src/status.rs` | 上升沿记 `answer_started_at`；耗时取 `d`，缺省沿用上一轮 |
| `src-tauri/src/lib.rs` | `ProfileStatus` 新增 `answer_started_at` / `answer_duration_ms`；5 条单测 |
| `src/components/TabStrip.vue` | 计时改由状态驱动，移除暂停/重置按钮 |
| `src/composables/useTabs.ts` | 移除 runtimeSeconds / runtimePaused / openedAt 及相关 API |
| `src/App.vue` | 移除对应 props / 事件接线 |
| `src/types.ts` | `ProfileStatus` 加字段；`UiPreferences` 删 `runtimeMode` |
| `src/lib/uiPreferences.ts` | 删 `runtimeMode` 默认值与归一化 |
| `src/components/AppPreferencesDialog.vue` | 删「运行时间模式」设置项 |
| `src/style.css` | `.tab-runtime` 改只读展示，生成中高亮 |
| `package.json` / `tauri.conf.json` / `Cargo.toml` | 版本 0.9.0 → 0.10.0 |

## 6. 验证

```
bash scripts/cargo-test.sh --lib               62 passed（新增 5 条）
cargo-msvc.sh clippy --lib --tests             0 warning
vue-tsc --noEmit + vite build                  通过
npm run verify:assets                          css / js / html 逐字节一致
```

新增单测：
- `answer_started_at_is_set_on_idle_to_generating_edge` —— 上升沿记开始时间且不漂移；
- `answer_duration_ms_is_taken_from_finish_report` —— 完成上报的实测耗时入账；
- `answer_duration_ms_survives_idle_report_without_measurement` —— 生成中刷新
  页面的补报不会覆盖上一轮实测值；
- `status_script_reports_round_trip_duration` —— 脚本打点与 `d=` 上报契约；
- `is_image_url_matches_the_page_side_heuristic` + 扩展名表同步测试 —— 宿主与
  页面脚本的图片判断口径钉死。
