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

// ---- 导入 / 导出 ----

interface ImportRowError { row: number; name: string; message: string }
interface ImportSkipped { row: number; name: string; reason: string }
interface ImportReport { created: Profile[]; updated: Profile[]; skipped: ImportSkipped[]; errors: ImportRowError[] }

const jsonText = ref('')
const importBusy = ref(false)
const fileBusy = ref(false)
/** json：完整配置（含模板）；csv：只有账号列表。 */
const dataFormat = ref<'json' | 'csv'>('json')
/** 同名账号的处理方式。 */
const conflictStrategy = ref<'rename' | 'skip' | 'overwrite'>('rename')
const lastReport = ref<ImportReport | null>(null)
const reportVisible = ref(false)

/** 把现有账号转成导入草稿，用于导出 CSV。 */
function profilesToDrafts() {
  return props.profiles.map(p => ({
    name: p.name,
    default_url: p.default_url || '',
    url_mode: p.url_mode || 'inherit',
    incognito: !!p.incognito,
    proxy: p.proxy || '',
    user_agent: p.user_agent || '',
    timezone: p.timezone || '',
    locale: p.locale || '',
    fingerprint_guard: p.fingerprint_guard !== false,
    group: p.group || '',
  }))
}

async function buildExport() {
  if (dataFormat.value === 'csv') {
    jsonText.value = await invoke<string>('build_accounts_csv', { drafts: profilesToDrafts() })
  } else {
    jsonText.value = JSON.stringify({ version: 17, exported_at: new Date().toISOString(), profiles: props.profiles, templates: templates.value }, null, 2)
  }
}
async function copyExport() { if (!jsonText.value) await buildExport(); await navigator.clipboard.writeText(jsonText.value); ElMessage.success('内容已复制') }

watch(visible, async open => {
  if (!open) return
  // 打开时读取模板（含一次性迁移），再基于最新模板重建导出内容。
  if (!templatesLoaded.value) await loadTemplates()
  await buildExport()
})

