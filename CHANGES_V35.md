# V35 · 0.13.0 — 账号总览按网站分类 + 工具栏「网站导航」+ 生成器全选/记忆

> 本轮由三条并行流程合并进 0.13.0 一个版本号，下面的功能清单、改动文件与验证结果覆盖三部分。

- 「全部账号历史」新增**网站维度筛选条**：`全部网站` + 每个网站及其账号数（账号多的排前）。它与原有的
  全部 / 已打开 / 回答完成 / 生成中 / 未打开 / 有下载 是**两个可叠加的维度**（例：`liblib.tv` + `有下载`）。
- 每行账号名右侧新增**网站徽标**（悬浮显示完整网址）。分组沿用「网站导航」的同一套真值：按配置的主页归类，
  继承模式账号跟随全局默认网址，登录跳转不会把账号挪到别的网站。
- 搜索框同时匹配网站名 / 域名 / 网址；搜「待设置」可直接找出没有可用主页的账号。
- 网站筛选在账号被删除或改主页导致该网站消失时自动退回「全部网站」，不会停在空列表。
- 修复：Chromium 会把 `https://not a url` 这类非法主机名解析成 `not%20a%20url`（Node 会直接抛错），
  垃圾主页因此会在界面上变成一个独立「网站」。`parseDirectorySite` 现在显式拒绝含 `%` 的主机名，
  两端判定一致（Node 侧该分支本就由 URL 解析器拦下，浏览器侧由本次新增判断兜住）。
- 新增 `countSiteFacets()`（`src/lib/siteDirectory.ts`）：按账号数统计网站并排序，只统计真正有账号的网站，
  不会带出「已保存但当前没有账号」的网站。
- 右侧快捷工具栏新增内置项「网站导航」（`directory`）：一键返回按网站分组的账号选择页，与地址栏左侧的
  「网站导航」按钮同源（`websiteDirectoryVisible = true`）。该项不依赖已打开的账号，空态也能点，默认开启。
- 新增内置项在**老用户 localStorage 上按锚点插入**（`insertIndexFor`）：缺失的内置项插到「默认顺序中它前一个
  已存在项」之后，因此升级后它出现在「主页」之后，而不是被追加到工具栏末尾。

## 同轮附带：多账号生成器（全选 / 记忆上次选择 / 显示登录账号）

- 「选择来源账号」步骤工具栏新增**全选**；带搜索词时额外出现**选中搜索结果**，只圈当前过滤结果。
- **记忆上次选择**：勾选结果写入 localStorage（`SOURCE_SELECTION_KEY`），下次打开弹窗自动恢复；
  恢复时按 `restoreSourceSelection()` 保序去重并剔除已删除的账号，存储损坏时降级为空而不是报错。
- 来源账号行内第二行用主题色显示**登录账号**，取 `ProfileStatus.email`、取不到退回 `account`
  （与侧栏口径一致）；搜索匹配也加入登录邮箱。该字段是页面被动上报的，没打开过的账号为空且不显示占位，
  **本轮没有新增任何抓取逻辑**。

## 改动文件

- `src/components/AccountOverviewDialog.vue` — 网站筛选条、网站徽标、搜索与筛选叠加、悬空筛选回落。
- `src/lib/siteDirectory.ts` — `countSiteFacets()` + 主机名 `%` 校验。
- `src/App.vue` — 传入 `global-default-url`（与网站导航同源）；快捷工具栏绑定 `@directory`。
- `src/style.css` — `.overview-sites` / `.overview-category__count` / `.overview-account-title` / `.overview-site-chip`。
- `src/types.ts` / `src/lib/uiPreferences.ts` / `src/components/BrowserActionDock.vue` — 新增 `directory` 内置项、锚点插入归一化、图标与事件。
- `scripts/test-site-directory.mjs` — 新增 2 个用例。
- `scripts/test-browser-toolbar.mjs` — 新增 6 个用例（插入位置、锚点跟随用户排序、按 defaultEnabled 补齐、去重、丢弃未知 id）。
- `src/lib/accountGenerator.ts` — `SOURCE_SELECTION_KEY` + `restoreSourceSelection()`（保序去重、剔除失效 id、损坏降级）。
- `src/composables/useAccountGenerator.ts` — `selectAll` / `allSelected` / `loginAccount(id)`，打开时恢复记忆、`watch(selectedIds)` 写回。
- `src/components/AccountGeneratorDialog.vue` / `src/account-generator.css` — 全选工具栏、行内登录账号行（`.ag-source-login`）。
- `scripts/test-account-generator.mjs` — 新增恢复记忆用例（共 13 例）。
- 版本三处同步 0.13.0（`package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml`，含 `Cargo.lock` 项目版本）。

## 验证

- `node --test scripts/test-site-directory.mjs scripts/test-ui-optimizations.mjs` → **16/16 通过**
  （新增用例覆盖：按账号数排序、计数、只统计有账号的网站、复用已保存网站名、不污染账号数据）。
- 变异检查 `.workbuddy-ai/probe/overview/mutate-site-facets.mjs` → **5 条变异全部被捕获**
  （不排序 / 计数不累加 / 同网站不合并 / 结果重复输出 / 计数起点 0）。
- 无头 Chrome 真机核验 `.workbuddy-ai/probe/overview/probe.mjs` → **16 项断言全通过**：分类条文案与计数、按数量排序、
  每行都有徽标、账号名没有被徽标挤出容器、网站分类条未被裁切、网站 × 分类叠加、按网站名搜索、空态。
  截图见 `.workbuddy-ai/probe/overview/shot-all.png` 与 `shot-liblib-downloaded.png`。
- `node --test scripts/test-browser-toolbar.mjs` → **6/6 通过**；变异检查 `.workbuddy-ai/probe/mutation-check-toolbar.mjs`
  → **4 条变异全部被捕获**（退回追加末尾 / 锚点差一 / 忽略 defaultEnabled / 去掉去重）。
- 无头 Chrome 真机核验 `.workbuddy-ai/probe/probe-dock.mjs` → **8 项断言全通过**：按钮共 9 个、第 3 个是「网站导航」、
  顺序紧跟「回到账号主页」、title 与 aria-label 一致、图标全部渲染、空态可点、点击触发 directory 且只触发一次。
  截图见 `.workbuddy-ai/probe/dock-shot.png`。
- `node --test scripts/test-account-generator.mjs` → **13/13 通过**；变异检查 `.workbuddy-ai/probe/mutate-selection.mjs`
  捕获全选/恢复记忆相关变异；无头 Chrome 探针 `.workbuddy-ai/probe/probe-generator.mjs` 核验全选按钮、
  选中搜索结果、登录账号行与记忆恢复，截图 `.workbuddy-ai/probe/account-generator-step2.png`。
- `vue-tsc --noEmit` 通过。

## 发布前复跑（2026-09-26，0.13.0 发版时）

- 四套前端用例合计 **35/35 通过**：account-generator 13、site-directory 10、ui-optimizations 6、browser-toolbar 6。
- `vue-tsc --noEmit` 通过。
- 上面引用的 `.workbuddy-ai/probe/`（探针夹具、变异脚本、截图）与 `.mut/`（无头 Chrome profile）**只留本地，不入版本库**：
  夹具 `probe/overview/main.ts` 抄自真实账号名与登录邮箱，公开仓库不宜收录。可复现的验证以
  `scripts/test-*.mjs` 四套用例为准（35 项，全部随源码入库）。
