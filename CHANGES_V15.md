# v15 / 0.6.1 变更说明

本版落实 `OPTIMIZATION_REPORT_V14.md` 的第 1 条建议：把账号模板从浏览器 localStorage 迁入 Rust store。

## 1. 账号模板改存 Rust store

- 新增 `list_profile_templates` / `save_profile_templates` 两个命令，写入 `profiles.json` 的 `profile_templates_v1` 键。
- 模板随应用数据目录一起存留，不再依赖某个 WebView 的 localStorage；后续做完整备份时可以统一迁移。
- 新增 `ProfileTemplate` 结构体，字段与原前端 `ProfileTemplate` 一一对应，全部带 serde 默认值以兼容旧数据。
- 保存时在 Rust 侧做规范化：去首尾空白、剔除无名/无 id 项、`url_mode` 非法值回落 `custom`、数量上限 200。
- 保存命令返回规范化后的列表，前端以此回写本地状态，保证前后端一致。

## 2. 旧数据自动一次性迁移

- 打开“账号管理工具”时先读 Rust store。
- 若 store 为空且 localStorage（`mab-profile-templates-v14`）里存在旧模板，自动写入 Rust store 并清空 localStorage，同时提示迁移数量。
- 迁移是幂等的：store 一旦有数据就不会再碰 localStorage，不会重复导入或覆盖。

## 3. 失败处理

- 读取模板失败不阻塞账号管理界面，退化为空列表并提示错误。
- 写入失败提示错误，并保留用户当前的本地编辑内容。

## 验证

- `npm run build`（vue-tsc --noEmit + vite build）通过，1629 模块。
- `cargo check`（lib + bin）通过，零警告零错误。
- `cargo build --release` 通过，产出 `tauri-multi-account-browser.exe`。
- 前后端命令名交叉核对：前端全部 `invoke` 调用均有对应注册命令。
- `ProfileTemplate` 前后端字段逐项 diff 一致（11 个字段）。

## 未包含

按需求本版只做模板迁移；`OPTIMIZATION_REPORT_V14.md` 第 3 条（批量创建改为单个 Rust command 原子提交）
与第 2 条（原生文件选择器 / CSV / 冲突策略）留待后续版本。
