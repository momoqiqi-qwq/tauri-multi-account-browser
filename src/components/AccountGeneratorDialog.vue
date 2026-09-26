<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, Check, Connection, MagicStick, Refresh, Search } from '@element-plus/icons-vue'
import type { Profile, ProfileStatus } from '../types'
import { useDialogSuppression } from '../lib/suppress'
import { useAccountGenerator } from '../composables/useAccountGenerator'
import '../account-generator.css'

const visible = defineModel<boolean>({ required: true })
const props = defineProps<{ profiles: Profile[]; globalDefaultUrl: string; initialUrl?: string; statuses: Record<string, ProfileStatus> }>()
const emit = defineEmits<{ created: [profiles: Profile[]]; refreshed: [profiles: Profile[]]; browse: [] }>()
useDialogSuppression(() => visible.value)
const {
  step, urls, selectedIds, selected, sourceQuery, sources, filteredSources, selectedSources,
  missingIds, busy, busyLabel, error, needsRecheck, result, options, parsed, plan, canGenerate,
  allSelected, loginAccount,
  toggleSource, selectAll, selectFiltered, clearSelection, nextStep, goStep, refreshSources, generate, close,
} = useAccountGenerator(visible, () => props.profiles, () => props.initialUrl, () => props.globalDefaultUrl, () => props.statuses, created => emit('created', created), fresh => emit('refreshed', fresh))
const advanced = ref<string[]>([])
const steps = ['网站与命名', '选择来源账号', '确认并生成']
const mainPanel = ref<HTMLElement | null>(null)
const previewTable = ref<HTMLElement | null>(null)
const previewPage = ref(1)
watch(step, async () => {
  previewPage.value = 1
  await nextTick()
  mainPanel.value?.scrollTo({ top: 0 })
  mainPanel.value?.querySelector<HTMLElement>('textarea, input:not(:disabled)')?.focus({ preventScroll: true })
})
watch(previewPage, () => { previewTable.value?.scrollTo({ top: 0 }) })
const previewRows = computed(() => plan.value.rows.slice((previewPage.value - 1) * 10, previewPage.value * 10))
watch(() => plan.value.rows.length, () => { previewPage.value = 1 })
const temporaryCount = computed(() => selectedSources.value.filter(profile => profile.incognito).length)
const proxyCount = computed(() => selectedSources.value.filter(profile => profile.proxy.trim()).length)
function beforeClose(done: () => void) { if (!busy.value) done() }
function advance() { previewPage.value = 1; nextStep() }
</script>

