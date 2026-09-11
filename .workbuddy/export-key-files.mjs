// 导出项目关键文件到 Downloads，并生成一份可直接粘贴给 AI 的合并 Markdown
import fs from 'node:fs';
import path from 'node:path';

const ROOT = 'E:/E-Develop-Project/tauri-multi-account-browser';
const DATE = new Date().toISOString().slice(0, 10);
const OUT = `C:/Users/yile/Downloads/tauri-browser-关键文件-${DATE}`;

const FILES = [
  // 前端
  'index.html',
  'src/main.ts',
  'src/App.vue',
  'src/types.ts',
  'src/style.css',
  'src/lib/suppress.ts',
  'src/components/AddressBar.vue',
  'src/components/BrowserViewport.vue',
  'src/components/DownloadsDialog.vue',
  'src/components/ProfileSettingsDialog.vue',
  'src/components/ProfileSidebar.vue',
  'src/components/StatusSidebar.vue',
  'src/components/TabStrip.vue',
  // 后端 Rust
  'src-tauri/src/main.rs',
  'src-tauri/src/lib.rs',
  'src-tauri/Cargo.toml',
  'src-tauri/build.rs',
  'src-tauri/tauri.conf.json',
  'src-tauri/capabilities/default.json',
  // 工程配置
  'package.json',
  'vite.config.ts',
  'tsconfig.json',
  'scripts/make-icon.mjs',
];

const LANG = {
  '.vue': 'vue', '.ts': 'ts', '.js': 'js', '.mjs': 'js',
  '.rs': 'rust', '.json': 'json', '.html': 'html', '.css': 'css', '.toml': 'toml',
};

fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });

const copied = [];
const missing = [];

for (const rel of FILES) {
  const src = path.join(ROOT, rel);
  if (!fs.existsSync(src)) { missing.push(rel); continue; }
  const dst = path.join(OUT, rel);
  fs.mkdirSync(path.dirname(dst), { recursive: true });
  fs.copyFileSync(src, dst);
  const content = fs.readFileSync(src, 'utf8');
  copied.push({ rel, content, lines: content.split('\n').length, bytes: Buffer.byteLength(content) });
}

// README 单独复制（作为背景资料，不进合并文件）
const readmeSrc = path.join(ROOT, 'README.md');
if (fs.existsSync(readmeSrc)) fs.copyFileSync(readmeSrc, path.join(OUT, 'README.md'));

// ---------- 生成合并文件 ----------
const totalLines = copied.reduce((a, f) => a + f.lines, 0);
let md = `# Tauri 多账户浏览器 - 关键源码合集

> 导出时间：${new Date().toLocaleString('zh-CN')}
> 技术栈：Tauri 2.x + Rust + Vue 3 + TypeScript + Element Plus
> 共 ${copied.length} 个文件，${totalLines} 行代码

## 给 AI 的说明

这是一个**多账户浏览器容器**：一个 Tauri 主窗口（Vue 渲染 UI 外壳）+ 每个账号一个原生 child WebView。
每个账号（Profile）有独立的持久化数据目录，Cookie / LocalStorage / IndexedDB 互相隔离。
项目**不保存任何用户名密码**，只保存账号元数据。

目录结构与原项目一致，文件按「后端 → 前端 → 配置」排序。

---

## 文件清单

| 文件 | 行数 | 说明 |
|---|---|---|
`;
const DESC = {
  'src-tauri/src/lib.rs': 'Rust 核心：Profile CRUD、WebView 创建/隐藏/显示/销毁、导航控制',
  'src-tauri/src/main.rs': 'Rust 入口',
  'src-tauri/Cargo.toml': 'Rust 依赖清单',
  'src-tauri/tauri.conf.json': 'Tauri 应用配置（窗口、打包、资源）',
  'src-tauri/capabilities/default.json': 'Tauri 权限配置',
  'src-tauri/build.rs': '构建脚本',
  'src/App.vue': 'Vue 主组件：整体布局与状态编排',
  'src/types.ts': 'TypeScript 类型定义',
  'src/main.ts': '前端入口',
  'src/style.css': '全局样式',
  'src/lib/suppress.ts': '控制台噪音抑制',
  'src/components/AddressBar.vue': '地址栏',
  'src/components/BrowserViewport.vue': 'WebView 容器占位',
  'src/components/DownloadsDialog.vue': '下载管理弹窗',
  'src/components/ProfileSettingsDialog.vue': '账号设置弹窗',
  'src/components/ProfileSidebar.vue': '账号侧边栏',
  'src/components/StatusSidebar.vue': '状态侧边栏',
  'src/components/TabStrip.vue': '标签栏（一个标签 = 一个账号）',
  'package.json': 'npm 依赖与脚本',
  'vite.config.ts': 'Vite 配置',
  'tsconfig.json': 'TypeScript 配置',
  'index.html': 'HTML 入口',
  'scripts/make-icon.mjs': '图标生成脚本',
};
for (const f of copied) md += `| \`${f.rel}\` | ${f.lines} | ${DESC[f.rel] || ''} |\n`;

md += `\n---\n\n# 源码\n`;
for (const f of copied) {
  const ext = path.extname(f.rel);
  md += `\n## ${f.rel}\n\n\`\`\`${LANG[ext] || ''}\n${f.content.replace(/\n$/, '')}\n\`\`\`\n`;
}

fs.writeFileSync(path.join(OUT, '00_全部源码_合并.md'), md, 'utf8');

// ---------- 生成说明文件 ----------
const guide = `# 导出说明

导出目录：\`${OUT}\`
导出时间：${new Date().toLocaleString('zh-CN')}

## 怎么用

### 方式一：直接把单个文件发给 AI（推荐）
把 \`00_全部源码_合并.md\` 拖给目标 AI，然后说你的需求。
这一个文件包含了全部 ${copied.length} 个源文件、共 ${totalLines} 行代码，AI 能一次性看到完整上下文。

### 方式二：按目录发
目录下的文件保持原项目结构，可以整个文件夹打包发给 AI。

## 目录结构

\`\`\`
${OUT}
├── 00_全部源码_合并.md      <- 合并文件，发这个最方便
├── 00_导出说明.md           <- 本文件
├── README.md                <- 项目原始文档（架构、平台限制说明）
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json
├── scripts/
│   └── make-icon.mjs
├── src/
│   ├── main.ts
│   ├── App.vue
│   ├── types.ts
│   ├── style.css
│   ├── lib/suppress.ts
│   └── components/  (7 个 .vue)
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/default.json
    └── src/
        ├── main.rs
        └── lib.rs
\`\`\`

## 文件行数

${copied.map((f) => `- ${f.rel} — ${f.lines} 行`).join('\n')}

## 没有导出什么

- node_modules/、src-tauri/target/、dist/ — 依赖和构建产物，不需要给 AI
- Cargo.lock、package-lock.json — 锁文件（如需排查依赖版本问题可以单独发）
- src-tauri/icons/、app-icon.png — 图片资源
- \`src-tauri/gen/\` — 平台工程生成物

## 改完怎么放回项目

AI 返回修改后的代码后，按文件路径覆盖回 \`E:\\\\E-Develop-Project\\\\tauri-multi-account-browser\` 对应位置即可。
覆盖前建议先 \`git\` 提交或备份当前版本。
`;
fs.writeFileSync(path.join(OUT, '00_导出说明.md'), guide, 'utf8');

console.log('OUT:', OUT);
console.log('copied:', copied.length, 'lines:', totalLines);
if (missing.length) console.log('MISSING:', missing.join(', '));
console.log('merged size(KB):', Math.round(Buffer.byteLength(md) / 1024));
