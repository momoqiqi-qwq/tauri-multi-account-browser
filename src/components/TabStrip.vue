<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useActivityClock } from '../composables/useActivityClock'
import { Close, Plus, Lock, Unlock, Timer } from '@element-plus/icons-vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { Profile, ProfileStatus } from '../types'

const props = defineProps<{
  profiles: Profile[]
  openTabs: string[]
  hibernated: string[]
  pinned: string[]
  activeId: string | null
  statuses: Record<string, ProfileStatus>
  showAiStatus: boolean
  width: 'compact' | 'standard' | 'wide'
  /** Directory covers this strip; no live repaint is needed until it becomes visible. */
  suspended?: boolean
}>()
const emit = defineEmits<{
  activate: [id: string]
  close: [id: string]
  create: []
  reorder: [ids: string[]]
  pin: [id: string]
}>()
let releaseMenu: (() => void) | null = null
function onMenuToggle(open: boolean) { if (open && !releaseMenu) releaseMenu = acquireWebviewSuppression(); else if (!open && releaseMenu) { releaseMenu(); releaseMenu = null } }
const profileIndex = computed(() => new Map(props.profiles.map(profile => [profile.id, profile])))
const openIds = computed(() => new Set(props.openTabs))
const sleepingIds = computed(() => new Set(props.hibernated))
const pinnedIds = computed(() => new Set(props.pinned))
const needsClock = computed(() => !props.suspended && props.openTabs.some(id => {
  const status = props.statuses[id]
  return !sleepingIds.value.has(id) && status?.answer_generating &&
    !!status.answer_started_at && Number.isFinite(Date.parse(status.answer_started_at))
}))
const nowMs = useActivityClock(() => needsClock.value)
onBeforeUnmount(() => releaseMenu?.())
function statusFor(id: string) { return props.statuses[id] }
// v28：标签上的时间是「AI 对话计时」，不再是标签打开时长。
// 生成中：用 answer_started_at（Rust 上升沿时钟）实时走表，新对话从零重算；
// 其余时间：冻结在最近一轮实测耗时（answer_duration_ms）上，没测过就是 00:00:00。
function runtimeMsFor(id: string): number {
  const status = statusFor(id)
  if (!status) return 0
  if (status.answer_generating && status.answer_started_at) {
    const started = Date.parse(status.answer_started_at)
    if (Number.isFinite(started)) return Math.max(0, nowMs.value - started)
  }
  return Math.max(0, status.answer_duration_ms || 0)
}
function runtimeFor(id: string): string {
  const total = Math.floor(runtimeMsFor(id) / 1000)
  const h = Math.floor(total / 3600), m = Math.floor((total % 3600) / 60), s = total % 60
  return [h, m, s].map(v => String(v).padStart(2, '0')).join(':')
}
function tabState(id: string): 'generating'|'ready'|'sleeping'|'open' { if (sleepingIds.value.has(id)) return 'sleeping'; const status=statusFor(id); if(props.showAiStatus&&status?.answer_generating)return'generating'; if(props.showAiStatus&&status?.answer_ready)return'ready'; return'open' }
const openProfiles = computed(() => props.openTabs.map(id => profileIndex.value.get(id)).filter((p): p is Profile => !!p))
const closedProfiles = computed(() => props.profiles.filter(profile => !openIds.value.has(profile.id)))
const draggingId=ref<string|null>(null)
function dropOn(targetId:string){ const source=draggingId.value; draggingId.value=null; if(!source||source===targetId)return; const ids=[...props.openTabs]; const a=ids.indexOf(source),b=ids.indexOf(targetId); if(a<0||b<0)return; const sourcePinned=props.pinned.includes(source), targetPinned=props.pinned.includes(targetId); if(sourcePinned!==targetPinned)return; ids.splice(a,1); ids.splice(b,0,source); emit('reorder',ids) }
</script>
<template>
  <div class="tab-strip" :data-tab-width="width">
    <div class="tab-scroll">
      <div v-for="profile in openProfiles" :key="profile.id" class="tab-item"
        :class="{ active: profile.id===activeId, generating: tabState(profile.id)==='generating', ready: tabState(profile.id)==='ready', sleeping: tabState(profile.id)==='sleeping', pinned: pinnedIds.has(profile.id) }"
        :title="`${profile.name} · AI 计时 ${runtimeFor(profile.id)}${tabState(profile.id)==='sleeping'?' · 已休眠':''}`"
        draggable="true" @dragstart="draggingId=profile.id" @dragend="draggingId=null" @dragover.prevent @drop.prevent="dropOn(profile.id)" @click="emit('activate',profile.id)" @auxclick.middle.prevent="!pinnedIds.has(profile.id)&&emit('close',profile.id)">
        <span class="tab-dot" :class="`state-${tabState(profile.id)}`" />
        <el-icon class="tab-pin" :title="pinnedIds.has(profile.id)?'取消固定':'固定标签'" @click.stop="emit('pin',profile.id)"><Lock v-if="pinnedIds.has(profile.id)"/><Unlock v-else/></el-icon>
        <span class="tab-title">{{ profile.name }}</span>
        <span class="tab-runtime" :class="{ running: tabState(profile.id)==='generating' }" title="AI 对话计时：发送后开始，回答完毕结束，新对话从零重算">
          <el-icon><Timer/></el-icon>{{ runtimeFor(profile.id) }}
        </span>
        <span v-if="tabState(profile.id)==='sleeping'" class="tab-sleep-badge">休眠</span>
        <span v-else-if="tabState(profile.id)==='generating'" class="tab-ai-loading"><i>.</i><i>.</i><i>.</i></span>
        <span v-else-if="tabState(profile.id)==='ready'" class="tab-ai-ready">完成</span>
        <span v-if="profile.incognito" class="tab-badge">临时</span>
        <el-icon v-if="!pinnedIds.has(profile.id)" class="tab-close" title="关闭标签页" @click.stop="emit('close',profile.id)"><Close/></el-icon>
      </div>
      <el-popover placement="bottom-start" :width="264" trigger="click" @show="onMenuToggle(true)" @hide="onMenuToggle(false)">
        <template #reference><button class="tab-add" title="打开其他账号或新建账号"><el-icon><Plus/></el-icon></button></template>
        <div class="tab-open-menu"><template v-if="closedProfiles.length"><small class="tab-open-label">未打开的账号</small><button v-for="profile in closedProfiles" :key="profile.id" class="tab-open-item" @click="emit('activate',profile.id)"><span class="tab-dot state-open"/><span>{{ profile.name }}</span></button></template><small v-else class="tab-open-label">所有账号都已打开</small><button class="tab-open-item new" @click="emit('create')"><el-icon><Plus/></el-icon><span>新建账号</span></button></div>
      </el-popover>
    </div>
  </div>
</template>
