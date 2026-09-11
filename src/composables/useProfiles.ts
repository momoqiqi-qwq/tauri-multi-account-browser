import { computed, nextTick, ref, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage, ElMessageBox } from 'element-plus'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { TabsApi } from './useTabs'
import type {
  Profile,
  ProfileIsolationSettings,
  ProfileStatus,
  ProfileStatusEventPayload,
} from '../types'

export interface ProfilesDeps {
  /** 账号列表。由宿主持有，标签 composable 也要读它。 */
  profiles: Ref<Profile[]>
  /** 标签页 API：删除/清空账号时要同步前端标签状态 */
  tabs: TabsApi
}

export interface ProfilesApi {
  profiles: Ref<Profile[]>
  /** 各账号最近一次上报的页面状态（chat01.ai 积分 / 登录账号） */
  statuses: Ref<Record<string, ProfileStatus>>
  /** 账号搜索关键词，同时过滤两个侧边栏 */
  searchQuery: Ref<string>
  /** 按名称 / 分组过滤后的账号，两个侧边栏共用；空关键词返回全部。 */
  filteredProfiles: Ref<Profile[]>
  /** 新建/编辑账号时的分组下拉：按侧边栏出现顺序去重。 */
  profileGroups: Ref<string[]>
  /** 新建账号对话框：null 表示新建，非空表示编辑该账号 */
  editingProfile: Ref<Profile | null>
  editorVisible: Ref<boolean>
  profileSaving: Ref<boolean>
  loadProfiles(): Promise<void>
  loadStatuses(): Promise<void>
  refreshStatuses(): Promise<void>
  applyStatusEvent(payload: ProfileStatusEventPayload): void
  profileById(id: string): Profile | undefined
  openCreateDialog(): void
  openEditDialog(profile: Profile): void
  createProfile(settings: ProfileIsolationSettings): Promise<void>
  updateProfile(id: string, settings: ProfileIsolationSettings): Promise<void>
  deleteProfile(profile: Profile): Promise<void>
  clearProfile(profile: Profile): Promise<void>
  cloneProfile(profile: Profile): Promise<void>
  reorderProfiles(ids: string[]): Promise<void>
  moveProfileToGroup(id: string, group: string, ids: string[]): Promise<void>
}

/** 账号生命周期：列表加载、CRUD、搜索过滤，以及各账号的运行状态缓存。 */
export function useProfiles(deps: ProfilesDeps): ProfilesApi {
  const { profiles, tabs } = deps

  const statuses = ref<Record<string, ProfileStatus>>({})
  const searchQuery = ref('')
  const editingProfile = ref<Profile | null>(null)
  const editorVisible = ref(false)
  const profileSaving = ref(false)

  const filteredProfiles = computed(() => {
    const query = searchQuery.value.trim().toLowerCase()
    if (!query) return profiles.value
    return profiles.value.filter(
      (p) =>
        p.name.toLowerCase().includes(query) || (p.group && p.group.toLowerCase().includes(query)),
    )
  })

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

  async function loadProfiles() {
    profiles.value = await invoke<Profile[]>('list_profiles')
  }

  async function loadStatuses() {
    statuses.value = await invoke<Record<string, ProfileStatus>>('get_profile_statuses').catch(
      () => ({}),
    )
  }

  async function refreshStatuses() {
    await invoke('refresh_profile_statuses').catch(() => undefined)
  }

  function applyStatusEvent(payload: ProfileStatusEventPayload) {
    const { id, ...status } = payload
    statuses.value = { ...statuses.value, [id]: status }
  }

  function profileById(id: string): Profile | undefined {
    return profiles.value.find((p) => p.id === id)
  }

  function openCreateDialog() {
    editingProfile.value = null
    editorVisible.value = true
  }

  function openEditDialog(profile: Profile) {
    editingProfile.value = profile
    editorVisible.value = true
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
      editorVisible.value = false
      await nextTick()
      await tabs.activate(profile.id)
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
    const wasOpen = tabs.openTabs.value.includes(id)
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
      editorVisible.value = false
      if (result.needs_reopen) {
        // 隔离配置变更后 WebView 已被后端销毁，重新打开以应用新配置。
        tabs.openTabs.value = tabs.openTabs.value.filter((tab) => tab !== id)
        if (wasOpen) {
          await nextTick()
          const reopened = await tabs.activate(id)
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

  async function deleteProfile(profile: Profile) {
    const release = acquireWebviewSuppression()
    try {
      await ElMessageBox.confirm(
        `删除“${profile.name}”将同时删除该账号的浏览数据，且不可恢复。`,
        '确认删除',
        { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
      )
      tabs.openTabs.value = tabs.openTabs.value.filter((tab) => tab !== profile.id)
      tabs.pinned.value = tabs.pinned.value.filter((tab) => tab !== profile.id)
      tabs.hibernated.value = tabs.hibernated.value.filter((tab) => tab !== profile.id)
      tabs.persist()
      if (tabs.activeId.value === profile.id) {
        tabs.activeId.value = null
        tabs.currentUrl.value = ''
      }
      await invoke('delete_profile', { id: profile.id })
      await loadProfiles()
      // 后端已同步清理该账号的状态缓存，前端保持一致。
      const { [profile.id]: _removed, ...rest } = statuses.value
      statuses.value = rest
      if (!tabs.activeId.value && tabs.openTabs.value.length > 0) {
        await tabs.activate(tabs.openTabs.value[tabs.openTabs.value.length - 1])
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
      tabs.openTabs.value = tabs.openTabs.value.filter((tab) => tab !== profile.id)
      if (tabs.activeId.value === profile.id) {
        const fallback = tabs.openTabs.value[tabs.openTabs.value.length - 1] ?? null
        if (fallback) {
          await tabs.activate(fallback)
        } else {
          tabs.activeId.value = null
          tabs.currentUrl.value = ''
        }
      }
      ElMessage.success('已清除账号数据，重新打开标签页即为全新会话')
    } finally {
      release()
    }
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

  return {
    profiles,
    statuses,
    searchQuery,
    filteredProfiles,
    profileGroups,
    editingProfile,
    editorVisible,
    profileSaving,
    loadProfiles,
    loadStatuses,
    refreshStatuses,
    applyStatusEvent,
    profileById,
    openCreateDialog,
    openEditDialog,
    createProfile,
    updateProfile,
    deleteProfile,
    clearProfile,
    cloneProfile,
    reorderProfiles,
    moveProfileToGroup,
  }
}
