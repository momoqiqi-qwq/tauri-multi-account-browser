# CHANGES_V25 — 点图片不再顶掉账号页，改成独立预览小窗

## 1. 需求

> 点击上传的图片不要自动切换我的账号的网址，你应该用一个小窗口出来，
> 把图片路径弄到这个小窗的窗头上，然后显示出这个图片。

要点三条：
1. 点图片**不能**再把当前账号页的网址换掉（现在会把整个聊天页顶成图片）；
2. 改用一个**小窗口**显示图片；
3. 小窗口的**窗头（标题栏）显示图片路径**。

## 2. 现象与根因

账号页里注入的 `NEW_WINDOW_PATCH_JS` 有一段"摘掉 target=_blank"的逻辑：

```js
document.addEventListener('click', function (e) {
  ...
  while (el && el.tagName !== 'A') el = el.parentElement;
  var target = (el.getAttribute('target') || '').toLowerCase();
  if (target === '_blank') el.removeAttribute('target');   // ← 元凶
}, true);
```

它的本意是好的：站点用 `target=_blank` 打开下载链接时，宿主的新窗口请求会被拒绝，
摘掉 target 让请求留在本页才能进 `on_download`。

但站点里的图片**也是**这种结构 —— `<a target="_blank" href=".../x.png"><img></a>`。
摘掉 target 之后图片就地整页导航：账号页的网址变成图片地址，聊天记录、滚动位置
全丢，用户得重新找会话。

`window.open(图片地址)` 那条路更绕一层：`.png` 命中 `isDownloadLike`，
补丁会主动 `location.href = url`，**故意**做整页导航 —— 同样是顶掉账号页。

## 3. 方案：图片走假导航 + 独立小窗

沿用 `mbstatus://` 的既有套路（宿主在 `on_navigation` 里拦截并取消，
远程页面拿不到任何 IPC 权限）：

```
点击图片 → 注入脚本 preventDefault
        → location.href = 'mbimage://open?u=<图片地址>&t=<文字提示>'
        → 宿主 on_navigation 拦下、return false（账号页一动不动）
        → 另开一个 WebviewWindow 显示图片，标题就是路径
```

- `lib.rs` 新增 `IMAGE_SCHEME = "mbimage"`，`webviews.rs` 新增 `query_map()`
  （顺便把 `mbstatus://` 那段的重复解析也收拢过来）。
- 建窗放在 `app.run_on_main_thread(...)` 的下一拍执行：`on_navigation` 回调里
  同步建 WebView2 会和 WebView2 自己的消息处理打架。

## 4. 小窗本身

`open_image_window()` / `build_image_window()`：

| 项 | 值 | 理由 |
| --- | --- | --- |
| 内容 | `WebviewUrl::External(图片地址)` | WebView2 会把图片响应直接渲染成图片文档 |
| 标题 | 图片路径（`image_window_title`） | 用户要的"窗头显示路径" |
| 尺寸 | 760×620，最小 320×240，可缩放，居中 | 比整页小，又不至于看不清 |
| label | `mb-image-<毫秒>-<自增序号>` | 连点两张图不会撞 label |

标题里的路径做了两件事：
- **百分号解码**：`Url::path()` 给的是 `%E5%9B%BE%E7%89%87.png` 这种原始编码，
  自己写了个 `percent_decode()`（不新增依赖），中文文件名在标题栏里直接可读；
- **超长保留尾部**：`clip_tail(…, 160)`，文件名和辨识度最高的段都在后面，
  截掉尾部等于没信息。

`IMAGE_PREVIEW_JS` 注入到小窗里，把 WebView2 生成的裸图片文档重新排版：

- 顶部 34px 路径栏：完整路径（可选中）+「复制路径」按钮；
- 下面是图片本体，默认「适应窗口」，按钮切「原始大小」；
- 图片没加载出来（403 / 链接过期）时给一条红色说明，而不是让用户对着空白猜；
- 不是图片文档（服务端返回了 HTML 或附件）时**什么都不做**，不拆人家页面。

路径用 `serde_json::to_string()` 注入成 JS 字符串字面量，不手写引号拼 ——
路径里可能有引号、反斜杠、中文。

### 4.1 数据目录：先复用账号的，失败再退默认

