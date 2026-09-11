// 校验 release 构建确实把前端资源嵌进了可执行文件。
//
// 背景：tauri 的 `compression` 是默认 feature，资源是 **brotli 压缩后** 嵌入的，
// 所以直接在 .exe 里 grep 前端源码字符串是搜不到的（会误判成"没打包前端"）。
// 正确的做法是去 tauri-build 的生成目录 `out/tauri-codegen-assets/` 拿到嵌入用的
// 字节，解压后跟 dist 里的产物逐字节比对。
//
// 另一个副产品：这个目录 **只有在开了 custom-protocol 时才会生成**。如果它不存在，
// 说明发布构建没开 custom-protocol —— 那样打出来的 exe 会去连 devUrl 而白屏。
//
// 用法: node scripts/verify-embedded-assets.mjs
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { brotliDecompressSync } from 'node:zlib'

const OUT_ROOT = join('src-tauri', 'target', 'release', 'build')

function findAssetsDir() {
  if (!existsSync(OUT_ROOT)) return null
  let newest = null
  for (const name of readdirSync(OUT_ROOT)) {
    if (!name.startsWith('tauri-multi-account-browser-')) continue
    const dir = join(OUT_ROOT, name, 'out', 'tauri-codegen-assets')
    if (!existsSync(dir)) continue
    const mtime = statSync(dir).mtimeMs
    if (!newest || mtime > newest.mtime) newest = { dir, mtime }
  }
  return newest?.dir ?? null
}

const assetsDir = findAssetsDir()
if (!assetsDir) {
  console.error(
    '没有找到 out/tauri-codegen-assets/。\n' +
      '通常意味着发布构建没开 custom-protocol（exe 会去连 devUrl 而白屏），' +
      '或者还没跑过 release 构建。\n' +
      '请用 scripts/tauri-msvc.sh build 构建，不要用裸 cargo build --release。'
  )
  process.exit(1)
}

// dist 的原始产物，按扩展名索引
const dist = {}
for (const f of readdirSync(join('dist', 'assets'))) {
  const kind = f.endsWith('.js') ? 'js' : f.endsWith('.css') ? 'css' : null
  if (kind) dist[kind] ??= readFileSync(join('dist', 'assets', f))
}
dist.html ??= readFileSync('dist/index.html')

let failed = 0
for (const f of readdirSync(assetsDir)) {
  const kind = f.slice(f.lastIndexOf('.') + 1)
  const expect = dist[kind]
  if (!expect) {
    console.log('SKIP ' + f + '（dist 里没有同类文件，可能是历史残留）')
    continue
  }
  let out
  try {
    out = brotliDecompressSync(readFileSync(join(assetsDir, f)))
  } catch (e) {
    console.log('MISS ' + f + ' 解压失败: ' + e.message)
    failed++
    continue
  }
  const same = out.length === expect.length && out.equals(expect)
  if (!same) failed++
  console.log(
    (same ? 'OK  ' : 'MISS') + ' ' + kind + '  解压后 ' + out.length + 'B  ' +
      (same ? '与 dist 逐字节一致' : '与 dist 不一致')
  )
}

console.log(
  failed === 0
    ? '\n前端资源已正确嵌入（含 custom-protocol）。'
    : '\n有 ' + failed + ' 个资源对不上。'
)
process.exit(failed === 0 ? 0 : 1)
