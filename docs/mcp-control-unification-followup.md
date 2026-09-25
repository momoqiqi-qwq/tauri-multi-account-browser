# 按钮与滑块控件风格统一（V34 · 0.12.1）跟进实施与验证

## 背景

- V30（0.10.2）的 `src/controls.css` 完成首轮控件主题统一；本轮需求仍是「按钮与滑块的风格与界面统一」，作为跟进优化。
- 以 v0.12.0 为基线重新盘点（2026-09-25）发现的剩余不统一点：
  1. `el-segmented` 分段控件在 5 个组件共 9 处使用，控件层未覆盖，选中项为 Element 默认的实色主蓝块，与安静风格冲突最明显。
  2. `el-switch` 开启态轨道仅 accent 16% 淡色，开关状态不够一眼可辨。
  3. 按钮类圆角混用 7/8/9/10/11/12/13px，缺少统一梯度。
  4. 焦点环存在 muted 2px 与 accent 2px 两套配方。
  5. 过渡时长硬编码 120ms，未走 motion.css 的 token。
  6. 导航页「N 列」胶囊按钮与瓦片「编辑」小按钮各自一套配色；步骤卡与可点击 el-tag 缺少 hover 反馈。

## 改动

- `src/controls.css` 新增 token：`--control-radius-sm/md/lg/pill`、`--control-focus-ring/--control-focus-offset`、`--control-thumb-on`。
- `el-segmented`：浅色轨道 + 主题色选中晕染 + 选中细描边，项圆角并入梯度。
- `el-switch`：开启态改为 accent 72% 轨道 + 浅色拇指（`--control-thumb-on` = white 86% + accent 14%）。
- el-button、快捷工具栏、标签条、导航页、主题卡、步骤卡的圆角收敛到 8/10/12/999 梯度；焦点环统一为 muted 2px。
- 步骤卡与可点击标签补 hover 反馈；过渡时长统一为 `--motion-fast`（与原 120ms 等值，视觉零回归）。
- 版本三处同步 0.12.1（含 Cargo.lock 项目版本）；变更说明见 `CHANGES_V34.md`。

## 验证

- 主机：`npm run build`（vue-tsc + vite）通过；`test:ui-optimizations` 6 条、`test:site-directory` 8 条、`test:account-generator` 12 条全过；`npm run verify:assets` 通过；`get_diagnostics` 对 src 返回 0 项；`git diff --check` 仅既有 CRLF 提示。
- 隔离渲染：在代理沙箱以 Vite + Element Plus 2.14.6 渲染真实应用外壳（注入 `__TAURI_INTERNALS__` 桩替代 Tauri 后端），对改动前后截图对比：浅色与 Claude 深色主题下分段控件选中态、开关开启态、胶囊按钮、卡片圆角与焦点环均符合安静控件配方。截图保留在代理沙箱，不入库。
- 边界：未做 Windows 实机交互验收；已安装版本需走既有桌面发布流程才会更新。
