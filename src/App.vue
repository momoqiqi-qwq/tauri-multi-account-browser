<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ElMessage, ElMessageBox } from 'element-plus'
import ProfileSidebar from './components/ProfileSidebar.vue'
import StatusSidebar from './components/StatusSidebar.vue'
import TabStrip from './components/TabStrip.vue'
import AddressBar from './components/AddressBar.vue'
import BrowserViewport from './components/BrowserViewport.vue'
import BrowserActionDock from './components/BrowserActionDock.vue'
import ProfileSettingsDialog from './components/ProfileSettingsDialog.vue'
import DownloadsDialog from './components/DownloadsDialog.vue'
import AppPreferencesDialog from './components/AppPreferencesDialog.vue'
import AccountOverviewDialog from './components/AccountOverviewDialog.vue'
import AccountToolsDialog from './components/AccountToolsDialog.vue'
import ProfileDiagnosisDialog from './components/ProfileDiagnosisDialog.vue'
import { acquireWebviewSuppression, isSuppressed, registerSuppressionHandler } from './lib/suppress'
import { applyUiPreferences, loadUiPreferences, saveUiPreferences } from './lib/uiPreferences'
import { normalizeProfileUrl, resolveProfileUrl } from './lib/profileUrl'
import type {
  AppSettings,
  BrowserBounds,
  CustomToolbarButton,
  DownloadEntry,
  Profile,
  ProfileIsolationSettings,
  ProfileStatus,
  ProfileStatusEventPayload,
  UiPreferences,
} from './types'

const profiles = ref<Profile[]>([])
/** 已打开的账号标签页（有序），每个标签页对应一个独立隔离的 WebView */
const openTabs = ref<string[]>([])
/** 每个已打开标签页本次会话的开始时间，用于显示运行时长 */
const tabOpenedAt = ref<Record<string, number>>({})
/** 标签实际运行秒数；休眠或手动暂停时停止累计 */
const tabRuntimeSeconds = ref<Record<string, number>>({})
const tabRuntimePaused = ref<Record<string, boolean>>({})
const hibernatedTabs = ref<string[]>([])
const pinnedTabs = ref<string[]>([])
const tabLastActiveAt = ref<Record<string, number>>({})
const activeId = ref<string | null>(null)
const currentUrl = ref('')
/** 各账号最近一次上报的页面状态（chat01.ai 积分 / 登录账号） */
const statuses = ref<Record<string, ProfileStatus>>({})
/** 账号搜索关键词，同时过滤两个侧边栏 */
const searchQuery = ref('')
/** 全局下载设置（下载目录 / chat01 AI 文件自动下载） */
const dlSettings = ref<AppSettings | null>(null)
/** 下载历史（按账号分组展示在下载对话框） */
const downloadHistory = ref<DownloadEntry[]>([])
const downloadsVisible = ref(false)
const viewport = ref<InstanceType<typeof BrowserViewport> | null>(null)
const profileSettingsVisible = ref(false)
const profileSaving = ref(false)
const appSettingsVisible = ref(false)
const accountOverviewVisible = ref(false)
const accountToolsVisible = ref(false)
const diagnosisVisible = ref(false)
const diagnosisProfile = ref<Profile | null>(null)
const diagnosisError = ref('')
const uiPreferences = ref<UiPreferences>(loadUiPreferences())
applyUiPreferences(uiPreferences.value)
const editingProfile = ref<Profile | null>(null)
/** 两个侧边栏的收起状态，记住用户上一次的选择 */
const profileSidebarCollapsed = ref(localStorage.getItem('mab-profile-sidebar-collapsed') === '1')
const statusSidebarCollapsed = ref(localStorage.getItem('mab-status-sidebar-collapsed') === '1')
let unlistenNavigation: UnlistenFn | undefined
let unlistenStatus: UnlistenFn | undefined
let unlistenDownload: UnlistenFn | undefined
let unlistenDownloadBlocked: UnlistenFn | undefined
let unlistenDownloadGuardBlocked: UnlistenFn | undefined
let resizeObserver: ResizeObserver | undefined
let autoRefreshTimer: ReturnType<typeof setInterval> | undefined
let tabLifecycleTimer: ReturnType<typeof setInterval> | undefined

