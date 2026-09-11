# CHANGES_V20 — 拆分 `App.vue` 为 composables

对应 `OPTIMIZATION_REPORT_V14.md` 第 6 条：**把 App.vue 中账号生命周期、
标签生命周期和设置加载拆成 composables**。

纯重构，无功能变更。

---

## 1. 拆分结果

| 文件 | 行数 | 职责 |
|---|---:|---|
| `src/App.vue` | 477（原 817） | 宿主编排：弹层可见性、地址栏/工具栏动作、事件监听、模板 |
| `src/composables/useProfiles.ts` | 290 | 账号列表 CRUD、搜索过滤、分组、运行状态缓存 |
| `src/composables/useTabs.ts` | 283 | 标签生命周期：打开/关闭/固定/排序、运行计时、自动休眠、会话恢复 |
| `src/composables/useSettings.ts` | 79 | 全局设置与下载历史的加载/保存 |

`App.vue` 的 477 行里模板占约 145 行，脚本约 330 行，剩下的都是
"宿主才该管的"：哪个对话框开着、地址栏怎么跳转、Tauri 事件怎么分发。

## 2. 依赖方向

```
App.vue
  ├─ useSettings()                            （无依赖）
  ├─ useTabs({ profiles, preferences,
  │            getBounds, globalDefaultUrl,
  │            onActivateError })             （依赖 profiles ref + 界面偏好）
  └─ useProfiles({ profiles, tabs })          （依赖 tabs）
```

依赖是**单向**的，`useSettings` → `useTabs` → `useProfiles`，没有环。

## 3. 关键技术决策

### 3.1 账号列表 ref 提到宿主持有

`useProfiles` 要**写**账号列表（CRUD 后重新拉取），`useTabs` 要**读**它
（按 id 取账号、恢复会话时剔除已删除的标签）。如果两边各自持有就会互相依赖。

解决办法：把 `const profiles = ref<Profile[]>([])` 放在 `App.vue`，
以参数形式传给两个 composable。代价是宿主多一行声明，换来的是
**不需要惰性 getter、不需要 `let x!: T` 的延迟赋值技巧**。

### 3.2 依赖用参数注入，而不是 import

三个 composable 之间**不互相 import**，全部通过参数注入：

- `useTabs` 需要"全局默认主页"→ 传 `globalDefaultUrl: () => string | undefined`
- `useTabs` 需要"弹诊断工具" → 传 `onActivateError: (profile, error) => void`
- `useTabs` 需要"浏览区域矩形" → 传 `getBounds: () => BrowserBounds | null | undefined`
- `useProfiles` 需要"删账号时同步标签" → 传整个 `tabs` 对象

好处是 composable 可以单测（塞 mock 进去即可），也不会出现循环 import。

### 3.3 `onSaved` 回调而不是反向依赖

原 `saveSettings()` 内部会 `await loadProfiles()`（改了全局主页后，
继承模式账号要立刻反映新主页）。如果直接在 `useSettings` 里 import
`useProfiles` 就形成环，所以改成**调用点传回调**：

```ts
// useSettings.ts
async function saveSettings(settings: AppSettings, onSaved?: () => Promise<void> | void)

// App.vue
function saveSettings(next: AppSettings) {
  return settings.saveSettings(next, accounts.loadProfiles)
}
```

### 3.4 模板里访问 composable 成员的注意事项

`<script setup>` **只对顶层 ref 自动解包**。如果把 composable 的返回值
整体赋给一个变量（`const tabs = useTabs(...)`），模板里写
`v-model="tabs.currentUrl"` 拿到的是 Ref 对象而不是值，会破坏响应式。

所以采取"**对象取函数、解构取 ref**"的混合写法：

```ts
const tabs = useTabs({ ... })
const { openTabs, activeId, currentUrl, ... } = tabs   // 需要在模板里当 prop/v-model 的
```
模板里 `@activate="tabs.activate"`（函数是普通属性，可以直接点），
`:open-tabs="openTabs"`（ref 走顶层解构，自动解包）。

## 4. 行为一致性

逐行比对过 diff，确认是**纯搬移**：

- `activateProfile` 里"先让后端创建 WebView，成功后再提交前端标签状态"的顺序不变；
- `deleteProfile` 清除 openTabs / pinned / hibernated 三张表 + 持久化 + 状态缓存清理的顺序不变；
- `clearProfile` 只从 openTabs 移除（不动 pinned/hibernated、不持久化）—— 保持原样；
- `saveSettings` 保存后回读一次配置（Rust 侧会规范化并强制部分字段）的逻辑不变；
- 卸载时 `stopAutoRefresh` / `stopLifecycle` / `persist` 的清理不变。

## 5. 验证

| 项目 | 结果 |
|---|---|
| `npx vue-tsc --noEmit` | 0 error |
| `npm run build` | 通过（1632 modules，产物 1,119.90 kB） |
| `cargo build --release` | 通过 |

版本号未 bump，仍是 **0.6.0**。

## 6. 后续注意

1. 新增账号相关逻辑优先放 `useProfiles`，标签相关放 `useTabs`，
   只有"宿主编排"（弹层开关、事件监听、布局同步）才留在 `App.vue`。
2. composable 之间**不要直接 import**，继续用参数注入。
3. 需要在模板里当 prop / v-model 用的 ref，记得在 `App.vue` 顶层解构出来。