<template>
  <el-dialog v-model="visible" class="account-generator" title="多账号生成器" width="1120px"
    append-to-body :close-on-click-modal="false" :close-on-press-escape="!busy" :show-close="!busy" :before-close="beforeClose">
    <template #header="{ titleId }">
      <div class="ag-heading"><span class="ag-heading__icon"><el-icon><MagicStick /></el-icon></span><div>
        <h2 :id="titleId">多账号生成器</h2><p>为多个网站，按所选账号配置创建独立浏览器容器。</p>
      </div></div>
    </template>

    <div v-if="result" class="ag-success">
      <span class="ag-success__icon"><el-icon><Check /></el-icon></span>
      <h3>已创建 {{ result.length }} 个账号</h3>
      <p>原账号保持不变，新账号已按主页自动归入网站导航。</p>
      <p class="ag-muted">登录态未复制，也没有自动打开网页。进入新账号后需要自行登录。</p>
      <div class="ag-result-list" aria-label="已生成账号">
        <div v-for="profile in result" :key="profile.id"><strong>{{ profile.name }}</strong><small>{{ profile.default_url }}</small></div>
      </div>
    </div>
    <template v-else>
      <nav class="ag-steps" aria-label="生成步骤">
        <button v-for="(label, i) in steps" :key="label" :disabled="busy || i + 1 > step + 1" :class="{ active: step === i + 1, complete: step > i + 1 }"
          :aria-current="step === i + 1 ? 'step' : undefined" @click="goStep(i + 1)">
          <span class="ag-step-number"><el-icon v-if="step > i + 1"><Check /></el-icon><template v-else>{{ i + 1 }}</template></span><span>{{ label }}</span>
        </button>
      </nav>
      <div class="ag-layout" :aria-busy="busy">
        <p class="ag-compact-summary">{{ parsed.targets.length }} 个网站 × {{ selectedIds.length }} 个来源 · 新建 {{ plan.drafts.length }} · 跳过 {{ plan.skipped }}</p>
        <section ref="mainPanel" class="ag-main" :aria-label="steps[step - 1]">
          <div v-if="step === 1" class="ag-step">
            <div class="ag-section-title"><h3>先填写目标网站</h3><p>一个网站对应一组新账号；名称可以随时在下一步预览。</p></div>
            <el-form label-position="top" :disabled="busy">
              <el-form-item label="目标网址" required>
                <el-input v-model="urls" type="textarea" :rows="5" resize="vertical" :maxlength="40000"
                  aria-label="目标网址" :placeholder="'Lovart | https://www.lovart.ai/home\nClaude | https://claude.ai/'" />
                <div class="ag-input-note"><span>每行一个网址，或使用「名称 | 网址」。</span><span>{{ parsed.targets.length }} 个网站<span v-if="parsed.duplicates"> · 合并 {{ parsed.duplicates }} 条重复网址</span></span></div>
              </el-form-item>
              <div v-if="parsed.errors.length" class="ag-notice ag-notice--error" role="alert"><p v-for="message in parsed.errors.slice(0, 4)" :key="message">{{ message }}</p><small v-if="parsed.errors.length > 4">还有 {{ parsed.errors.length - 4 }} 项错误，请逐行检查。</small></div>
              <div class="ag-form-grid">
                <el-form-item label="公共名称前缀（可选）"><el-input v-model="options.prefix" maxlength="40" placeholder="例如：设计项目" aria-label="公共名称前缀" /></el-form-item>
                <el-form-item label="命名顺序"><el-select v-model="options.order" aria-label="命名顺序"><el-option label="账号名称 · 网站名称" value="source-site" /><el-option label="网站名称 · 账号名称" value="site-source" /></el-select></el-form-item>
              </div>
              <el-collapse v-model="advanced" class="ag-advanced">
                <el-collapse-item title="更多命名与分组设置" name="naming">
                  <div class="ag-form-grid">
                    <el-form-item label="名称分隔符"><el-select v-model="options.separator" aria-label="名称分隔符"><el-option label="间隔点 ·" value=" · " /><el-option label="短横线 -" value=" - " /><el-option label="斜线 /" value=" / " /><el-option label="空格" value=" " /></el-select></el-form-item>
                    <el-form-item label="统一账号分组（可选）"><el-input v-model="options.group" maxlength="80" placeholder="留空沿用来源账号分组" aria-label="统一账号分组" /></el-form-item>
                  </div>
                  <el-checkbox v-model="options.copyTags">保留来源账号的标签</el-checkbox>
                </el-collapse-item>
              </el-collapse>
            </el-form>
          </div>

          <div v-else-if="step === 2" class="ag-step ag-step--list">
            <div class="ag-section-title"><h3>选择要沿用的账号配置</h3><p>继承代理、User-Agent、时区、语言与隔离设置，不复制账号会话。</p></div>
            <div class="ag-source-toolbar">
              <el-input v-model="sourceQuery" :prefix-icon="Search" clearable placeholder="搜索账号、分组、标签或登录邮箱" aria-label="搜索来源账号" :disabled="busy" />
              <el-button :icon="Refresh" :loading="busy" @click="refreshSources">刷新</el-button>
            </div>
            <div class="ag-selection-bar"><span>已选 <strong>{{ selectedIds.length }}</strong> 个 · 当前显示 {{ filteredSources.length }} 个</span><div><el-button text :disabled="busy || !sources.length || allSelected" @click="selectAll">全选</el-button><el-button v-if="sourceQuery.trim()" text :disabled="busy || !filteredSources.length" @click="selectFiltered">选中搜索结果</el-button><el-button text :disabled="busy || !selectedIds.length" @click="clearSelection">清空选择</el-button></div></div>
            <div v-if="missingIds.length" class="ag-notice ag-notice--error" role="alert">{{ missingIds.length }} 个来源账号已删除，请清空后重新选择。</div>
            <div class="ag-source-list">
              <el-checkbox v-for="profile in filteredSources" :key="profile.id" class="ag-source-row" :class="{ selected: selected.has(profile.id) }"
                :model-value="selected.has(profile.id)" :disabled="busy" :aria-label="`选择来源：${profile.name}`" @change="(value: string | number | boolean) => toggleSource(profile.id, !!value)">
                <span class="ag-source-avatar">{{ [...profile.name][0] || 'A' }}</span>
                <span class="ag-source-copy"><strong>{{ profile.name }}</strong><small>{{ profile.group || '未分组' }}<span v-if="profile.timezone"> · {{ profile.timezone }}</span></small><small v-if="loginAccount(profile.id)" class="ag-source-login" :title="loginAccount(profile.id)">{{ loginAccount(profile.id) }}</small></span>
                <span class="ag-source-badges"><span>{{ profile.proxy ? '沿用代理' : '直连' }}</span><span v-if="profile.incognito">临时</span></span>
              </el-checkbox>
              <div v-if="!filteredSources.length" class="ag-list-empty">{{ sourceQuery ? '没有匹配的账号，试试其他关键词。' : '还没有来源账号，请先新建一个账号并配置好参数。' }}</div>
            </div>
            <p class="ag-muted">筛选不会清除已选项。不同新容器仍可能使用相同代理出口，并不代表不同 IP。</p>
          </div>

          <div v-else class="ag-step ag-step--list">
            <div class="ag-section-title"><h3>确认即将生成的账号</h3><p>核对名称和网站后再提交。仅创建新账号，不修改来源账号。</p></div>
            <div class="ag-dedupe"><el-checkbox v-model="options.skipExisting" :disabled="busy" @change="previewPage = 1">跳过同名且主页相同的账号</el-checkbox><small>名称精确匹配，主页按完整 URL 比较；关闭后会为重名追加序号。</small></div>
            <div ref="previewTable" class="ag-preview-table">
              <table><thead><tr><th>新账号名称</th><th>来源账号</th><th>目标网站</th><th>处理</th></tr></thead><tbody>
                <tr v-for="row in previewRows" :key="row.key" :class="{ skipped: row.skipped }"><td><strong>{{ row.draft.name }}</strong><small v-if="row.renamed">已追加重名序号</small></td><td>{{ row.sourceName }}</td><td><span>{{ row.target.name }}</span><small :title="row.target.url">{{ row.target.url }}</small></td><td><span class="ag-row-status" :class="{ skip: row.skipped }">{{ row.skipped ? '跳过' : '新建' }}</span></td></tr>
              </tbody></table>
              <div v-if="!plan.rows.length" class="ag-list-empty">请返回前两步完成网站与来源账号设置。</div>
            </div>
            <el-pagination v-if="plan.rows.length > 10" v-model:current-page="previewPage" :page-size="10" :total="plan.rows.length" :disabled="busy" layout="prev, pager, next" small />
            <div v-if="plan.errors.length" class="ag-notice ag-notice--error" role="alert"><p v-for="message in plan.errors.slice(0, 3)" :key="message">{{ message }}</p></div>
            <p v-else-if="!plan.drafts.length" class="ag-notice">所有组合都已存在，无需重复生成。</p>
          </div>
        </section>

        <aside class="ag-summary" aria-label="生成摘要">
          <span class="ag-eyebrow">本次生成</span>
          <div class="ag-equation"><span><strong>{{ parsed.targets.length }}</strong><small>个网站</small></span><b>×</b><span><strong>{{ selectedIds.length }}</strong><small>个来源账号</small></span></div>
          <dl><div><dt>组合总数</dt><dd>{{ plan.combinations }}</dd></div><div><dt>跳过已有</dt><dd>{{ plan.skipped }}</dd></div><div class="ag-summary__total"><dt>预计新建</dt><dd>{{ plan.drafts.length }}</dd></div></dl>
          <p class="ag-muted">单次最多 200 个组合。生成前会重新检查账号配置和重复项。</p>
          <hr />
          <h4>配置与会话</h4>
          <ul><li>沿用来源配置，创建新容器</li><li>不复制 Cookie、登录态或历史</li><li>不自动打开生成的网页</li><li>{{ options.copyTags ? '保留来源标签' : '不复制来源标签' }}</li><li>{{ options.group.trim() ? `统一分组：${options.group.trim()}` : '沿用来源账号分组' }}</li></ul>
          <p v-if="selectedIds.length" class="ag-muted">{{ proxyCount }} 个来源配置了代理<span v-if="temporaryCount">；{{ temporaryCount }} 个为临时账号，其临时模式也会沿用</span>。</p>
          <p v-if="step === 2 && selectedIds.length && plan.errors.length" class="ag-notice ag-notice--error" role="alert">{{ plan.errors[0] }}</p>
        </aside>
      </div>
    </template>

    <template #footer>
      <div class="ag-footer">
        <div class="ag-footer__message"><p v-if="busy" role="status">{{ busyLabel }}</p><p v-else-if="error" class="ag-error-text" role="alert">{{ error }}</p><p v-else>{{ result ? '生成完成，原账号与登录会话保持不变。' : '只创建新的独立容器，不会注册网站账号。' }}</p></div>
        <div class="ag-footer__actions">
          <template v-if="result"><el-button @click="close">完成</el-button><el-button type="primary" :icon="Connection" @click="emit('browse')">查看网站导航</el-button></template>
          <template v-else>
            <el-button :disabled="busy" @click="close">取消</el-button>
            <el-button v-if="step > 1" :icon="ArrowLeft" :disabled="busy" @click="goStep(step - 1)">上一步</el-button>
            <el-button v-if="step < 3" type="primary" :disabled="busy" :icon="ArrowRight" @click="advance">{{ step === 1 ? '选择来源账号' : '预览生成结果' }}</el-button>
            <el-button v-else-if="needsRecheck" type="primary" :loading="busy" @click="refreshSources">重新检查账号</el-button>
            <el-button v-else type="primary" :loading="busy" :disabled="!canGenerate" :icon="MagicStick" @click="generate">生成 {{ plan.drafts.length }} 个账号</el-button>
          </template>
        </div>
      </div>
    </template>
  </el-dialog>
</template>