/** 按名称 / 分组过滤账号，两个侧边栏共用；空关键词返回全部。 */
const filteredProfiles = computed(() => {
  const query = searchQuery.value.trim().toLowerCase()
  if (!query) return profiles.value
  return profiles.value.filter(
    (p) =>
      p.name.toLowerCase().includes(query) || (p.group && p.group.toLowerCase().includes(query)),
  )
})

/** 新建/编辑账号时的分组下拉：按侧边栏出现顺序去重。 */
const profileGroups = computed(() => {
  const seen = new Set<string>()
  const groups: string[] = []
  for (const profile of profiles.value) {
    const group = profile.group.trim()
    if (!group || seen.has(group)) continue
    seen.add(group)
    groups.push(group)
  }
  return groups
})



const TAB_SESSION_KEY = 'mab-tab-session-v12'
function persistTabSession() {
  localStorage.setItem(TAB_SESSION_KEY, JSON.stringify({
    openTabs: openTabs.value,
    activeId: activeId.value,
    pinned: pinnedTabs.value,
  }))
}

function restoreTabSessionState() {
  try {
    const raw = JSON.parse(localStorage.getItem(TAB_SESSION_KEY) || '{}')
    const valid = new Set(profiles.value.map((p) => p.id))
    openTabs.value = Array.isArray(raw.openTabs) ? raw.openTabs.filter((id: unknown): id is string => typeof id === 'string' && valid.has(id)) : []
    pinnedTabs.value = Array.isArray(raw.pinned) ? raw.pinned.filter((id: unknown): id is string => typeof id === 'string' && valid.has(id)) : []
    const restoredActive = typeof raw.activeId === 'string' && openTabs.value.includes(raw.activeId) ? raw.activeId : openTabs.value[openTabs.value.length - 1] || null
    activeId.value = restoredActive
    const now = Date.now()
    const opened: Record<string, number> = {}; const last: Record<string, number> = {}; const seconds: Record<string, number> = {}
    for (const id of openTabs.value) { opened[id] = now; last[id] = now; seconds[id] = 0 }
    tabOpenedAt.value = opened; tabLastActiveAt.value = last; tabRuntimeSeconds.value = seconds
    hibernatedTabs.value = openTabs.value.filter((id) => id !== restoredActive)
  } catch { /* ignore stale state */ }
}

function togglePin(id: string) {
  pinnedTabs.value = pinnedTabs.value.includes(id) ? pinnedTabs.value.filter((item) => item !== id) : [...pinnedTabs.value, id]
  const pinned = openTabs.value.filter((item) => pinnedTabs.value.includes(item))
  const normal = openTabs.value.filter((item) => !pinnedTabs.value.includes(item))
  openTabs.value = [...pinned, ...normal]
  persistTabSession()
}
function reorderTabs(ids: string[]) { openTabs.value = ids; persistTabSession() }
function toggleRuntimePause(id: string) { tabRuntimePaused.value = { ...tabRuntimePaused.value, [id]: !tabRuntimePaused.value[id] } }
function resetRuntime(id: string) { tabRuntimeSeconds.value = { ...tabRuntimeSeconds.value, [id]: 0 }; tabOpenedAt.value = { ...tabOpenedAt.value, [id]: Date.now() } }

async function hibernateTab(id: string) {
  if (id === activeId.value || hibernatedTabs.value.includes(id) || pinnedTabs.value.includes(id)) return
  await invoke('hibernate_profile', { id }).catch(() => undefined)
  if (!hibernatedTabs.value.includes(id)) hibernatedTabs.value = [...hibernatedTabs.value, id]
}

