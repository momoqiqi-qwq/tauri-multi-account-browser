import type {
  BrowserToolbarBuiltinId,
  BrowserToolbarItem,
  CustomToolbarButton,
  ThemeId,
  UiPreferences,
} from '../types'

export const THEME_OPTIONS: Array<{
  id: ThemeId
  name: string
  description: string
  swatches: [string, string, string]
}> = [
  { id: 'classic', name: '经典蓝灰', description: '保留原版熟悉的深蓝侧栏', swatches: ['#f5f7fb', '#111827', '#2563eb'] },
  { id: 'claude-light', name: 'Claude 浅色', description: '暖白、克制、阅读友好', swatches: ['#f7f5f2', '#ffffff', '#cc785c'] },
  { id: 'claude-dark', name: 'Claude 深色', description: '暖黑背景与柔和橙色强调', swatches: ['#1f1e1d', '#292725', '#d9896a'] },
  { id: 'midnight-blue', name: '午夜蓝', description: '深海蓝黑，适合夜间工作', swatches: ['#07111f', '#0d1b2a', '#4f8cff'] },
  { id: 'graphite', name: '石墨黑', description: '中性黑灰，专注且低干扰', swatches: ['#151515', '#222222', '#a3a3a3'] },
  { id: 'glass-blue', name: '玻璃蓝', description: '通透冷蓝与玻璃质感', swatches: ['#eaf4ff', '#f7fbff', '#3182ce'] },
  { id: 'paper-cream', name: '纸张奶油', description: '纸张质感，长时间阅读舒适', swatches: ['#f7f1e3', '#fffaf0', '#9a6b3f'] },
  { id: 'amethyst', name: '紫晶', description: '深紫底色与紫晶高光', swatches: ['#171224', '#241b36', '#a78bfa'] },
  { id: 'win11', name: 'Win11', description: '清爽浅色、圆角与系统蓝', swatches: ['#f3f6fb', '#ffffff', '#0f6cbd'] },
  { id: 'sunset', name: '日落橙紫', description: '暖橙与暮紫渐变氛围', swatches: ['#2a1830', '#3a203d', '#f58a5c'] },
  { id: 'mint-light', name: '薄荷浅色', description: '轻盈薄荷绿，清新明亮', swatches: ['#eefaf5', '#ffffff', '#2f9e78'] },
]

const STORAGE_KEY = 'mab-ui-preferences-v1'

export const BROWSER_TOOLBAR_BUILTINS: Array<{
  id: BrowserToolbarBuiltinId
  label: string
  description: string
  defaultEnabled: boolean
}> = [
  { id: 'reload', label: '刷新', description: '刷新当前账号页面，并遵守“刷新后恢复当前位置”设置', defaultEnabled: true },
  { id: 'home', label: '主页', description: '回到当前账号配置的默认打开网址', defaultEnabled: true },
  { id: 'back', label: '后退', description: '当前账号浏览历史后退', defaultEnabled: false },
  { id: 'forward', label: '前进', description: '当前账号浏览历史前进', defaultEnabled: false },
  { id: 'copy-url', label: '复制网址', description: '复制当前账号正在浏览的网址', defaultEnabled: true },
  { id: 'overview', label: '账号总览', description: '打开全部账号状态总览', defaultEnabled: true },
  { id: 'downloads', label: '下载', description: '打开下载历史', defaultEnabled: true },
  { id: 'settings', label: '设置', description: '打开更多设置', defaultEnabled: true },
]

const BUILTIN_IDS = new Set<BrowserToolbarBuiltinId>(BROWSER_TOOLBAR_BUILTINS.map((item) => item.id))

function normalizeToolbarItems(value: BrowserToolbarItem[] | undefined): BrowserToolbarItem[] {
  const result: BrowserToolbarItem[] = []
  const seen = new Set<BrowserToolbarBuiltinId>()
  if (Array.isArray(value)) {
    for (const raw of value) {
      if (!raw || !BUILTIN_IDS.has(raw.id) || seen.has(raw.id)) continue
      seen.add(raw.id)
      result.push({ id: raw.id, enabled: raw.enabled !== false })
    }
  }
  for (const item of BROWSER_TOOLBAR_BUILTINS) {
    if (!seen.has(item.id)) result.push({ id: item.id, enabled: item.defaultEnabled })
  }
  return result
}

function normalizeCustomToolbarButtons(value: CustomToolbarButton[] | undefined): CustomToolbarButton[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<string>()
  return value.slice(0, 12).map((raw, index) => {
    const base = String(raw?.id || `custom-${index + 1}`)
      .trim()
      .replace(/[^a-zA-Z0-9_-]/g, '')
      .slice(0, 48) || `custom-${index + 1}`
    let id = base
    let suffix = 2
    while (seen.has(id)) id = `${base}-${suffix++}`
    seen.add(id)
    return {
      id,
      label: String(raw?.label || '').trim().slice(0, 30),
      icon: String(raw?.icon || '').trim().slice(0, 4),
      url: String(raw?.url || '').trim().slice(0, 2048),
      enabled: raw?.enabled !== false,
    }
  })
}

