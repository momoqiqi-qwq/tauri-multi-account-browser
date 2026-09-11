<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage, ElMessageBox } from 'element-plus'
import type { AppSettings, Profile } from '../types'
import { normalizeProfileUrl } from '../lib/profileUrl'

interface ProfileTemplate {
  id: string
  name: string
  default_url: string
  url_mode: 'inherit' | 'custom'
  incognito: boolean
  proxy: string
  user_agent: string
  timezone: string
  locale: string
  fingerprint_guard: boolean
  group: string
}

const visible = defineModel<boolean>({ required: true })
const props = defineProps<{ profiles: Profile[]; appSettings: AppSettings | null }>()
const emit = defineEmits<{ changed: [] }>()

/** v14 旧版把模板存在 localStorage；首次加载时一次性搬进 Rust store。 */
const LEGACY_STORAGE_KEY = 'mab-profile-templates-v14'

const templates = ref<ProfileTemplate[]>([])
/** 是否已从 Rust store 读过模板（含一次性迁移），避免重复读写。 */
const templatesLoaded = ref(false)

function readLegacyTemplates(): ProfileTemplate[] {
  try {
    const v = JSON.parse(localStorage.getItem(LEGACY_STORAGE_KEY) || '[]')
    return Array.isArray(v) ? v : []
  } catch {
    return []
  }
}

/** 读取模板；若 Rust store 为空且 localStorage 有旧数据，则自动迁移一次。 */
async function loadTemplates() {
  try {
    let list = await invoke<ProfileTemplate[]>('list_profile_templates')
    if (!list.length) {
      const legacy = readLegacyTemplates()
      if (legacy.length) {
        list = await invoke<ProfileTemplate[]>('save_profile_templates', { templates: legacy })
        localStorage.removeItem(LEGACY_STORAGE_KEY)
        ElMessage.success(`已把 ${list.length} 个账号模板迁移到应用数据目录`)
      }
    }
    templates.value = list
    templatesLoaded.value = true
  } catch (e) {
    // 读取失败不阻塞界面，退化为空列表。
    templatesLoaded.value = true
    ElMessage.error(`读取账号模板失败：${String(e)}`)
  }
}

/** 保存模板。用 Rust 返回的规范化结果回写本地状态，避免前后端状态漂移。 */
async function persistTemplates() {
  try {
    templates.value = await invoke<ProfileTemplate[]>('save_profile_templates', { templates: templates.value })
  } catch (e) {
    // 写入失败时保留用户当前的本地编辑内容，不静默丢弃。
    ElMessage.error(`保存账号模板失败：${String(e)}`)
  }
}

const templateForm = reactive<ProfileTemplate>({ id: '', name: '', default_url: '', url_mode: 'inherit', incognito: false, proxy: '', user_agent: '', timezone: '', locale: '', fingerprint_guard: true, group: '' })
function resetTemplate() { Object.assign(templateForm, { id: '', name: '', default_url: '', url_mode: 'inherit', incognito: false, proxy: '', user_agent: '', timezone: '', locale: '', fingerprint_guard: true, group: '' }) }
function saveTemplate() {
  if (!templateForm.name.trim()) return ElMessage.warning('请输入模板名称')
  if (templateForm.url_mode === 'custom') { try { templateForm.default_url = normalizeProfileUrl(templateForm.default_url) } catch (e) { return ElMessage.error(String(e)) } }
  const item = { ...templateForm, id: templateForm.id || crypto.randomUUID(), name: templateForm.name.trim(), group: templateForm.group.trim() }
  const idx = templates.value.findIndex(t => t.id === item.id)
  if (idx >= 0) templates.value[idx] = item; else templates.value.push(item)
  persistTemplates(); resetTemplate(); ElMessage.success('模板已保存')
}
function editTemplate(t: ProfileTemplate) { Object.assign(templateForm, t) }
async function removeTemplate(t: ProfileTemplate) {
  const confirmed = await ElMessageBox.confirm(`删除模板“${t.name}”？`, '删除模板', { type: 'warning' }).then(() => true).catch(() => false)
  if (!confirmed) return
  templates.value = templates.value.filter(x => x.id !== t.id); persistTemplates()
}

