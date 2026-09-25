import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

/** One clock only while a visible UI actually needs live time. No accumulated drift. */
export function useActivityClock(needed: () => boolean) {
  const now = ref(Date.now())
  const visible = ref(!document.hidden)
  let timer: ReturnType<typeof setInterval> | undefined
  function stop() {
    if (timer !== undefined) clearInterval(timer)
    timer = undefined
  }
  const stopWatch = watch([needed, visible], ([active, pageVisible]) => {
    stop()
    // Refresh on resume/status changes; background time must not freeze the elapsed value.
    now.value = Date.now()
    if (active && pageVisible) timer = setInterval(() => { now.value = Date.now() }, 1000)
  }, { immediate: true })
  function visibilityChanged() { visible.value = !document.hidden }
  onMounted(() => document.addEventListener('visibilitychange', visibilityChanged))
  onBeforeUnmount(() => {
    stopWatch()
    stop()
    document.removeEventListener('visibilitychange', visibilityChanged)
  })
  return now
}
