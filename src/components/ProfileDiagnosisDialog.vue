<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import type { Profile } from '../types'
const visible = defineModel<boolean>({ required: true })
const props = defineProps<{ profile: Profile | null; error: string }>()
const emit = defineEmits<{ retry: [id: string]; changed: [] }>()
const info = ref<any>(null); const busy = ref(false)
async function load() { if (!props.profile) return; info.value = await invoke('diagnose_profile', { id: props.profile.id }).catch(() => null) }
watch(visible, open => { if (open) void load() })
async function repair(action: string, retry = true) {
  if (!props.profile) return; busy.value = true
  try { await invoke('repair_profile_startup', { id: props.profile.id, action }); emit('changed'); await load(); ElMessage.success('修复已完成'); if (retry) { visible.value = false; emit('retry', props.profile.id) } }
  catch (e) { ElMessage.error(`修复失败：${String(e)}`) } finally { busy.value = false }
}
</script>
<template>
  <el-dialog v-model="visible" title="账号启动诊断" width="650px" append-to-body>
    <el-alert :title="`“${profile?.name || ''}” 打开失败`" :description="error" type="error" :closable="false" show-icon />
    <el-descriptions v-if="info" :column="1" border style="margin-top:14px">
      <el-descriptions-item label="主页模式">{{ info.url_mode === 'inherit' ? '继承全局' : '单独覆盖' }}</el-descriptions-item>
      <el-descriptions-item label="有效主页">{{ info.effective_home_url }}</el-descriptions-item>
      <el-descriptions-item label="最后网址">{{ info.last_url || '空' }} <el-tag :type="info.last_url_valid ? 'success' : 'danger'">{{ info.last_url_valid ? '有效' : '异常' }}</el-tag></el-descriptions-item>
      <el-descriptions-item label="代理">{{ info.proxy_configured ? (info.proxy_valid ? '已配置 · 格式正常' : '已配置 · 格式异常') : '直连' }}</el-descriptions-item>
    </el-descriptions>
    <div style="margin-top:16px;display:flex;flex-wrap:wrap;gap:8px">
      <el-button :loading="busy" @click="repair('clear_last_url')">清除错误 last_url 并重试</el-button>
      <el-button :loading="busy" @click="repair('home')">回账号主页</el-button>
      <el-button :loading="busy" @click="repair('global')">切换全局主页</el-button>
      <el-button v-if="info?.proxy_configured" :loading="busy" type="warning" @click="repair('disable_proxy')">禁用代理后重试</el-button>
      <el-button type="primary" @click="profile && emit('retry', profile.id); visible = false">直接重试</el-button>
    </div>
  </el-dialog>
</template>
