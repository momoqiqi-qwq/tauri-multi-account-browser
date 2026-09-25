import { computed, ref, watch, type Ref } from 'vue'
import { ElMessage } from 'element-plus'
import type { Profile } from '../types'
import {
  ALL_SITES, DIRECTORY_STORAGE_KEY, buildWebsiteGroups, parseDirectorySite,
  readDirectoryPreferences, indexWebsiteGroups, filterDirectoryEntries, type DirectoryEntry,
} from '../lib/siteDirectory'

export function useWebsiteDirectory(profiles: () => Profile[], globalUrl: () => string, loading: Ref<boolean>) {
  let raw: string | null = null
  try { raw = localStorage.getItem(DIRECTORY_STORAGE_KEY) } catch { /* Storage may be unavailable. */ }
  const initial = readDirectoryPreferences(raw)
  const sites = ref(initial.sites)
  const selected = ref(initial.selected)
  const columns = ref(initial.columns)
  const query = ref('')
  const groups = computed(() => buildWebsiteGroups(profiles(), globalUrl(), sites.value))
  const current = computed(() => groups.value.find(group => group.key === selected.value))
  const inSite = computed(() => current.value?.profiles ?? profiles())
  const index = computed(() => indexWebsiteGroups(groups.value))
  const entries = computed(() => inSite.value
    .map(profile => index.value.get(profile.id))
    .filter((entry): entry is DirectoryEntry => !!entry))
  const matches = computed(() => filterDirectoryEntries(entries.value, query.value))
  let warned = false

  function persist() {
    try {
      localStorage.setItem(DIRECTORY_STORAGE_KEY, JSON.stringify({
        selected: selected.value, columns: columns.value, sites: sites.value,
      }))
    } catch {
      if (!warned) ElMessage.warning('无法保存网站导航偏好，本次会话仍可正常使用')
      warned = true
    }
  }
  watch([selected, columns, sites], persist, { deep: true })
  watch([groups, loading], () => {
    if (!loading.value && selected.value !== ALL_SITES && !current.value) selected.value = ALL_SITES
  }, { immediate: true })

  function selectSite(key: string) {
    selected.value = key
    query.value = ''
  }
  function saveSite(url: string, name: string) {
    const site = parseDirectorySite(url, name)
    const existing = sites.value.findIndex(item => item.key === site.key)
    if (existing < 0 && sites.value.length >= 200) throw new Error('最多保存 200 个网站')
    sites.value = existing < 0
      ? [...sites.value, site]
      : sites.value.map(item => item.key === site.key ? site : item)
    selectSite(site.key)
  }
  function removeEmptySite() {
    if (!current.value || current.value.profiles.length) return
    sites.value = sites.value.filter(site => site.key !== selected.value)
    selected.value = ALL_SITES
  }
  return { selected, columns, query, groups, current, inSite, matches, selectSite, saveSite, removeEmptySite }
}
