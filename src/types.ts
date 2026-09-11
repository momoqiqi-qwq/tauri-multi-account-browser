export interface Profile {
  id: string
  name: string
  note: string
  avatar: string | null
  default_url: string
  /** inherit 跟随全局默认网址；custom 使用账号独立网址 */
  url_mode: 'inherit' | 'custom'
  last_url: string
  created_at: string
  order: number
  incognito: boolean
  /** 独立代理，空字符串表示直连 */
  proxy: string
  /** 自定义 User-Agent，空表示系统默认 */
  user_agent: string
  /** IANA 时区，空表示跟随系统 */
  timezone: string
  /** 语言/区域（BCP-47，如 zh-CN），空表示跟随系统 */
  locale: string
  /** 指纹防护（画布/GPU/硬件噪声） */
  fingerprint_guard: boolean
  /** 账号分组，空表示未分组 */
  group: string
  /** 自由标签，用于跨分组归类 */
  tags: string[]
  /** 收藏 */
  favorite: boolean
  /** 最近一次被打开的时间（RFC3339），没打开过为 null */
  last_used_at: string | null
}

export interface BrowserBounds {
  x: number
  y: number
  width: number
  height: number
}

/** 新建 / 编辑账号时的隔离设置表单 */
export interface ProfileIsolationSettings {
  name: string
  defaultUrl: string
  urlMode: 'inherit' | 'custom'
  incognito: boolean
  proxy: string
  userAgent: string
  timezone: string
  locale: string
  fingerprintGuard: boolean
  group: string
  /** 自由标签；Rust 侧会 trim / 去重 / 限长 */
  tags: string[]
}

/** 侧边栏的视图筛选 */
export type ProfileViewMode = 'all' | 'favorite' | 'recent'

/** `update_profile_batch` 的补丁；全是可选，省略即"不改" */
export interface ProfileBatchPatch {
  url_mode?: 'inherit' | 'custom'
  default_url?: string
  tags_add?: string[]
  tags_remove?: string[]
  tags_set?: string[]
  favorite?: boolean
  group?: string
}

/** 从账号页面（chat01.ai）抓到的状态：积分、登录账号名、邮箱、代理出口 */
export interface ProfileStatus {
  credits: string | null
  account: string | null
  email: string | null
  updated_at: string | null
  proxy_ip: string | null
  proxy_region: string | null
  proxy_isp: string | null
  proxy_tested_at: string | null
  /** 后台账号的 AI 回答已完成，等待用户查看 */
  answer_ready: boolean
  /** 当前页面是否检测到 AI 正在生成回答 */
  answer_generating: boolean
  /** 最近一次回答完成时间 */
  answer_finished_at: string | null
}

/** profile:status 事件负载 */
export interface ProfileStatusEventPayload extends ProfileStatus {
  id: string
}

/** 一条下载历史记录（profile:download 事件负载 / get_download_history 元素） */
export interface DownloadEntry {
  id: string
  /** 下载发生时的账号名快照（账号删除后仍可读） */
  profile_name: string
  file_name: string
  path: string
  size: number
  finished_at: string
  success: boolean
}

/** 全局应用设置：下载目录与 chat01 AI 文件自动下载 */
export interface DownloadGuardRule {
  domain: string
  seconds: number
}

export interface AppSettings {
  download_dir: string
  auto_ai_download: boolean
  /** 自动下载时跳过下载历史中已经成功下载过的同名文件 */
  skip_downloaded_files: boolean
  /** 删除下载文件时是否先显示二次确认 */
  confirm_delete_download: boolean
  /** 下载历史最多保留的记录数 */
  download_history_limit: number
  /** 页面开始加载后的下载保护秒数，0 表示关闭 */
  download_guard_seconds: number
  /** 指定可信下载域名使用单独保护时长；0 表示该域名立即允许 */
  download_guard_rules: DownloadGuardRule[]
  /** 保护期拦截后自动排队，并在剩余时间结束后重试 */
  download_guard_auto_retry: boolean
  ai_exts: string
  /** 继承模式账号共用的主页 */
  global_default_url: string
}

