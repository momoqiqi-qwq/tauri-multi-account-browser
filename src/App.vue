<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ElMessage } from 'element-plus'
import ProfileSidebar from './components/ProfileSidebar.vue'
import StatusSidebar from './components/StatusSidebar.vue'
import TitleBar from './components/TitleBar.vue'
import TabStrip from './components/TabStrip.vue'
import AddressBar from './components/AddressBar.vue'
import BrowserViewport from './components/BrowserViewport.vue'
import WebsiteDirectory from './components/WebsiteDirectory.vue'
import BrowserActionDock from './components/BrowserActionDock.vue'
import ProfileSettingsDialog from './components/ProfileSettingsDialog.vue'
import DownloadsDialog from './components/DownloadsDialog.vue'
import AppPreferencesDialog from './components/AppPreferencesDialog.vue'
import AccountOverviewDialog from './components/AccountOverviewDialog.vue'
import AccountToolsDialog from './components/AccountToolsDialog.vue'
import AccountGeneratorDialog from './components/AccountGeneratorDialog.vue'
import { ALL_SITES } from './lib/siteDirectory'
import ProfileDiagnosisDialog from './components/ProfileDiagnosisDialog.vue'
import { useProfiles } from './composables/useProfiles'
import { useSettings } from './composables/useSettings'
import { useTabs } from './composables/useTabs'
import { isSuppressed, registerSuppressionHandler } from './lib/suppress'
import { applyUiPreferences, loadUiPreferences, saveUiPreferences } from './lib/uiPreferences'
import { comboFromEvent, findShortcut, type ShortcutSpec } from './lib/shortcuts'
import { normalizeProfileUrl } from './lib/profileUrl'
import type {
  AppSettings,
  CustomToolbarButton,
  DownloadEntry,
  Profile,
  ProfileIsolationSettings,
  ProfileStatusEventPayload,
  ProxyTestResult,
  UiPreferences,
} from './types'

// ---- 跨 composable 共享的状态 ----
/**
 * 账号列表。`useProfiles` 写它（CRUD 后重新拉取），`useTabs` 读它（按 id 取账号、
 * 恢复会话时剔除已删除的标签）。放在宿主持有可以避免两个 composable 互相依赖。
 */
const profiles = ref<Profile[]>([])

// ---- 界面偏好与宿主 UI 状态 ----
const uiPreferences = ref<UiPreferences>(loadUiPreferences())
applyUiPreferences(uiPreferences.value)
const viewport = ref<InstanceType<typeof BrowserViewport> | null>(null)
const addressBarRef = ref<InstanceType<typeof AddressBar> | null>(null)
const downloadsVisible = ref(false)
const appSettingsVisible = ref(false)
const accountOverviewVisible = ref(false)
const accountToolsVisible = ref(false)
const accountGeneratorVisible = ref(false)
const generatorInitialUrl = ref<string | undefined>()
const websiteDirectoryRef = ref<InstanceType<typeof WebsiteDirectory> | null>(null)
const diagnosisVisible = ref(false)
const diagnosisProfile = ref<Profile | null>(null)
const diagnosisError = ref('')
const websiteDirectoryVisible = ref(true)
const directoryOpeningId = ref<string | null>(null)
const directoryLoading = ref(true)
const directoryLoadError = ref('')
const appReady = ref(false)
const createInitialUrl = ref<string | undefined>()
/** 两个侧边栏的收起状态，记住用户上一次的选择 */
const profileSidebarCollapsed = ref(localStorage.getItem('mab-profile-sidebar-collapsed') === '1')
const statusSidebarCollapsed = ref(localStorage.getItem('mab-status-sidebar-collapsed') === '1')

// ---- 三个领域 composable ----
const settings = useSettings()
const { dlSettings, downloadHistory } = settings

const tabs = useTabs({
  profiles,
  preferences: uiPreferences,
  getBounds: () => viewport.value?.getBounds(),
  globalDefaultUrl: () => settings.dlSettings.value?.global_default_url,
  onActivateError: (profile, error) => {
    diagnosisProfile.value = profile
    diagnosisError.value = error
    diagnosisVisible.value = true
  },
})
const {
  openTabs,
  activeId,
  currentUrl,
  hibernated: hibernatedTabs,
  pinned: pinnedTabs,
} = tabs

