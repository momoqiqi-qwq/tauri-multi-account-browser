/**
 * 原生账号 WebView 永远渲染在主 WebView（Vue UI）之上，
 * 因此 Vue 的下拉菜单、弹窗等落到网页区域的内容会被盖住。
 * 这里提供引用计数式的抑制开关：任何弹层打开期间临时隐藏账号 WebView，
 * 全部关闭后再恢复。
 */

import { onBeforeUnmount, watch } from 'vue'

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

/**
 * 对话框专用：打开期间抑制账号 WebView，关闭或组件卸载时恢复。
 *
 * 漏掉这一步的对话框不会被任何人发现 —— 只有当账号页恰好铺满视口区域时才表现为
 * “弹窗打不开 / 点不到”，所以做成一个必须显式调用的开关而不是靠各自复制粘贴。
 *
 * 传 getter 而不是 ref，`defineModel` 返回的 ModelRef 也能直接喂进来。
 */
export function useDialogSuppression(isOpen: () => boolean) {
  let release: (() => void) | null = null
  watch(isOpen, (open) => {
    release?.()
    release = open ? acquireWebviewSuppression() : null
  })
  onBeforeUnmount(() => {
    release?.()
    release = null
  })
}