const batch = reactive({ prefix: '账号', start: 1, count: 5, digits: 2, templateId: '', group: '' })
const batchBusy = ref(false)
const batchPreview = computed(() => Array.from({ length: Math.min(batch.count, 8) }, (_, i) => `${batch.prefix}${String(batch.start + i).padStart(batch.digits, '0')}`))
async function createBatch() {
  if (!batch.prefix.trim() || batch.count < 1 || batch.count > 200) return ElMessage.warning('批量数量需为 1-200')
  const t = templates.value.find(x => x.id === batch.templateId)
  batchBusy.value = true
  try {
    // 一次性把所有草稿交给 Rust：任一不合法则整批不落盘，不会留下半成品账号。
    const drafts = Array.from({ length: batch.count }, (_, i) => ({
      name: `${batch.prefix.trim()}${String(batch.start + i).padStart(batch.digits, '0')}`,
      default_url: t?.default_url || props.appSettings?.global_default_url || '',
      url_mode: t?.url_mode || 'inherit',
      incognito: t?.incognito || false,
      proxy: t?.proxy || '',
      user_agent: t?.user_agent || '',
      timezone: t?.timezone || '',
      locale: t?.locale || '',
      fingerprint_guard: t?.fingerprint_guard ?? true,
      group: batch.group.trim() || t?.group || '',
    }))
    const created = await invoke<Profile[]>('create_profiles_bulk', { drafts })
    emit('changed')
    ElMessage.success(`已创建 ${created.length} 个账号`)
  } catch (e) {
    ElMessage.error(`批量创建失败，未创建任何账号：${String(e)}`)
  } finally {
    batchBusy.value = false
  }
}

const jsonText = ref('')
const importBusy = ref(false)
function buildExport() {
  jsonText.value = JSON.stringify({ version: 14, exported_at: new Date().toISOString(), profiles: props.profiles, templates: templates.value }, null, 2)
}
async function copyExport() { if (!jsonText.value) buildExport(); await navigator.clipboard.writeText(jsonText.value); ElMessage.success('JSON 已复制') }
watch(visible, async open => {
  if (!open) return
  // 打开时读取模板（含一次性迁移），再基于最新模板重建导出内容。
  if (!templatesLoaded.value) await loadTemplates()
  buildExport()
})
async function importJson() {
  let data: any
  try { data = JSON.parse(jsonText.value) } catch { return ElMessage.error('JSON 格式错误') }
  const list = Array.isArray(data) ? data : data?.profiles
  if (!Array.isArray(list)) return ElMessage.error('没有找到 profiles 数组')
  const valid = list.filter((x: any) => x && typeof x.name === 'string' && x.name.trim())
  if (!valid.length) return ElMessage.warning('没有可导入的账号')
  const confirmed = await ElMessageBox.confirm(`检测到 ${valid.length} 个账号，将以“新账号”方式导入，不覆盖现有账号。`, '导入预检查', { type: 'info' }).catch(() => false)
  if (!confirmed) return
  importBusy.value = true
  try {
    // 与批量创建一样走单次原子提交：整批校验通过才落盘。
    const drafts = valid.map((x: any) => ({
      name: x.name,
      default_url: x.default_url || props.appSettings?.global_default_url || '',
      url_mode: x.url_mode === 'custom' ? 'custom' : 'inherit',
      incognito: !!x.incognito,
      proxy: x.proxy || '',
      user_agent: x.user_agent || '',
      timezone: x.timezone || '',
      locale: x.locale || '',
      fingerprint_guard: x.fingerprint_guard !== false,
      group: x.group || '',
    }))
    const created = await invoke<Profile[]>('create_profiles_bulk', { drafts })
    if (Array.isArray(data?.templates)) {
      templates.value = data.templates
      await persistTemplates()
    }
    emit('changed')
    ElMessage.success(`成功导入 ${created.length} 个账号`)
  } catch (e) {
    ElMessage.error(`导入失败，未创建任何账号：${String(e)}`)
  } finally {
    importBusy.value = false
  }
}
</script>

