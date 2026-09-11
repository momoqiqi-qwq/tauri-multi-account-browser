<script setup lang="ts">
import { onBeforeUnmount, reactive, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { acquireWebviewSuppression } from '../lib/suppress'
import type { Profile, ProfileIsolationSettings } from '../types'
import { DEFAULT_PROFILE_URL, normalizeProfileUrl } from '../lib/profileUrl'

const visible = defineModel<boolean>({ required: true })

const props = defineProps<{
  /** null 表示新建，否则为编辑该账号 */
  profile: Profile | null
  /** 已有分组，用于新建/编辑账号时直接下拉选择 */
  groups: string[]
  /** 已有标签，用于下拉建议 */
  knownTags: string[]
  saving: boolean
  globalDefaultUrl: string
}>()

const emit = defineEmits<{
  create: [settings: ProfileIsolationSettings]
  update: [id: string, settings: ProfileIsolationSettings]
}>()

const TIMEZONES = [
  'Asia/Shanghai',
  'Asia/Hong_Kong',
  'Asia/Taipei',
  'Asia/Singapore',
  'Asia/Tokyo',
  'Asia/Seoul',
  'Europe/London',
  'Europe/Berlin',
  'America/New_York',
  'America/Los_Angeles',
  'UTC',
]

const LOCALES = ['zh-CN', 'zh-TW', 'en-US', 'en-GB', 'ja-JP', 'ko-KR', 'de-DE', 'fr-FR', 'pt-BR', 'ru-RU']

const form = reactive<ProfileIsolationSettings>({
  name: '',
  defaultUrl: DEFAULT_PROFILE_URL,
  urlMode: 'inherit',
  incognito: false,
  proxy: '',
  userAgent: '',
  timezone: '',
  locale: '',
  fingerprintGuard: true,
  group: '',
  tags: [],
})

// 对话框是全屏遮罩，打开期间隐藏账号 WebView，否则遮罩和表单会被网页盖住。
let releaseDialog: (() => void) | null = null
watch(visible, (open) => {
  if (open) {
    releaseDialog?.()
    releaseDialog = acquireWebviewSuppression()
    const p = props.profile
    form.name = p?.name ?? ''
    form.defaultUrl = p?.default_url?.trim() || props.globalDefaultUrl || DEFAULT_PROFILE_URL
    form.urlMode = p?.url_mode ?? 'inherit'
    form.incognito = p?.incognito ?? false
    form.proxy = p?.proxy ?? ''
    form.userAgent = p?.user_agent ?? ''
    form.timezone = p?.timezone ?? ''
    form.locale = p?.locale ?? ''
    form.fingerprintGuard = p?.fingerprint_guard ?? true
    form.group = p?.group ?? ''
    form.tags = [...(p?.tags ?? [])]
  } else {
    releaseDialog?.()
    releaseDialog = null
  }
})
onBeforeUnmount(() => {
  releaseDialog?.()
  releaseDialog = null
})

// WebView2 是常青 Chromium 引擎：取当前环境真实 UA 并去掉 Edge 标识，
// 得到与引擎版本一致的 Chrome UA。谷歌对内嵌环境的拦截多来自
// Edg/WebView 特征，同源 Chrome UA 可避免“浏览器不安全”提示。
function applyChromeUa() {
  form.userAgent = navigator.userAgent.replace(/\s*Edg\/[\d.]+/, '').trim()
}

function normalizeDefaultUrl() {
  try {
    form.defaultUrl = normalizeProfileUrl(form.defaultUrl)
    return true
  } catch (error) {
    ElMessage.error(String(error))
    return false
  }
}

function submit() {
  if (props.saving) return
  if (!form.name.trim()) {
    ElMessage.warning('请填写账号昵称')
    return
  }
  if (form.urlMode === 'custom' && !normalizeDefaultUrl()) return
  if (form.urlMode === 'inherit') form.defaultUrl = props.globalDefaultUrl || DEFAULT_PROFILE_URL

  const settings = { ...form, name: form.name.trim() }
  if (props.profile) {
    emit('update', props.profile.id, settings)
  } else {
    emit('create', settings)
  }
  // 是否关闭由父组件在保存成功后决定；失败时保留用户已填写内容。
}
</script>

<template>
  <el-dialog
    v-model="visible"
    :title="profile ? `账号设置 · ${profile.name}` : '新建账号'"
    width="520px"
    append-to-body
  >
    <el-form label-position="top">
      <el-form-item label="昵称" required>
        <el-input v-model="form.name" placeholder="例如：Chat01 工作号" @keyup.enter="submit" />
      </el-form-item>
      <el-form-item label="分组（可选，用于侧边栏折叠归类）">
        <el-select
          v-model="form.group"
          filterable
          allow-create
          default-first-option
          clearable
          placeholder="选择已有分组，或输入新分组"
          style="width: 100%"
        >
          <el-option v-for="group in groups" :key="group" :label="group" :value="group" />
        </el-select>
        <small class="iso-hint">下拉会显示现有分组；也可以直接输入新的分组名称。</small>
      </el-form-item>
      <el-form-item label="标签（可选，用于跨分组归类与批量筛选）">
        <el-select
          v-model="form.tags"
          multiple
          filterable
          allow-create
          default-first-option
          :multiple-limit="12"
          placeholder="输入后回车即可新建标签"
          style="width: 100%"
        >
          <el-option v-for="tag in knownTags" :key="tag" :label="tag" :value="tag" />
        </el-select>
        <small class="iso-hint">打标后可在侧边栏顶部按标签筛选，或用批量操作条一次性打给多个账号。</small>
      </el-form-item>
      <el-form-item label="主页来源">
        <el-segmented v-model="form.urlMode" :options="[{ label: '继承全局', value: 'inherit' }, { label: '单独覆盖', value: 'custom' }]" />
        <small class="iso-hint">继承模式会跟随全局默认主页：{{ globalDefaultUrl || DEFAULT_PROFILE_URL }}</small>
      </el-form-item>
      <el-form-item v-if="form.urlMode === 'custom'" label="默认打开网址">
        <el-input
          v-model="form.defaultUrl"
          :placeholder="DEFAULT_PROFILE_URL"
          @blur="normalizeDefaultUrl"
          @keyup.enter="submit"
        />
        <small class="iso-hint">此账号单独使用该网址；“主页”按钮也会回到这里。</small>
      </el-form-item>

      <div class="iso-section-title">隔离与防关联</div>
      <el-form-item label="独立代理（每个账号一个出口 IP，强烈建议配置）">
        <el-input v-model="form.proxy" placeholder="http://127.0.0.1:7890 或 socks5://192.168.1.10:1080" />
        <small class="iso-hint">
          Cookie/登录态本就按账号物理隔离；同一 IP 登录多个账号是最常见的封号诱因。
          每个账号配一个固定独立代理可大幅降低关联风险；配置后自动禁止 WebRTC 绕过代理直连。
        </small>
      </el-form-item>
      <el-form-item label="时区（与代理 IP 所在地保持一致）">
        <el-select v-model="form.timezone" filterable allow-create default-first-option clearable placeholder="跟随系统">
          <el-option label="跟随系统" value="" />
          <el-option v-for="tz in TIMEZONES" :key="tz" :label="tz" :value="tz" />
        </el-select>
      </el-form-item>
      <el-form-item label="语言 / 区域（与代理 IP 所在地保持一致）">
        <el-select v-model="form.locale" filterable allow-create default-first-option clearable placeholder="跟随系统">
          <el-option label="跟随系统" value="" />
          <el-option v-for="loc in LOCALES" :key="loc" :label="loc" :value="loc" />
        </el-select>
      </el-form-item>
      <el-form-item label="User-Agent（留空使用系统默认）">
        <el-input
          v-model="form.userAgent"
          type="textarea"
          :rows="2"
          placeholder="一般留空；同站点对 UA 校验严格或谷歌登录被拦截时再覆盖"
        />
        <div class="iso-ua-actions">
          <el-button size="small" @click="applyChromeUa">填入 Chrome 同源 UA</el-button>
          <el-button size="small" @click="form.userAgent = ''">恢复系统默认</el-button>
        </div>
        <small class="iso-hint">
          谷歌登录防踢：账号要固定一套 UA + IP + 时区，不要频繁清数据。
          若谷歌提示“此浏览器或应用可能不安全”，点击“填入 Chrome 同源 UA”。
        </small>
      </el-form-item>
      <el-form-item>
        <el-switch v-model="form.fingerprintGuard" />
        <span class="iso-switch-label">指纹防护（画布 / GPU / 硬件信息按账号加确定性噪声）</span>
      </el-form-item>
      <el-form-item>
        <el-checkbox v-model="form.incognito">隐身 / 临时账号（关闭即清空，不落盘）</el-checkbox>
      </el-form-item>
    </el-form>

    <template #footer>
      <el-button :disabled="saving" @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="submit">
        {{ profile ? '保存' : '创建并打开' }}
      </el-button>
    </template>
  </el-dialog>
</template>
