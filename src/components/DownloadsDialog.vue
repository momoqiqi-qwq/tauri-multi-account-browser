<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage, ElMessageBox } from 'element-plus'
import { FolderOpened, Delete, Rank, Search } from '@element-plus/icons-vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { AppSettings, DownloadEntry, Profile, UiPreferences } from '../types'

const visible = defineModel<boolean>({ required: true })

const props = defineProps<{
  profiles: Profile[]
  history: DownloadEntry[]
  settings: AppSettings | null
  uiPreferences: UiPreferences
}>()

const emit = defineEmits<{
  cleared: []
  changed: []
  updateUi: [preferences: UiPreferences]
}>()

let releaseDialog: (() => void) | null = null
watch(visible, (open) => {
  if (open) {
    releaseDialog?.()
    releaseDialog = acquireWebviewSuppression()
  } else {
    releaseDialog?.()
    releaseDialog = null
  }
})
onBeforeUnmount(() => {
  releaseDialog?.()
  releaseDialog = null
})

interface Group {
  key: string
  name: string
  icon: string
  deleted: boolean
  entries: DownloadEntry[]
}

const query = ref('')
const statusFilter = ref<'all' | 'success' | 'failed'>('all')
const currentPage = ref(1)
const selectedKeys = ref<string[]>([])

function entryKey(entry: DownloadEntry): string {
  return `${entry.id}\u0000${entry.finished_at}\u0000${entry.path}\u0000${entry.file_name}`
}

const filteredEntries = computed(() => {
  const q = query.value.trim().toLowerCase()
  return props.history.filter((entry) => {
    if (statusFilter.value === 'success' && !entry.success) return false
    if (statusFilter.value === 'failed' && entry.success) return false
    if (!q) return true
    return [entry.file_name, entry.profile_name, entry.path].some((value) => value.toLowerCase().includes(q))
  })
})

const pageSize = computed(() => props.uiPreferences.downloadRowsPerPage)
const totalPages = computed(() => Math.max(1, Math.ceil(filteredEntries.value.length / pageSize.value)))
const pageEntries = computed(() => {
  const start = (currentPage.value - 1) * pageSize.value
  return filteredEntries.value.slice(start, start + pageSize.value)
})

watch([filteredEntries, pageSize], () => {
  if (currentPage.value > totalPages.value) currentPage.value = totalPages.value
})
watch([query, statusFilter], () => {
  currentPage.value = 1
  // 筛选条件变化时清空选择，避免批量操作误包含当前看不见的记录。
  selectedKeys.value = []
})
watch(
  () => props.history,
  (history) => {
    const valid = new Set(history.map(entryKey))
    selectedKeys.value = selectedKeys.value.filter((key) => valid.has(key))
  },
  { deep: true },
)

function groupEntries(entries: DownloadEntry[]): Group[] {
  const byId = new Map<string, DownloadEntry[]>()
  for (const entry of entries) {
    const list = byId.get(entry.id) ?? []
    list.push(entry)
    byId.set(entry.id, list)
  }
  const result: Group[] = []
  for (const profile of props.profiles) {
    const grouped = byId.get(profile.id)
    if (!grouped?.length) continue
    result.push({ key: profile.id, name: profile.name, icon: profile.name.slice(0, 1).toUpperCase(), deleted: false, entries: grouped })
    byId.delete(profile.id)
  }
  for (const [id, grouped] of byId) {
    const name = grouped[0]?.profile_name || '未知账号'
    result.push({ key: id, name, icon: name.slice(0, 1).toUpperCase(), deleted: true, entries: grouped })
  }
  return result
}

const groups = computed(() => groupEntries(pageEntries.value))
const selectedSet = computed(() => new Set(selectedKeys.value))
const selectedEntries = computed(() => props.history.filter((entry) => selectedSet.value.has(entryKey(entry))))
const allFilteredSelected = computed(
  () => filteredEntries.value.length > 0 && filteredEntries.value.every((entry) => selectedSet.value.has(entryKey(entry))),
)
const someFilteredSelected = computed(
  () => filteredEntries.value.some((entry) => selectedSet.value.has(entryKey(entry))) && !allFilteredSelected.value,
)

function isSelected(entry: DownloadEntry) {
  return selectedSet.value.has(entryKey(entry))
}

function setEntrySelected(entry: DownloadEntry, checked: boolean) {
  const keys = new Set(selectedKeys.value)
  const key = entryKey(entry)
  if (checked) keys.add(key)
  else keys.delete(key)
  selectedKeys.value = [...keys]
}