<template>
  <el-dialog v-model="visible" title="账号管理工具 · v14" width="860px" append-to-body>
    <el-tabs>
      <el-tab-pane label="账号模板">
        <el-form label-width="110px">
          <el-form-item label="模板名称"><el-input v-model="templateForm.name" placeholder="例如：客服模板" /></el-form-item>
          <el-form-item label="主页来源"><el-segmented v-model="templateForm.url_mode" :options="[{label:'继承全局',value:'inherit'},{label:'单独覆盖',value:'custom'}]" /></el-form-item>
          <el-form-item v-if="templateForm.url_mode === 'custom'" label="默认网址"><el-input v-model="templateForm.default_url" placeholder="https://example.com/" /></el-form-item>
          <el-form-item label="默认分组"><el-input v-model="templateForm.group" /></el-form-item>
          <el-form-item label="代理"><el-input v-model="templateForm.proxy" placeholder="留空直连" /></el-form-item>
          <el-form-item label="时区"><el-input v-model="templateForm.timezone" placeholder="留空跟随系统，例如 Asia/Shanghai" /></el-form-item>
          <el-form-item label="语言 / 区域"><el-input v-model="templateForm.locale" placeholder="留空跟随系统，例如 zh-CN" /></el-form-item>
          <el-form-item label="User-Agent"><el-input v-model="templateForm.user_agent" /></el-form-item>
          <el-form-item><el-checkbox v-model="templateForm.incognito">隐身 / 临时账号</el-checkbox></el-form-item>
          <el-form-item><el-switch v-model="templateForm.fingerprint_guard" /><span style="margin-left:8px">指纹防护</span></el-form-item>
          <el-form-item><el-button type="primary" @click="saveTemplate">保存模板</el-button><el-button @click="resetTemplate">清空</el-button></el-form-item>
        </el-form>
        <el-table :data="templates" size="small" empty-text="暂无模板">
          <el-table-column prop="name" label="模板" /><el-table-column prop="group" label="分组" /><el-table-column label="主页"><template #default="{row}">{{ row.url_mode === 'inherit' ? '继承全局' : row.default_url }}</template></el-table-column>
          <el-table-column width="150"><template #default="{row}"><el-button link @click="editTemplate(row)">编辑</el-button><el-button link type="danger" @click="removeTemplate(row)">删除</el-button></template></el-table-column>
        </el-table>
      </el-tab-pane>
      <el-tab-pane label="批量创建">
        <el-form label-width="110px">
          <el-form-item label="名称前缀"><el-input v-model="batch.prefix" /></el-form-item>
          <el-form-item label="起始编号"><el-input-number v-model="batch.start" :min="0" /></el-form-item>
          <el-form-item label="创建数量"><el-input-number v-model="batch.count" :min="1" :max="200" /></el-form-item>
          <el-form-item label="编号位数"><el-input-number v-model="batch.digits" :min="1" :max="6" /></el-form-item>
          <el-form-item label="使用模板"><el-select v-model="batch.templateId" clearable placeholder="不使用模板"><el-option v-for="t in templates" :key="t.id" :label="t.name" :value="t.id" /></el-select></el-form-item>
          <el-form-item label="覆盖分组"><el-input v-model="batch.group" placeholder="留空使用模板分组" /></el-form-item>
          <el-form-item label="预览"><el-tag v-for="name in batchPreview" :key="name" style="margin:3px">{{ name }}</el-tag><span v-if="batch.count > 8">…</span></el-form-item>
          <el-form-item><el-button type="primary" :loading="batchBusy" @click="createBatch">批量创建</el-button></el-form-item>
        </el-form>
      </el-tab-pane>
      <el-tab-pane label="导入 / 导出">
        <el-alert title="导入采用新增模式，不覆盖现有账号；会话 Cookie/登录态不会导出。导入前会先预检查数量。" type="info" :closable="false" />
        <el-input v-model="jsonText" type="textarea" :rows="16" style="margin-top:12px" placeholder="粘贴 v14 JSON，或点击生成导出数据" />
        <div style="margin-top:12px"><el-button @click="buildExport">生成导出 JSON</el-button><el-button @click="copyExport">复制</el-button><el-button type="primary" :loading="importBusy" @click="importJson">预检查并导入</el-button></div>
      </el-tab-pane>
    </el-tabs>
  </el-dialog>
</template>
