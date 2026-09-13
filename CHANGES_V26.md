# CHANGES_V26 — 下载历史弹窗：记录多了不再把卡片和弹窗底部顶出去

## 1. 需求

> 修复这是因为太多下载文件导致被顶的问题，添加可以设定显示多少列，太多的话添加滑动条。

拆成三条：
1. 下载记录一多，弹窗里的内容被"顶"得看不全 —— 要修；
2. 增加一个"显示多少列"的设置；
3. 内容超出时要有可见的滚动条。

## 2. 现象与根因

用户截图（`Clipboard_Screenshot.png`）里，每个账号卡片只剩**标题栏 + 半行记录**：
第 2 张卡片（"1 个文件"）只露出文件名的一小截，第 5 张卡片（"3 个文件"）只显示 2 行。
卡片之间互相不重叠，也没有滚动条 —— 说明**不是容器在滚动裁切，而是每张卡片被单独压扁了**。
按比例算：内容总高 ≈ 656px、容器 58vh ≈ 506px，缺的 150px 被按卡片高度比例分摊掉了，
这正是 flex 子项收缩（`flex-shrink`）的特征。

旧样式（`src/style.css`）：

```css
.dl-groups { display: flex; flex-direction: column; gap: 14px; max-height: 58vh; overflow-y: auto; ... }
.dl-group  { border: 1px solid var(--border); border-radius: 12px; overflow: hidden; background: var(--surface); }
```

`.dl-groups` 是 flex 列容器，子项 `.dl-group` 默认 `flex-shrink: 1`；而 `.dl-group` 又带
`overflow: hidden`，按规范它的"自动最小尺寸"因此为 0 —— 于是卡片可以被压到任意矮，
内容被 `overflow: hidden` 裁掉。**既看不全记录，也永远不出现滚动条**，用户自然觉得"被顶住了"。

另外 `element-plus` 给 `.el-dialog` 的默认 `margin: 15vh auto 50px`，弹窗上边距吃掉 15vh，
记录一多时弹窗总高（6vh 只是它的 40%）会超过视口，底部的「清空历史 / 关闭」被顶出屏幕，
必须滚动整页才能点到。

### 复现与验证方式

由于这是纯视觉问题，改之前先做了可复现的最小页面，用 **真实构建产物 CSS + 真实
element-plus DOM** 渲染同一批数据（5 个账号 / 8 条记录），再用 headless Chromium 截图对比：

- 旧样式：卡片被压扁、内容裁切、无滚动条 —— 与用户截图一致（已复现）；
- 新样式（1 / 2 / 3 列）：卡片保持自然高度，列表出现滚动条，弹窗底部按钮始终在视口内；
- 窄窗口（1057×620）：底部按钮仍可见，列表滚动，卡片不被压扁。

## 3. 方案

三条改动，互不依赖：

### 3.1 列表容器从 flex 换成 grid（治"压扁"）

```css
.dl-groups {
  min-height: 120px;
  max-height: 62vh;                                  /* 老引擎兜底 */
  max-height: max(160px, calc(86vh - 280px));         /* 按视口换算 */
  display: grid;
  grid-template-columns: repeat(var(--dl-cols, 1), minmax(0, 1fr));
  align-content: start;
  align-items: start;
  gap: 12px;
  overflow-y: auto;
  ...
}
```

grid 的 `auto` 行按内容定高，**容器再矮也只是出现滚动条**，不存在"把行压扁"这种行为，
从机制上消除了这个 bug（比给 flex 子项加 `flex: 0 0 auto` 更彻底：
那只是保住当前这几条规则，任何后续新增的 flex 相关样式都可能把它带回来）。

### 3.2 弹窗高度改为"跟着视口算"（治"底部被顶出去"）

```css
.downloads-dialog { margin: 6vh auto 24px !important; }
```

列表高度上限写成 `max(160px, 86vh - 280px)`，其中 280px 是头部 + 工具栏 + 底部的估算高度，
于是：

