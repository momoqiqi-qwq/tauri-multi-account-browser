/** Session-only, bounded scroll memory. Never synchronously write storage during scrolling. */
const positions = new Map<string, number>()
const MAX_SITES = 50
export function rememberDirectoryScroll(site: string, top: number) {
  if (!Number.isFinite(top)) return
  positions.delete(site)
  positions.set(site, Math.max(0, top))
  if (positions.size > MAX_SITES) positions.delete(positions.keys().next().value!)
}
export function directoryScrollTop(site: string): number { return positions.get(site) ?? 0 }
