<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Close, Plus, Lock, Unlock, VideoPause, VideoPlay, RefreshLeft } from '@element-plus/icons-vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { Profile, ProfileStatus } from '../types'

const props = defineProps<{
  profiles: Profile[]
  openTabs: string[]
  openedAt: Record<string, number>
  runtimeSeconds: Record<string, number>
  runtimePaused: Record<string, boolean>
  hibernated: string[]
  pinned: string[]
  activeId: string | null
  statuses: Record<string, ProfileStatus>
  showAiStatus: boolean
  width: 'compact' | 'standard' | 'wide'
}>()
const emit = defineEmits<{
  activate: [id: string]
  close: [id: string]
  create: []
  reorder: [ids: string[]]
  pin: [id: string]
  pauseRuntime: [id: string]
  resetRuntime: [id: string]
}>()
let releaseMenu: (() => void) | null = null
function onMenuToggle(open: boolean) { if (open && !releaseMenu) releaseMenu = acquireWebviewSuppression(); else if (!open && releaseMenu) { releaseMenu(); releaseMenu = null } }
const nowMs = ref(Date.now()); let runtimeTimer: ReturnType<typeof setInterval> | undefined
onMounted(() => { runtimeTimer = setInterval(() => { nowMs.value = Date.now() }, 1000) })
onBeforeUnmount(() => { if (runtimeTimer !== undefined) clearInterval(runtimeTimer); releaseMenu?.() })
function runtimeFor(id: string): string { const total=Math.max(0,Math.floor(props.runtimeSeconds[id]||0)); const h=Math.floor(total/3600),m=Math.floor((total%3600)/60),s=total%60; return [h,m,s].map(v=>String(v).padStart(2,'0')).join(':') }
function statusFor(id: string) { return props.statuses[id] }
function tabState(id: string): 'generating'|'ready'|'sleeping'|'open' { if (props.hibernated.includes(id)) return 'sleeping'; const status=statusFor(id); if(props.showAiStatus&&status?.answer_generating)return'generating'; if(props.showAiStatus&&status?.answer_ready)return'ready'; return'open' }
const openProfiles=computed(()=>props.openTabs.map(id=>props.profiles.find(p=>p.id===id)).filter((p):p is Profile=>Boolean(p)))
const closedProfiles=computed(()=>{const open=new Set(props.openTabs);return props.profiles.filter(p=>!open.has(p.id))})
const draggingId=ref<string|null>(null)
function dropOn(targetId:string){ const source=draggingId.value; draggingId.value=null; if(!source||source===targetId)return; const ids=[...props.openTabs]; const a=ids.indexOf(source),b=ids.indexOf(targetId); if(a<0||b<0)return; const sourcePinned=props.pinned.includes(source), targetPinned=props.pinned.includes(targetId); if(sourcePinned!==targetPinned)return; ids.splice(a,1); ids.splice(b,0,source); emit('reorder',ids) }
</script>
<template>
  <div class="tab-strip" :data-tab-width="width">
    <div class="tab-scroll">
      <div v-for="profile in openProfiles" :key="profile.id" class="tab-item"
        :class="{ active: profile.id===activeId, generating: tabState(profile.id)==='generating', ready: tabState(profile.id)==='ready', sleeping: tabState(profile.id)==='sleeping', pinned: pinned.includes(profile.id) }"
        :title="`${profile.name} · 运行 ${runtimeFor(profile.id)}${runtimePaused[profile.id]?' · 已暂停':''}${tabState(profile.id)==='sleeping'?' · 已休眠':''}`"
        draggable="true" @dragstart="draggingId=profile.id" @dragend="draggingId=null" @dragover.prevent @drop.prevent="dropOn(profile.id)" @click="emit('activate',profile.id)" @auxclick.middle.prevent="!pinned.includes(profile.id)&&emit('close',profile.id)">
        <span class="tab-dot" :class="`state-${tabState(profile.id)}`" />
        <el-icon class="tab-pin" :title="pinned.includes(profile.id)?'取消固定':'固定标签'" @click.stop="emit('pin',profile.id)"><Lock v-if="pinned.includes(profile.id)"/><Unlock v-else/></el-icon>
        <span class="tab-title">{{ profile.name }}</span>
        <button class="tab-runtime" :class="{ paused: runtimePaused[profile.id] }" :title="runtimePaused[profile.id]?'点击继续计时':'点击暂停计时'" @click.stop="emit('pauseRuntime',profile.id)">
          <el-icon><VideoPlay v-if="runtimePaused[profile.id]"/><VideoPause v-else/></el-icon>{{ runtimeFor(profile.id) }}
        </button>
        <el-icon class="tab-runtime-reset" title="重置运行时间" @click.stop="emit('resetRuntime',profile.id)"><RefreshLeft/></el-icon>
        <span v-if="tabState(profile.id)==='sleeping'" class="tab-sleep-badge">休眠</span>
        <span v-else-if="tabState(profile.id)==='generating'" class="tab-ai-loading"><i>.</i><i>.</i><i>.</i></span>
        <span v-else-if="tabState(profile.id)==='ready'" class="tab-ai-ready">完成</span>
        <span v-if="profile.incognito" class="tab-badge">临时</span>
        <el-icon v-if="!pinned.includes(profile.id)" class="tab-close" title="关闭标签页" @click.stop="emit('close',profile.id)"><Close/></el-icon>
      </div>
      <el-popover placement="bottom-start" :width="264" trigger="click" @show="onMenuToggle(true)" @hide="onMenuToggle(false)">
        <template #reference><button class="tab-add" title="打开其他账号或新建账号"><el-icon><Plus/></el-icon></button></template>
        <div class="tab-open-menu"><template v-if="closedProfiles.length"><small class="tab-open-label">未打开的账号</small><button v-for="profile in closedProfiles" :key="profile.id" class="tab-open-item" @click="emit('activate',profile.id)"><span class="tab-dot state-open"/><span>{{ profile.name }}</span></button></template><small v-else class="tab-open-label">所有账号都已打开</small><button class="tab-open-item new" @click="emit('create')"><el-icon><Plus/></el-icon><span>新建账号</span></button></div>
      </el-popover>
    </div>
  </div>
</template>
