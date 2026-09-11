/**
 * 原生账号 WebView 永远渲染在主 WebView（Vue UI）之上，
 * 因此 Vue 的下拉菜单、弹窗等落到网页区域的内容会被盖住。
 * 这里提供引用计数式的抑制开关：任何弹层打开期间临时隐藏账号 WebView，
 * 全部关闭后再恢复。
 */

type SuppressionHandler = (suppress: boolean) => void

let handler: SuppressionHandler | null = null
let count = 0

export function registerSuppressionHandler(h: SuppressionHandler | null) {
  handler = h
  // 组件在 handler 尚未注册时就打开了弹层的兜底。
  if (count > 0 && handler) handler(true)
}

export function isSuppressed(): boolean {
  return count > 0
}

/** 获取一次抑制引用，返回释放函数；计数归零时才真正恢复 WebView。 */
export function acquireWebviewSuppression(): () => void {
  count += 1
  handler?.(true)
  let released = false
  return () => {
    if (released) return
    released = true
    count -= 1
    if (count === 0) handler?.(false)
  }
}
