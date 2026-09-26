<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { Search, Refresh, Download, Loading, CircleCheck, Warning } from '@element-plus/icons-vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import { ALL_SITES, buildWebsiteGroups, countSiteFacets, indexWebsiteGroups } from '../lib/siteDirectory'
import type { DownloadEntry, Profile, ProfileStatus } from '../types'

const visible = defineModel<boolean>({ required: true })
const props = defineProps<{
  profiles: Profile[]
  statuses: Record<string, ProfileStatus>
  history: DownloadEntry[]
  activeId: string | null
  openTabs: string[]
  /** 继承模式账号的网站归属靠它解析，必须与网站导航用同一份真值 */
  globalDefaultUrl: string
}>()
const emit = defineEmits<{
  activate: [id: string]
  refresh: []
}>()

const query = ref('')
const category = ref<'all' | 'open' | 'ready' | 'generating' | 'closed' | 'downloaded'>('all')
/** 网站筛选；ALL_SITES 表示不按网站收窄 */
const siteFilter = ref<string>(ALL_SITES)

const categories = [
  { value: 'all', label: '全部' },
  { value: 'open', label: '已打开' },
  { value: 'ready', label: '回答完成' },
  { value: 'generating', label: '生成中' },
  { value: 'closed', label: '未打开' },
  { value: 'downloaded', label: '有下载' },
] as const
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
onBeforeUnmount(() => releaseDialog?.())

// 与网站导航共用同一套分组真值：按配置的主页归类，不会被登录跳转带跑。
const directory = computed(() => indexWebsiteGroups(buildWebsiteGroups(props.profiles, props.globalDefaultUrl)))
const siteFacets = computed(() => countSiteFacets(directory.value.values()))
// 账号被删除 / 改主页后，悬空的网站筛选要退回「全部网站」，否则列表会空着。
watch(siteFacets, (facets) => {
  if (siteFilter.value !== ALL_SITES && !facets.some((facet) => facet.key === siteFilter.value)) {
    siteFilter.value = ALL_SITES
  }
})

const latestDownloadByProfile = computed(() => {
  const map = new Map<string, DownloadEntry>()
  for (const entry of props.history) {
    if (!map.has(entry.id)) map.set(entry.id, entry)
  }
  return map
})

const rows = computed(() => {
  const q = query.value.trim().toLowerCase()
  return props.profiles
    .map((profile) => {
      const status = props.statuses[profile.id]
      const lastDownload = latestDownloadByProfile.value.get(profile.id) ?? null
      const site = directory.value.get(profile.id)?.site ?? null
      return {
        profile,
        status,
        lastDownload,
        site,
        siteFields: [site?.name, site?.key, site?.url].map((value) => String(value || '').toLowerCase()),
        open: props.openTabs.includes(profile.id),
        active: props.activeId === profile.id,
      }
    })
    .filter((row) => {
      if (siteFilter.value !== ALL_SITES && row.site?.key !== siteFilter.value) return false
      if (category.value === 'open' && !row.open) return false
      if (category.value === 'ready' && !row.status?.answer_ready) return false
      if (category.value === 'generating' && !row.status?.answer_generating) return false
      if (category.value === 'closed' && row.open) return false
      if (category.value === 'downloaded' && !row.lastDownload) return false
      if (!q) return true
      return [
        row.profile.name,
        row.profile.group,
        row.status?.email,
        row.status?.account,
        row.lastDownload?.file_name,
        ...row.siteFields,
      ].some((v) => String(v || '').toLowerCase().includes(q))
    })
})

function formatTime(iso: string | null | undefined): string {
  if (!iso) return '暂无'
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return '暂无'
  return date.toLocaleString('zh-CN', { hour12: false })
}

function activate(id: string) {
  emit('activate', id)
  visible.value = false
}
</script>

<template>
  <el-dialog v-model="visible" title="全部账号历史" width="min(980px, 94vw)" append-to-body class="account-overview-dialog">
    <div class="overview-toolbar">
      <el-input v-model="query" clearable :prefix-icon="Search" placeholder="搜索账号、分组、邮箱、网站或最近文件" />
      <el-button :icon="Refresh" @click="emit('refresh')">刷新状态</el-button>
    </div>
    <div class="overview-categories" aria-label="历史分类">
      <button
        v-for="item in categories"
        :key="item.value"
        type="button"
        class="overview-category"
        :class="{ active: category === item.value }"
        :aria-pressed="category === item.value"
        @click="category = item.value"
      >{{ item.label }}</button>
    </div>

    <div class="overview-sites" aria-label="按网站分类">
      <span class="overview-sites__label">网站</span>
      <button
        type="button"
        class="overview-category"
        :class="{ active: siteFilter === ALL_SITES }"
        :aria-pressed="siteFilter === ALL_SITES"
        @click="siteFilter = ALL_SITES"
      >全部网站<span class="overview-category__count">{{ profiles.length }}</span></button>
      <button
        v-for="facet in siteFacets"
        :key="facet.key"
        type="button"
        class="overview-category"
        :class="{ active: siteFilter === facet.key }"
        :aria-pressed="siteFilter === facet.key"
        :title="facet.key === '@unknown' ? '账号没有可解析的主页' : facet.key"
        @click="siteFilter = facet.key"
      >{{ facet.name }}<span class="overview-category__count">{{ facet.count }}</span></button>
    </div>

    <div class="account-overview-list">
      <button
        v-for="row in rows"
        :key="row.profile.id"
        type="button"
        class="account-overview-row"
        :class="{ active: row.active }"
        @click="activate(row.profile.id)"
      >
        <div class="overview-account-main">
          <span class="overview-avatar">{{ row.profile.name.slice(0, 1).toUpperCase() }}</span>
          <div class="overview-account-copy">
            <span class="overview-account-title">
              <strong>{{ row.profile.name }}</strong>
              <span v-if="row.site" class="overview-site-chip" :title="row.site.url || row.site.key">{{ row.site.name }}</span>
            </span>
            <small>{{ row.profile.group || '未分组' }} · {{ row.status?.email || row.status?.account || '未识别登录账号' }}</small>
          </div>
        </div>

        <div class="overview-state">
          <el-icon v-if="row.status?.answer_generating" class="overview-generating"><Loading /></el-icon>
          <el-icon v-else-if="row.status?.answer_ready" class="overview-ready"><CircleCheck /></el-icon>
          <el-icon v-else-if="row.open"><CircleCheck /></el-icon>
          <el-icon v-else class="overview-muted"><Warning /></el-icon>
          <span>{{ row.status?.answer_generating ? 'AI 使用中…' : row.status?.answer_ready ? '回答完成' : row.open ? '已打开' : '未打开' }}</span>
        </div>

        <div class="overview-metric">
          <small>积分</small>
          <strong>{{ row.status?.credits || '—' }}</strong>
        </div>

        <div class="overview-download">
          <div class="overview-download-title">
            <el-icon><Download /></el-icon>
            <span :title="row.lastDownload?.file_name || ''">{{ row.lastDownload?.file_name || '暂无下载' }}</span>
          </div>
          <small>{{ formatTime(row.lastDownload?.finished_at) }}</small>
        </div>

        <div class="overview-updated">
          <small>状态更新</small>
          <span>{{ formatTime(row.status?.updated_at) }}</span>
        </div>
      </button>
      <el-empty v-if="rows.length === 0" description="没有匹配的账号" />
    </div>

    <template #footer>
      <small class="overview-footer-hint">点击任意账号即可直接跳转到该账号。</small>
      <el-button @click="visible = false">关闭</el-button>
    </template>
  </el-dialog>
</template>
