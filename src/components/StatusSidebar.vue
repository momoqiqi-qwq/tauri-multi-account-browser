<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, watch } from 'vue'
import { Refresh, ArrowLeft, ArrowRight, Setting, Download } from '@element-plus/icons-vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { AppSettings, Profile, ProfileStatus } from '../types'

const props = defineProps<{
  profiles: Profile[]
  statuses: Record<string, ProfileStatus>
  activeId: string | null
  openTabs: string[]
  collapsed: boolean
  dlSettings: AppSettings | null
  downloadCount: number
  showAccount: boolean
  showProxy: boolean
  showUpdatedTime: boolean
}>()

const emit = defineEmits<{
  activate: [id: string]
  refresh: []
  toggle: []
  saveSettings: [settings: AppSettings]
  openDownloads: []
}>()

const rows = computed(() =>
  props.profiles.map((profile) => {
    const status = props.statuses[profile.id] ?? null
    // 谷歌账号的登录名就是邮箱，优先展示，昵称兜底。
    const account = status?.email || status?.account || ''
    const credits = status?.credits || ''
    const creditNum = parseFloat(credits)
    const lowCredits = credits !== '' && Number.isFinite(creditNum) && creditNum <= 0
    const proxyLine = status?.proxy_ip
      ? `${status.proxy_region || '未知地区'} · ${status.proxy_ip}`
      : ''
    return {
      profile,
      status,
      credits,
      account,
      lowCredits,
      answerReady: Boolean(status?.answer_ready),
      answerGenerating: Boolean(status?.answer_generating),
      proxyLine,
      proxyIsp: status?.proxy_isp || '',
      open: props.openTabs.includes(profile.id),
    }
  }),
)

