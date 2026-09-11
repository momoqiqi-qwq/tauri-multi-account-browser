<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ElMessage } from 'element-plus'
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
import { useProfiles } from './composables/useProfiles'
import { useSettings } from './composables/useSettings'
import { useTabs } from './composables/useTabs'
import { isSuppressed, registerSuppressionHandler } from './lib/suppress'
import { applyUiPreferences, loadUiPreferences, saveUiPreferences } from './lib/uiPreferences'
import { normalizeProfileUrl } from './lib/profileUrl'
import type {
  AppSettings,
  CustomToolbarButton,
  DownloadEntry,
  Profile,
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
const downloadsVisible = ref(false)
const appSettingsVisible = ref(false)
const accountOverviewVisible = ref(false)
const accountToolsVisible = ref(false)
const diagnosisVisible = ref(false)
const diagnosisProfile = ref<Profile | null>(null)
const diagnosisError = ref('')
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
  openedAt: tabOpenedAt,
  runtimeSeconds: tabRuntimeSeconds,
  runtimePaused: tabRuntimePaused,
  hibernated: hibernatedTabs,
  pinned: pinnedTabs,
} = tabs

const accounts = useProfiles({ profiles, tabs })
const {
  statuses,
  searchQuery,
  filteredProfiles,
  profileGroups,
  editingProfile,
  editorVisible,
  profileSaving,
} = accounts

let unlistenNavigation: UnlistenFn | undefined
let unlistenStatus: UnlistenFn | undefined
let unlistenDownload: UnlistenFn | undefined
let unlistenDownloadBlocked: UnlistenFn | undefined
let unlistenDownloadGuardBlocked: UnlistenFn | undefined
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

onMounted(async () => {
  registerSuppressionHandler((suppress) => {
    if (!activeId.value) return
    void invoke('set_profile_webview_visible', { id: activeId.value, visible: !suppress }).catch(
      () => undefined,
    )
  })
  await accounts.loadProfiles()
  tabs.restoreSession()
  // 这三项互不依赖，并行加载可明显减少账号较多/历史较长时的首屏等待。
  await Promise.all([
    accounts.loadStatuses(),
    settings.loadSettings(),
    settings.loadDownloadHistory(),
  ])
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
    accounts.applyStatusEvent(event.payload)
  })
  unlistenDownload = await listen<DownloadEntry>('profile:download', (event) => {
    const entry = event.payload
    settings.pushDownloadEntry(entry)
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
    await tabs.activate(restored)
  }
  startAutoRefresh()
  tabs.startLifecycle()

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
  tabs.stopLifecycle()
  tabs.persist()
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
      @open-create="accounts.openCreateDialog"
      @open-tools="accountToolsVisible = true"
      @open-settings="accounts.openEditDialog"
      @activate="tabs.activate"
      @delete="accounts.deleteProfile"
      @clear="accounts.clearProfile"
      @reorder="accounts.reorderProfiles"
      @move-group="accounts.moveProfileToGroup"
      @clone="accounts.cloneProfile"
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
      @activate="tabs.activate"
      @refresh="accounts.refreshStatuses"
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
        @activate="tabs.activate"
        @close="tabs.closeTab"
        @reorder="tabs.reorder"
        @pin="tabs.togglePin"
        @pause-runtime="tabs.toggleRuntimePause"
        @reset-runtime="tabs.resetRuntime"
        @create="accounts.openCreateDialog"
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
        <BrowserViewport ref="viewport" :empty="!activeId" @create="accounts.openCreateDialog" />
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
      @activate="tabs.activate"
      @refresh="accounts.refreshStatuses"
    />
    <ProfileSettingsDialog
      v-model="editorVisible"
      :profile="editingProfile"
      :groups="profileGroups"
      :saving="profileSaving"
      :global-default-url="dlSettings?.global_default_url || 'https://chat01.ai/'"
      @create="accounts.createProfile"
      @update="accounts.updateProfile"
    />
    <AccountToolsDialog
      v-model="accountToolsVisible"
      :profiles="profiles"
      :app-settings="dlSettings"
      @changed="accounts.loadProfiles"
    />
    <ProfileDiagnosisDialog
      v-model="diagnosisVisible"
      :profile="diagnosisProfile"
      :error="diagnosisError"
      @changed="accounts.loadProfiles"
      @retry="tabs.activate"
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