function startTabLifecycle() {
  if (tabLifecycleTimer !== undefined) clearInterval(tabLifecycleTimer)
  tabLifecycleTimer = setInterval(() => {
    const now = Date.now()
    const next = { ...tabRuntimeSeconds.value }
    for (const id of openTabs.value) {
      if (!hibernatedTabs.value.includes(id) && !tabRuntimePaused.value[id]) next[id] = (next[id] || 0) + 1
      const inactiveMs = now - (tabLastActiveAt.value[id] || now)
      if (uiPreferences.value.tabSleepEnabled && id !== activeId.value && !pinnedTabs.value.includes(id) && !hibernatedTabs.value.includes(id) && inactiveMs >= uiPreferences.value.tabSleepMinutes * 60_000) void hibernateTab(id)
    }
    tabRuntimeSeconds.value = next
  }, 1000)
}

function persistSidebarState(key: string, collapsed: boolean) {
  localStorage.setItem(key, collapsed ? '1' : '0')
}

function toggleProfileSidebar() {
  profileSidebarCollapsed.value = !profileSidebarCollapsed.value
  persistSidebarState('mab-profile-sidebar-collapsed', profileSidebarCollapsed.value)
}

function toggleStatusSidebar() {
  statusSidebarCollapsed.value = !statusSidebarCollapsed.value
  persistSidebarState('mab-status-sidebar-collapsed', statusSidebarCollapsed.value)
}

function profileById(id: string): Profile | undefined {
  return profiles.value.find((p) => p.id === id)
}

async function loadProfiles() {
  profiles.value = await invoke<Profile[]>('list_profiles')
}

async function loadStatuses() {
  statuses.value = await invoke<Record<string, ProfileStatus>>('get_profile_statuses').catch(
    () => ({}),
  )
}

async function loadSettings() {
  dlSettings.value = await invoke<AppSettings>('get_app_settings').catch(() => null)
}

async function loadDownloadHistory() {
  downloadHistory.value = await invoke<DownloadEntry[]>('get_download_history').catch(() => [])
}

function normalizeExtensionWhitelist(raw: string): string {
  const seen = new Set<string>()
  return raw
    .split(',')
    .map((item) => item.trim().replace(/^\.+/, '').toLowerCase())
    .filter((item) => /^[a-z0-9]{1,12}$/.test(item))
    .filter((item) => {
      if (seen.has(item)) return false
      seen.add(item)
      return true
    })
    .join(',')
}

async function saveSettings(settings: AppSettings) {
  const normalized = { ...settings, ai_exts: normalizeExtensionWhitelist(settings.ai_exts) }
  try {
    await invoke('set_app_settings', { settings: normalized })
    dlSettings.value = await invoke<AppSettings>('get_app_settings')
    await loadProfiles()
    ElMessage.success('应用设置已保存；继承模式账号已同步全局默认主页')
  } catch (error) {
    ElMessage.error(`保存应用设置失败：${String(error)}`)
  }
}

function updateUiPreferences(preferences: UiPreferences) {
  uiPreferences.value = saveUiPreferences(preferences)
  // 工具栏位置/尺寸改变会直接改变原生子 WebView 的可用矩形。
  // 等 Vue 完成布局后立即同步，避免切换“底部 -> 右侧”时旧 WebView 短暂盖住宿主按钮。
  void nextTick().then(() => syncBounds())
}

function stopAutoRefresh() {
  if (autoRefreshTimer !== undefined) {
    clearInterval(autoRefreshTimer)
    autoRefreshTimer = undefined
  }
}

function startAutoRefresh() {
  stopAutoRefresh()
  const seconds = uiPreferences.value.autoRefreshSeconds
  if (!Number.isFinite(seconds) || seconds <= 0) return
  autoRefreshTimer = setInterval(() => {
    if (!activeId.value || isSuppressed()) return
    if (uiPreferences.value.pauseAutoRefreshWhileGenerating && statuses.value[activeId.value]?.answer_generating) return
    const action = uiPreferences.value.restorePositionAfterRefresh ? 'reload_restore' : 'reload'
    void invoke('browser_action', { id: activeId.value, action }).catch(() => undefined)
  }, Math.max(1, seconds) * 1000)
}

