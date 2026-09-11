<script setup lang="ts">
import { onBeforeUnmount, reactive, watch } from 'vue'
import { acquireWebviewSuppression } from '../lib/suppress'
import { BROWSER_TOOLBAR_BUILTINS, THEME_OPTIONS } from '../lib/uiPreferences'
import type {
  AppSettings,
  BrowserToolbarBuiltinId,
  CustomToolbarButton,
  ThemeId,
  UiPreferences,
} from '../types'

const visible = defineModel<boolean>({ required: true })

const props = defineProps<{
  uiPreferences: UiPreferences
  downloadSettings: AppSettings | null
}>()

const emit = defineEmits<{
  saveUi: [preferences: UiPreferences]
  saveDownloads: [settings: AppSettings]
}>()

function clonePreferences(value: UiPreferences): UiPreferences {
  return JSON.parse(JSON.stringify(value)) as UiPreferences
}

const uiForm = reactive<UiPreferences>(clonePreferences(props.uiPreferences))
const dlForm = reactive<AppSettings>({
  download_dir: '',
  auto_ai_download: false,
  skip_downloaded_files: true,
  confirm_delete_download: true,
  download_history_limit: 200,
  download_guard_seconds: 3,
  download_guard_rules: [],
  download_guard_auto_retry: true,
  ai_exts: '',
  global_default_url: 'https://chat01.ai/',
})

function hydrate() {
  Object.assign(uiForm, clonePreferences(props.uiPreferences))
  if (props.downloadSettings) Object.assign(dlForm, props.downloadSettings)
}

watch(
  () => [props.uiPreferences, props.downloadSettings] as const,
  hydrate,
  { deep: true, immediate: true },
)

let releaseDialog: (() => void) | null = null
watch(visible, (open) => {
  if (open) {
    hydrate()
    releaseDialog?.()
    releaseDialog = acquireWebviewSuppression()
  } else {
    releaseDialog?.()
    releaseDialog = null
  }
})

onBeforeUnmount(() => {
  releaseDialog?.()
  releaseDialog = null
})

function chooseTheme(theme: ThemeId) {
  uiForm.theme = theme
}

function builtinInfo(id: BrowserToolbarBuiltinId) {
  return BROWSER_TOOLBAR_BUILTINS.find((item) => item.id === id)!
}

function moveBuiltin(index: number, delta: number) {
  const target = index + delta
  if (target < 0 || target >= uiForm.browserToolbarItems.length) return
  const next = [...uiForm.browserToolbarItems]
  ;[next[index], next[target]] = [next[target], next[index]]
  uiForm.browserToolbarItems = next
}

function newCustomId() {
  return `custom-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`
}

function addCustomButton() {
  if (uiForm.customToolbarButtons.length >= 12) return
  uiForm.customToolbarButtons.push({
    id: newCustomId(),
    label: '快捷入口',
    icon: '🔗',
    url: 'https://',
    enabled: true,
  })
}

function moveCustom(index: number, delta: number) {
  const target = index + delta
  if (target < 0 || target >= uiForm.customToolbarButtons.length) return
  const next = [...uiForm.customToolbarButtons]
  ;[next[index], next[target]] = [next[target], next[index]]
  uiForm.customToolbarButtons = next
}

function removeCustom(index: number) {
  uiForm.customToolbarButtons.splice(index, 1)
}

function cleanCustomButtons(): CustomToolbarButton[] {
  return uiForm.customToolbarButtons.map((button) => ({
    ...button,
    label: button.label.trim(),
    icon: button.icon.trim(),
    url: button.url.trim(),
  }))
}

function addGuardRule() {
  if (dlForm.download_guard_rules.length >= 20) return
  dlForm.download_guard_rules.push({ domain: '', seconds: 0 })
}

function removeGuardRule(index: number) {
  dlForm.download_guard_rules.splice(index, 1)
}

function save() {
  emit('saveUi', {
    ...clonePreferences(uiForm),
    customToolbarButtons: cleanCustomButtons(),
  })
  if (props.downloadSettings) {
    emit('saveDownloads', {
      ...dlForm,
      download_dir: dlForm.download_dir.trim(),
      ai_exts: dlForm.ai_exts.trim(),
      download_guard_rules: dlForm.download_guard_rules
        .map((rule) => ({ domain: rule.domain.trim().toLowerCase(), seconds: Math.max(0, Math.min(60, Math.round(Number(rule.seconds) || 0))) }))
        .filter((rule) => rule.domain),
    })
  }
  visible.value = false
}
</script>

