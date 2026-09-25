import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import ts from 'typescript'

async function loadPureModule(path) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } })
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`)
}
const { buildWebsiteGroups, indexWebsiteGroups, filterDirectoryEntries, searchDirectoryProfiles, parseDirectorySite } = await loadPureModule('../src/lib/siteDirectory.ts')
const { rememberDirectoryScroll, directoryScrollTop } = await loadPureModule('../src/lib/directoryScroll.ts')
const profile = (id, extra = {}) => ({ id, name: id, default_url: 'https://chat01.ai/', url_mode: 'custom', order: 0, tags: [], group: '', note: '', ...extra })

test('indexed display shares one website descriptor and preserves profile references', () => {
  const profiles = [profile('a'), profile('b')]
  const index = indexWebsiteGroups(buildWebsiteGroups(profiles, 'https://chat01.ai'))
  assert.equal(index.get('a').profile, profiles[0])
  assert.equal(index.get('a').site, index.get('b').site)
  assert.equal(index.get('a').fallback, '01')
})
test('indexed search matches legacy search, including whitespace, case and tag changes', () => {
  const profiles = [profile('Work', { note: '主账号', group: '工作', tags: ['常用'] }), profile('Other')]
  for (const query of ['', ' ', 'WORK', ' 主账号 ', '常用', '工作', 'CHAT01', 'missing']) {
    const entries = [...indexWebsiteGroups(buildWebsiteGroups(profiles, '')).values()]
    assert.deepEqual(filterDirectoryEntries(entries, query).map(e => e.profile.id), searchDirectoryProfiles(profiles, query).map(p => p.id))
  }
  profiles[0].tags = ['新标签']
  const updated = [...indexWebsiteGroups(buildWebsiteGroups(profiles, '')).values()]
  assert.equal(filterDirectoryEntries(updated, '常用').length, 0)
  assert.equal(filterDirectoryEntries(updated, '新标签').length, 1)
})
test('site renaming updates display names without altering account data', () => {
  const p = profile('a'), snapshot = JSON.stringify(p)
  const index = indexWebsiteGroups(buildWebsiteGroups([p], '', [parseDirectorySite('https://chat01.ai', '我的网站')]))
  assert.equal(index.get('a').site.name, '我的网站')
  assert.equal(JSON.stringify(p), snapshot)
})
test('1000-account indexing and repeated search perform no additional URL parsing', () => {
  const OriginalURL = globalThis.URL
  let parses = 0
  globalThis.URL = class extends OriginalURL { constructor(...args) { super(...args); parses++ } }
  try {
    const groups = buildWebsiteGroups(Array.from({ length: 1000 }, (_, i) => profile(`account-${i}`)), '')
    const initialParses = parses
    assert.equal(initialParses, 1000)
    const entries = [...indexWebsiteGroups(groups).values()]
    for (let i = 0; i < 30; i++) filterDirectoryEntries(entries, `account-${i}`)
    assert.equal(parses, initialParses)
    assert.equal(entries.length, 1000)
  } finally { globalThis.URL = OriginalURL }
})
test('scroll memory isolates websites and rejects invalid positions', () => {
  rememberDirectoryScroll('a', 280)
  rememberDirectoryScroll('b', 560)
  assert.equal(directoryScrollTop('a'), 280)
  assert.equal(directoryScrollTop('b'), 560)
  rememberDirectoryScroll('a', NaN)
  rememberDirectoryScroll('a', Infinity)
  assert.equal(directoryScrollTop('a'), 280)
  rememberDirectoryScroll('b', -10)
  assert.equal(directoryScrollTop('b'), 0)
  assert.equal(directoryScrollTop('never-seen'), 0)
})
test('scroll memory is bounded to 50 recently written websites', () => {
  rememberDirectoryScroll('oldest', 999)
  for (let i = 0; i < 50; i++) rememberDirectoryScroll(`bounded-${i}`, i + 1)
  assert.equal(directoryScrollTop('oldest'), 0)
  assert.equal(directoryScrollTop('bounded-49'), 50)
})
