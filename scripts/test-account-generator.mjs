import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import ts from 'typescript'

function compiled(path) {
  return ts.transpileModule(readFileSync(new URL(path, import.meta.url), 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
}
const data = text => 'data:text/javascript;base64,' + Buffer.from(text).toString('base64')
const directory = data(compiled('../src/lib/siteDirectory.ts'))
const source = compiled('../src/lib/accountGenerator.ts').replace(/from ['"]\.\/siteDirectory['"]/, `from '${directory}'`)
const { parseGeneratorTargets: parse, planAccountGeneration: plan, generatorPlanSignature: signature, restoreSourceSelection: restore, SOURCE_SELECTION_KEY: selectionKey } = await import(data(source))
const options = { prefix: '', order: 'source-site', separator: ' · ', group: '', copyTags: true, skipExisting: true }
const make = (id, name = id, extra = {}) => ({ id, name, note: '不复制备注', avatar: 'private-avatar', default_url: 'https://source.test/', url_mode: 'custom', last_url: 'https://login.test/', created_at: '2026-09-25', order: 1, incognito: false, proxy: 'socks5://127.0.0.1:1080', user_agent: 'Source UA', timezone: 'Asia/Shanghai', locale: 'zh-CN', fingerprint_guard: true, group: '工作', tags: ['项目'], favorite: true, last_used_at: '2026-09-25', ...extra })
const a = make('a', 'Alice'), b = make('b', 'Bob', { incognito: true })
const targets = parse('Claude | claude.ai\nGitHub | https://github.com/').targets
const build = (profiles = [a, b], ids = ['a', 'b'], opts = options, sites = targets) => plan(sites, ids, profiles, opts, 'https://global.test/')

test('normalizes URLs, labels, blank lines and exact URL duplicates', () => {
  const parsed = parse(' Claude | CLAUDE.ai \r\n\nhttps://claude.ai/\nhttps://github.com/#one\nhttps://github.com/#two')
  assert.equal(parsed.errors.length, 0); assert.equal(parsed.duplicates, 1); assert.equal(parsed.targets.length, 3)
  assert.equal(parsed.targets[0].name, 'Claude'); assert.equal(parsed.targets[0].url, 'https://claude.ai/')
  assert.deepEqual(parsed.targets.map(t => t.line), [1, 4, 5])
})
test('rejects unsupported schemes, credentials, blank targets and invalid labels', () => {
  for (const url of ['javascript:alert(1)', 'file:///a', 'https://u:p@host.test/', 'Name | ', 'a'.repeat(49) + ' | example.com']) assert.equal(parse(url).errors.length, 1, url)
  assert.equal(parse('https://example.com/a|b').errors.length, 0)
  assert.equal(parse('😀'.repeat(48) + ' | example.com').targets[0].name, '😀'.repeat(48))
})
test('bounds input size before planning', () => {
  assert.equal(parse(Array.from({ length: 201 }, (_, i) => `a${i}.test`).join('\n')).targets.length, 0)
  assert.equal(parse('https://example.com/' + 'x'.repeat(4096)).errors.length, 1)
})
test('cross-product creates independent drafts with only the allowed config fields', () => {
  const p = build(); assert.equal(p.drafts.length, 4); assert.equal(p.errors.length, 0)
  const draft = p.drafts[0]
  assert.equal(draft.name, 'Alice · Claude'); assert.equal(draft.default_url, 'https://claude.ai/'); assert.equal(draft.url_mode, 'custom')
  for (const key of ['proxy', 'user_agent', 'timezone', 'locale', 'fingerprint_guard', 'incognito', 'group']) assert.equal(draft[key], a[key])
  assert.deepEqual(Object.keys(draft).sort(), ['name','default_url','url_mode','incognito','proxy','user_agent','timezone','locale','fingerprint_guard','group','tags'].sort())
  assert.equal(p.drafts[1].incognito, true)
  assert.deepEqual(a.tags, ['项目']); draft.tags.push('new'); assert.deepEqual(a.tags, ['项目'])
})
test('prefix, order, separator, group and tag options are reflected in drafts', () => {
  const p = build([a], ['a'], { ...options, prefix: ' Q4 ', order: 'site-source', separator: ' / ', group: ' 设计 ', copyTags: false })
  assert.equal(p.drafts[0].name, 'Q4 / Claude / Alice'); assert.equal(p.drafts[0].group, '设计'); assert.deepEqual(p.drafts[0].tags, [])
})
test('empty or missing selections never generate a partial batch', () => {
  assert.equal(build([], ['missing']).drafts.length, 0)
  assert.match(build([], ['missing']).errors.join(''), /不存在/)
  assert.equal(build([a], []).drafts.length, 0)
  assert.equal(build([a], ['a'], options, []).drafts.length, 0)
  assert.equal(build([a], ['a','a']).drafts.length, 2)
})
test('supports 200 combinations and blocks 201 before materializing drafts', () => {
  const profiles = Array.from({ length: 201 }, (_, i) => make('id' + i))
  assert.equal(build(profiles, profiles.slice(0,200).map(p => p.id), options, targets.slice(0,1)).drafts.length, 200)
  const over = build(profiles, profiles.map(p => p.id), options, targets.slice(0,1)); assert.equal(over.drafts.length, 0); assert.equal(over.rows.length, 0); assert.match(over.errors[0], /201/)
})
test('repeated source names get deterministic suffixes instead of dropping distinct sources', () => {
  const p = build([a, make('b', 'Alice')]); assert.deepEqual(p.drafts.slice(0,2).map(d => d.name), ['Alice · Claude', 'Alice · Claude (2)'])
  assert.equal(p.rows[1].renamed, true)
})
test('repeat generation skips exact name + full homepage, never overwrites', () => {
  const first = build(); const existing = first.drafts.map((d,i) => make('new'+i,d.name,d))
  const again = build([a,b,...existing]); assert.equal(again.drafts.length, 0); assert.equal(again.skipped, 4)
  assert.equal(existing[0].name, first.drafts[0].name)
  const rename = build([a,b,...existing], ['a','b'], { ...options, skipExisting: false })
  assert.equal(rename.drafts.length, 4); assert.equal(rename.drafts[0].name, 'Alice · Claude (2)')
})
test('duplicate policy uses full configured home, not login redirect or host grouping', () => {
  const existing = make('e', 'Alice · Claude', { default_url: 'https://claude.ai/path', last_url: 'https://claude.ai/' })
  assert.equal(build([a, existing], ['a']).skipped, 0)
  const inherited = make('e', 'Alice · Global', { url_mode: 'inherit', default_url: 'https://old.test/' })
  assert.equal(build([a, inherited], ['a'], options, parse('Global | global.test').targets).skipped, 1)
  const upper = make('e', 'alice · Claude', { default_url: 'https://claude.ai/' })
  assert.equal(build([a, upper], ['a']).skipped, 0)
})
test('names over 80 Unicode code points block the whole batch without silent truncation', () => {
  assert.equal(build([make('long', 'a'.repeat(80))], ['long']).drafts.length, 0)
  const p = build([make('emoji', '😀'.repeat(60))], ['emoji'], options, parse('X | x.test').targets)
  assert.equal(p.errors.length, 0); assert.equal([...p.drafts[0].name].length, 64)
})
test('preflight signatures detect changed configs and new duplicates, but ignore unrelated history', () => {
  const original = signature(build()); assert.notEqual(original, signature(build([{...a, proxy: ''}, b])))
  assert.equal(original, signature(build([{...a,last_url:'https://elsewhere.test/'},b])))
  assert.notEqual(original, signature(build([a,b,make('e','Alice · Claude',{default_url:'https://claude.ai/'})])))
})
test('remembered source selection keeps order, drops deleted accounts and survives corrupted storage', () => {
  const ids = ['a', 'b', 'c']
  assert.deepEqual(restore(JSON.stringify(['c', 'a']), ids), ['c', 'a'])
  // 记忆里已删除的账号绝不能复活，否则下次打开会直接卡在「来源账号已删除」错误上
  assert.deepEqual(restore(JSON.stringify(['a', 'gone', 'c']), ids), ['a', 'c'])
  assert.deepEqual(restore(JSON.stringify(['a', 'a', 'b']), ids), ['a', 'b'])
  assert.deepEqual(restore(JSON.stringify(['a', 7, null, {}, '']), ids), ['a'])
  assert.deepEqual(restore(JSON.stringify(['a']), []), [])
  for (const broken of [null, undefined, '', '{not json', JSON.stringify({ a: 1 }), JSON.stringify('a')]) {
    assert.deepEqual(restore(broken, ids), [], String(broken))
  }
  assert.equal(selectionKey, 'mab-generator-source-selection-v1')
})
