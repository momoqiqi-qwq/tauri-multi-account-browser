import { computed, reactive, ref, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Profile } from '../types'
import {
  parseGeneratorTargets, planAccountGeneration, generatorPlanSignature,
  type GeneratorOptions,
} from '../lib/accountGenerator'

export function useAccountGenerator(
  visible: Ref<boolean>, profiles: () => Profile[], initialUrl: () => string | undefined,
  globalUrl: () => string, onCreated: (profiles: Profile[]) => void, onRefreshed: (profiles: Profile[]) => void,
) {
  const step = ref(1)
  const urls = ref('')
  const selectedIds = ref<string[]>([])
  const sourceQuery = ref('')
  const sources = ref<Profile[]>([])
  const busy = ref(false)
  const busyLabel = ref('')
  const error = ref('')
  const needsRecheck = ref(false)
  const result = ref<Profile[] | null>(null)
  const options = reactive<GeneratorOptions>({ prefix: '', order: 'source-site', separator: ' · ', group: '', copyTags: true, skipExisting: true })
  const parsed = computed(() => parseGeneratorTargets(urls.value))
  const plan = computed(() => planAccountGeneration(parsed.value.targets, selectedIds.value, sources.value, options, globalUrl()))
  const selected = computed(() => new Set(selectedIds.value))
  const sourceIndex = computed(() => new Map(sources.value.map(profile => [profile.id, profile])))
  const missingIds = computed(() => selectedIds.value.filter(id => !sourceIndex.value.has(id)))
  const filteredSources = computed(() => {
    const q = sourceQuery.value.trim().toLocaleLowerCase()
    return sources.value.filter(profile => !q || [profile.name, profile.group, profile.default_url, ...(profile.tags ?? [])]
      .some(value => value.toLocaleLowerCase().includes(q)))
  })
  const selectedSources = computed(() => selectedIds.value.map(id => sourceIndex.value.get(id)).filter((p): p is Profile => !!p))
  const canGenerate = computed(() => !busy.value && !needsRecheck.value && !parsed.value.errors.length && !plan.value.errors.length && plan.value.drafts.length > 0)

  watch(visible, open => {
    if (!open) return
    sources.value = profiles()
    step.value = 1
    error.value = ''
    result.value = null
    sourceQuery.value = ''
    if (initialUrl()) urls.value = initialUrl()!
  })
  watch([urls, selectedIds, options], () => { if (!needsRecheck.value) error.value = '' }, { deep: true })
  watch(profiles, next => { if (visible.value && !busy.value) sources.value = next })

  function toggleSource(id: string, checked: boolean) {
    if (busy.value) return
    const next = new Set(selectedIds.value)
    if (checked) next.add(id); else next.delete(id)
    selectedIds.value = [...next]
  }
  function selectFiltered() {
    if (busy.value) return
    selectedIds.value = [...new Set([...selectedIds.value, ...filteredSources.value.map(profile => profile.id)])]
  }
  function clearSelection() { if (!busy.value) selectedIds.value = [] }
  function nextStep() {
    error.value = ''
    if (step.value === 1 && (!parsed.value.targets.length || parsed.value.errors.length)) {
      error.value = parsed.value.errors[0] || '请先填写目标网站。'; return
    }
    if (step.value === 2 && plan.value.errors.length) { error.value = plan.value.errors[0]; return }
    step.value = Math.min(3, step.value + 1)
  }
  function goStep(next: number) {
    if (busy.value || result.value) return
    error.value = ''
    if (next < step.value) step.value = next
    else if (next === step.value + 1) nextStep()
  }
  async function refreshSources() {
    if (busy.value) return
    busy.value = true
    busyLabel.value = '正在检查最新账号…'
    error.value = ''
    try {
      sources.value = await invoke<Profile[]>('list_profiles')
      onRefreshed(sources.value)
      needsRecheck.value = false
      // Missing selections are kept visible as an error, never silently dropped from a batch.
    } catch (e) { error.value = `无法读取最新账号：${String(e)}` }
    finally { busy.value = false }
  }
  async function generate() {
    if (!canGenerate.value || result.value || step.value !== 3) return
    const confirmed = generatorPlanSignature(plan.value)
    busy.value = true
    busyLabel.value = '正在核对最新账号与配置…'
    error.value = ''
    let submitted = false
    try {
      const latest = await invoke<Profile[]>('list_profiles')
      const checked = planAccountGeneration(parsed.value.targets, selectedIds.value, latest, options, globalUrl())
      sources.value = latest
      onRefreshed(latest)
      if (generatorPlanSignature(checked) !== confirmed) {
        error.value = '账号配置或重复项有变化，预览已更新。请核对后再次点击生成。'
        return
      }
      if (checked.errors.length || !checked.drafts.length) {
        error.value = checked.errors[0] || '没有需要新建的账号。'; return
      }
      busyLabel.value = `正在创建 ${checked.drafts.length} 个独立账号…`
      submitted = true
      const created = await invoke<Profile[]>('create_profiles_bulk', { drafts: checked.drafts })
      result.value = created
      onCreated(created)
    } catch (e) {
      needsRecheck.value = submitted
      error.value = submitted
        ? `未确认生成成功：${String(e)}。请先重新检查账号列表，再决定是否重试。`
        : `检查失败，尚未提交生成：${String(e)}`
    } finally { busy.value = false }
  }
  function close() { if (!busy.value) visible.value = false }
  return {
    step, urls, selectedIds, selected, sourceQuery, sources, filteredSources, selectedSources,
    missingIds, busy, busyLabel, error, needsRecheck, result, options, parsed, plan, canGenerate,
    toggleSource, selectFiltered, clearSelection, nextStep, goStep, refreshSources, generate, close,
  }
}