```
弹窗总高 ≈ 6vh(上边距) + 280 + 列表 + 24(下边距)
         ≤ 6vh + 280 + (86vh - 280) + 24 = 92vh + 24px ≤ 视口高度
```

记录再多，底部的「清空历史 / 关闭」也一定在屏幕内。`62vh` 那行是老引擎的兜底值。

### 3.3 每行列数可调（治"想看更密一点"）

新增偏好项 `downloadColumns`（1 / 2 / 3，默认 **2**）：

- `src/types.ts`：`UiPreferences` 增加 `downloadColumns: number`；
- `src/lib/uiPreferences.ts`：`DEFAULT_UI_PREFERENCES.downloadColumns = 2`，
  `normalizeUiPreferences` 收敛到 `{1,2,3}`（列太多会让卡片窄到放不下文件名和按钮，故封顶 3）；
- `src/components/DownloadsDialog.vue`：工具栏「已选 x/y」和「20 行/页」之间加一个「N 列」下拉，
  并把列数写成容器上的 CSS 变量 `--dl-cols`；
- `src/components/AppPreferencesDialog.vue`：更多设置 → 下载 → 新增「下载历史每行列数」。

列数的语义：

- **分组视图**：一行放几个账号卡片，卡片内部记录仍占满卡片宽度；
- **列表视图**：一行放几条记录（`.dl-flat-group .dl-items` 同样铺成网格）。

窗口宽度 ≤ 860px 时强制收回 1 列（多列会把文件名挤到看不清）：

```css
@media (max-width: 860px) { .dl-groups { --dl-cols: 1 !important; } }
```

`!important` 是必须的：`--dl-cols` 是模板写在元素上的内联自定义属性，
作者样式表里只有 `!important` 能覆盖内联值。

### 3.4 滚动条看得见

`scrollbar-gutter: stable` 预留槽位，避免出现滚动条时列表宽度跳一下；
滚动条本身沿用既有 `::-webkit-scrollbar` 规则，只把滑块颜色从 `var(--border)`
改成 `color-mix(var(--muted) 40%)`（悬停 62%），并去掉滑块边框 —— 原来滑块颜色和卡片背景
几乎同色，即使出现也看不出来。

## 4. 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/types.ts` | `UiPreferences` 新增 `downloadColumns` |
| `src/lib/uiPreferences.ts` | 默认值 2、`columnOptions` 收敛、`DEFAULT_UI_PREFERENCES` |
| `src/components/DownloadsDialog.vue` | 列数下拉、`--dl-cols` 内联变量、`updateColumns()` |
| `src/components/AppPreferencesDialog.vue` | 下载页新增「下载历史每行列数」 |
| `src/style.css` | 弹窗 margin、`.dl-groups` 改 grid + 视口高度上限、`.dl-column-size`、列表视图网格、滚动条配色、860px 断点 |

## 5. 验证

```
vue-tsc --noEmit + vite build                    通过
verify:assets                                    前端资源逐字节一致
真实样式复现页（headless Chromium 1057×1000 / 1057×620）
  旧样式        卡片被压扁、无滚动条               与用户截图一致（复现成功）
  新样式 1 列   卡片完整、有滚动条、底部按钮可见   通过
  新样式 2 列   5 张卡片 3 行铺开、无裁切         通过
  新样式 3 列   无裁切，文件名截断（有 title 提示） 通过
  列表视图 2 列 8 条记录 2×4 铺开、无裁切         通过
  窄窗口 620px  底部按钮仍在视口内、列表滚动        通过
```

## 6. 已知限制

- `--dl-cols` 通过内联自定义属性下发，若把列数改成 4 及以上需要同步放开
  `uiPreferences.ts` 的 `columnOptions` 与两处 `<el-option>`，否则会被收敛回 2。
- 分组视图用 grid 而不是瀑布流（masonry）：同一行里某个账号记录特别多时，
  相邻卡片下方会留出空白（CSS 多列布局能填满，但 `column-count` 跟滚动容器冲突，
  会横着长出列，所以没用）。默认 2 列时一般看不出来。