watch(
  () => uiPreferences.value.autoRefreshSeconds,
  () => startAutoRefresh(),
)
watch(activeId, () => startAutoRefresh())

async function refreshStatuses() {
  await invoke('refresh_profile_statuses').catch(() => undefined)
}

function openAccountOverview() {
  accountOverviewVisible.value = true
  void refreshStatuses()
}

async function cloneProfile(profile: Profile) {
  try {
    const copy = await invoke<Profile>('clone_profile', { id: profile.id })
    await loadProfiles()
    ElMessage.success(`已创建副本“${copy.name}”，会话数据为全新状态`)
  } catch (error) {
    ElMessage.error(`克隆账号失败：${String(error)}`)
  }
}

async function testProxy(profile: Profile) {
  const loading = ElMessage({
    message: profile.proxy
      ? `正在经代理检测出口 IP：${profile.name}`
      : `正在检测直连出口 IP：${profile.name}`,
    duration: 0,
  })
  try {
    const result = await invoke<{ ok: boolean; ip: string; region: string; isp: string; error: string }>(
      'test_profile_proxy',
      { id: profile.id },
    )
    if (result.ok) {
      ElMessage.success(`${profile.name} 出口：${result.ip}（${result.region} · ${result.isp}）`)
    } else {
      ElMessage.error(`${profile.name} 代理检测失败：${result.error}`)
    }
  } catch (e) {
    ElMessage.error(`代理检测失败：${e}`)
  } finally {
    loading.close()
  }
}

async function activateProfile(id: string): Promise<boolean> {
  const bounds = viewport.value?.getBounds()
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
    // 这样即使 URL/系统 WebView 异常，也不会留下一个“看似打开但点不进去”的坏标签。
    await invoke('activate_profile', { id, bounds })

    if (previousActive && previousActive !== id) {
      tabLastActiveAt.value = { ...tabLastActiveAt.value, [previousActive]: Date.now() }
    }
    if (!alreadyOpen) {
      openTabs.value = [...openTabs.value, id]
      tabOpenedAt.value = { ...tabOpenedAt.value, [id]: Date.now() }
      tabRuntimeSeconds.value = {
        ...tabRuntimeSeconds.value,
        [id]: uiPreferences.value.runtimeMode === 'app-total' ? (tabRuntimeSeconds.value[id] || 0) : 0,
      }
    }

    hibernatedTabs.value = hibernatedTabs.value.filter((tab) => tab !== id)
    tabLastActiveAt.value = { ...tabLastActiveAt.value, [id]: Date.now() }
    activeId.value = id
    currentUrl.value = resolveProfileUrl({ ...profile, default_url: profile.url_mode === 'inherit' ? (dlSettings.value?.global_default_url || profile.default_url) : profile.default_url })
    persistTabSession()

    // 有弹层处于打开状态时（例如“+”菜单展开中去侧栏点了账号），保持抑制。
    if (isSuppressed()) {
      await invoke('set_profile_webview_visible', { id, visible: false }).catch(() => undefined)
    }
    return true
  } catch (error) {
    diagnosisProfile.value = profile
    diagnosisError.value = String(error)
    diagnosisVisible.value = true
    ElMessage.error(`打开“${profile.name}”失败，已打开诊断工具`)
    return false
  }
}

async function closeTab(id: string) {
  if (pinnedTabs.value.includes(id)) { ElMessage.info('固定标签需先取消固定才能关闭'); return }
  if (openTabs.value.includes(id)) {
    await invoke('close_profile_tab', { id })
    openTabs.value = openTabs.value.filter((tab) => tab !== id)
    const { [id]: _closedAt, ...remainingOpenedAt } = tabOpenedAt.value
    tabOpenedAt.value = remainingOpenedAt
    hibernatedTabs.value = hibernatedTabs.value.filter((tab) => tab !== id)
    persistTabSession()
  }
  if (activeId.value === id) {
    const fallback = openTabs.value[openTabs.value.length - 1] ?? null
    if (fallback) {
      await activateProfile(fallback)
    } else {
      activeId.value = null
      currentUrl.value = ''
    }
  }
}

