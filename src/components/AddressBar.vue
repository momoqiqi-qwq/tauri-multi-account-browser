<script setup lang="ts">
import { ArrowLeft, ArrowRight, Refresh, Right, Setting, DataAnalysis } from '@element-plus/icons-vue'

const model = defineModel<string>({ required: true })
defineProps<{ disabled: boolean }>()
const emit = defineEmits<{
  navigate: [url: string]
  back: []
  forward: []
  reload: []
  accountOverview: []
  settings: []
}>()
</script>

<template>
  <header class="address-bar">
    <el-button circle :icon="ArrowLeft" :disabled="disabled" title="后退" @click="emit('back')" />
    <el-button circle :icon="ArrowRight" :disabled="disabled" title="前进" @click="emit('forward')" />
    <el-button circle :icon="Refresh" :disabled="disabled" title="刷新" @click="emit('reload')" />
    <el-input
      v-model="model"
      class="url-input"
      :disabled="disabled"
      placeholder="输入网址，例如 https://chat01.ai/"
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
