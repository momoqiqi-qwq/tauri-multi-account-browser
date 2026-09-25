import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import ts from 'typescript'

// Test pure grouping logic with the existing TypeScript dependency; no test runtime added.
const source = await readFile(new URL('../src/lib/siteDirectory.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } })
const { ALL_SITES, UNKNOWN_SITE, parseDirectorySite, siteForProfile, buildWebsiteGroups, searchDirectoryProfiles, readDirectoryPreferences } =
  await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`)
const profile = (id, url, extra = {}) => ({ id, name: id, default_url: url, url_mode: 'custom', last_url: 'https://login.example.org/', order: 0, tags: [], group: '', note: '', ...extra })

test('normalizes scheme, case, www, paths, explicit known alias', () => {
  assert.equal(parseDirectorySite(' HTTPS://WWW.Chat01.ai/a?q=1 ').key, 'chat01.ai')
  assert.equal(parseDirectorySite('claude.ai').name, 'Claude')
  assert.equal(parseDirectorySite('https://chat.openai.com/').key, 'chatgpt.com')
  assert.equal(parseDirectorySite('https://claude.ai', '工作 Claude').name, '工作 Claude')
})
test('rejects invalid schemes, credentials and empty input', () => {
  for (const url of ['', 'ftp://example.com', 'javascript:alert(1)', 'file:///C:/test', 'https://a b', 'https://u:p@example.com']) {
    assert.throws(() => parseDirectorySite(url))
  }
})
test('keeps unrelated subdomains and non-default ports separate', () => {
  assert.notEqual(parseDirectorySite('a.example.co.uk').key, parseDirectorySite('b.example.co.uk').key)
  assert.notEqual(parseDirectorySite('https://localhost:3000').key, parseDirectorySite('https://localhost:4000').key)
})
test('uses configured homepage, never login redirect/last_url', () => {
  assert.equal(siteForProfile(profile('a', 'https://chat01.ai'), 'https://claude.ai').key, 'chat01.ai')
  assert.equal(siteForProfile(profile('a', 'https://chat01.ai', { url_mode: 'inherit' }), 'https://claude.ai').key, 'claude.ai')
  assert.equal(siteForProfile(profile('a', 'invalid url'), 'https://claude.ai').key, UNKNOWN_SITE)
})
test('groups accounts, preserves tags/manual groups and input order', () => {
  const a = profile('a', 'https://chat01.ai', { order: 2, group: '工作', tags: ['常用'] })
  const b = profile('b', 'https://www.chat01.ai/chat', { order: 1 })
  const c = profile('c', 'https://claude.ai')
  const input = [a, b, c], snapshot = JSON.stringify(input)
  const groups = buildWebsiteGroups(input, 'https://chat01.ai')
  assert.deepEqual(groups.find(g => g.key === 'chat01.ai').profiles.map(p => p.id), ['b', 'a'])
  assert.equal(groups.length, 2)
  assert.equal(JSON.stringify(input), snapshot)
})
test('retains saved empty sites and custom names without duplicating the same domain', () => {
  const saved = [parseDirectorySite('https://github.com'), parseDirectorySite('https://chat01.ai', '我的 AI')]
  const groups = buildWebsiteGroups([profile('a', 'https://chat01.ai')], 'https://claude.ai', saved)
  assert.equal(groups[0].profiles.length, 0)
  assert.equal(groups[1].name, '我的 AI')
  assert.equal(groups.length, 2)
})
test('searches names, notes, tags, groups and URLs case-insensitively', () => {
  const list = [profile('Work', 'https://chat01.ai', { tags: ['常用'], note: '主账号', group: '工作' }), profile('Other', 'https://claude.ai')]
  for (const query of ['work', ' 常用 ', '主账号', '工作', 'CHAT01']) assert.equal(searchDirectoryProfiles(list, query)[0].id, 'Work')
  assert.equal(searchDirectoryProfiles(list, 'not-found').length, 0)
  assert.equal(searchDirectoryProfiles(list, ' ').length, 2)
})
test('recovers corrupt local preferences and validates saved website records', () => {
  for (const raw of ['bad json', 'null', '42']) assert.equal(readDirectoryPreferences(raw).selected, ALL_SITES)
  const parsed = readDirectoryPreferences(JSON.stringify({ columns: 99, selected: 'claude.ai', sites: [null, {url:'javascript:alert(1)'}, {url:'https://claude.ai',name:'A'}, {url:'https://www.claude.ai',name:'B'}] }))
  assert.equal(parsed.columns, 2)
  assert.equal(parsed.sites.length, 1)
  assert.equal(parsed.sites[0].name, 'B')
  assert.equal(readDirectoryPreferences('{"columns":1}').columns, 1)
})