function updatedAtLabel(iso: string | null | undefined): string {
  if (!iso) return ''
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

// 下载设置面板：本地副本编辑，保存时整体上报。
const dlForm = reactive({
  download_dir: '',
  auto_ai_download: false,
  skip_downloaded_files: true,
  confirm_delete_download: true,
  download_history_limit: 200,
  ai_exts: '',
})
watch(
  () => props.dlSettings,
  (settings) => {
    if (settings) {
      dlForm.download_dir = settings.download_dir
      dlForm.auto_ai_download = settings.auto_ai_download
      dlForm.skip_downloaded_files = true
      dlForm.confirm_delete_download = settings.confirm_delete_download
      dlForm.download_history_limit = settings.download_history_limit
      dlForm.ai_exts = settings.ai_exts
    }
  },
  { immediate: true },
)

function submitSettings() {
  // 这里只编辑下载相关字段，其余设置（下载保护、全局默认主页等）由“更多设置”管理。
  // Rust 侧 set_app_settings 是**整体覆盖写盘**（不做字段级合并），
  // 所以必须把未编辑的字段原样透传，否则保存下载设置会把它们重置成默认值。
  const base = props.dlSettings
  emit('saveSettings', {
    download_dir: dlForm.download_dir.trim(),
    auto_ai_download: dlForm.auto_ai_download,
    skip_downloaded_files: true,
    confirm_delete_download: dlForm.confirm_delete_download,
    download_history_limit: dlForm.download_history_limit,
    ai_exts: dlForm.ai_exts.trim(),
    download_guard_seconds: base?.download_guard_seconds ?? 3,
    download_guard_rules: base?.download_guard_rules ?? [],
    download_guard_auto_retry: base?.download_guard_auto_retry ?? true,
    global_default_url: base?.global_default_url ?? 'https://chat01.ai/',
  })
}

let releaseSettingsPopover: (() => void) | null = null
function onSettingsPopoverVisible(open: boolean) {
  if (open && !releaseSettingsPopover) {
    releaseSettingsPopover = acquireWebviewSuppression()
  } else if (!open && releaseSettingsPopover) {
    releaseSettingsPopover()
    releaseSettingsPopover = null
  }
}

onBeforeUnmount(() => {
  releaseSettingsPopover?.()
  releaseSettingsPopover = null
})
</script>

<template>
  <aside class="status-sidebar" :class="{ collapsed }">
    <template v-if="!collapsed">
      <div class="sidebar-title">
        <div>
          <strong>账号状态</strong>
          <small>积分 · AI 回答 · 谷歌账号</small>
        </div>
        <div class="sidebar-title-actions">
          <el-badge
            :value="downloadCount"
            :hidden="downloadCount === 0"
            :max="99"
            class="dl-badge"
          >
            <el-button circle size="small" :icon="Download" title="下载历史" @click="emit('openDownloads')" />
          </el-badge>
          <el-popover placement="bottom-end" :width="272" trigger="click" @show="onSettingsPopoverVisible(true)" @hide="onSettingsPopoverVisible(false)">
            <template #reference>
              <el-button circle size="small" :icon="Setting" title="下载设置" />
            </template>
            <div class="dl-form">
              <div class="dl-form-title">下载设置</div>
              <label class="dl-label">下载保存目录</label>
              <el-input v-model="dlForm.download_dir" size="small" placeholder="默认：应用数据目录/downloads" />
              <label class="dl-label">
                <el-checkbox v-model="dlForm.auto_ai_download" size="small">自动下载 AI 生成的文件</el-checkbox>
              </label>
              <label class="dl-label">
                <el-checkbox v-model="dlForm.skip_downloaded_files" size="small" disabled>永久禁止重复下载</el-checkbox>
              </label>
              <label class="dl-label">
                <el-checkbox v-model="dlForm.confirm_delete_download" size="small">删除下载文件前显示确认</el-checkbox>
              </label>
              <label class="dl-label">历史记录上限</label>
              <el-select v-model="dlForm.download_history_limit" size="small">
                <el-option :value="100" label="100 条" />
                <el-option :value="200" label="200 条" />
                <el-option :value="500" label="500 条" />
                <el-option :value="1000" label="1000 条" />
              </el-select>
              <label class="dl-label">扩展名白名单（逗号分隔，留空 = 全部）</label>
              <el-input v-model="dlForm.ai_exts" size="small" placeholder="md,txt,csv,docx,pdf,png,zip" />
              <el-button type="primary" size="small" class="dl-save" @click="submitSettings">保存</el-button>
              <small class="dl-hint">开启后，chat01 页面新出现的下载链接会按白名单自动下载；可跳过历史中已成功下载过的同名文件。</small>
            </div>
          </el-popover>
          <el-tooltip content="让已打开的账号页面立刻重新抓取" placement="bottom">
            <el-button circle size="small" :icon="Refresh" @click="emit('refresh')" />
          </el-tooltip>
          <el-button circle size="small" :icon="ArrowLeft" title="收起侧边栏" @click="emit('toggle')" />
        </div>
      </div>

      <div class="status-list">
        <div
          v-for="row in rows"
          :key="row.profile.id"
          class="status-card"
          :class="{ active: row.profile.id === activeId }"
          @click="emit('activate', row.profile.id)"
        >
          <div class="status-head">
            <span class="status-profile-name">{{ row.profile.name }}</span>
            <i v-if="row.lowCredits" class="warn-dot" title="积分不足（0 分）" />
            <i class="status-dot" :class="{ live: row.open }" :title="row.open ? '已打开' : '未打开'" />
          </div>
          <div class="status-credits" :class="{ zero: row.lowCredits }">
            <template v-if="row.credits">
              <span class="credits-value">{{ row.credits }}</span>
              <span class="credits-unit">积分</span>
            </template>
            <template v-else>
              <span class="credits-unknown">{{ row.open ? '抓取中…' : '未获取' }}</span>
            </template>
            <span
              v-if="row.answerReady"
              class="answer-ready-text"
              title="AI 回答已完成，点击该账号查看"
            >回答完成</span>
            <span v-else-if="row.answerGenerating" class="answer-generating-text">回答中…</span>
            <span v-if="row.lowCredits" class="credits-warn-text">积分不足</span>
          </div>
          <div v-if="showAccount" class="status-account" :title="row.account || '谷歌未登录'">
            <template v-if="row.account">
              <svg class="g-icon" viewBox="0 0 24 24" aria-hidden="true">
                <path fill="#4285F4" d="M23.5 12.3c0-.9-.1-1.5-.3-2.2H12v4.1h6.5c-.1 1.1-.8 2.7-2.4 3.8l3.7 2.9c2.3-2.1 3.7-5.2 3.7-8.6z" />
                <path fill="#34A853" d="M12 24c3.2 0 6-1.1 7.9-2.9l-3.7-2.9c-1 .7-2.4 1.2-4.2 1.2-3.2 0-6-2.1-6.9-5.1L1.2 17.2C3.2 21.2 7.3 24 12 24z" />
                <path fill="#FBBC05" d="M5.1 14.3c-.3-.7-.4-1.5-.4-2.3s.2-1.6.4-2.3L1.2 6.8C.4 8.4 0 10.1 0 12s.4 3.6 1.2 5.2l3.9-2.9z" />
                <path fill="#EA4335" d="M12 4.7c1.8 0 3 .8 3.7 1.4l3.3-3.2C17.9 1.1 15.2 0 12 0 7.3 0 3.2 2.8 1.2 6.8l3.9 2.9C6 6.8 8.8 4.7 12 4.7z" />
              </svg>
              <span class="account-text">{{ row.account }}</span>
            </template>
            <template v-else>
              <svg class="g-icon g-icon-off" viewBox="0 0 24 24" aria-hidden="true">
                <path fill="#475569" d="M23.5 12.3c0-.9-.1-1.5-.3-2.2H12v4.1h6.5c-.1 1.1-.8 2.7-2.4 3.8l3.7 2.9c2.3-2.1 3.7-5.2 3.7-8.6z" />
                <path fill="#64748b" d="M12 24c3.2 0 6-1.1 7.9-2.9l-3.7-2.9c-1 .7-2.4 1.2-4.2 1.2-3.2 0-6-2.1-6.9-5.1L1.2 17.2C3.2 21.2 7.3 24 12 24z" />
                <path fill="#94a3b8" d="M5.1 14.3c-.3-.7-.4-1.5-.4-2.3s.2-1.6.4-2.3L1.2 6.8C.4 8.4 0 10.1 0 12s.4 3.6 1.2 5.2l3.9-2.9z" />
                <path fill="#64748b" d="M12 4.7c1.8 0 3 .8 3.7 1.4l3.3-3.2C17.9 1.1 15.2 0 12 0 7.3 0 3.2 2.8 1.2 6.8l3.9 2.9C6 6.8 8.8 4.7 12 4.7z" />
              </svg>
              <span class="account-text muted">谷歌未登录</span>
            </template>
          </div>
          <div v-if="showProxy && row.proxyLine" class="status-proxy" :title="`出口 ISP：${row.proxyIsp || '未知'}`">
            <span class="proxy-text">{{ row.proxyLine }}</span>
          </div>
          <small v-if="showUpdatedTime && row.status?.answer_finished_at" class="status-time">{{ updatedAtLabel(row.status.answer_finished_at) }} AI使用</small>
        </div>

        <div v-if="rows.length === 0" class="status-empty">还没有账号</div>
      </div>
    </template>

    <button v-else class="collapsed-toggle" title="展开侧边栏" @click="emit('toggle')">
      <el-icon><ArrowRight /></el-icon>
    </button>
  </aside>
</template>
