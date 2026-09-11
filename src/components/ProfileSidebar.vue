<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Plus,
  MoreFilled,
  Delete,
  Brush,
  Setting,
  ArrowLeft,
  ArrowRight,
  Search,
  CaretRight,
  CopyDocument,
  Connection,
  Promotion,
  Tools,
  Star,
  StarFilled,
  Operation,
} from '@element-plus/icons-vue'
import ProfileBatchBar from './ProfileBatchBar.vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { Profile, ProfileBatchPatch, ProfileViewMode } from '../types'

const props = defineProps<{
  profiles: Profile[]
  total: number
  activeId: string | null
  openTabs: string[]
  collapsed: boolean
  search: string
  /** 侧边栏视图模式（全部 / 收藏 / 最近使用） */
  viewMode: ProfileViewMode
  /** 当前选中的标签，null 表示不按标签过滤 */
  activeTag: string | null
  /** 所有账号上出现过的标签，用于标签筛选条 */
  allTags: string[]
  /** 批量修改进行中（由父组件的 useProfiles 提供） */
  batchBusy: boolean
}>()

const emit = defineEmits<{
  openCreate: []
  openTools: []
  openSettings: [profile: Profile]
  activate: [id: string]
  delete: [profile: Profile]
  clear: [profile: Profile]
  reorder: [ids: string[]]
  moveGroup: [id: string, group: string, ids: string[]]
  toggle: []
  clone: [profile: Profile]
  testProxy: [profile: Profile]
  toggleFavorite: [profile: Profile]
  batch: [ids: string[], patch: ProfileBatchPatch]
  'update:search': [value: string]
  'update:viewMode': [value: ProfileViewMode]
  'update:activeTag': [value: string | null]
}>()

const searchModel = computed({
  get: () => props.search,
  set: (value: string) => emit('update:search', value),
})

const viewModel = computed({
  get: () => props.viewMode,
  set: (value: ProfileViewMode) => emit('update:viewMode', value),
})

function toggleTag(tag: string) {
  emit('update:activeTag', props.activeTag === tag ? null : tag)
}

// ---- 批量选择 ----
const selectMode = ref(false)
const selected = ref<string[]>([])

function toggleSelectMode() {
  selectMode.value = !selectMode.value
  selected.value = []
}
function isSelected(id: string) {
  return selected.value.includes(id)
}
function toggleSelected(id: string) {
  selected.value = isSelected(id)
    ? selected.value.filter((item) => item !== id)
    : [...selected.value, id]
}
function selectVisible() {
  selected.value = props.profiles.map((p) => p.id)
}
function applyBatch(patch: ProfileBatchPatch) {
  if (!selected.value.length) return
  const ids = [...selected.value]
  // 清空选择但保留选择模式，方便连着做下一个批量操作。
  // 失败时父组件会提示，Rust 侧是原子提交，不存在改了一半的情况。
  selected.value = []
  emit('batch', ids, patch)
}

let releaseMenu: (() => void) | null = null
function onMenuVisible(open: boolean) {
  if (open && !releaseMenu) {
    releaseMenu = acquireWebviewSuppression()
  } else if (!open && releaseMenu) {
    releaseMenu()
    releaseMenu = null
  }
}

let draggedId: string | null = null
let draggedGroup: string | null = null
const dragOverGroup = ref<string | null>(null)

function isolationSummary(profile: Profile): string {
  const mode = profile.incognito ? '临时账号' : '持久账号'
  const net = profile.proxy ? '独立代理' : '直连'
  return `${mode} · ${net}`
}