export type ThemeId =
  | 'classic'
  | 'claude-light'
  | 'claude-dark'
  | 'midnight-blue'
  | 'graphite'
  | 'glass-blue'
  | 'paper-cream'
  | 'amethyst'
  | 'win11'
  | 'sunset'
  | 'mint-light'


export type BrowserToolbarBuiltinId =
  | 'reload'
  | 'home'
  | 'back'
  | 'forward'
  | 'copy-url'
  | 'overview'
  | 'downloads'
  | 'settings'

export interface BrowserToolbarItem {
  id: BrowserToolbarBuiltinId
  enabled: boolean
}

export interface CustomToolbarButton {
  id: string
  /** 按钮悬浮提示/设置页名称 */
  label: string
  /** 可选 Emoji/短字符图标；留空使用链接图标 */
  icon: string
  /** 点击后在当前账号 WebView 内打开 */
  url: string
  enabled: boolean
}

/** 诊断错误码：Rust 侧 `DiagnosticCode` 的 snake_case 序列化结果 */
export type DiagnosticCode =
  | 'unknown'
  | 'webview2_runtime'
  | 'data_dir'
  | 'proxy'
  | 'network'
  | 'invalid_url'
  | 'bad_last_url'

/** WebView2 Runtime 检测结果 */
export interface WebView2Status {
  installed: boolean
  /** 形如 131.0.2903.86；检测不到时为空串 */
  version: string
  /** 版本来源：registry-hklm / registry-hkcu / filesystem / none */
  source: string
}

/** 账号数据目录占用 */
export interface DataDirUsage {
  exists: boolean
  path: string
  bytes: number
  file_count: number
  /** 为 true 表示文件数超过统计上限，bytes / file_count 只是部分结果 */
  truncated: boolean
}

/** `diagnose_profile` 的返回值 */
export interface ProfileDiagnostic {
  id: string
  name: string
  url_mode: string
  global_default_url: string
  effective_home_url: string
  last_url: string
  last_url_valid: boolean
  proxy_configured: boolean
  proxy_valid: boolean
  /** 打开失败原因的归类 */
  code: DiagnosticCode
  /** 给用户的修复建议 */
  hint: string
  runtime: WebView2Status
  data_dir: DataDirUsage
}

/** `test_profile_proxy` 的返回值 */
export interface ProxyTestResult {
  ok: boolean
  ip: string
  region: string
  isp: string
  error: string
  /** 往返耗时（毫秒） */
  latency_ms: number
  /** 请求是否真的走了代理 */
  via_proxy: boolean
  /** 实际使用的代理地址，直连时为空串 */
  endpoint: string
}

export interface UiPreferences {
  theme: ThemeId
  density: 'comfortable' | 'compact'
  fontScale: number
  animations: boolean
  /** 自动刷新间隔（秒），0 表示关闭 */
  autoRefreshSeconds: number
  /** 宿主侧工具栏：独立于远程网页，不依赖页面脚本注入 */
  browserToolbarEnabled: boolean
  browserToolbarPosition: 'right' | 'bottom'
  browserToolbarCompact: boolean
  browserToolbarItems: BrowserToolbarItem[]
  customToolbarButtons: CustomToolbarButton[]
  /** 刷新后恢复刷新前的滚动/锚点位置 */
  restorePositionAfterRefresh: boolean
  /** 标签页显示 AI 生成/完成状态 */
  showTabAiStatus: boolean
  /** 账号状态卡显示登录账号 */
  showStatusAccount: boolean
  /** 账号状态卡显示代理出口 */
  showStatusProxy: boolean
  /** 账号状态卡显示 AI 使用时间 */
  showAiUsageTime: boolean
  downloadRowsPerPage: number
  downloadView: 'grouped' | 'flat'
  /** 标签页宽度 */
  tabWidth: 'compact' | 'standard' | 'wide'
  /** 后台标签页自动休眠 */
  tabSleepEnabled: boolean
  /** 进入后台多少分钟后销毁 WebView；标签本身仍保留 */
  tabSleepMinutes: number
  /** 运行时间显示方式 */
  runtimeMode: 'current-open' | 'app-total'
}
