<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'
import type { DiagnosticCode, Profile, ProfileDiagnostic } from '../types'

const visible = defineModel<boolean>({ required: true })
const props = defineProps<{ profile: Profile | null; error: string }>()
const emit = defineEmits<{ retry: [id: string]; changed: [] }>()

const info = ref<ProfileDiagnostic | null>(null)
const busy = ref(false)
const opening = ref(false)

const CODE_LABEL: Record<DiagnosticCode, string> = {
  unknown: '未能定位',
  webview2_runtime: 'WebView2 Runtime',
  data_dir: '数据目录',
  proxy: '代理',
  network: '网络',
  invalid_url: '网址格式',
  bad_last_url: '上次网址',
}

const CODE_TAG: Record<DiagnosticCode, 'info' | 'success' | 'warning' | 'danger'> = {
  unknown: 'info',
  webview2_runtime: 'danger',
  data_dir: 'danger',
  proxy: 'warning',
  network: 'warning',
  invalid_url: 'warning',
  bad_last_url: 'warning',
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

const dataUsageText = computed(() => {
  const dir = info.value?.data_dir
  if (!dir) return '—'
  if (!dir.exists) return '尚未创建（该账号还没启动过）'
  const size = formatBytes(dir.bytes)
  return dir.truncated
    ? `${size} · ${dir.file_count}+ 个文件（已超过统计上限）`
    : `${size} · ${dir.file_count} 个文件`
})

async function load() {
  if (!props.profile) return
  // 原始错误一并传过去：Rust 侧据此归类并给出针对性建议。
  info.value = await invoke<ProfileDiagnostic>('diagnose_profile', {
    id: props.profile.id,
    error: props.error,
  }).catch(() => null)
}

watch(visible, (open) => {
  if (open) void load()
})

async function repair(action: string, retry = true) {
  if (!props.profile) return
  busy.value = true
  try {
    await invoke('repair_profile_startup', { id: props.profile.id, action })
    emit('changed')
    await load()
    ElMessage.success('修复已完成')
    if (retry) {
      visible.value = false
      emit('retry', props.profile.id)
    }
  } catch (e) {
    ElMessage.error(`修复失败：${String(e)}`)
  } finally {
    busy.value = false
  }
}

async function openDataDir() {
  if (!props.profile) return
  opening.value = true
  try {
    await invoke('open_profile_data_dir', { id: props.profile.id })
  } catch (e) {
    ElMessage.error(`打开数据目录失败：${String(e)}`)
  } finally {
    opening.value = false
  }
}
</script>

<template>
  <el-dialog v-model="visible" title="账号启动诊断" width="680px" append-to-body>
    <el-alert
      :title="`“${profile?.name || ''}” 打开失败`"
      :description="error"
      type="error"
      :closable="false"
      show-icon
    />

    <template v-if="info">
      <el-alert
        :title="`原因归类：${CODE_LABEL[info.code]}`"
        :description="info.hint"
        :type="info.code === 'unknown' ? 'info' : 'warning'"
        :closable="false"
        show-icon
        style="margin-top: 12px"
      />

      <el-descriptions :column="1" border style="margin-top: 14px">
        <el-descriptions-item label="错误归类">
          <el-tag :type="CODE_TAG[info.code]">{{ CODE_LABEL[info.code] }}</el-tag>
        </el-descriptions-item>
        <el-descriptions-item label="WebView2 Runtime">
          <el-tag :type="info.runtime.installed ? 'success' : 'danger'">
            {{ info.runtime.installed ? `已安装 ${info.runtime.version}` : '未检测到' }}
          </el-tag>
          <span v-if="info.runtime.installed" class="diag-muted">
            来源 {{ info.runtime.source }}
          </span>
        </el-descriptions-item>
        <el-descriptions-item label="数据目录占用">
          {{ dataUsageText }}
          <el-button
            v-if="info.data_dir.exists"
            link
            type="primary"
            :loading="opening"
            @click="openDataDir"
          >
            打开目录
          </el-button>
        </el-descriptions-item>
        <el-descriptions-item label="主页模式">
          {{ info.url_mode === 'inherit' ? '继承全局' : '单独覆盖' }}
        </el-descriptions-item>
        <el-descriptions-item label="有效主页">{{ info.effective_home_url }}</el-descriptions-item>
        <el-descriptions-item label="最后网址">
          {{ info.last_url || '空' }}
          <el-tag :type="info.last_url_valid ? 'success' : 'danger'">
            {{ info.last_url_valid ? '有效' : '异常' }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item label="代理">
          {{ info.proxy_configured ? (info.proxy_valid ? '已配置 · 格式正常' : '已配置 · 格式异常') : '直连' }}
        </el-descriptions-item>
      </el-descriptions>
    </template>

    <div style="margin-top: 16px; display: flex; flex-wrap: wrap; gap: 8px">
      <el-button :loading="busy" @click="repair('clear_last_url')">清除错误 last_url 并重试</el-button>
      <el-button :loading="busy" @click="repair('home')">回账号主页</el-button>
      <el-button :loading="busy" @click="repair('global')">切换全局主页</el-button>
      <el-button
        v-if="info?.proxy_configured"
        :loading="busy"
        type="warning"
        @click="repair('disable_proxy')"
      >
        禁用代理后重试
      </el-button>
      <el-button type="primary" @click="profile && emit('retry', profile.id); visible = false">
        直接重试
      </el-button>
    </div>
  </el-dialog>
</template>

<style scoped>
.diag-muted {
  margin-left: 8px;
  font-size: 12px;
  opacity: 0.6;
}
</style>