function onProfileDragStart(event: DragEvent, id: string) {
  draggedId = id
  draggedGroup = null
  event.dataTransfer?.setData('text/plain', id)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function onProfileDragEnd() {
  draggedId = null
  dragOverGroup.value = null
}

function onGroupDragEnd() {
  draggedGroup = null
  dragOverGroup.value = null
}

function orderedIdsMovingProfile(targetId: string): string[] {
  const ids = props.profiles.map((p) => p.id)
  if (!draggedId || draggedId === targetId) return ids
  const from = ids.indexOf(draggedId)
  const to = ids.indexOf(targetId)
  if (from < 0 || to < 0) return ids
  const [moved] = ids.splice(from, 1)
  const targetAfterRemoval = ids.indexOf(targetId)
  ids.splice(targetAfterRemoval, 0, moved)
  return ids
}

function onProfileDrop(target: Profile) {
  if (!draggedId || draggedId === target.id) return
  const ids = orderedIdsMovingProfile(target.id)
  const source = props.profiles.find((p) => p.id === draggedId)
  const movingId = draggedId
  draggedId = null
  dragOverGroup.value = null
  if (source && source.group !== target.group) {
    emit('moveGroup', movingId, target.group || '', ids)
  } else {
    emit('reorder', ids)
  }
}

function onDropOnGroup(groupName: string) {
  if (!draggedId) return
  const ids = props.profiles.map((p) => p.id)
  const movingId = draggedId
  const from = ids.indexOf(movingId)
  if (from < 0) return
  const [moved] = ids.splice(from, 1)
  const targetItems = props.profiles.filter((p) => p.group === groupName && p.id !== movingId)
  if (targetItems.length) {
    const lastTargetId = targetItems[targetItems.length - 1].id
    const insertAt = ids.indexOf(lastTargetId) + 1
    ids.splice(insertAt, 0, moved)
  } else {
    ids.push(moved)
  }
  draggedId = null
  dragOverGroup.value = null
  emit('moveGroup', movingId, groupName, ids)
}

function onGroupDragStart(event: DragEvent, name: string) {
  draggedGroup = name
  draggedId = null
  event.dataTransfer?.setData('text/plain', `group:${name}`)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function onGroupDrop(targetName: string) {
  dragOverGroup.value = null
  if (draggedId) {
    onDropOnGroup(targetName)
    return
  }
  if (draggedGroup === null || draggedGroup === targetName) return

  const groupOrder = groups.value.map((g) => g.name)
  const from = groupOrder.indexOf(draggedGroup)
  const to = groupOrder.indexOf(targetName)
  if (from < 0 || to < 0) return
  const [movedGroup] = groupOrder.splice(from, 1)
  groupOrder.splice(groupOrder.indexOf(targetName), 0, movedGroup)
  const ids = groupOrder.flatMap((name) => groups.value.find((g) => g.name === name)?.items.map((p) => p.id) ?? [])
  draggedGroup = null
  emit('reorder', ids)
}

/** 搜索 / 收藏 / 最近 / 标签筛选下都摊平成一个列表：
 *  此时账号已不按完整顺序出现，按分组折叠会让人误以为"某些账号消失了"。 */
const flattened = computed(
  () =>
    !!props.search.trim() || props.viewMode !== 'all' || !!props.activeTag,
)

/** 按账号顺序形成分组；分组顺序也跟随账号顺序，因此可通过拖拽整个分组调整。 */
const groups = computed(() => {
  if (flattened.value) return [{ name: '', items: props.profiles }]
  const buckets: { name: string; items: Profile[] }[] = []
  for (const profile of props.profiles) {
    const name = profile.group || ''
    let bucket = buckets.find((b) => b.name === name)
    if (!bucket) {
      bucket = { name, items: [] }
      buckets.push(bucket)
    }
    bucket.items.push(profile)
  }
  return buckets
})

function loadCollapsedGroups(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem('mab-collapsed-groups') ?? '[]')
    return new Set(Array.isArray(raw) ? raw.filter((item): item is string => typeof item === 'string') : [])
  } catch {
    return new Set()
  }
}

const collapsedGroups = ref(loadCollapsedGroups())

function groupStorageKey(name: string) {
  return name || '__ungrouped__'
}

function isGroupCollapsed(name: string): boolean {
  return collapsedGroups.value.has(groupStorageKey(name))
}