async function createProfile(settings: ProfileIsolationSettings) {
  if (profileSaving.value) return
  profileSaving.value = true
  try {
    const profile = await invoke<Profile>('create_profile', {
      name: settings.name,
      defaultUrl: settings.defaultUrl,
      urlMode: settings.urlMode,
      incognito: settings.incognito,
      proxy: settings.proxy,
      userAgent: settings.userAgent,
      timezone: settings.timezone,
      locale: settings.locale,
      fingerprintGuard: settings.fingerprintGuard,
      group: settings.group,
    })
    await loadProfiles()
    profileSettingsVisible.value = false
    await nextTick()
    await activateProfile(profile.id)
  } catch (error) {
    // 保存失败时保留对话框和用户输入，避免修正一个字段后还要重新填写整张表单。
    ElMessage.error(`创建账号失败：${String(error)}`)
  } finally {
    profileSaving.value = false
  }
}

async function updateProfile(id: string, settings: ProfileIsolationSettings) {
  if (profileSaving.value) return
  profileSaving.value = true
  const wasOpen = openTabs.value.includes(id)
  try {
    const result = await invoke<{ profile: Profile; needs_reopen: boolean }>('update_profile', {
      id,
      name: settings.name,
      defaultUrl: settings.defaultUrl,
      urlMode: settings.urlMode,
      incognito: settings.incognito,
      proxy: settings.proxy,
      userAgent: settings.userAgent,
      timezone: settings.timezone,
      locale: settings.locale,
      fingerprintGuard: settings.fingerprintGuard,
      group: settings.group,
    })
    await loadProfiles()
    profileSettingsVisible.value = false
    if (result.needs_reopen) {
      // 隔离配置变更后 WebView 已被后端销毁，重新打开以应用新配置。
      openTabs.value = openTabs.value.filter((tab) => tab !== id)
      if (wasOpen) {
        await nextTick()
        const reopened = await activateProfile(id)
        if (reopened) ElMessage.success('隔离设置已生效：该账号已用新配置重建浏览器')
      } else {
        ElMessage.success('已保存，下次打开该账号时生效')
      }
    } else {
      ElMessage.success('已保存')
    }
  } catch (error) {
    ElMessage.error(`保存账号失败：${String(error)}`)
  } finally {
    profileSaving.value = false
  }
}

function openCreateDialog() {
  editingProfile.value = null
  profileSettingsVisible.value = true
}

function openSettingsDialog(profile: Profile) {
  editingProfile.value = profile
  profileSettingsVisible.value = true
}

async function deleteProfile(profile: Profile) {
  const release = acquireWebviewSuppression()
  try {
    await ElMessageBox.confirm(
      `删除“${profile.name}”将同时删除该账号的浏览数据，且不可恢复。`,
      '确认删除',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
    )
    openTabs.value = openTabs.value.filter((tab) => tab !== profile.id)
    pinnedTabs.value = pinnedTabs.value.filter((tab) => tab !== profile.id)
    hibernatedTabs.value = hibernatedTabs.value.filter((tab) => tab !== profile.id)
    persistTabSession()
    if (activeId.value === profile.id) {
      activeId.value = null
      currentUrl.value = ''
    }
    await invoke('delete_profile', { id: profile.id })
    await loadProfiles()
    // 后端已同步清理该账号的状态缓存，前端保持一致。
    const { [profile.id]: _removed, ...rest } = statuses.value
    statuses.value = rest
    if (!activeId.value && openTabs.value.length > 0) {
      await activateProfile(openTabs.value[openTabs.value.length - 1])
    }
  } finally {
    release()
  }
}

