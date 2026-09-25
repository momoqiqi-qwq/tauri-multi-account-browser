import { nextTick, onBeforeUnmount, ref, watch, type Ref } from 'vue'
import { directoryScrollTop, rememberDirectoryScroll } from '../lib/directoryScroll'

export function useDirectoryScroll(selected: Ref<string>, query: Ref<string>) {
  const grid = ref<HTMLElement | null>(null)
  function remember() {
    if (grid.value && !query.value.trim()) rememberDirectoryScroll(selected.value, grid.value.scrollTop)
  }
  function restore() {
    if (grid.value) grid.value.scrollTop = query.value.trim() ? 0 : directoryScrollTop(selected.value)
  }
  watch([selected, query], (_, [previousSite, previousQuery]) => {
    // Default pre-flush still sees the previous site's DOM, before keyed replacement.
    if (grid.value && !previousQuery.trim()) rememberDirectoryScroll(previousSite, grid.value.scrollTop)
    void nextTick().then(restore)
  })
  watch(grid, restore, { flush: 'post' })
  onBeforeUnmount(remember)
  return { grid, remember }
}
