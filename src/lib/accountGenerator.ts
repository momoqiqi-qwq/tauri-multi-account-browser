import type { Profile } from '../types'
import { parseDirectorySite } from './siteDirectory'

export const MAX_GENERATED_PROFILES = 200
/** 记住「上次选择的来源账号」所用的 localStorage 键。 */
export const SOURCE_SELECTION_KEY = 'mab-generator-source-selection-v1'
export interface GeneratorTarget { name: string; url: string; line: number }
export interface TargetParseResult { targets: GeneratorTarget[]; errors: string[]; duplicates: number }
export interface GeneratorOptions {
  prefix: string
  order: 'source-site' | 'site-source'
  separator: string
  group: string
  copyTags: boolean
  skipExisting: boolean
}
export interface GeneratedDraft {
  name: string
  default_url: string
  url_mode: 'custom'
  incognito: boolean
  proxy: string
  user_agent: string
  timezone: string
  locale: string
  fingerprint_guard: boolean
  group: string
  tags: string[]
}
export interface GeneratorRow {
  key: string
  sourceId: string
  sourceName: string
  target: GeneratorTarget
  draft: GeneratedDraft
  skipped: boolean
  renamed: boolean
}
export interface GeneratorPlan {
  rows: GeneratorRow[]
  drafts: GeneratedDraft[]
  errors: string[]
  combinations: number
  skipped: number
}

export function parseGeneratorTargets(text: string): TargetParseResult {
  const targets: GeneratorTarget[] = [], errors: string[] = [], seen = new Set<string>()
  let duplicates = 0
  const lines = text.split(/\r?\n/)
  if (lines.filter(line => line.trim()).length > MAX_GENERATED_PROFILES) {
    return { targets, errors: ['单次最多填写 200 行网址，请分批生成。'], duplicates }
  }
  lines.forEach((line, index) => {
    const input = line.trim()
    if (!input) return
    const bar = /^https?:\/\//i.test(input) ? -1 : input.indexOf('|')
    const label = bar < 0 ? '' : input.slice(0, bar).trim()
    const rawUrl = bar < 0 ? input : input.slice(bar + 1).trim()
    try {
      if ([...label].length > 48) throw new Error('网站名称不能超过 48 个字符')
      if (rawUrl.length > 4096) throw new Error('网址过长，请控制在 4096 个字符以内')
      const site = parseDirectorySite(rawUrl, label)
      if (seen.has(site.url)) { duplicates++; return }
      seen.add(site.url)
      targets.push({ name: label || site.name, url: site.url, line: index + 1 })
    } catch (error) {
      errors.push(`第 ${index + 1} 行：${error instanceof Error ? error.message : String(error)}`)
    }
  })
  return { targets, errors, duplicates }
}

function duplicateKey(name: string, url: string) { return JSON.stringify([name.trim(), url]) }
function uniqueName(base: string, url: string, occupied: Set<string>): string {
  let name = base, suffix = 2
  while (occupied.has(duplicateKey(name, url))) name = `${base} (${suffix++})`
  return name
}

/** Pure cross-product plan. Never carries source IDs, cookies, history or last_url into drafts. */
export function planAccountGeneration(
  targets: GeneratorTarget[], selectedIds: string[], profiles: Profile[],
  options: GeneratorOptions, globalDefaultUrl: string,
): GeneratorPlan {
  const errors: string[] = [], rows: GeneratorRow[] = []
  const ids = [...new Set(selectedIds)]
  const combinations = targets.length * ids.length
  if (!targets.length) errors.push('请至少填写一个有效网站。')
  if (!ids.length) errors.push('请至少选择一个来源账号。')
  if (combinations > MAX_GENERATED_PROFILES) errors.push(`本次组合为 ${combinations} 个，超过单次 200 个上限，请减少网站或来源账号。`)
  const sourceIndex = new Map(profiles.map(profile => [profile.id, profile]))
  const missing = ids.filter(id => !sourceIndex.has(id))
  if (missing.length) errors.push(`${missing.length} 个已选来源账号已不存在，请重新选择。`)
  if (errors.length) return { rows, drafts: [], errors, combinations, skipped: 0 }

  const existing = new Set<string>()
  for (const profile of profiles) {
    try {
      const url = parseDirectorySite(profile.url_mode === 'inherit' ? globalDefaultUrl || profile.default_url : profile.default_url).url
      existing.add(duplicateKey(profile.name, url))
    } catch { /* Invalid existing homes cannot match valid target URLs. */ }
  }
  const planned = new Set<string>(), occupied = new Set(existing)
  for (const target of targets) {
    for (const id of ids) {
      const source = sourceIndex.get(id)!
      const parts = options.order === 'site-source' ? [target.name, source.name] : [source.name, target.name]
      if (options.prefix.trim()) parts.unshift(options.prefix.trim())
      const base = parts.join(options.separator).trim()
      // Distinct sources can have the same nickname: preserve both with deterministic suffixes.
      let name = uniqueName(base, target.url, planned)
      planned.add(duplicateKey(name, target.url))
      const skipped = options.skipExisting && existing.has(duplicateKey(name, target.url))
      if (!skipped) name = uniqueName(name, target.url, occupied)
      occupied.add(duplicateKey(name, target.url))
      if ([...name].length > 80) errors.push(`“${source.name} → ${target.name}”的名称超过 80 个字符，请缩短前缀、网站名或来源账号名。`)
      rows.push({
        key: JSON.stringify([target.url, id]), sourceId: id, sourceName: source.name, target, skipped, renamed: name !== base,
        draft: {
          name, default_url: target.url, url_mode: 'custom',
          incognito: source.incognito, proxy: source.proxy, user_agent: source.user_agent,
          timezone: source.timezone, locale: source.locale, fingerprint_guard: source.fingerprint_guard,
          group: options.group.trim() || source.group, tags: options.copyTags ? [...(source.tags ?? [])] : [],
        },
      })
    }
  }
  return { rows, drafts: errors.length ? [] : rows.filter(row => !row.skipped).map(row => row.draft), errors, combinations, skipped: rows.filter(row => row.skipped).length }
}

/** Compare the actual intended write, including skips and config, before final submission. */
export function generatorPlanSignature(plan: GeneratorPlan): string {
  return JSON.stringify({ errors: plan.errors, rows: plan.rows.map(row => ({ sourceId: row.sourceId, skipped: row.skipped, draft: row.draft })) })
}

/**
 * 还原上一次记住的来源账号选择。
 *
 * 记忆是跨会话的，而账号随时可能被删除：这里必须只保留 `knownIds` 里仍存在的
 * id，否则下一次打开生成器会凭空带出一批「已删除」的选择，直接卡在错误状态上。
 * 存储内容被外部改坏（非数组 / 非字符串元素 / 重复项）时一律静默降级，
 * 不让脏数据阻塞生成器本身。
 */
export function restoreSourceSelection(raw: string | null | undefined, knownIds: Iterable<string>): string[] {
  const known = new Set(knownIds)
  let parsed: unknown
  try { parsed = JSON.parse(raw ?? '') } catch { return [] }
  if (!Array.isArray(parsed)) return []
  const seen = new Set<string>(), result: string[] = []
  for (const id of parsed) {
    if (typeof id !== 'string' || seen.has(id) || !known.has(id)) continue
    seen.add(id)
    result.push(id)
  }
  return result
}
