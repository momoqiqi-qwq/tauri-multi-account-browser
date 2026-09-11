<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  ArrowLeft,
  ArrowRight,
  CopyDocument,
  DataAnalysis,
  Download,
  House,
  Link,
  Refresh,
  Setting,
} from '@element-plus/icons-vue'
import type { BrowserToolbarBuiltinId, BrowserToolbarItem, CustomToolbarButton } from '../types'

const props = defineProps<{
  disabled: boolean
  position: 'right' | 'bottom'
  compact: boolean
  items: BrowserToolbarItem[]
  customButtons: CustomToolbarButton[]
}>()

const emit = defineEmits<{
  back: []
  forward: []
  reload: []
  home: []
  copyUrl: []
  accountOverview: []
  downloads: []
  settings: []
  custom: [button: CustomToolbarButton]
}>()

const metadata: Record<BrowserToolbarBuiltinId, {
  label: string
  icon: typeof Refresh
  browserOnly: boolean
}> = {
  reload: { label: '刷新当前页面', icon: Refresh, browserOnly: true },
  home: { label: '回到账号主页', icon: House, browserOnly: true },
  back: { label: '后退', icon: ArrowLeft, browserOnly: true },
  forward: { label: '前进', icon: ArrowRight, browserOnly: true },
  'copy-url': { label: '复制当前网址', icon: CopyDocument, browserOnly: true },
  overview: { label: '全部账号状态', icon: DataAnalysis, browserOnly: false },
  downloads: { label: '下载历史', icon: Download, browserOnly: false },
  settings: { label: '更多设置', icon: Setting, browserOnly: false },
}

const visibleItems = computed(() => props.items.filter((item) => item.enabled))
const visibleCustomButtons = computed(() => props.customButtons.filter((item) => item.enabled && item.url.trim()))
const reloadPulse = ref(false)

function shortCustomIcon(value: string) {
  return Array.from(value.trim()).slice(0, 2).join('')
}

function runBuiltin(id: BrowserToolbarBuiltinId) {
  if (metadata[id].browserOnly && props.disabled) return
  if (id === 'reload') {
    reloadPulse.value = true
    window.setTimeout(() => { reloadPulse.value = false }, 650)
    emit('reload')
  } else if (id === 'home') emit('home')
  else if (id === 'back') emit('back')
  else if (id === 'forward') emit('forward')
  else if (id === 'copy-url') emit('copyUrl')
  else if (id === 'overview') emit('accountOverview')
  else if (id === 'downloads') emit('downloads')
  else if (id === 'settings') emit('settings')
}
</script>

<template>
  <aside
    class="browser-action-dock"
    :class="[`dock-${position}`, { compact }]"
    aria-label="浏览器快捷工具栏"
  >
    <div class="browser-action-dock__group">
      <button
        v-for="item in visibleItems"
        :key="item.id"
        class="browser-action-button"
        :class="{ 'is-reloading': item.id === 'reload' && reloadPulse }"
        type="button"
        :disabled="metadata[item.id].browserOnly && disabled"
        :aria-label="metadata[item.id].label"
        :title="metadata[item.id].label"
        @click="runBuiltin(item.id)"
      >
        <el-icon><component :is="metadata[item.id].icon" /></el-icon>
      </button>

      <span v-if="visibleCustomButtons.length && visibleItems.length" class="browser-action-separator" />

      <button
        v-for="button in visibleCustomButtons"
        :key="button.id"
        class="browser-action-button custom"
        type="button"
        :disabled="disabled"
        :aria-label="button.label || button.url"
        :title="button.label || button.url"
        @click="emit('custom', button)"
      >
        <span v-if="button.icon.trim()" class="custom-toolbar-icon">{{ shortCustomIcon(button.icon) }}</span>
        <el-icon v-else><Link /></el-icon>
      </button>
    </div>
  </aside>
</template>