<template>
  <el-dialog v-model="visible" title="更多设置" width="820px" append-to-body class="preferences-dialog">
    <el-tabs>
      <el-tab-pane label="外观">
        <div class="settings-section">
          <div class="settings-section-head">
            <div>
              <strong>主题</strong>
              <small>选择更适合当前环境的视觉风格</small>
            </div>
          </div>
          <div class="theme-grid">
            <button
              v-for="theme in THEME_OPTIONS"
              :key="theme.id"
              class="theme-card"
              :class="{ active: uiForm.theme === theme.id }"
              type="button"
              @click="chooseTheme(theme.id)"
            >
              <span class="theme-swatches">
                <i v-for="swatch in theme.swatches" :key="swatch" :style="{ background: swatch }" />
              </span>
              <span class="theme-copy">
                <strong>{{ theme.name }}</strong>
                <small>{{ theme.description }}</small>
              </span>
              <span v-if="uiForm.theme === theme.id" class="theme-selected">已选</span>
            </button>
          </div>
        </div>

        <div class="settings-section settings-two-col">
          <label class="settings-field">
            <span>标签页宽度</span>
            <el-segmented v-model="uiForm.tabWidth" :options="[{ label: '紧凑', value: 'compact' }, { label: '标准', value: 'standard' }, { label: '宽', value: 'wide' }]" />
            <small>账号较多时建议使用紧凑；宽模式会优先展示更完整的账号名。</small>
          </label>
          <label class="settings-field">
            <span>运行时间模式</span>
            <el-segmented v-model="uiForm.runtimeMode" :options="[{ label: '本次打开', value: 'current-open' }, { label: '应用累计', value: 'app-total' }]" />
            <small>“应用累计”在本次应用运行期间累计 WebView 实际运行时间，休眠/手动暂停期间不计时。</small>
          </label>
          <label class="settings-switch-row settings-inline-switch">
            <span><strong>后台标签页自动休眠</strong><small>销毁长期不用的后台 WebView，标签和登录数据保留；切回时自动重建。</small></span>
            <el-switch v-model="uiForm.tabSleepEnabled" />
          </label>
          <label class="settings-field">
            <span>后台休眠等待时间</span>
            <el-input-number v-model="uiForm.tabSleepMinutes" :min="1" :max="1440" :step="1" controls-position="right" />
            <small>单位：分钟。固定标签不会自动休眠。</small>
          </label>
          <label class="settings-field">
            <span>自动刷新</span>
            <el-input-number v-model="uiForm.autoRefreshSeconds" :min="0" :max="86400" :step="1" controls-position="right" />
            <small>单位：秒。设置为 0 表示关闭；仅自动刷新当前正在查看的账号标签页，且只在 AI 正在生成回答时才刷。</small>
          </label>
          <label class="settings-switch-row settings-inline-switch">
            <span>
              <strong>刷新后恢复当前位置</strong>
              <small>刷新前记住滚动位置和 URL 锚点，页面加载后自动回到原位置。</small>
            </span>
            <el-switch v-model="uiForm.restorePositionAfterRefresh" />
          </label>
        </div>

        <div class="settings-section">
          <div class="settings-section-head">
            <div><strong>状态显示自由度</strong><small>按你的使用习惯决定界面显示哪些信息</small></div>
          </div>
          <div class="settings-switch-list settings-switch-grid">
            <label class="settings-switch-row"><span><strong>标签页 AI 状态</strong><small>生成中显示动态“...”，完成后显示完成提示</small></span><el-switch v-model="uiForm.showTabAiStatus" /></label>
            <label class="settings-switch-row"><span><strong>显示登录账号</strong><small>右侧账号状态卡显示邮箱/账号名</small></span><el-switch v-model="uiForm.showStatusAccount" /></label>
            <label class="settings-switch-row"><span><strong>显示代理出口</strong><small>右侧状态卡显示代理地区与 IP</small></span><el-switch v-model="uiForm.showStatusProxy" /></label>
            <label class="settings-switch-row"><span><strong>显示 AI 使用时间</strong><small>右侧状态卡显示最近一次 AI 完成回答的时间</small></span><el-switch v-model="uiForm.showStatusUpdatedTime" /></label>
          </div>
        </div>

        <div class="settings-section settings-two-col">
          <label class="settings-field">
            <span>界面密度</span>
            <el-segmented v-model="uiForm.density" :options="[{ label: '舒适', value: 'comfortable' }, { label: '紧凑', value: 'compact' }]" />
          </label>
          <label class="settings-field">
            <span>文字大小</span>
            <el-select v-model="uiForm.fontScale">
              <el-option :value="90" label="90% · 更紧凑" />
              <el-option :value="100" label="100% · 默认" />
              <el-option :value="110" label="110% · 更易读" />
            </el-select>
          </label>
          <label class="settings-switch-row">
            <span>
              <strong>界面动画</strong>
              <small>关闭后减少侧栏、悬停与弹层过渡</small>
            </span>
            <el-switch v-model="uiForm.animations" />
          </label>
        </div>
      </el-tab-pane>

      <el-tab-pane label="快捷工具栏">
        <div class="settings-note-card toolbar-note-card">
          <strong>工具栏现在属于应用外壳，不再注入网页</strong>
          <p>即使远程网页卡住、没有刷新或页面脚本加载失败，刷新/主页等按钮也会正常显示和响应。</p>
        </div>

        <div class="settings-section settings-two-col">
          <label class="settings-switch-row">
            <span><strong>显示快捷工具栏</strong><small>关闭后网页区域会自动占满空出来的空间</small></span>
            <el-switch v-model="uiForm.browserToolbarEnabled" />
          </label>
          <label class="settings-field">
            <span>工具栏位置</span>
            <el-segmented
              v-model="uiForm.browserToolbarPosition"
              :options="[{ label: '右侧', value: 'right' }, { label: '底部', value: 'bottom' }]"
            />
          </label>
          <label class="settings-switch-row settings-inline-switch">
            <span><strong>紧凑按钮</strong><small>减少工具栏占用空间，适合小窗口</small></span>
            <el-switch v-model="uiForm.browserToolbarCompact" />
          </label>
        </div>

        <div class="settings-section">
          <div class="settings-section-head">
            <div><strong>内置按钮</strong><small>可自由开关和调整顺序；从上到下对应右侧工具栏顺序</small></div>
          </div>
          <div class="toolbar-config-list">
            <div v-for="(item, index) in uiForm.browserToolbarItems" :key="item.id" class="toolbar-config-row">
              <el-switch v-model="item.enabled" />
              <div class="toolbar-config-copy">
                <strong>{{ builtinInfo(item.id).label }}</strong>
                <small>{{ builtinInfo(item.id).description }}</small>
              </div>
              <div class="toolbar-config-actions">
                <el-button size="small" :disabled="index === 0" @click="moveBuiltin(index, -1)">上移</el-button>
                <el-button size="small" :disabled="index === uiForm.browserToolbarItems.length - 1" @click="moveBuiltin(index, 1)">下移</el-button>
              </div>
            </div>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-head">
            <div><strong>自定义网址按钮</strong><small>最多 12 个。点击后在当前账号标签页内打开指定网址。</small></div>
            <el-button type="primary" plain size="small" :disabled="uiForm.customToolbarButtons.length >= 12" @click="addCustomButton">添加按钮</el-button>
          </div>

          <div v-if="uiForm.customToolbarButtons.length" class="custom-toolbar-list">
            <div v-for="(button, index) in uiForm.customToolbarButtons" :key="button.id" class="custom-toolbar-row">
              <el-switch v-model="button.enabled" />
              <el-input v-model="button.icon" maxlength="4" class="custom-toolbar-icon-input" placeholder="🔗" title="Emoji/短字符图标" />
              <el-input v-model="button.label" maxlength="30" class="custom-toolbar-label-input" placeholder="按钮名称" />
              <el-input v-model="button.url" maxlength="2048" class="custom-toolbar-url-input" placeholder="https://example.com/" />
              <div class="toolbar-config-actions compact-actions">
                <el-button size="small" :disabled="index === 0" @click="moveCustom(index, -1)">↑</el-button>
                <el-button size="small" :disabled="index === uiForm.customToolbarButtons.length - 1" @click="moveCustom(index, 1)">↓</el-button>
                <el-button size="small" type="danger" plain @click="removeCustom(index)">删除</el-button>
              </div>
            </div>
          </div>
          <div v-else class="toolbar-empty-state">暂无自定义按钮。可以添加常用站点、后台入口或工作页面。</div>
        </div>
      </el-tab-pane>

      <el-tab-pane label="账号默认">
        <div class="settings-section">
          <div class="settings-section-head"><div><strong>全局默认账号网址</strong><small>新建账号默认继承；修改后所有“继承全局”的账号会立即使用新主页。</small></div></div>
          <label class="settings-field"><span>默认主页</span><el-input v-model="dlForm.global_default_url" placeholder="https://example.com/" clearable /><small>支持 http/https；留空保存时后端会恢复应用默认主页。</small></label>
        </div>
      </el-tab-pane>
      <el-tab-pane label="下载">
        <div class="settings-section settings-two-col">
          <label class="settings-field">
            <span>下载历史每页行数</span>
            <el-select v-model="uiForm.downloadRowsPerPage">
              <el-option :value="10" label="10 条" />
              <el-option :value="20" label="20 条" />
              <el-option :value="50" label="50 条" />
              <el-option :value="100" label="100 条" />
            </el-select>
          </label>
          <label class="settings-field">
            <span>下载历史默认视图</span>
            <el-segmented v-model="uiForm.downloadView" :options="[{ label: '按账号分组', value: 'grouped' }, { label: '列表', value: 'flat' }]" />
          </label>
        </div>

        <div class="settings-section">
          <label class="settings-field">
            <span>下载保存目录</span>
            <el-input v-model="dlForm.download_dir" placeholder="默认：应用数据目录/downloads" />
          </label>
          <label class="settings-field">
            <span>下载历史保留上限</span>
            <el-select v-model="dlForm.download_history_limit">
              <el-option :value="100" label="100 条" />
              <el-option :value="200" label="200 条" />
              <el-option :value="500" label="500 条" />
              <el-option :value="1000" label="1000 条" />
            </el-select>
            <small>数量越大越方便追溯，但会增加少量本地存储与渲染开销。</small>
          </label>
          <div class="settings-switch-list">
            <label class="settings-switch-row">
              <span><strong>自动下载 AI 生成的文件</strong><small>检测到符合条件的新下载链接后自动下载</small></span>
              <el-switch v-model="dlForm.auto_ai_download" />
            </label>
            <label class="settings-switch-row">
              <span><strong>永久禁止重复下载</strong><small>成功下载过一次后永久拦截，即使删除下载历史或本地文件也不会再次下载</small></span>
              <el-switch v-model="dlForm.skip_downloaded_files" disabled />
            </label>
            <label class="settings-switch-row">
              <span><strong>删除前确认</strong><small>单个和批量删除下载文件前显示确认</small></span>
              <el-switch v-model="dlForm.confirm_delete_download" />
            </label>
          </div>
          <label class="settings-field">
            <span>页面打开后的下载保护</span>
            <el-input-number v-model="dlForm.download_guard_seconds" :min="0" :max="60" :step="1" controls-position="right" />
            <small>单位：秒。默认 3 秒；在页面开始加载及加载完成后的保护期内，手动/自动下载都会被后端硬拦截。设为 0 可关闭。</small>
          </label>
          <label class="settings-switch-row">
            <span><strong>保护期下载自动排队</strong><small>下载在保护期内被拦截时加入一次性队列，保护期结束后自动重试，无需再次点击。</small></span>
            <el-switch v-model="dlForm.download_guard_auto_retry" />
          </label>
          <div class="settings-field">
            <span>下载保护域名白名单 / 单独延迟</span>
            <small>按下载链接域名匹配；例如 files.chat01.ai = 0 秒表示该可信域名不等待。子域名也会匹配。</small>
            <div class="guard-rule-list">
              <div v-for="(rule, index) in dlForm.download_guard_rules" :key="index" class="guard-rule-row">
                <el-input v-model="rule.domain" placeholder="files.example.com" />
                <el-input-number v-model="rule.seconds" :min="0" :max="60" :step="1" controls-position="right" />
                <el-button text type="danger" @click="removeGuardRule(index)">删除</el-button>
              </div>
              <el-button size="small" @click="addGuardRule">+ 添加域名规则</el-button>
            </div>
          </div>
          <label class="settings-field">
            <span>自动下载扩展名白名单</span>
            <el-input v-model="dlForm.ai_exts" placeholder="md,txt,csv,docx,pdf,png,zip" />
            <small>逗号分隔；留空表示允许全部扩展名。启用白名单后，没有可识别扩展名或扩展名不在列表中的链接不会自动下载。</small>
          </label>
        </div>
      </el-tab-pane>

      <el-tab-pane label="体验">
        <div class="settings-note-card">
          <strong>已纳入本版的体验优化</strong>
          <p>宿主快捷工具栏永久可见并与网页解耦；支持按钮开关/排序、自定义网址入口、右侧/底部布局和紧凑模式；同时保留自动刷新、刷新位置恢复与下载管理。</p>
        </div>
        <div class="settings-note-card secondary">
          <strong>仍建议后续加入</strong>
          <p>当前版本已加入标签休眠、运行时间控制、下载域名规则/队列、标签宽度/排序/固定和启动恢复。后续建议继续做快捷键中心、批量导入导出与更完整的下载失败重试面板。</p>
        </div>
      </el-tab-pane>
    </el-tabs>

    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" @click="save">保存设置</el-button>
    </template>
  </el-dialog>
</template>
