import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { test } from 'node:test'
import ts from 'typescript'

async function loadPureModule(path) {
  const source = await readFile(new URL(path, import.meta.url), 'utf8')
  const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } })
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`)
}

const { normalizeUiPreferences } = await loadPureModule('../src/lib/uiPreferences.ts')

const ids = (items) => items.map((item) => item.id)
const enabledOf = (items, id) => items.find((item) => item.id === id)?.enabled

/** v34 及以前默认写入 localStorage 的顺序（当时还没有 directory 项）。 */
const LEGACY_ORDER = ['reload', 'home', 'back', 'forward', 'copy-url', 'overview', 'downloads', 'settings']

test('新增的「网站导航」插到「主页」之后，而不是被追加到工具栏末尾', () => {
  const saved = LEGACY_ORDER.map((id) => ({ id, enabled: true }))
  const items = normalizeUiPreferences({ browserToolbarItems: saved }).browserToolbarItems
  assert.deepEqual(ids(items), ['reload', 'home', 'directory', 'back', 'forward', 'copy-url', 'overview', 'downloads', 'settings'])
  // 新增项默认可用，老用户无需进设置页手动打开
  assert.equal(enabledOf(items, 'directory'), true)
})

test('锚点跟随用户自己的排序：把「主页」挪到首位时，新增项也跟到首位', () => {
  const saved = ['home', ...LEGACY_ORDER.filter((id) => id !== 'home')].map((id) => ({ id, enabled: true }))
  const items = normalizeUiPreferences({ browserToolbarItems: saved }).browserToolbarItems
  assert.deepEqual(ids(items).slice(0, 3), ['home', 'directory', 'reload'])
})

test('没有历史数据时给出完整默认顺序，且不重复', () => {
  const items = normalizeUiPreferences({}).browserToolbarItems
  assert.deepEqual(ids(items), ['reload', 'home', 'directory', 'back', 'forward', 'copy-url', 'overview', 'downloads', 'settings'])
  assert.equal(new Set(ids(items)).size, items.length)
})

test('用户已经关掉的新增项保持关闭，不会被重新打开或重复插入', () => {
  const saved = [...LEGACY_ORDER, 'directory'].map((id) => ({ id, enabled: id !== 'directory' }))
  const items = normalizeUiPreferences({ browserToolbarItems: saved }).browserToolbarItems
  assert.equal(items.filter((item) => item.id === 'directory').length, 1)
  assert.equal(enabledOf(items, 'directory'), false)
})

test('缺失项按各自的默认开关补齐：back 默认关闭，新增项默认开启', () => {
  const saved = LEGACY_ORDER.filter((id) => id !== 'back').map((id) => ({ id, enabled: true }))
  const items = normalizeUiPreferences({ browserToolbarItems: saved }).browserToolbarItems
  assert.deepEqual(ids(items), ['reload', 'home', 'directory', 'back', 'forward', 'copy-url', 'overview', 'downloads', 'settings'])
  // 这两个默认值写死在 BROWSER_TOOLBAR_BUILTINS 里；改默认值时应同步改这里
  assert.equal(enabledOf(items, 'back'), false)
  assert.equal(enabledOf(items, 'directory'), true)
})

test('未知 id 被丢弃，已知项的开关状态原样保留', () => {
  const saved = [
    { id: 'reload', enabled: false },
    { id: 'not-a-button', enabled: true },
    { id: 'reload', enabled: true },
    { id: 'settings', enabled: false },
  ]
  const items = normalizeUiPreferences({ browserToolbarItems: saved }).browserToolbarItems
  assert.equal(items.filter((item) => item.id === 'reload').length, 1)
  assert.equal(enabledOf(items, 'reload'), false)
  assert.equal(enabledOf(items, 'settings'), false)
  assert.equal(ids(items).includes('not-a-button'), false)
})
