# V34 · 0.12.1 — 按钮与滑块控件风格统一

- 新增统一控件形状 token：`--control-radius-sm/md/lg/pill` 与焦点环 `--control-focus-ring` / `--control-focus-offset`；el-button、快捷工具栏、标签条、导航页、主题卡与步骤卡按钮收敛到同一圆角梯度（小图标 8 / 标准 10 / 大卡片 12 / 胶囊 999）。
- el-segmented 分段控件（外观设置、账号设置、下载历史等 9 处）并入安静控件配方：浅色轨道 + 主题色选中晕染 + 选中细描边，替换 Element 默认的实色主蓝选中块。
- el-switch 开启态由「accent 16% 淡轨道 + 主题色拇指」改为「主题色轨道 + 浅色拇指」，开关状态一眼可辨，且颜色仍全部由 accent 派生，11 套主题自动适配。
- 导航页「N 列」胶囊按钮与瓦片「编辑」小按钮改用全局安静按钮配方（control-bg + 细边框 + hover），不再各自一套配色。
- 多账号生成器步骤卡补充 hover 反馈；侧栏与批量条的可点击标签（tag-chip / quick-tag）获得与按钮一致的 hover 渐变。
- 焦点环统一为 muted 2px outline（快捷工具栏按钮原 accent 焦点环并入全局配方）；过渡时长统一为 motion.css 的 `--motion-fast` token，数值与原 120ms 相同，视觉零回归。
