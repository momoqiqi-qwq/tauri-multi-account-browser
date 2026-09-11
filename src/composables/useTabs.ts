import { ref, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import { isSuppressed } from '../lib/suppress'
import { resolveProfileUrl } from '../lib/profileUrl'
import type { BrowserBounds, Profile, UiPreferences } from '../types'

const TAB_SESSION_KEY = 'mab-tab-session-v12'

export interface TabsDeps {
  /**
   * 已加载的账号列表。只用于两处：按 id 取账号、恢复会话时剔除已删除的标签。
   * 由宿主持有这个 ref —— 账号列表同时被 `useProfiles` 写、被这里读，
   * 放在宿主可以避免两个 composable 互相依赖。
   */
  profiles: Ref<Profile[]>
  preferences: Ref<UiPreferences>
  /** 宿主浏览区域的可用矩形；尚未挂载时返回 null / undefined */
  getBounds: () => BrowserBounds | null | undefined
  /** 继承模式账号的全局默认主页 */
  globalDefaultUrl: () => string | undefined
  /** 打开失败时交给宿主弹诊断工具 */
  onActivateError: (profile: Profile, error: string) => void
}

export interface TabsApi {
  /** 已打开的标签页 id，有序 */
  openTabs: Ref<string[]>
  activeId: Ref<string | null>
  currentUrl: Ref<string>
  openedAt: Ref<Record<string, number>>
  /** 各标签累计运行秒数；休眠或手动暂停时停止累计 */
  runtimeSeconds: Ref<Record<string, number>>
  runtimePaused: Ref<Record<string, boolean>>
  /** 已休眠（WebView 被销毁但标签保留）的 id */
  hibernated: Ref<string[]>
  pinned: Ref<string[]>
  activate(id: string): Promise<boolean>
  closeTab(id: string): Promise<void>
  togglePin(id: string): void
  reorder(ids: string[]): void
  toggleRuntimePause(id: string): void
  resetRuntime(id: string): void
  /** 从 localStorage 恢复上次会话；必须在账号列表加载完成后调用 */
  restoreSession(): void
  startLifecycle(): void
  stopLifecycle(): void
  persist(): void
}

/**
 * 标签页生命周期：打开/关闭/固定/排序、运行计时、自动休眠、会话恢复。
 *
 * 只管"哪些标签在前端存在、哪个是当前标签"，不去动账号列表本身。
 */
export function useTabs(deps: TabsDeps): TabsApi {
  const { profiles, preferences, getBounds, globalDefaultUrl, onActivateError } = deps

  const openTabs = ref<string[]>([])
  const activeId = ref<string | null>(null)
  const currentUrl = ref('')
  const openedAt = ref<Record<string, number>>({})
  const runtimeSeconds = ref<Record<string, number>>({})
  const runtimePaused = ref<Record<string, boolean>>({})
  const hibernated = ref<string[]>([])
  const pinned = ref<string[]>([])
  const lastActiveAt = ref<Record<string, number>>({})

  let lifecycleTimer: ReturnType<typeof setInterval> | undefined

  function persist() {
    localStorage.setItem(
      TAB_SESSION_KEY,
      JSON.stringify({
        openTabs: openTabs.value,
        activeId: activeId.value,
        pinned: pinned.value,
      }),
    )
  }

  function profileById(id: string): Profile | undefined {
    return profiles.value.find((p) => p.id === id)
  }

  function restoreSession() {
    try {
      const raw = JSON.parse(localStorage.getItem(TAB_SESSION_KEY) || '{}')
      const valid = new Set(profiles.value.map((p) => p.id))
      openTabs.value = Array.isArray(raw.openTabs)
        ? raw.openTabs.filter((id: unknown): id is string => typeof id === 'string' && valid.has(id))
        : []
      pinned.value = Array.isArray(raw.pinned)
        ? raw.pinned.filter((id: unknown): id is string => typeof id === 'string' && valid.has(id))
        : []
      const restoredActive =
        typeof raw.activeId === 'string' && openTabs.value.includes(raw.activeId)
          ? raw.activeId
          : openTabs.value[openTabs.value.length - 1] || null
      activeId.value = restoredActive
      const now = Date.now()
      const opened: Record<string, number> = {}
      const last: Record<string, number> = {}
      const seconds: Record<string, number> = {}
      for (const id of openTabs.value) {
        opened[id] = now
        last[id] = now
        seconds[id] = 0
      }
      openedAt.value = opened
      lastActiveAt.value = last
      runtimeSeconds.value = seconds
      // 恢复时只保留当前标签的 WebView，其余标记为休眠，等真正切过去再创建。
      hibernated.value = openTabs.value.filter((id) => id !== restoredActive)
    } catch {
      /* 忽略过期/损坏的会话数据 */
    }
  }

  function togglePin(id: string) {
    pinned.value = pinned.value.includes(id)
      ? pinned.value.filter((item) => item !== id)
      : [...pinned.value, id]
    const pinnedIds = openTabs.value.filter((item) => pinned.value.includes(item))
    const normal = openTabs.value.filter((item) => !pinned.value.includes(item))
    openTabs.value = [...pinnedIds, ...normal]
    persist()
  }

  function reorder(ids: string[]) {
    openTabs.value = ids
    persist()
  }

  function toggleRuntimePause(id: string) {
    runtimePaused.value = { ...runtimePaused.value, [id]: !runtimePaused.value[id] }
  }

  function resetRuntime(id: string) {
    runtimeSeconds.value = { ...runtimeSeconds.value, [id]: 0 }
    openedAt.value = { ...openedAt.value, [id]: Date.now() }
  }

  async function hibernateTab(id: string) {
    if (id === activeId.value) return
    if (hibernated.value.includes(id) || pinned.value.includes(id)) return
    await invoke('hibernate_profile', { id }).catch(() => undefined)
    if (!hibernated.value.includes(id)) hibernated.value = [...hibernated.value, id]
  }

  async function activate(id: string): Promise<boolean> {
    const bounds = getBounds()
    if (!bounds) {
      ElMessage.warning('浏览区域尚未就绪，请再试一次')
      return false
    }

    const profile = profileById(id)
    if (!profile) {
      ElMessage.error('账号不存在或已被删除')
      return false
    }

    const previousActive = activeId.value
    const alreadyOpen = openTabs.value.includes(id)
    try {
      // 先让后端成功创建/显示 WebView，再提交前端标签状态。
      // 这样即使 URL/系统 WebView 异常，也不会留下一个"看似打开但点不进去"的坏标签。
      await invoke('activate_profile', { id, bounds })

      if (previousActive && previousActive !== id) {
        lastActiveAt.value = { ...lastActiveAt.value, [previousActive]: Date.now() }
      }
      if (!alreadyOpen) {
        openTabs.value = [...openTabs.value, id]
        openedAt.value = { ...openedAt.value, [id]: Date.now() }
        runtimeSeconds.value = {
          ...runtimeSeconds.value,
          [id]:
            preferences.value.runtimeMode === 'app-total' ? (runtimeSeconds.value[id] || 0) : 0,
        }
      }

      hibernated.value = hibernated.value.filter((tab) => tab !== id)
      lastActiveAt.value = { ...lastActiveAt.value, [id]: Date.now() }
      activeId.value = id
      currentUrl.value = resolveProfileUrl({
        ...profile,
        default_url:
          profile.url_mode === 'inherit'
            ? (globalDefaultUrl() || profile.default_url)
            : profile.default_url,
      })
      persist()

      // 有弹层处于打开状态时（例如"+"菜单展开中去侧栏点了账号），保持抑制。
      if (isSuppressed()) {
        await invoke('set_profile_webview_visible', { id, visible: false }).catch(() => undefined)
      }
      return true
    } catch (error) {
      onActivateError(profile, String(error))
      ElMessage.error(`打开“${profile.name}”失败，已打开诊断工具`)
      return false
    }
  }

  async function closeTab(id: string) {
    if (pinned.value.includes(id)) {
      ElMessage.info('固定标签需先取消固定才能关闭')
      return
    }
    if (openTabs.value.includes(id)) {
      await invoke('close_profile_tab', { id })
      openTabs.value = openTabs.value.filter((tab) => tab !== id)
      const { [id]: _closedAt, ...remainingOpenedAt } = openedAt.value
      openedAt.value = remainingOpenedAt
      hibernated.value = hibernated.value.filter((tab) => tab !== id)
      persist()
    }
    if (activeId.value === id) {
      const fallback = openTabs.value[openTabs.value.length - 1] ?? null
      if (fallback) {
        await activate(fallback)
      } else {
        activeId.value = null
        currentUrl.value = ''
      }
    }
  }

  function startLifecycle() {
    if (lifecycleTimer !== undefined) clearInterval(lifecycleTimer)
    lifecycleTimer = setInterval(() => {
      const now = Date.now()
      const next = { ...runtimeSeconds.value }
      for (const id of openTabs.value) {
        if (!hibernated.value.includes(id) && !runtimePaused.value[id]) {
          next[id] = (next[id] || 0) + 1
        }
        const inactiveMs = now - (lastActiveAt.value[id] || now)
        if (
          preferences.value.tabSleepEnabled &&
          id !== activeId.value &&
          !pinned.value.includes(id) &&
          !hibernated.value.includes(id) &&
          inactiveMs >= preferences.value.tabSleepMinutes * 60_000
        ) {
          void hibernateTab(id)
        }
      }
      runtimeSeconds.value = next
    }, 1000)
  }

  function stopLifecycle() {
    if (lifecycleTimer !== undefined) {
      clearInterval(lifecycleTimer)
      lifecycleTimer = undefined
    }
  }

  return {
    openTabs,
    activeId,
    currentUrl,
    openedAt,
    runtimeSeconds,
    runtimePaused,
    hibernated,
    pinned,
    activate,
    closeTab,
    togglePin,
    reorder,
    toggleRuntimePause,
    resetRuntime,
    restoreSession,
    startLifecycle,
    stopLifecycle,
    persist,
  }
}
