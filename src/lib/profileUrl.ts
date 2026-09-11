/**
 * 账号网址统一规则。
 *
 * 前端只负责即时反馈；Rust 后端仍会再次校验，避免绕过 UI 的调用写入脏数据。
 */
export const DEFAULT_PROFILE_URL = 'https://chat01.ai/'

/** 将用户输入规范化为可导航的 http(s) URL。空值自动使用应用默认主页。 */
export function normalizeProfileUrl(raw: string | null | undefined): string {
  const trimmed = String(raw ?? '').trim()
  const candidate = trimmed || DEFAULT_PROFILE_URL
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(candidate) && !/^https?:\/\//i.test(candidate)) {
    throw new Error('只支持 http:// 或 https:// 网址')
  }
  const withScheme = /^https?:\/\//i.test(candidate) ? candidate : `https://${candidate}`

  let parsed: URL
  try {
    parsed = new URL(withScheme)
  } catch {
    throw new Error('网址格式不正确，请输入类似 https://example.com/ 的地址')
  }

  if (!['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname) {
    throw new Error('只支持 http:// 或 https:// 网址')
  }
  return parsed.toString()
}

/**
 * 打开账号时优先恢复 last_url；没有/损坏时回到账号默认主页；两者都没有则用应用默认主页。
 */
export function resolveProfileUrl(profile?: {
  last_url?: string | null
  default_url?: string | null
} | null): string {
  for (const raw of [profile?.last_url, profile?.default_url]) {
    if (!String(raw ?? '').trim()) continue
    try {
      return normalizeProfileUrl(raw)
    } catch {
      // 继续尝试下一层兜底，避免旧数据中的坏 URL 让账号无法进入。
    }
  }
  return DEFAULT_PROFILE_URL
}

/** 账号“主页”只看 default_url；缺失/损坏时使用应用默认主页。 */
export function resolveProfileHomeUrl(profile?: { default_url?: string | null } | null): string {
  try {
    return normalizeProfileUrl(profile?.default_url)
  } catch {
    return DEFAULT_PROFILE_URL
  }
}
