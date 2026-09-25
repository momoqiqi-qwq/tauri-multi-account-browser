<script setup lang="ts">
import { ref } from 'vue'
import type { InputInstance } from 'element-plus'
import { ArrowLeft, ArrowRight, Refresh, Right, Setting, DataAnalysis, Grid } from '@element-plus/icons-vue'

const model = defineModel<string>({ required: true })
defineProps<{ disabled: boolean }>()
const emit = defineEmits<{
  navigate: [url: string]
  back: []
  forward: []
  reload: []
  accountOverview: []
  settings: []
  directory: []
}>()

const urlInput = ref<InputInstance | null>(null)

/** 供 Ctrl+L 快捷键使用：聚焦并全选网址输入框。 */
function focusUrl() {
  urlInput.value?.focus()
  urlInput.value?.select()
}

defineExpose({ focusUrl })
</script>

<template>
  <header class="address-bar">
    <el-button :icon="Grid" title="按网站切换账号" @click="emit('directory')">网站导航</el-button>
    <el-button circle :icon="ArrowLeft" :disabled="disabled" title="后退 (Alt+←)" @click="emit('back')" />
    <el-button circle :icon="ArrowRight" :disabled="disabled" title="前进 (Alt+→)" @click="emit('forward')" />
    <el-button circle :icon="Refresh" :disabled="disabled" title="刷新 (F5)" @click="emit('reload')" />
    <el-input
      ref="urlInput"
      v-model="model"
      class="url-input"
      :disabled="disabled"
      placeholder="输入网址，例如 https://chat01.ai/（Ctrl+L 快速聚焦）"
      @keyup.enter="emit('navigate', model)"
    >
      <template #append>
        <el-button :icon="Right" title="打开网址" @click="emit('navigate', model)" />
      </template>
    </el-input>
    <el-button circle :icon="DataAnalysis" title="全部账号状态" class="account-overview-button" @click="emit('accountOverview')" />
    <el-button circle :icon="Setting" title="更多设置" class="global-settings-button" @click="emit('settings')" />
  </header>
</template>