function toggleGroup(name: string) {
  const key = groupStorageKey(name)
  const next = new Set(collapsedGroups.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  collapsedGroups.value = next
  localStorage.setItem('mab-collapsed-groups', JSON.stringify([...next]))
}

const iconCache = ref<Record<string, string>>({})
const failedIconIds = ref(new Set<string>())
const pendingIconIds = new Set<string>()

async function ensureIcon(profile: Profile) {
  if (iconCache.value[profile.id] || failedIconIds.value.has(profile.id) || pendingIconIds.has(profile.id)) return
  pendingIconIds.add(profile.id)
  try {
    const dataUrl = await invoke<string | null>('get_profile_icon', { id: profile.id })
    if (dataUrl) iconCache.value = { ...iconCache.value, [profile.id]: dataUrl }
    else failedIconIds.value = new Set([...failedIconIds.value, profile.id])
  } catch {
    failedIconIds.value = new Set([...failedIconIds.value, profile.id])
  } finally {
    pendingIconIds.delete(profile.id)
  }
}

watch(() => props.profiles, (list) => list.forEach(ensureIcon), { immediate: true })

function iconFor(profile: Profile): string | null {
  return iconCache.value[profile.id] ?? null
}
</script>

<template>
  <aside class="sidebar" :class="{ collapsed }">
    <template v-if="!collapsed">
      <div class="sidebar-title">
        <div>
          <strong>账号容器</strong>
          <small>{{ total }} 个账号 · 可拖动账号和分组</small>
        </div>
        <div class="sidebar-title-actions">
          <el-button circle size="small" :icon="Tools" title="账号管理工具" @click="emit('openTools')" />
          <el-button circle size="small" :icon="Plus" title="新建账号" @click="emit('openCreate')" />
          <el-button circle size="small" :icon="ArrowLeft" title="收起侧边栏" @click="emit('toggle')" />
        </div>
      </div>

      <el-input v-model="searchModel" class="sidebar-search" size="small" placeholder="搜索账号 / 分组" :prefix-icon="Search" clearable />

      <div class="view-filter">
        <el-radio-group v-model="viewModel" size="small">
          <el-radio-button value="all">全部</el-radio-button>
          <el-radio-button value="favorite">收藏</el-radio-button>
          <el-radio-button value="recent">最近</el-radio-button>
        </el-radio-group>
        <el-button
          size="small"
          :type="selectMode ? 'primary' : 'default'"
          :icon="Operation"
          title="批量选择账号"
          @click="toggleSelectMode"
        >
          批量
        </el-button>
      </div>

      <div v-if="allTags.length" class="tag-filter">
        <el-tag
          v-for="tag in allTags"
          :key="tag"
          size="small"
          :effect="activeTag === tag ? 'dark' : 'plain'"
          class="tag-chip"
          @click="toggleTag(tag)"
        >
          {{ tag }}
        </el-tag>
      </div>

      <div v-if="selectMode" class="select-hint">
        <el-button link type="primary" size="small" @click="selectVisible">全选当前列表（{{ profiles.length }}）</el-button>
      </div>

      <div class="profile-list">
        <template v-for="group in groups" :key="group.name">
          <div
            v-if="!flattened"
            class="group-header"
            :class="{ collapsed: isGroupCollapsed(group.name), 'drag-over': dragOverGroup === group.name }"
            draggable="true"
            @dragstart="onGroupDragStart($event, group.name)"
            @dragend="onGroupDragEnd"
            @dragover.prevent="dragOverGroup = group.name"
            @dragleave="dragOverGroup = null"
            @drop.prevent="onGroupDrop(group.name)"
            @click="toggleGroup(group.name)"
          >
            <el-icon class="group-chevron"><CaretRight /></el-icon>
            <span class="group-name">{{ group.name || '未分组' }}</span>
            <small>{{ group.items.length }}</small>
          </div>

          <template v-if="flattened || !isGroupCollapsed(group.name)">
            <el-dropdown
              v-for="profile in group.items"
              :key="profile.id"
              trigger="contextmenu"
              placement="bottom-start"
              @visible-change="onMenuVisible"
            >
              <div
                class="profile-card"
                :class="{ active: profile.id === activeId }"
                :draggable="!flattened"
                @dragstart="onProfileDragStart($event, profile.id)"
                @dragend="onProfileDragEnd"
                @dragover.prevent
                @drop.prevent="onProfileDrop(profile)"
                @click="selectMode ? toggleSelected(profile.id) : emit('activate', profile.id)"
              >
                <el-checkbox
                  v-if="selectMode"
                  class="profile-check"
                  :model-value="isSelected(profile.id)"
                  @click.stop
                  @change="toggleSelected(profile.id)"
                />
                <div class="avatar" :title="profile.default_url">
                  <img v-if="iconFor(profile)" class="avatar-icon" :src="iconFor(profile) ?? ''" :alt="profile.name" />
                  <span v-else>{{ profile.name.slice(0, 1).toUpperCase() }}</span>
                </div>
                <div class="profile-copy">
                  <div class="profile-name">
                    <span>{{ profile.name }}</span>
                    <i class="status-dot" :class="{ live: openTabs.includes(profile.id) }" :title="openTabs.includes(profile.id) ? '已作为标签页打开' : '未打开'" />
                  </div>
                  <small>{{ isolationSummary(profile) }}</small>
                  <div v-if="profile.tags?.length" class="profile-tags">
                    <el-tag v-for="tag in profile.tags" :key="tag" size="small" effect="plain">{{ tag }}</el-tag>
                  </div>
                </div>
                <el-button
                  text
                  circle
                  class="fav-btn"
                  :icon="profile.favorite ? StarFilled : Star"
                  :class="{ on: profile.favorite }"
                  :title="profile.favorite ? '取消收藏' : '收藏'"
                  @click.stop="emit('toggleFavorite', profile)"
                />
                <el-dropdown trigger="click" @click.stop @visible-change="onMenuVisible">
                  <el-button text circle :icon="MoreFilled" @click.stop />
                  <template #dropdown>
                    <el-dropdown-menu>
                      <el-dropdown-item :icon="Setting" @click="emit('openSettings', profile)">隔离设置…</el-dropdown-item>
                      <el-dropdown-item :icon="Connection" @click="emit('testProxy', profile)">检测代理出口</el-dropdown-item>
                      <el-dropdown-item :icon="CopyDocument" @click="emit('clone', profile)">克隆账号</el-dropdown-item>
                      <el-dropdown-item :icon="Brush" @click="emit('clear', profile)">清除数据</el-dropdown-item>
                      <el-dropdown-item divided :icon="Delete" @click="emit('delete', profile)">删除</el-dropdown-item>
                    </el-dropdown-menu>
                  </template>
                </el-dropdown>
              </div>
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item :icon="Promotion" @click="emit('activate', profile.id)">打开 / 切换到此账号</el-dropdown-item>
                  <el-dropdown-item :icon="Setting" @click="emit('openSettings', profile)">账号设置</el-dropdown-item>
                  <el-dropdown-item :icon="Connection" @click="emit('testProxy', profile)">检测代理出口</el-dropdown-item>
                  <el-dropdown-item :icon="CopyDocument" @click="emit('clone', profile)">克隆账号</el-dropdown-item>
                  <el-dropdown-item :icon="Brush" @click="emit('clear', profile)">清除账号数据</el-dropdown-item>
                  <el-dropdown-item divided :icon="Delete" @click="emit('delete', profile)">删除账号</el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </template>
        </template>
        <div v-if="profiles.length === 0" class="list-empty">没有匹配的账号</div>
      </div>

      <ProfileBatchBar
        v-if="selectMode"
        :selected="selected"
        :known-tags="allTags"
        :busy="batchBusy"
        @apply="applyBatch"
        @clear="selected = []"
      />

      <el-button class="new-profile" type="primary" :icon="Plus" @click="emit('openCreate')">新建账号</el-button>
    </template>

    <button v-else class="collapsed-toggle" title="展开侧边栏" @click="emit('toggle')">
      <el-icon><ArrowRight /></el-icon>
    </button>
  </aside>
</template>

<style scoped>
.view-filter {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 0 2px 8px;
}
.tag-filter {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 0 2px 8px;
}
.tag-chip {
  cursor: pointer;
  user-select: none;
}
.select-hint {
  padding: 0 2px 6px;
}
.profile-check {
  margin-right: 2px;
}
.profile-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 3px;
  margin-top: 3px;
}
.fav-btn {
  opacity: 0.45;
}
.fav-btn.on,
.profile-card:hover .fav-btn {
  opacity: 1;
}
.fav-btn.on {
  color: var(--el-color-warning);
}
</style>