async function clearProfile(profile: Profile) {
  const release = acquireWebviewSuppression()
  try {
    await ElMessageBox.confirm(
      `将清空“${profile.name}”的 Cookie、LocalStorage、IndexedDB 和缓存，所有登录态会退出。`,
      '清除账号数据',
      { type: 'warning' },
    )
    await invoke('clear_profile_data', { id: profile.id })
    openTabs.value = openTabs.value.filter((tab) => tab !== profile.id)
    if (activeId.value === profile.id) {
      const fallback = openTabs.value[openTabs.value.length - 1] ?? null
      if (fallback) {
        await activateProfile(fallback)
      } else {
        activeId.value = null
        currentUrl.value = ''
      }
    }
    ElMessage.success('已清除账号数据，重新打开标签页即为全新会话')
  } finally {
    release()
  }
}

async function navigate(url: string) {
  if (!activeId.value) return
  let normalized: string
  try {
    normalized = normalizeProfileUrl(url)
  } catch (error) {
    ElMessage.error(String(error))
    return
  }
  try {
    await invoke('navigate_profile', { id: activeId.value, url: normalized })
  } catch (error) {
    ElMessage.error(`打开网址失败：${String(error)}`)
  }
}

async function browserAction(action: 'back' | 'forward' | 'reload') {
  if (!activeId.value) return
  await browserActionFor(activeId.value, action)
}

async function goHome() {
  const profile = activeId.value ? profileById(activeId.value) : undefined
  if (!profile) return
  const home = profile.url_mode === 'inherit'
    ? (dlSettings.value?.global_default_url || profile.default_url)
    : profile.default_url
  await navigate(normalizeProfileUrl(home))
}

async function copyCurrentUrl() {
  const value = currentUrl.value.trim()
  if (!value) return
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value)
    } else {
      const input = document.createElement('textarea')
      input.value = value
      input.style.cssText = 'position:fixed;left:-9999px;top:-9999px;opacity:0;'
      document.body.appendChild(input)
      input.select()
      const copied = document.execCommand('copy')
      input.remove()
      if (!copied) throw new Error('系统剪贴板不可用')
    }
    ElMessage.success('当前网址已复制')
  } catch (error) {
    ElMessage.error(`复制网址失败：${String(error)}`)
  }
}

async function openCustomToolbarButton(button: CustomToolbarButton) {
  if (!activeId.value || !button.url.trim()) return
  await navigate(button.url.trim())
}

/** 对指定账号执行浏览器动作；宿主工具栏和地址栏统一走这里，确保遵守刷新偏好。 */
async function browserActionFor(id: string, action: 'back' | 'forward' | 'reload') {
  const resolvedAction = action === 'reload' && uiPreferences.value.restorePositionAfterRefresh
    ? 'reload_restore'
    : action
  try {
    await invoke('browser_action', { id, action: resolvedAction })
  } catch (error) {
    ElMessage.error(`浏览器操作失败：${String(error)}`)
  }
}

async function syncBounds() {
  const bounds = viewport.value?.getBounds()
  if (!bounds) return
  await invoke('sync_profile_bounds', { bounds, activeId: activeId.value }).catch(() => undefined)
}

/** 拖动窗口大小时 ResizeObserver 会高频触发，合并成停止变化后的一次 IPC。 */
let syncBoundsTimer: ReturnType<typeof setTimeout> | undefined
function scheduleSyncBounds() {
  if (syncBoundsTimer !== undefined) return
  syncBoundsTimer = setTimeout(() => {
    syncBoundsTimer = undefined
    void syncBounds()
  }, 80)
}

async function reorderProfiles(ids: string[]) {
  try {
    await invoke('reorder_profiles', { ids })
    await loadProfiles()
  } catch (error) {
    ElMessage.error(`账号排序保存失败：${String(error)}`)
    await loadProfiles().catch(() => undefined)
  }
}

async function moveProfileToGroup(id: string, group: string, ids: string[]) {
  try {
    await invoke('move_profile_to_group', { id, group, ids })
    await loadProfiles()
  } catch (error) {
    ElMessage.error(`移动账号失败：${String(error)}`)
    await loadProfiles().catch(() => undefined)
  }
}

