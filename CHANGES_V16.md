# v16 / 批量创建改为原子提交

本版落实 `OPTIMIZATION_REPORT_V14.md` 第 3 条建议：批量创建与 JSON 导入改为单次 Rust 命令原子提交，
避免「第 N 个失败时留下部分结果」。

## 1. 新增 `create_profiles_bulk` 命令

- 入参 `drafts: Vec<ProfileDraft>`，返回实际创建的 `Vec<Profile>`。
- **先整体校验，全部通过后才写盘**：任一草稿不合格立即返回错误，`save_profiles` 不会被调用，
  因此不会留下半成品账号。
- 错误信息带序号（`第 3 个账号：账号昵称不能为空`），便于用户定位问题条目。
- 单次上限 200（`MAX_BULK_PROFILES`），空列表直接拒绝。
- `order` 在构造阶段一次性按 `base + offset` 固定，避免中途插入影响排序。

## 2. 抽出 `ProfileDraft` + `build_profile` 共用校验

- 原来 `create_profile` 的校验逻辑内联在命令里；现抽成 `build_profile()`，**单个创建与批量创建共用同一套校验口径**。
- `ProfileDraft` 全字段带 serde 默认值，前端可以只传 `name`。
- `create_profile` 行为不变，仍返回单个 `Profile`。

## 3. 前端改为单次调用

- `AccountToolsDialog.vue` 的「批量创建」与「JSON 导入」不再循环 `invoke`。
- 失败提示由「已创建 N 个，随后失败」改为「批量创建失败，未创建任何账号」——
  语义更准确，且不再需要 `emit('changed')` 去刷新可能残留的部分数据。
- 成功时用后端返回的长度作为创建数量（而非前端计数）。

## 4. 顺带补上 Rust 单元测试

`src-tauri/src/lib.rs` 末尾新增 `#[cfg(test)] mod tests`，15 个用例覆盖：
- 昵称校验（空/纯空白/80 字符边界/中文 trim）
- URL 规范化（裸域名补 https、大小写 scheme、拒绝 ftp/file、空值回落默认）
- 代理校验（http/https/socks5 通过，ftp 与非法串拒绝）
- 时区与语言的白名单转义（**注入串必须被拒绝**，如 `Asia/Shanghai'; alert(1)//`）
- 模板规范化（trim、剔除无效项、非法 url_mode 修复、数量上限）
- `ProfileDraft` 反序列化（最小字段 / 前端完整 payload 形状）

## 验证

- `npm run build`（vue-tsc --noEmit + vite build）通过，1629 模块。
- `cargo test --lib`：**15 passed; 0 failed**。
- `cargo check`（lib + bin）通过，零警告零错误。
- `cargo build --release` 通过，产出 `tauri-multi-account-browser.exe`。
- 前后端命令交叉核对：前端所有 `invoke` 均有对应注册命令。
- `create_profile`（单个创建，App.vue 仍在用）与 `create_profiles_bulk`（批量/导入）职责分离，均已注册。

## 未包含

`OPTIMIZATION_REPORT_V14.md` 第 2 条（原生文件选择器 / CSV / 冲突策略 / 逐行错误报告）
与第 4 条（`schema_version` + 显式 migration）留待后续。