blob: 图片（上传后的本地预览就是这个）和需要 cookie 的图片，只有落在**同一个
存储分区**里才取得出来，所以优先给预览窗复用账号自己的 `UserDataFolder`。

WebView2 不允许同一个数据目录挂两套不同的 `CoreWebView2EnvironmentOptions`
（账号配了代理 / 语言时参数就不一样），所以复用失败时换一个 label、退回默认
数据目录再建一次：这时 http(s) 图片照常能看，只有会话内图片可能加载不出来。
两条路都失败就静默放弃 —— 绝不能因为预览窗建不起来而影响账号页。

## 5. 识别规则：宁可漏，不可误伤

只认三种来源：

1. blob: 开头的地址（上传后的本地预览）；
2. `http(s)` 且路径以 `png / jpg / jpeg / gif / webp / bmp / svg / avif / ico / heic / heif / jfif / tif(f)` 结尾；
3. 链接没有真地址（空、`#`、`javascript:`）但里面包着图片 —— 图片本身就是目标。

特意**没有**采用"链接里包着 img 就当图片"的宽口径：卡片式列表的缩略图也是
这个结构，宽口径会把"点缩略图进详情页"也变成弹小窗。

另外两条保护：

- **`download` 属性优先**：链接带 `download` 就按下载走，不弹窗（用户的明确意图）；
- **只接管真人点击**（`e.isTrusted`）：`element.click()` 这类合成事件一律放行。
  这条是必须的 —— 默认 AI 自动下载扩展名白名单里就含 `png,jpg,jpeg,webp`，
  自动下载脚本是靠 `el.click()` 触发锚点的，如果合成的点击也被拦成预览窗，
  「AI 文件自动下载」会被整条打断，还会满屏弹窗。

`window.open(图片地址)` 也接到同一条路上（原来是故意整页导航），
否则站点自己弹图还是会把账号页顶掉。

## 6. 验证

```
vue-tsc --noEmit                                 0 error（发布构建时一并跑）
clippy --lib --tests                             0 warning
cargo test --lib                                 54 passed（本轮新增 6 条）
node --check 导出的 NEW_WINDOW_PATCH_JS/IMAGE_PREVIEW_JS   语法通过
bash scripts/tauri-msvc.sh build --no-bundle     构建通过
npm run verify:assets                            css / js / html 逐字节一致
```

新增的 6 条单测（`src-tauri/src/lib.rs`）：

- `image_preview_navigation_decodes_target_and_hint` —— 假导航的解析必须和注入脚本
  拼出来的完全对上（scheme / host=open / 编码后的地址 / 中文提示）；
- `image_window_title_shows_decoded_path_with_query` —— 路径可读、查询串保留；
- `image_window_title_falls_back_to_hint_for_blob` —— blob: 用 alt/title 兜底；
- `clip_tail_keeps_the_tail_and_marks_truncation` —— 超长保留尾部；
- `percent_decode_handles_valid_and_broken_escapes` —— 非法/截断的 `%` 不能 panic；
- `image_preview_script_and_host_agree_on_contract` —— 脚本与宿主必须对齐
  `mbimage://open`，改一边忘另一边就静默失效。

## 7. 已知限制

- 图片直链**没有扩展名**时（例如 `https://cdn.x.com/abc123`）不识别，仍会走原来的
  整页导航。这种地址没有可靠特征，宽口径识别会误伤卡片缩略图，先不做。
- 账号配了代理 / 语言时，预览窗复用不了账号数据目录，需要登录态才给看的图片
  可能加载不出来（会给红色说明并保留路径）。
- 预览窗是独立窗口，不跟随账号的代理设置 —— 走代理的账号点图会直连一次。

## 8. 新增/修改文件

| 文件 | 说明 |
| --- | --- |
| `src-tauri/src/lib.rs` | 新增 `IMAGE_SCHEME`；补 6 条单测 |
| `src-tauri/src/webviews.rs` | `NEW_WINDOW_PATCH_JS` 增图片拦截；新增 `IMAGE_PREVIEW_JS`、`query_map`、`clip_tail`、`percent_decode`、`image_window_title`、`build_image_window`、`open_image_window`；`on_navigation` 增 `mbimage://` 分支 |