const accounts = useProfiles({ profiles, tabs })
const {
  statuses,
  searchQuery,
  filteredProfiles,
  visibleProfiles,
  viewMode,
  activeTag,
  allTags,
  profileGroups,
  editingProfile,
  editorVisible,
  profileSaving,
  batchBusy,
} = accounts

/** Directory is a host page, not a new browser session or another account store. */
function openCreateDialog(url?: string) {
  createInitialUrl.value = url
  accounts.openCreateDialog()
}

async function openAccountGenerator(url?: string) {
  generatorInitialUrl.value = url
  accountGeneratorVisible.value = true
  // Acquire the next dialog's native suppression before releasing the tools dialog's.
  await nextTick()
  accountToolsVisible.value = false
}

function acceptGeneratedProfiles(created: Profile[]) {
  // The command has already committed. Merge its acknowledged result immediately;
  // never turn a later refresh failure into a misleading 'creation failed' retry.
  const merged = new Map(profiles.value.map(profile => [profile.id, profile]))
  for (const profile of created) merged.set(profile.id, profile)
  profiles.value = [...merged.values()].sort((a, b) => a.order - b.order)
}

async function browseGeneratedProfiles() {
  websiteDirectoryVisible.value = true
  await nextTick()
  websiteDirectoryRef.value?.selectSite(ALL_SITES)
  accountGeneratorVisible.value = false
}

async function loadDirectoryData(): Promise<boolean> {
  directoryLoading.value = true
  directoryLoadError.value = ''
  try {
    await Promise.all([accounts.loadProfiles(), settings.loadSettings()])
    return true
  } catch (error) {
    directoryLoadError.value = String(error)
    return false
  } finally {
    directoryLoading.value = false
  }
}

async function activateDirectoryProfile(id: string) {
  if (directoryOpeningId.value || !appReady.value) return
  directoryOpeningId.value = id
  try {
    if (await tabs.activate(id)) {
      websiteDirectoryVisible.value = false
      await nextTick()
      await syncBounds()
    }
  } finally {
    directoryOpeningId.value = null
  }
}

function returnToBrowser() {
  if (activeId.value) void activateDirectoryProfile(activeId.value)
}

async function createProfileFromEditor(form: ProfileIsolationSettings) {
  if (directoryOpeningId.value) return
  if (websiteDirectoryVisible.value) directoryOpeningId.value = '@creating'
  try {
    await accounts.createProfile(form)
    if (!editorVisible.value && !diagnosisVisible.value && activeId.value) {
      websiteDirectoryVisible.value = false
    }
  } finally {
    directoryOpeningId.value = null
  }
}

let unlistenNavigation: UnlistenFn | undefined
let unlistenStatus: UnlistenFn | undefined
let unlistenDownload: UnlistenFn | undefined
let unlistenDownloadBlocked: UnlistenFn | undefined
let unlistenDownloadGuardBlocked: UnlistenFn | undefined
let unlistenHotkey: UnlistenFn | undefined
let resizeObserver: ResizeObserver | undefined
let autoRefreshTimer: ReturnType<typeof setInterval> | undefined
let syncBoundsTimer: ReturnType<typeof setTimeout> | undefined

/** 保存全局设置后再拉一次账号列表：继承模式账号的主页依赖 global_default_url。 */
function saveSettings(next: AppSettings) {
  return settings.saveSettings(next, accounts.loadProfiles)
}

function updateUiPreferences(preferences: UiPreferences) {
  uiPreferences.value = saveUiPreferences(preferences)
  // 工具栏位置/尺寸改变会直接改变原生子 WebView 的可用矩形。
  // 等 Vue 完成布局后立即同步，避免切换"底部 -> 右侧"时旧 WebView 短暂盖住宿主按钮。
  void nextTick().then(() => syncBounds())
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
    const id = activeId.value
    if (!id || isSuppressed()) return
    // 只在 AI 正在回答时才刷新 —— 平时刷新是浪费，也要避免打断任务。
    // 见 v24：旧行为是「回答时暂停」，现在反过来了；想完全不刷就把秒数设为 0。
    if (!statuses.value[id]?.answer_generating) return
    const action = uiPreferences.value.restorePositionAfterRefresh ? 'reload_restore' : 'reload'
    void invoke('browser_action', { id, action }).catch(() => undefined)
  }, Math.max(1, seconds) * 1000)
}