onMounted(async () => {
  registerSuppressionHandler((suppress) => {
    if (!activeId.value) return
    void invoke('set_profile_webview_visible', { id: activeId.value, visible: !suppress }).catch(
      () => undefined,
    )
  })
  await loadProfiles()
  restoreTabSessionState()
  // 这三项互不依赖，并行加载可明显减少账号较多/历史较长时的首屏等待。
  await Promise.all([loadStatuses(), loadSettings(), loadDownloadHistory()])
  unlistenNavigation = await listen<{ id: string; url: string }>('profile:navigated', (event) => {
    const { id, url } = event.payload
    const p = profiles.value.find((item) => item.id === id)
    if (p) p.last_url = url
    if (activeId.value === id) currentUrl.value = url
  })
  unlistenDownloadBlocked = await listen<string>('profile:download-blocked', (event) => {
    ElMessage.warning(`已阻止重复下载：${event.payload}`)
  })
  unlistenDownloadGuardBlocked = await listen<{ seconds: number; queued: boolean }>('profile:download-guard-blocked', (event) => {
    ElMessage.warning(event.payload.queued ? `下载处于保护期，已加入队列，将在约 ${event.payload.seconds} 秒后自动继续` : `页面刚打开，已阻止下载；请等待约 ${event.payload.seconds} 秒`)
  })
  unlistenStatus = await listen<ProfileStatusEventPayload>('profile:status', (event) => {
    const { id, ...status } = event.payload
    statuses.value = { ...statuses.value, [id]: status }
  })
  unlistenDownload = await listen<DownloadEntry>('profile:download', (event) => {
    const entry = event.payload
    const limit = dlSettings.value?.download_history_limit ?? 200
    downloadHistory.value = [entry, ...downloadHistory.value].slice(0, limit)
    if (entry.success) {
      ElMessage.success(`下载完成：${entry.file_name}`)
    } else {
      ElMessage.error(`下载失败：${entry.file_name}`)
    }
  })

  if (activeId.value) {
    const restored = activeId.value
    activeId.value = null
    await nextTick()
    await activateProfile(restored)
  }
  startAutoRefresh()
  startTabLifecycle()

  resizeObserver = new ResizeObserver(() => scheduleSyncBounds())
  const el = viewport.value?.element()
  if (el) resizeObserver.observe(el)
})

onBeforeUnmount(() => {
  registerSuppressionHandler(null)
  unlistenNavigation?.()
  unlistenDownloadBlocked?.()
  unlistenDownloadGuardBlocked?.()
  unlistenStatus?.()
  unlistenDownload?.()
  resizeObserver?.disconnect()
  stopAutoRefresh()
  persistTabSession()
  if (tabLifecycleTimer !== undefined) clearInterval(tabLifecycleTimer)
  if (syncBoundsTimer !== undefined) clearTimeout(syncBoundsTimer)
})
</script>

