# CHANGES_V24 — AI 使用时间显示 + 自动刷新改为「只在 AI 回答时刷新」

## 1. 背景

侧边栏卡片原来显示 "06:59 更新"，那个时间其实是 `status.updated_at`：
页面扫描脚本每次抓取时都会重写它，跟"打开标签"或"自动刷新"强相关，跟"AI 用没用"没关系。

自动刷新是固定间隔 + 「AI 回答时暂停」，但 AI 回答时正好是页面最有变化的时候，
暂停刷新反而拿不到最新积分/状态，闲时刷新又纯属浪费。

本轮改两件事：
1. 卡片显示真正的「最近一次 AI 完成回答的时间」，持久保留。
2. 自动刷新只在 AI 正在生成回答时才触发，平时不刷。

## 2. 持久化 `answer_finished_at`

`status.rs::save_profile_status` 旧逻辑：

```rust
if status.answer_ready {
    status.answer_finished_at = previous
        .answer_finished_at.clone()
        .or_else(|| Some(Utc::now().to_rfc3339()));
} else {
    status.answer_finished_at = None;   // ← 会被清掉
}
```

旧逻辑把 `answer_finished_at` 和 `answer_ready` 绑成一对："当前 ready 的那一条回答的完成时间"。
用户切回账号视为已读 → `clear_profile_answer_ready` 把它俩一起清掉。
所以即便之前用过 AI，只要读过，这个时间就再也拿不回来了。

新逻辑：把 `answer_finished_at` 拆出来当作"最近一次 AI 完成回答"的**历史**时间戳，**永不主动清零**。

- 仅在 `answer_ready` 从 `false` 翻到 `true`（"刚刚答完"的瞬间）才把时间覆盖为 `now`，
  避免同一轮 ready 被多次扫描时时间戳随每次扫描往前漂。
- 其它任何情况（仍在生成、反复扫描、用户已读）都沿用上一次的旧值。
- `clear_profile_answer_ready` 现在只清 `answer_ready`（未读提醒），不动 `answer_finished_at`。

> 旧数据里被清成 `None` 的历史时间是救不回来的 —— 没办法从 store 反推"上次 AI
> 是几点答完的"。从这一版起，新发生的 AI 完成事件都会写进 `answer_finished_at` 并
> 永久保留。

## 3. 卡片显示

`src/components/StatusSidebar.vue` 把渲染条件从 `updated_at` 换成 `answer_finished_at`：

```vue
<small v-if="showUpdatedTime && row.status?.answer_finished_at" class="status-time">
  {{ updatedAtLabel(row.status.answer_finished_at) }} AI使用
</small>
```

文案从 `更新` 改成 `AI使用`，跟时间戳的实际语义对齐（是 AI 完成回答的时刻，不是
最近一次抓取）。**没**用过 AI 的账号不显示时间 —— 用户要求的是 AI 使用时间，不是
兜底时间。

`src/components/AppPreferencesDialog.vue` 的开关文案同步：
- 标题：`显示更新时间` → `显示 AI 使用时间`
- 描述：`右侧状态卡显示最近一次状态抓取时间` → `右侧状态卡显示最近一次 AI 完成回答的时间`
- 底层字段名仍是 `showStatusUpdatedTime`（localStorage 兼容）；想彻底改名以后再说。

## 4. 自动刷新翻转

`src/App.vue::startAutoRefresh` 旧逻辑（"AI 回答时暂停"）：

```ts
if (uiPreferences.value.pauseAutoRefreshWhileGenerating
    && statuses.value[activeId.value]?.answer_generating) return
```

新逻辑（"AI 回答时才刷"）：

```ts
if (!statuses.value[id]?.answer_generating) return
```

意图：闲时刷新是浪费 + 容易打断还没开始的会话；AI 真的在生成时，积分/状态在变，
刷新才有意义。仍由 `autoRefreshSeconds > 0` 决定总开关（0 表示完全不刷）。

## 5. 移除 `pauseAutoRefreshWhileGenerating` 偏好

旧偏好字段（`UiPreferences.pauseAutoRefreshWhileGenerating`）的语义已经被新行为
**直接反转**，再保留会让用户困惑。直接干掉：

- `src/types.ts` 接口里删字段。
- `src/lib/uiPreferences.ts` 默认值 + 迁移都删。
- `src/components/AppPreferencesDialog.vue` 删掉那个开关。
- `src/components/AppPreferencesDialog.vue` 自动刷新的小字说明补上「只在 AI 正在生成
  回答时才刷」。

旧 localStorage 里的 `pauseAutoRefreshWhileGenerating` 字段会留在原地，不会再被读；
下次写偏好时被自然覆盖。

## 6. TabStrip 没动

用户提到"标签页的显示时间"，但项目里的 TabStrip（顶部标签栏）本就没有每个标签的
时间显示 —— "标签页"指的是侧边栏卡片（每张卡 = 一个账号标签）。改的就是侧边栏卡片。

## 7. 验证

```
vue-tsc --noEmit              0 error
clippy --lib --tests          0 warning
cargo test --lib              43 passed
bash scripts/tauri-msvc.sh build --no-bundle   ✓
npm run verify:assets         css / js / html 三项逐字节一致
```

没有为新行为加 Rust 单测 —— `save_profile_status` 需要 `AppHandle` + webview emit，
测起来比较重；前端展示和自动刷新都是 UI 层，肉眼确认更直接。

## 8. 新增/修改文件

| 文件 | 说明 |
| --- | --- |
| `src-tauri/src/status.rs` | `save_profile_status` 仅在 `answer_ready` 边沿翻 true 时覆盖时间，其它情况沿用旧值；`clear_profile_answer_ready` 不再清时间 |
| `src/components/StatusSidebar.vue` | 渲染 `answer_finished_at`，文案 `AI使用` |
| `src/App.vue` | 自动刷新只在 `answer_generating=true` 时触发 |
| `src/types.ts` | 删 `pauseAutoRefreshWhileGenerating` 字段 |
| `src/lib/uiPreferences.ts` | 默认值与迁移同步删除 |
| `src/components/AppPreferencesDialog.vue` | 删开关 + 更新两处文案 |