/**
 * 键盘快捷键（v27）。
 *
 * 按键有两个来源：
 * 1. 宿主 UI 自己的 keydown（焦点在侧栏/地址栏等宿主界面时）；
 * 2. 账号 WebView 经 `mbkey://press?k=<组合>` 假导航转发（焦点在网页里时，
 *    见 Rust 侧 KEY_FORWARD_JS 与 disable_browser_accelerator_keys）。
 *
 * 两个来源都归一化成 `combo` 字符串后查同一张表。组合键写法固定为
 * `ctrl → alt → shift → 键名`，键名小写；Rust 侧 KEY_FORWARD_JS 里的拼接顺序
 * 必须与 `comboFromEvent` 保持一致，改一边要同步另一边。
 */

export type HotkeyAction =
  | 'tab-next'
  | 'tab-prev'
  | 'tab-index'
  | 'tab-close'
  | 'tab-new'
  | 'address-bar'
  | 'reload'
  | 'back'
  | 'forward'
  | 'downloads'

export interface ShortcutSpec {
  /** 归一化组合键；Ctrl+数字用 ctrl-1 一类的占位写法展示时换成 keys 字段 */
  combo: string
  /** 给人看的按键写法 */
  keys: string
  label: string
  action: HotkeyAction
  /** tab-index 动作对应的序号（1 起；9 表示最后一个标签） */
  index?: number
}

const BASIC_SHORTCUTS: ShortcutSpec[] = [
  { combo: 'ctrl+tab', keys: 'Ctrl+Tab', label: '下一个标签页', action: 'tab-next' },
  { combo: 'ctrl+shift+tab', keys: 'Ctrl+Shift+Tab', label: '上一个标签页', action: 'tab-prev' },
  { combo: 'ctrl+w', keys: 'Ctrl+W', label: '关闭当前标签页', action: 'tab-close' },
  { combo: 'ctrl+t', keys: 'Ctrl+T', label: '新建账号', action: 'tab-new' },
  { combo: 'ctrl+l', keys: 'Ctrl+L', label: '聚焦地址栏', action: 'address-bar' },
  { combo: 'ctrl+r', keys: 'Ctrl+R', label: '刷新当前页面', action: 'reload' },
  { combo: 'ctrl+f5', keys: 'Ctrl+F5', label: '刷新当前页面', action: 'reload' },
  { combo: 'f5', keys: 'F5', label: '刷新当前页面', action: 'reload' },
  { combo: 'alt+arrowleft', keys: 'Alt+←', label: '后退', action: 'back' },
  { combo: 'alt+arrowright', keys: 'Alt+→', label: '前进', action: 'forward' },
  { combo: 'ctrl+j', keys: 'Ctrl+J', label: '下载历史', action: 'downloads' },
]

/** Ctrl+1..8 切到第 N 个标签，Ctrl+9 切到最后一个（与常见浏览器一致）。 */
const DIGIT_SHORTCUTS: ShortcutSpec[] = Array.from({ length: 9 }, (_, i) => {
  const index = i + 1
  return {
    combo: `ctrl+${index}`,
    keys: `Ctrl+${index}`,
    label: index === 9 ? '切到最后一个标签页' : `切换到第 ${index} 个标签页`,
    action: 'tab-index' as const,
    index,
  }
})

export const SHORTCUTS: ShortcutSpec[] = [...DIGIT_SHORTCUTS, ...BASIC_SHORTCUTS]

const COMBO_INDEX = new Map(SHORTCUTS.map((spec) => [spec.combo, spec]))

export function findShortcut(combo: string): ShortcutSpec | undefined {
  return COMBO_INDEX.get(combo)
}

/**
 * 从键盘事件归一化出组合键字符串；不属于快捷键的按键返回 null。
 * 只认 Ctrl/Cmd、Alt 和 F5；Shift 只参与 Ctrl+Shift+Tab；Ctrl+Alt（AltGr）
 * 一律返回 null，避免抢输入法的特殊字符。
 */
export function comboFromEvent(event: KeyboardEvent): string | null {
  if (event.isComposing) return null
  const key = (event.key || '').toLowerCase()
  const ctrl = event.ctrlKey || event.metaKey
  const alt = event.altKey
  const digit = /^[1-9]$/.test(key)
  if (!ctrl && !alt && key !== 'f5') return null
  if (digit) {
    if (!ctrl || alt) return null
  } else if (!['tab', 'w', 't', 'l', 'r', 'j', 'f5', 'arrowleft', 'arrowright'].includes(key)) {
    return null
  } else if (key === 'arrowleft' || key === 'arrowright') {
    if (!alt || ctrl) return null
  } else if (alt) {
    return null
  }
  if (event.shiftKey && key !== 'tab') return null
  if (ctrl && alt) return null
  const parts: string[] = []
  if (ctrl) parts.push('ctrl')
  if (alt) parts.push('alt')
  if (event.shiftKey) parts.push('shift')
  parts.push(key)
  return parts.join('+')
}