<template>
  <div class="app-shell">
    <ProfileSidebar
      :profiles="filteredProfiles"
      :total="profiles.length"
      :active-id="activeId"
      :open-tabs="openTabs"
      :collapsed="profileSidebarCollapsed"
      v-model:search="searchQuery"
      @toggle="toggleProfileSidebar"
      @open-create="openCreateDialog"
      @open-tools="accountToolsVisible = true"
      @open-settings="openSettingsDialog"
      @activate="activateProfile"
      @delete="deleteProfile"
      @clear="clearProfile"
      @reorder="reorderProfiles"
      @move-group="moveProfileToGroup"
      @clone="cloneProfile"
      @test-proxy="testProxy"
    />
    <StatusSidebar
      :profiles="filteredProfiles"
      :statuses="statuses"
      :active-id="activeId"
      :open-tabs="openTabs"
      :collapsed="statusSidebarCollapsed"
      :dl-settings="dlSettings"
      :download-count="downloadHistory.length"
      :show-account="uiPreferences.showStatusAccount"
      :show-proxy="uiPreferences.showStatusProxy"
      :show-updated-time="uiPreferences.showStatusUpdatedTime"
      @toggle="toggleStatusSidebar"
      @activate="activateProfile"
      @refresh="refreshStatuses"
      @save-settings="saveSettings"
      @open-downloads="downloadsVisible = true"
    />
    <main class="main-pane">
      <TabStrip
        :profiles="profiles"
        :open-tabs="openTabs"
        :opened-at="tabOpenedAt"
        :runtime-seconds="tabRuntimeSeconds"
        :runtime-paused="tabRuntimePaused"
        :hibernated="hibernatedTabs"
        :pinned="pinnedTabs"
        :width="uiPreferences.tabWidth"
        :active-id="activeId"
        :statuses="statuses"
        :show-ai-status="uiPreferences.showTabAiStatus"
        @activate="activateProfile"
        @close="closeTab"
        @reorder="reorderTabs"
        @pin="togglePin"
        @pause-runtime="toggleRuntimePause"
        @reset-runtime="resetRuntime"
        @create="openCreateDialog"
      />
      <AddressBar
        v-model="currentUrl"
        :disabled="!activeId"
        @navigate="navigate"
        @back="browserAction('back')"
        @forward="browserAction('forward')"
        @reload="browserAction('reload')"
        @account-overview="openAccountOverview"
        @settings="appSettingsVisible = true"
      />
      <div
        class="browser-workspace"
        :class="`dock-${uiPreferences.browserToolbarPosition}`"
      >
        <BrowserViewport ref="viewport" :empty="!activeId" @create="openCreateDialog" />
        <BrowserActionDock
          v-if="uiPreferences.browserToolbarEnabled"
          :disabled="!activeId"
          :position="uiPreferences.browserToolbarPosition"
          :compact="uiPreferences.browserToolbarCompact"
          :items="uiPreferences.browserToolbarItems"
          :custom-buttons="uiPreferences.customToolbarButtons"
          @back="browserAction('back')"
          @forward="browserAction('forward')"
          @reload="browserAction('reload')"
          @home="goHome"
          @copy-url="copyCurrentUrl"
          @account-overview="openAccountOverview"
          @downloads="downloadsVisible = true"
          @settings="appSettingsVisible = true"
          @custom="openCustomToolbarButton"
        />
      </div>
    </main>
    <AccountOverviewDialog
      v-model="accountOverviewVisible"
      :profiles="profiles"
      :statuses="statuses"
      :history="downloadHistory"
      :active-id="activeId"
      :open-tabs="openTabs"
      @activate="activateProfile"
      @refresh="refreshStatuses"
    />
    <ProfileSettingsDialog
      v-model="profileSettingsVisible"
      :profile="editingProfile"
      :groups="profileGroups"
      :saving="profileSaving"
      :global-default-url="dlSettings?.global_default_url || 'https://chat01.ai/'"
      @create="createProfile"
      @update="updateProfile"
    />
    <AccountToolsDialog
      v-model="accountToolsVisible"
      :profiles="profiles"
      :app-settings="dlSettings"
      @changed="loadProfiles"
    />
    <ProfileDiagnosisDialog
      v-model="diagnosisVisible"
      :profile="diagnosisProfile"
      :error="diagnosisError"
      @changed="loadProfiles"
      @retry="activateProfile"
    />
    <DownloadsDialog
      v-model="downloadsVisible"
      :profiles="profiles"
      :history="downloadHistory"
      :settings="dlSettings"
      :ui-preferences="uiPreferences"
      @cleared="downloadHistory = []"
      @changed="loadDownloadHistory"
      @update-ui="updateUiPreferences"
    />
    <AppPreferencesDialog
      v-model="appSettingsVisible"
      :ui-preferences="uiPreferences"
      :download-settings="dlSettings"
      @save-ui="updateUiPreferences"
      @save-downloads="saveSettings"
    />
  </div>
</template>