function toggleAll(checked: boolean) {
  const keys = new Set(selectedKeys.value)
  for (const entry of filteredEntries.value) {
    if (checked) keys.add(entryKey(entry))
    else keys.delete(entryKey(entry))
  }
  selectedKeys.value = [...keys]
}

function groupSelectedState(group: Group) {
  const count = group.entries.filter((entry) => selectedSet.value.has(entryKey(entry))).length
  return { checked: count === group.entries.length && count > 0, indeterminate: count > 0 && count < group.entries.length }
}

function toggleGroup(group: Group, checked: boolean) {
  const keys = new Set(selectedKeys.value)
  for (const entry of group.entries) {
    if (checked) keys.add(entryKey(entry))
    else keys.delete(entryKey(entry))
  }
  selectedKeys.value = [...keys]
}

function updateRowsPerPage(value: number) {
  emit('updateUi', { ...props.uiPreferences, downloadRowsPerPage: value })
  currentPage.value = 1
}

function updateView(value: string | number | boolean) {
  const downloadView = value === 'flat' ? 'flat' : 'grouped'
  emit('updateUi', { ...props.uiPreferences, downloadView })
}

function formatSize(bytes: number): string {
  if (!bytes) return '—'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`
}

function formatTime(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  return date.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function fileIcon(fileName: string): string {
  const ext = (/\.([a-z0-9]+)$/i.exec(fileName)?.[1] ?? '').toLowerCase()
  if (['png', 'jpg', 'jpeg', 'webp', 'gif', 'svg'].includes(ext)) return '🖼️'
  if (['zip', 'rar', '7z', 'gz'].includes(ext)) return '🗜️'
  if (['csv', 'xlsx', 'xls'].includes(ext)) return '📊'
  if (ext === 'pdf') return '📕'
  if (['doc', 'docx'].includes(ext)) return '📘'
  if (['md', 'txt'].includes(ext)) return '📄'
  if (['html', 'htm'].includes(ext)) return '🌐'
  if (ext === 'json') return '🧾'
  return '📄'
}

async function reveal(path: string) {
  try {
    await invoke('reveal_download', { path })
  } catch (e) {
    ElMessage.error(`${e}`)
  }
}

async function removeEntry(entry: DownloadEntry) {
  if (props.settings?.confirm_delete_download !== false) {
    try {
      await ElMessageBox.confirm(
        `确定删除“${entry.file_name}”吗？文件本体和这条下载记录都会删除。`,
        '删除下载文件',
        { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
      )
    } catch {
      return
    }
  }
  try {
    if (entry.path) await invoke('delete_download', { path: entry.path })
    else await invoke('remove_download_history_entries', { entries: [entry] })
    selectedKeys.value = selectedKeys.value.filter((key) => key !== entryKey(entry))
    emit('changed')
    ElMessage.success(entry.path ? '文件已删除' : '下载记录已移除')
  } catch (e) {
    ElMessage.error(`删除失败：${e}`)
  }
}

async function moveEntry(entry: DownloadEntry) {
  const parent = entry.path.replace(/[\\/][^\\/]+$/, '')
  let destinationDir: string | null
  try {
    destinationDir = await invoke<string | null>('pick_move_directory', { initialDir: parent })
  } catch (e) {
    ElMessage.error(`打开文件夹选择器失败：${e}`)
    return
  }
  if (!destinationDir) return
  try {
    await invoke<string>('move_download', { path: entry.path, destinationDir })
    emit('changed')
    ElMessage.success('文件已移动')
  } catch (e) {
    ElMessage.error(`移动失败：${e}`)
  }
}

async function batchMove() {
  const entries = selectedEntries.value.filter((entry) => entry.success && entry.path)
  if (!entries.length) {
    ElMessage.warning('选中的记录里没有可移动的文件')
    return
  }
  const parent = entries[0].path.replace(/[\\/][^\\/]+$/, '')
  let destinationDir: string | null
  try {
    destinationDir = await invoke<string | null>('pick_move_directory', { initialDir: parent })
  } catch (e) {
    ElMessage.error(`打开文件夹选择器失败：${e}`)
    return
  }
  if (!destinationDir) return

  let moved = 0
  let failed = 0
  for (const entry of entries) {
    try {
      await invoke('move_download', { path: entry.path, destinationDir })
      moved += 1
    } catch {
      failed += 1
    }
  }
  selectedKeys.value = []
  emit('changed')
  if (failed) ElMessage.warning(`已移动 ${moved} 个，${failed} 个失败`)
  else ElMessage.success(`已移动 ${moved} 个文件`)
}

async function batchRemoveHistory() {
  const entries = selectedEntries.value
  if (!entries.length) return
  try {
    await ElMessageBox.confirm(
      `从下载历史中移除选中的 ${entries.length} 条记录吗？已下载的文件会保留。`,
      '移除历史记录',
      { type: 'warning', confirmButtonText: '仅移除记录', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    await invoke('remove_download_history_entries', { entries })
    selectedKeys.value = []
    emit('changed')
    ElMessage.success(`已移除 ${entries.length} 条历史记录，文件未删除`)
  } catch (e) {
    ElMessage.error(`移除失败：${e}`)
  }
}

async function batchDelete() {
  const entries = selectedEntries.value
  if (!entries.length) return
  if (props.settings?.confirm_delete_download !== false) {
    try {
      await ElMessageBox.confirm(
        `确定处理选中的 ${entries.length} 条记录吗？存在的下载文件会一并删除，操作不可恢复。`,
        '批量删除',
        { type: 'warning', confirmButtonText: '删除选中', cancelButtonText: '取消' },
      )
    } catch {
      return
    }
  }

  const historyOnly: DownloadEntry[] = []
  let deleted = 0
  let failed = 0
  for (const entry of entries) {
    if (!entry.path) {
      historyOnly.push(entry)
      continue
    }
    try {
      await invoke('delete_download', { path: entry.path })
      deleted += 1
    } catch {
      failed += 1
    }
  }
  if (historyOnly.length) {
    try {
      await invoke('remove_download_history_entries', { entries: historyOnly })
      deleted += historyOnly.length
    } catch {
      failed += historyOnly.length
    }
  }
  selectedKeys.value = []
  emit('changed')
  if (failed) ElMessage.warning(`已处理 ${deleted} 条，${failed} 条失败`)
  else ElMessage.success(`已删除 ${deleted} 条记录`)
}

async function clearAll() {
  try {
    await ElMessageBox.confirm(
      `将清空全部 ${props.history.length} 条下载记录（不会删除已下载的文件）。`,
      '清空下载历史',
      { type: 'warning', confirmButtonText: '清空', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    await invoke('clear_download_history')
    selectedKeys.value = []
    emit('cleared')
    ElMessage.success('已清空下载历史')
  } catch (e) {
    ElMessage.error(`清空失败：${e}`)
  }
}
</script>

<template>
  <el-dialog v-model="visible" title="下载历史" width="min(920px, 92vw)" append-to-body class="downloads-dialog">
    <div v-if="history.length" class="dl-toolbar">
      <div class="dl-toolbar-main">
        <el-input v-model="query" clearable :prefix-icon="Search" placeholder="搜索文件名、账号或路径" class="dl-search" />
        <el-select v-model="statusFilter" class="dl-filter">
          <el-option label="全部状态" value="all" />
          <el-option label="仅成功" value="success" />
          <el-option label="仅失败" value="failed" />
        </el-select>
        <el-segmented
          :model-value="uiPreferences.downloadView"
          :options="[{ label: '分组', value: 'grouped' }, { label: '列表', value: 'flat' }]"
          @update:model-value="updateView($event)"
        />
      </div>
      <div class="dl-toolbar-secondary">
        <el-checkbox
          :model-value="allFilteredSelected"
          :indeterminate="someFilteredSelected"
          @update:model-value="toggleAll(Boolean($event))"
        >
          全选{{ query || statusFilter !== 'all' ? '筛选结果' : '全部' }}
        </el-checkbox>
        <span class="dl-selection-count">已选 {{ selectedEntries.length }} / {{ filteredEntries.length }}</span>
        <el-select
          :model-value="pageSize"
          class="dl-page-size"
          @update:model-value="updateRowsPerPage(Number($event))"
        >
          <el-option :value="10" label="10 行/页" />
          <el-option :value="20" label="20 行/页" />
          <el-option :value="50" label="50 行/页" />
          <el-option :value="100" label="100 行/页" />
        </el-select>
      </div>
    </div>

    <div v-if="filteredEntries.length" class="dl-groups">
      <template v-if="uiPreferences.downloadView === 'grouped'">
        <section v-for="group in groups" :key="group.key" class="dl-group">
          <header class="dl-group-head">
            <el-checkbox
              :model-value="groupSelectedState(group).checked"
              :indeterminate="groupSelectedState(group).indeterminate"
              aria-label="选择该账号当前页记录"
              @update:model-value="toggleGroup(group, Boolean($event))"
            />
            <span class="dl-group-avatar">{{ group.icon }}</span>
            <div class="dl-group-title">
              <strong>{{ group.name }}</strong>
              <small>{{ group.entries.length }} 个文件 · 当前页共 {{ formatSize(group.entries.reduce((sum, e) => sum + e.size, 0)) }}<template v-if="group.deleted"> · 账号已删除</template></small>
            </div>
          </header>
          <div class="dl-items">
            <div v-for="entry in group.entries" :key="entryKey(entry)" class="dl-item" :class="{ selected: isSelected(entry) }">
              <el-checkbox :model-value="isSelected(entry)" aria-label="选择下载记录" @update:model-value="setEntrySelected(entry, Boolean($event))" />
              <span class="dl-file-icon">{{ fileIcon(entry.file_name) }}</span>
              <div class="dl-file-copy">
                <span class="dl-file-name" :class="{ failed: !entry.success }" :title="entry.path">
                  {{ entry.file_name }}<template v-if="!entry.success"> · 失败</template>
                </span>
                <small class="dl-file-meta">{{ formatSize(entry.size) }} · {{ formatTime(entry.finished_at) }}</small>
              </div>
              <div class="dl-file-actions">
                <template v-if="entry.success && entry.path">
                  <el-button circle size="small" :icon="FolderOpened" title="打开所在文件夹" @click="reveal(entry.path)" />
                  <el-button circle size="small" :icon="Rank" title="移动文件" @click="moveEntry(entry)" />
                </template>
                <el-button circle size="small" type="danger" plain :icon="Delete" :title="entry.path ? '删除文件' : '移除记录'" @click="removeEntry(entry)" />
              </div>
            </div>
          </div>
        </section>
      </template>

      <section v-else class="dl-group dl-flat-group">
        <div class="dl-items">
          <div v-for="entry in pageEntries" :key="entryKey(entry)" class="dl-item" :class="{ selected: isSelected(entry) }">
            <el-checkbox :model-value="isSelected(entry)" aria-label="选择下载记录" @update:model-value="setEntrySelected(entry, Boolean($event))" />
            <span class="dl-file-icon">{{ fileIcon(entry.file_name) }}</span>
            <div class="dl-file-copy">
              <span class="dl-file-name" :class="{ failed: !entry.success }" :title="entry.path">{{ entry.file_name }}<template v-if="!entry.success"> · 失败</template></span>
              <small class="dl-file-meta">{{ entry.profile_name || '未知账号' }} · {{ formatSize(entry.size) }} · {{ formatTime(entry.finished_at) }}</small>
            </div>
            <div class="dl-file-actions">
              <template v-if="entry.success && entry.path">
                <el-button circle size="small" :icon="FolderOpened" title="打开所在文件夹" @click="reveal(entry.path)" />
                <el-button circle size="small" :icon="Rank" title="移动文件" @click="moveEntry(entry)" />
              </template>
              <el-button circle size="small" type="danger" plain :icon="Delete" :title="entry.path ? '删除文件' : '移除记录'" @click="removeEntry(entry)" />
            </div>
          </div>
        </div>
      </section>
    </div>
    <div v-else-if="history.length" class="dl-empty">没有符合当前筛选条件的下载记录</div>
    <div v-else class="dl-empty">还没有下载记录，去账号页面下载一个文件试试</div>

    <div v-if="filteredEntries.length > pageSize" class="dl-pagination">
      <el-pagination
        v-model:current-page="currentPage"
        :page-size="pageSize"
        :total="filteredEntries.length"
        layout="prev, pager, next"
        small
      />
    </div>

    <template #footer>
      <div class="dl-footer">
        <div class="dl-batch-actions">
          <el-button v-if="selectedEntries.length" size="small" @click="selectedKeys = []">取消选择</el-button>
          <el-button v-if="selectedEntries.length" size="small" :icon="Rank" @click="batchMove">移动选中</el-button>
          <el-button v-if="selectedEntries.length" size="small" @click="batchRemoveHistory">仅移除记录</el-button>
          <el-button v-if="selectedEntries.length" size="small" type="danger" plain :icon="Delete" @click="batchDelete">删除文件+记录 ({{ selectedEntries.length }})</el-button>
        </div>
        <div class="dl-footer-right">
          <el-button v-if="history.length" type="danger" plain size="small" :icon="Delete" @click="clearAll">清空历史</el-button>
          <el-button size="small" @click="visible = false">关闭</el-button>
        </div>
      </div>
    </template>
  </el-dialog>
</template>