/** 从当前文本框解析出导入草稿；失败返回 null 并已提示。 */
async function parseDrafts(): Promise<any[] | null> {
  const text = jsonText.value.trim()
  if (!text) { ElMessage.warning('请先粘贴内容或从文件读取'); return null }
  if (dataFormat.value === 'csv') {
    // CSV 交给 Rust 解析：引号/逗号/换行的转义规则自己实现很容易出错。
    const drafts = await invoke<any[]>('parse_accounts_csv', { text })
    return drafts.filter(x => x && typeof x.name === 'string' && x.name.trim())
  }
  let data: any
  try { data = JSON.parse(text) } catch { ElMessage.error('JSON 格式错误'); return null }
  const list = Array.isArray(data) ? data : data?.profiles
  if (!Array.isArray(list)) { ElMessage.error('没有找到 profiles 数组'); return null }
  return list
    .filter((x: any) => x && typeof x.name === 'string' && x.name.trim())
    .map((x: any) => ({
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
}

async function runImport() {
  const drafts = await parseDrafts()
  if (!drafts || !drafts.length) return ElMessage.warning('没有可导入的账号')
  const conflictLabel = { rename: '自动改名', skip: '跳过已有', overwrite: '覆盖同名账号' }[conflictStrategy.value]
  const confirmed = await ElMessageBox.confirm(
    `检测到 ${drafts.length} 个账号。同名账号处理方式：${conflictLabel}。`,
    '导入预检查', { type: 'info' },
  ).catch(() => false)
  if (!confirmed) return
  importBusy.value = true
  try {
    // 逐行尽力而为：坏行只记进 errors，其余照常导入，结果逐行反馈。
    const report = await invoke<ImportReport>('import_profiles', {
      drafts,
      options: { conflict: conflictStrategy.value, dryRun: false },
    })
    lastReport.value = report
    emit('changed')
    await buildExport()
    if (report.errors.length || report.skipped.length) {
      reportVisible.value = true
    }
    const parts = [`新建 ${report.created.length}`]
    if (report.updated.length) parts.push(`覆盖 ${report.updated.length}`)
    if (report.skipped.length) parts.push(`跳过 ${report.skipped.length}`)
    if (report.errors.length) parts.push(`失败 ${report.errors.length}`)
    ElMessage.success(`导入完成：${parts.join('，')}`)
  } catch (e) {
    ElMessage.error(`导入失败：${String(e)}`)
  } finally {
    importBusy.value = false
  }
}

/** 用系统原生对话框选文件读入，省得手动复制粘贴。 */
async function importFromFile() {
  const path = await invoke<string | null>('pick_import_file')
  if (!path) return
  fileBusy.value = true
  try {
    jsonText.value = await invoke<string>('read_import_file', { path })
    dataFormat.value = path.toLowerCase().endsWith('.csv') ? 'csv' : 'json'
    ElMessage.success('已读取文件，请点“预检查并导入”')
  } catch (e) {
    ElMessage.error(`读取文件失败：${String(e)}`)
  } finally {
    fileBusy.value = false
  }
}

/** 用系统原生"另存为"对话框导出到文件。 */
async function exportToFile() {
  if (!jsonText.value) await buildExport()
  const defaultName = dataFormat.value === 'csv' ? 'accounts.csv' : 'accounts.json'
  const path = await invoke<string | null>('pick_export_file', { defaultName })
  if (!path) return
  fileBusy.value = true
  try {
    await invoke('write_export_file', { path, content: jsonText.value })
    ElMessage.success('已导出到文件')
  } catch (e) {
    ElMessage.error(`导出失败：${String(e)}`)
  } finally {
    fileBusy.value = false
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
        <el-alert title="会话 Cookie / 登录态不会导出。CSV 只含账号，不含模板；JSON 含模板。导入前会先预检查并显示逐行结果。" type="info" :closable="false" />
        <el-form label-width="110px" style="margin-top:12px">
          <el-form-item label="数据格式">
            <el-segmented v-model="dataFormat" :options="[{label:'JSON',value:'json'},{label:'CSV',value:'csv'}]" @change="buildExport" />
          </el-form-item>
          <el-form-item label="同名处理">
            <el-segmented v-model="conflictStrategy" :options="[{label:'自动改名',value:'rename'},{label:'跳过',value:'skip'},{label:'覆盖',value:'overwrite'}]" />
          </el-form-item>
        </el-form>
        <div style="margin-bottom:8px">
          <el-button :loading="fileBusy" @click="importFromFile">从文件导入…</el-button>
          <el-button :loading="fileBusy" @click="exportToFile">导出到文件…</el-button>
          <el-button @click="buildExport">重新生成</el-button>
          <el-button @click="copyExport">复制</el-button>
        </div>
        <el-input v-model="jsonText" type="textarea" :rows="14" placeholder="粘贴 JSON / CSV，或点“从文件导入”选择文件" />
        <div style="margin-top:12px"><el-button type="primary" :loading="importBusy" @click="runImport">预检查并导入</el-button></div>
      </el-tab-pane>
    </el-tabs>

    <!-- 逐行导入结果：坏行要能对着原文件定位，不能只丢一句"导入失败" -->
    <el-dialog v-model="reportVisible" title="导入结果明细" width="720px" append-to-body>
      <el-alert v-if="lastReport" :closable="false" type="info" :title="`新建 ${lastReport.created.length} · 覆盖 ${lastReport.updated.length} · 跳过 ${lastReport.skipped.length} · 失败 ${lastReport.errors.length}`" />
      <template v-if="lastReport?.errors.length">
        <h4 style="margin:16px 0 8px">失败的行</h4>
        <el-table :data="lastReport.errors" size="small" max-height="260">
          <el-table-column prop="row" label="行" width="70" />
          <el-table-column prop="name" label="账号" />
          <el-table-column prop="message" label="原因" />
        </el-table>
      </template>
      <template v-if="lastReport?.skipped.length">
        <h4 style="margin:16px 0 8px">跳过的行</h4>
        <el-table :data="lastReport.skipped" size="small" max-height="200">
          <el-table-column prop="row" label="行" width="70" />
          <el-table-column prop="name" label="账号" />
          <el-table-column prop="reason" label="原因" />
        </el-table>
      </template>
    </el-dialog>
  </el-dialog>
</template>
