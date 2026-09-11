# CHANGES_V22 — 账号标签 / 收藏 / 最近使用 / 批量编辑

对应 `OPTIMIZATION_REPORT_V14.md` 第 8 条：**增加账号标签、收藏、最近使用、
批量选择与批量修改主页模式**。

这是本轮唯一**动数据结构**的改动。

---

## 1. 数据结构

`Profile` 新增三个字段，全部带 `#[serde(default)]`，旧数据反序列化不受影响：

| 字段 | 类型 | 说明 |
|---|---|---|
| `tags` | `Vec<String>` | 自由标签，跨分组归类 |
| `favorite` | `bool` | 收藏 |
| `last_used_at` | `Option<String>` | 最近一次打开时间（RFC3339） |

`SCHEMA_VERSION` **1 → 2**，新增 `migrate_v1_to_v2`。

> 这一步**不是**为了补默认值（serde 已经兜底），而是把已存在的脏数据规范化：
> 历史写入路径没走过 `normalize_tags`，标签可能有空白、重复、超长。
> 迁移只改真正有问题的字段，`changed` 为 false 时不写盘。

`normalize_tags()`（在 `validation.rs`）：去空白、丢空串、去重（**大小写不敏感**但保留原样大小写）、
单标签限 24 字符、每账号最多 12 个标签、保持首次出现顺序。

## 2. 批量修改

新增 `update_profile_batch(ids, patch)`，patch 全字段可选（省略即"不改"）：

```rust
struct ProfileBatchPatch {
    url_mode: Option<String>,      // inherit / custom
    default_url: Option<String>,   // 空串表示不改
    tags_add: Option<Vec<String>>,
    tags_remove: Option<Vec<String>>,   // 大小写不敏感
    tags_set: Option<Vec<String>>,      // Some([]) 表示清空
    favorite: Option<bool>,
    group: Option<String>,
}
```

**沿用 v16 的原子提交原则**：先全量校验（url_mode 合法性、default_url 可解析、
id 都存在），全部通过后才 `save_profiles` 一次。任一项不合格 → 整批不改。
不要用前端循环 `update_profile` —— 中途失败会留下半截状态。

切到 `inherit` 但没给新网址时，以全局主页为准（而不是留着旧的 custom 值）；
改了主页后，还停在旧主页的账号会同步到新主页。

命令体抽成 `update_profile_batch_impl<R: Runtime>()` 以便单测
（与 `import_profiles_impl` 同一套路）。

## 3. 最近使用

- Rust：`activate_profile` 里写入 `last_used_at` 并落盘。
- 前端：`useTabs.activate` 同步更新本地副本，让侧边栏「最近」立刻反映，
  省一次列表往返。与已有的 `profile:navigated` 改本地 `last_url` 是同一套路。

## 4. CSV 支持标签

`CSV_HEADER` 从 10 列扩到 11 列，新增 `tags`，多个标签用 `|` 分隔
（标签本身可能含逗号，用逗号分隔会和 CSV 语义混淆）。

**只允许在末尾追加列**：历史导出的 CSV 靠位置解析，缺 trailing 列时
serde default 兜底，旧文件仍可导入。已加测试锁死这条。

## 5. 前端

- `src/types.ts`：`Profile` 三字段、`ProfileIsolationSettings.tags`、
  `ProfileBatchPatch`、`ProfileViewMode`。
- `useProfiles`：新增 `viewMode` / `activeTag` / `allTags` / `visibleProfiles`
  （搜索 + 视图 + 标签三层筛选叠加）、`updateBatch()`、`toggleFavorite()`。
- `useTabs.activate`：`last_used_at` 乐观更新。
- `ProfileSidebar.vue`：
  - 顶部视图切换「全部 / 收藏 / 最近」+「批量」按钮；
  - 有标签时显示标签筛选条（点一下选中，再点取消）；
  - 每个账号卡片加收藏星标（默认半透明，hover / 已收藏时高亮）；
  - 卡片上显示该账号的标签；
  - 批量模式下卡片变勾选，底部出现操作条。
  - 搜索 / 收藏 / 最近 / 标签筛选任一生效时，列表**摊平不分组**
    （否则按分组折叠会让人误以为"某些账号消失了"）。
- `ProfileBatchBar.vue`（新）：主页改继承 / 改独立网址、收藏 / 取消收藏、
  加标签 / 移除标签 / 快速打标、移动分组、清空标签。
- `ProfileSettingsDialog.vue`：新增标签多选（allow-create，上限 12）。
- `AccountToolsDialog.vue`：导出草稿带上 tags。

## 6. 测试

新增 5 个用例（**共 42 个，全过**）：

- `normalize_tags_trims_dedupes_and_caps`
- `normalize_tags_caps_length_and_count`
- `update_profile_batch_is_atomic_and_applies_patch` —— 覆盖整批成功、
  非法输入整批不改、id 不存在整批不改、删除标签大小写不敏感
- `tags_survive_json_roundtrip_on_profile` —— 缺字段走 serde default
- `csv_parse_tolerates_old_exports_without_tags_column` —— 旧 CSV 仍可导入

顺手修了测试基建的一个坑：`save_profiles` 是**整表覆盖**，
原来的 `seed_profile()` 循环调用会互相覆盖，改成 `seed_profiles()` 一次性写入。

## 7. 构建修正：`cargo build --release` 产出的不是生产二进制

（这轮复查时发现，属于既有问题，与 v22 功能无关但影响所有发布）

**现象**：`cargo build --release` 编译成功，但产出的 exe 里**没有前端资源**。
用 `useTabs.ts` 里的纯 ASCII 标记 `mab-tab-session-v12` 去 exe 里搜，0 命中；
Rust 侧的 `mbstatus` 能搜到 —— 说明不是搜索方法的问题。

**根因**：`src-tauri/Cargo.toml` **完全没有 `[features]` 段**，缺了官方模板里的：

```toml
[features]
custom-protocol = ["tauri/custom-protocol"]
```

tauri 2.11.5 里 `is_dev()` 的定义是 `!cfg!(feature = "custom-protocol")` ——
没开这个 feature，应用就跑在**开发模式**，会去连 `devUrl`（http://localhost:1420）
而不是加载打包进来的前端，表现为打开后白屏 / 连不上。官方注释也明确写了
"Feature managed by the Tauri CLI"，`tauri build` 会自动带上它。

**验证对比**：

| 构建方式 | exe 大小 | 资源名是否嵌入 |
|---|---:|---|
| `cargo build --release` | 5,118,976 B | 否 |
| `cargo build --release --features custom-protocol` | 5,491,712 B | 是（`index-*.js` / `index-*.css`）|

**处理**：补上 `[features]` 段并加注释说明原因。
真正的发布仍应走 `npm run tauri:build`（CLI 会自动带 feature 并打包安装包）；
直接用 cargo 时必须显式 `--features custom-protocol`。

> 顺带一提：之前几轮"release 构建通过"的结论，只能证明**编译通过**，
> 不能证明二进制可用 —— 现在已经能区分这两件事了。

## 8. 验证

| 项目 | 结果 |
|---|---|
| `cargo check --lib --tests` | 0 error 0 warning |
| `bash scripts/cargo-test.sh --lib` | **42 passed / 0 failed** |
| `npm run build`（vue-tsc + vite） | 通过 |
| `cargo build --release` | 通过 |

版本号未 bump，仍是 **0.6.0**。