watch(
  () => uiPreferences.value.autoRefreshSeconds,
  () => startAutoRefresh(),
)
watch(activeId, () => startAutoRefresh())

function openAccountOverview() {
  accountOverviewVisible.value = true
  void accounts.refreshStatuses()
}

async function testProxy(profile: Profile) {
  const loading = ElMessage({
    message: profile.proxy
      ? `正在经代理检测出口 IP：${profile.name}`
      : `正在检测直连出口 IP：${profile.name}`,
    duration: 0,
  })
  try {
    const result = await invoke<ProxyTestResult>('test_profile_proxy', { id: profile.id })
    if (result.ok) {
      const route = result.via_proxy ? `代理 ${result.endpoint}` : '直连'
      ElMessage.success(
        `${profile.name} 出口：${result.ip}（${result.region} · ${result.isp}） · ${route} · ${result.latency_ms} ms`,
      )
    } else {
      ElMessage.error(`${profile.name} 代理检测失败：${result.error}`)
    }
  } catch (e) {
    ElMessage.error(`代理检测失败：${e}`)
  } finally {
    loading.close()
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

async function browserAction(action: 'back' | 'forward' | 'reload') {
  if (!activeId.value) return
  await browserActionFor(activeId.value, action)
}

// ---- 键盘快捷键（v27）----
// 焦点在宿主 UI 时走 window keydown；焦点在账号网页里时按键由注入脚本经
// mbkey:// 假导航转发回来（profile:hotkey 事件），两路最终都进 runShortcut。
function cycleTab(delta: number) {
  const list = openTabs.value
  if (list.length < 2) return
  const current = activeId.value ? list.indexOf(activeId.value) : -1
  const next = list[(current + delta + list.length) % list.length]
  if (next) void tabs.activate(next)
}

async function focusAddressBar() {
  // 账号 WebView 通常持有原生焦点，先把焦点交回主界面，输入框才能接收输入。
  try {
    await invoke('focus_main_ui')
  } catch {
    /* 焦点已经在宿主侧时主界面可能尚未创建为独立 WebView，忽略即可 */
  }
  addressBarRef.value?.focusUrl()
}

function runShortcut(spec: ShortcutSpec) {
  // 弹层打开期间（此时账号 WebView 已被抑制隐藏）不响应全局快捷键，
  // 避免在对话框底下偷偷切标签、关标签。
  if (isSuppressed()) return
  switch (spec.action) {
    case 'tab-next':
      cycleTab(1)
      break
    case 'tab-prev':
      cycleTab(-1)
      break
    case 'tab-index': {
      const list = openTabs.value
      const target = spec.index === 9 ? list[list.length - 1] : list[(spec.index || 1) - 1]
      if (target && target !== activeId.value) void tabs.activate(target)
      break
    }
    case 'tab-close':
      if (activeId.value) void tabs.closeTab(activeId.value)
      break
    case 'tab-new':
      openCreateDialog()
      break
    case 'address-bar':
      void focusAddressBar()
      break
    case 'reload':
      void browserAction('reload')
      break
    case 'back':
      void browserAction('back')
      break
    case 'forward':
      void browserAction('forward')
      break
    case 'downloads':
      downloadsVisible.value = true
      break
  }
}

function onHostKeydown(event: KeyboardEvent) {
  const combo = comboFromEvent(event)
  if (!combo) return
  const spec = findShortcut(combo)
  if (!spec) return
  event.preventDefault()
  runShortcut(spec)
}

async function goHome() {
  const profile = activeId.value ? accounts.profileById(activeId.value) : undefined
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

async function syncBounds() {
  const bounds = viewport.value?.getBounds()
  if (!bounds) return
  await invoke('sync_profile_bounds', { bounds, activeId: activeId.value }).catch(() => undefined)
}

/** 拖动窗口大小时 ResizeObserver 会高频触发，合并成停止变化后的一次 IPC。 */
function scheduleSyncBounds() {
  if (syncBoundsTimer !== undefined) return
  syncBoundsTimer = setTimeout(() => {
    syncBoundsTimer = undefined
    void syncBounds()
  }, 80)
}

/**
 * 注册事件监听，失败只让出这一个事件而不抛错。
 *
 * 调用点一律用 Promise.all 并行注册：串行 await 时任意一个监听注册失败（或被
 * 阻塞），后面全部跟着不执行 —— 键盘入口、自动刷新、原生视图边界同步会静默失效。
 */
function listenOrNone<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn | undefined> {
  return listen<T>(event, (e) => handler(e.payload)).catch(() => undefined)
}

onMounted(async () => {
  registerSuppressionHandler((suppress) => {
    if (!activeId.value) return
    void invoke('set_profile_webview_visible', { id: activeId.value, visible: !suppress }).catch(
      () => undefined,
    )
  })
  const loaded = await loadDirectoryData()
  if (loaded) tabs.restoreSession()
  // 状态和下载记录独立加载；主页设置已经随目录账号加载，确保网站归类正确。
  await Promise.all([
    accounts.loadStatuses(),
    settings.loadDownloadHistory(),
  ])
  window.addEventListener('keydown', onHostKeydown)
  const [nav, blocked, guardBlocked, statusEvent, hotkey, download] = await Promise.all([
    listenOrNone<{ id: string; url: string }>('profile:navigated', ({ id, url }) => {
      const p = profiles.value.find((item) => item.id === id)
      if (p) p.last_url = url
      if (activeId.value === id) currentUrl.value = url
    }),
    listenOrNone<string>('profile:download-blocked', (fileName) => {
      ElMessage.warning(`已阻止重复下载：${fileName}`)
    }),
    listenOrNone<{ seconds: number; queued: boolean }>('profile:download-guard-blocked', (event) => {
      ElMessage.warning(event.queued ? `下载处于保护期，已加入队列，将在约 ${event.seconds} 秒后自动继续` : `页面刚打开，已阻止下载；请等待约 ${event.seconds} 秒`)
    }),
    listenOrNone<ProfileStatusEventPayload>('profile:status', (payload) => {
      accounts.applyStatusEvent(payload)
    }),
    listenOrNone<{ id: string; combo: string }>('profile:hotkey', (event) => {
      const spec = findShortcut(event.combo)
      if (spec) runShortcut(spec)
    }),
    listenOrNone<DownloadEntry>('profile:download', (entry) => {
      settings.pushDownloadEntry(entry)
      if (entry.success) {
        ElMessage.success(`下载完成：${entry.file_name}`)
      } else {
        ElMessage.error(`下载失败：${entry.file_name}`)
      }
    }),
  ])
  unlistenNavigation = nav
  unlistenDownloadBlocked = blocked
  unlistenDownloadGuardBlocked = guardBlocked
  unlistenStatus = statusEvent
  unlistenHotkey = hotkey
  unlistenDownload = download

  if (activeId.value) {
    const restored = activeId.value
    activeId.value = null
    await nextTick()
    await tabs.activate(restored)
  }
  startAutoRefresh()
  tabs.startLifecycle()

  resizeObserver = new ResizeObserver(() => scheduleSyncBounds())
  const el = viewport.value?.element()
  if (el) resizeObserver.observe(el)
  appReady.value = true
  if (websiteDirectoryVisible.value) void invoke('focus_main_ui').catch(() => undefined)
})

onBeforeUnmount(() => {
  registerSuppressionHandler(null)
  unlistenNavigation?.()
  unlistenDownloadBlocked?.()
  unlistenDownloadGuardBlocked?.()
  unlistenStatus?.()
  unlistenDownload?.()
  unlistenHotkey?.()
  window.removeEventListener('keydown', onHostKeydown)
  resizeObserver?.disconnect()
  stopAutoRefresh()
  tabs.stopLifecycle()
  tabs.persist()
  if (syncBoundsTimer !== undefined) clearTimeout(syncBoundsTimer)
})
</script>

<template>
  <div class="app-shell">
    <TitleBar />
    <div class="app-body">
      <div class="browser-shell" :inert="websiteDirectoryVisible" :aria-hidden="websiteDirectoryVisible || undefined">
      <ProfileSidebar
        :profiles="visibleProfiles"
        :total="profiles.length"
        :active-id="activeId"
        :open-tabs="openTabs"
        :collapsed="profileSidebarCollapsed"
        :all-tags="allTags"
        :batch-busy="batchBusy"
        v-model:search="searchQuery"
        v-model:view-mode="viewMode"
        v-model:active-tag="activeTag"
        @toggle="toggleProfileSidebar"
        @open-create="openCreateDialog"
        @open-tools="accountToolsVisible = true"
        @open-settings="accounts.openEditDialog"
        @activate="tabs.activate"
        @delete="accounts.deleteProfile"
        @clear="accounts.clearProfile"
        @reorder="accounts.reorderProfiles"
        @move-group="accounts.moveProfileToGroup"
        @clone="accounts.cloneProfile"
        @test-proxy="testProxy"
        @toggle-favorite="accounts.toggleFavorite"
        @batch="(ids, patch) => accounts.updateBatch(ids, patch, `已修改 ${ids.length} 个账号`)"
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
        :show-ai-usage-time="uiPreferences.showAiUsageTime"
        @toggle="toggleStatusSidebar"
        @activate="tabs.activate"
        @refresh="accounts.refreshStatuses"
        @save-settings="saveSettings"
        @open-downloads="downloadsVisible = true"
      />
      <main class="main-pane">
        <TabStrip
          :profiles="profiles"
          :open-tabs="openTabs"
          :hibernated="hibernatedTabs"
          :pinned="pinnedTabs"
          :width="uiPreferences.tabWidth"
          :suspended="websiteDirectoryVisible"
          :active-id="activeId"
          :statuses="statuses"
          :show-ai-status="uiPreferences.showTabAiStatus"
          @activate="tabs.activate"
          @close="tabs.closeTab"
          @reorder="tabs.reorder"
          @pin="tabs.togglePin"
          @create="openCreateDialog"
        />
        <AddressBar
          ref="addressBarRef"
          v-model="currentUrl"
          :disabled="!activeId"
          @navigate="navigate"
          @back="browserAction('back')"
          @forward="browserAction('forward')"
          @reload="browserAction('reload')"
          @account-overview="openAccountOverview"
          @directory="websiteDirectoryVisible = true"
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
      </div>
      <WebsiteDirectory
        v-if="websiteDirectoryVisible"
        ref="websiteDirectoryRef"
        :profiles="profiles"
        :global-default-url="dlSettings?.global_default_url || 'https://chat01.ai/'"
        :active-id="activeId"
        :open-tabs="openTabs"
        :loading="directoryLoading || !appReady"
        :error="directoryLoadError"
        :opening-id="directoryOpeningId"
        @activate="activateDirectoryProfile"
        @create="openCreateDialog"
        @edit="accounts.openEditDialog"
        @close="returnToBrowser"
        @retry="loadDirectoryData"
        @tools="accountToolsVisible = true"
        @generator="openAccountGenerator"
        @settings="appSettingsVisible = true"
      />
    </div>
    <AccountOverviewDialog
      v-model="accountOverviewVisible"
      :profiles="profiles"
      :statuses="statuses"
      :history="downloadHistory"
      :active-id="activeId"
      :open-tabs="openTabs"
      @activate="tabs.activate"
      @refresh="accounts.refreshStatuses"
    />
    <ProfileSettingsDialog
      v-model="editorVisible"
      :profile="editingProfile"
      :groups="profileGroups"
      :known-tags="allTags"
      :saving="profileSaving"
      :initial-url="createInitialUrl"
      :global-default-url="dlSettings?.global_default_url || 'https://chat01.ai/'"
      @create="createProfileFromEditor"
      @update="accounts.updateProfile"
    />
    <AccountToolsDialog
      v-model="accountToolsVisible"
      :profiles="profiles"
      :app-settings="dlSettings"
      @changed="accounts.loadProfiles"
      @generator="openAccountGenerator()"
    />
    <AccountGeneratorDialog
      v-model="accountGeneratorVisible"
      :profiles="profiles"
      :initial-url="generatorInitialUrl"
      :global-default-url="dlSettings?.global_default_url || 'https://chat01.ai/'"
      @created="acceptGeneratedProfiles"
      @refreshed="profiles = $event"
      @browse="browseGeneratedProfiles"
    />
    <ProfileDiagnosisDialog
      v-model="diagnosisVisible"
      :profile="diagnosisProfile"
      :error="diagnosisError"
      @changed="accounts.loadProfiles"
      @retry="activateDirectoryProfile"
    />
    <DownloadsDialog
      v-model="downloadsVisible"
      :profiles="profiles"
      :history="downloadHistory"
      :settings="dlSettings"
      :ui-preferences="uiPreferences"
      @cleared="downloadHistory = []"
      @changed="settings.loadDownloadHistory"
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