export const DEFAULT_UI_PREFERENCES: UiPreferences = {
  theme: 'classic',
  density: 'comfortable',
  fontScale: 100,
  animations: true,
  autoRefreshSeconds: 0,
  browserToolbarEnabled: true,
  browserToolbarPosition: 'right',
  browserToolbarCompact: false,
  browserToolbarItems: BROWSER_TOOLBAR_BUILTINS.map((item) => ({ id: item.id, enabled: item.defaultEnabled })),
  customToolbarButtons: [],
  restorePositionAfterRefresh: true,
  showTabAiStatus: true,
  showStatusAccount: true,
  showStatusProxy: true,
  showAiUsageTime: true,
  downloadRowsPerPage: 20,
  downloadView: 'grouped',
  downloadColumns: 2,
  tabWidth: 'standard',
  tabSleepEnabled: true,
  tabSleepMinutes: 15,
  runtimeMode: 'current-open',
}

const themeIds = new Set(THEME_OPTIONS.map((theme) => theme.id))
const rowOptions = new Set([10, 20, 50, 100])
/** 每行列数：列太多会让账号卡片窄到放不下文件名和操作按钮，所以封顶 3 列。 */
const columnOptions = new Set([1, 2, 3])

export function normalizeUiPreferences(value: Partial<UiPreferences> | null | undefined): UiPreferences {
  const theme = value?.theme && themeIds.has(value.theme) ? value.theme : DEFAULT_UI_PREFERENCES.theme
  const density = value?.density === 'compact' ? 'compact' : 'comfortable'
  const fontScale = [90, 100, 110].includes(Number(value?.fontScale)) ? Number(value?.fontScale) : 100
  const autoRefreshSecondsRaw = Number(value?.autoRefreshSeconds)
  const autoRefreshSeconds = Number.isFinite(autoRefreshSecondsRaw)
    ? Math.min(86400, Math.max(0, Math.round(autoRefreshSecondsRaw)))
    : 0
  const downloadRowsPerPage = rowOptions.has(Number(value?.downloadRowsPerPage))
    ? Number(value?.downloadRowsPerPage)
    : DEFAULT_UI_PREFERENCES.downloadRowsPerPage
  const downloadView = value?.downloadView === 'flat' ? 'flat' : 'grouped'
  const downloadColumns = columnOptions.has(Number(value?.downloadColumns))
    ? Number(value?.downloadColumns)
    : DEFAULT_UI_PREFERENCES.downloadColumns
  const tabWidth = value?.tabWidth === 'compact' || value?.tabWidth === 'wide' ? value.tabWidth : 'standard'
  const tabSleepMinutesRaw = Number(value?.tabSleepMinutes)
  const tabSleepMinutes = Number.isFinite(tabSleepMinutesRaw) ? Math.min(1440, Math.max(1, Math.round(tabSleepMinutesRaw))) : 15
  const runtimeMode = value?.runtimeMode === 'app-total' ? 'app-total' : 'current-open'
  // v24 迁移：旧键 showStatusUpdatedTime 已从 UiPreferences 类型里移除，这里单独
  // cast 出来读一次，避免污染主类型。
  const legacy = value as (Partial<UiPreferences> & { showStatusUpdatedTime?: unknown }) | undefined
  return {
    theme,
    density,
    fontScale,
    animations: value?.animations !== false,
    autoRefreshSeconds,
    browserToolbarEnabled: value?.browserToolbarEnabled !== false,
    browserToolbarPosition: value?.browserToolbarPosition === 'bottom' ? 'bottom' : 'right',
    browserToolbarCompact: value?.browserToolbarCompact === true,
    browserToolbarItems: normalizeToolbarItems(value?.browserToolbarItems),
    customToolbarButtons: normalizeCustomToolbarButtons(value?.customToolbarButtons),
    restorePositionAfterRefresh: value?.restorePositionAfterRefresh !== false,
    showTabAiStatus: value?.showTabAiStatus !== false,
    showStatusAccount: value?.showStatusAccount !== false,
    showStatusProxy: value?.showStatusProxy !== false,
    // v24 之后语义变为「显示 AI 使用时间」。新字段名为 showAiUsageTime；旧字段
    // showStatusUpdatedTime 仅作读取兼容 —— 旧 localStorage 里存在该键就沿用其值，
    // 否则取默认 true。新数据持久化时只写新键。
    //
    // 旧键已从 UiPreferences 类型里移除（避免误用），迁移这里单独 cast 一下读出来。
    showAiUsageTime: legacy?.showAiUsageTime ?? (legacy?.showStatusUpdatedTime !== false),
    downloadRowsPerPage,
    downloadView,
    downloadColumns,
    tabWidth,
    tabSleepEnabled: value?.tabSleepEnabled !== false,
    tabSleepMinutes,
    runtimeMode,
  }
}

export function loadUiPreferences(): UiPreferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return normalizeUiPreferences(raw ? JSON.parse(raw) : null)
  } catch {
    return { ...DEFAULT_UI_PREFERENCES }
  }
}

export function applyUiPreferences(preferences: UiPreferences) {
  const root = document.documentElement
  root.dataset.theme = preferences.theme
  root.dataset.density = preferences.density
  root.dataset.animations = preferences.animations ? 'on' : 'off'
  root.style.fontSize = `${preferences.fontScale}%`
}

export function saveUiPreferences(preferences: UiPreferences): UiPreferences {
  const normalized = normalizeUiPreferences(preferences)
  localStorage.setItem(STORAGE_KEY, JSON.stringify(normalized))
  applyUiPreferences(normalized)
  return normalized
}
