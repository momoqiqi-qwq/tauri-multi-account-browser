import type { Profile } from '../types'

export interface DirectorySite {
  key: string
  name: string
  url: string
}
export interface WebsiteGroup extends DirectorySite {
  profiles: Profile[]
}
export interface DirectoryPreferences {
  selected: string
  columns: 1 | 2
  sites: DirectorySite[]
}
export const DIRECTORY_STORAGE_KEY = 'mab-site-directory-v1'
export const ALL_SITES = '@all'
export const UNKNOWN_SITE = '@unknown'
const SITE_NAMES: Record<string, string> = {
  'chat01.ai': 'Chat01',
  'claude.ai': 'Claude',
  'chatgpt.com': 'ChatGPT',
  'github.com': 'GitHub',
  'openrouter.ai': 'OpenRouter',
  'gemini.google.com': 'Gemini',
  'chat.deepseek.com': 'DeepSeek',
  'grok.com': 'Grok',
}

/** Group by configured home, never last_url (login redirects must not move accounts). */
export function parseDirectorySite(raw: string, name = ''): DirectorySite {
  const input = raw.trim()
  if (!input) throw new Error('请填写网站网址')
  if (/^[a-z][a-z0-9+.-]*:/i.test(input) && !/^https?:\/\//i.test(input) && !/^[^/?#\s]+:\d+(?:[/?#]|$)/.test(input)) {
    throw new Error('网站地址只支持 http:// 或 https://')
  }
  let url: URL
  try { url = new URL(/^https?:\/\//i.test(input) ? input : `https://${input}`) }
  catch { throw new Error('网址格式不正确') }
  if (!url.hostname || !['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new Error('请使用不含用户名或密码的 http(s) 网站地址')
  }
  let host = url.hostname.toLowerCase().replace(/\.$/, '').replace(/^www\./, '')
  // Explicit alias only; never collapse arbitrary subdomains or multi-part public suffixes.
  if (host === 'chat.openai.com') host = 'chatgpt.com'
  const key = host + (url.port ? `:${url.port}` : '')
  return { key, name: name.trim().slice(0, 48) || (Object.hasOwn(SITE_NAMES, key) ? SITE_NAMES[key] : key), url: url.toString() }
}

export function siteForProfile(profile: Profile, globalDefaultUrl: string): DirectorySite {
  const home = profile.url_mode === 'inherit'
    ? (globalDefaultUrl.trim() || profile.default_url)
    : profile.default_url
  try { return parseDirectorySite(home) }
  catch { return { key: UNKNOWN_SITE, name: '待设置网站', url: '' } }
}

export function buildWebsiteGroups(
  profiles: Profile[], globalDefaultUrl: string, saved: DirectorySite[] = [],
): WebsiteGroup[] {
  const groups = new Map<string, WebsiteGroup>()
  for (const site of saved) groups.set(site.key, { ...site, profiles: [] })
  // Stable ordering; do not mutate the account store or its existing manual groups/tags.
  for (const profile of [...profiles].sort((a, b) => a.order - b.order)) {
    const site = siteForProfile(profile, globalDefaultUrl)
    if (!groups.has(site.key)) groups.set(site.key, { ...site, profiles: [] })
    groups.get(site.key)!.profiles.push(profile)
  }
  return [...groups.values()]
}

export interface DirectoryEntry {
  profile: Profile
  site: DirectorySite
  fallback: string
  searchFields: string[]
}
function profileSearchFields(profile: Profile): string[] {
  return [profile.name, profile.note, profile.group, profile.default_url, ...(profile.tags ?? [])]
    .map(value => (value || '').toLocaleLowerCase())
}

/** Reuse already resolved groups; never parse a URL again for each template binding. */
export function indexWebsiteGroups(groups: WebsiteGroup[]): Map<string, DirectoryEntry> {
  const index = new Map<string, DirectoryEntry>()
  for (const group of groups) {
    const site: DirectorySite = { key: group.key, name: group.name, url: group.url }
    const fallback = site.key === 'chat01.ai' ? '01' : [...site.name].slice(0, 2).join('').toUpperCase()
    for (const profile of group.profiles) {
      index.set(profile.id, { profile, site, fallback, searchFields: profileSearchFields(profile) })
    }
  }
  return index
}

export function filterDirectoryEntries(entries: DirectoryEntry[], query: string): DirectoryEntry[] {
  const q = query.trim().toLocaleLowerCase()
  return q ? entries.filter(entry => entry.searchFields.some(field => field.includes(q))) : entries
}

export function searchDirectoryProfiles(profiles: Profile[], query: string): Profile[] {
  const q = query.trim().toLocaleLowerCase()
  if (!q) return profiles
  return profiles.filter(profile => profileSearchFields(profile).some(field => field.includes(q)))
}

export function readDirectoryPreferences(raw: string | null): DirectoryPreferences {
  const defaults: DirectoryPreferences = { selected: ALL_SITES, columns: 2, sites: [] }
  try {
    const value = JSON.parse(raw || '{}')
    if (!value || typeof value !== 'object') return defaults
    const sites = new Map<string, DirectorySite>()
    for (const entry of Array.isArray(value.sites) ? value.sites.slice(0, 200) : []) {
      try {
        if (typeof entry?.url !== 'string') continue
        const site = parseDirectorySite(entry.url, typeof entry.name === 'string' ? entry.name : '')
        sites.set(site.key, site)
      } catch { /* Ignore stale or invalid saved entries, not the whole directory. */ }
    }
    return {
      selected: typeof value.selected === 'string' ? value.selected : ALL_SITES,
      columns: value.columns === 1 ? 1 : 2,
      sites: [...sites.values()],
    }
  } catch { return defaults }
}
