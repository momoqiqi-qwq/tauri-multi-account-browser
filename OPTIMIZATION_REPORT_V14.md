# v14 实现与后续优化建议

v14 已把账号默认网址从“每账号复制一个字符串”升级为正式继承模型，并加入模板、批量管理、配置导入导出和启动诊断自救能力。

下一阶段最值得做的是：

1. 把模板从 localStorage 迁入 Rust store，支持随完整备份统一迁移。
2. 导入导出增加原生文件选择器、CSV、冲突策略和逐行错误报告。
3. 批量创建改为单个 Rust command 原子提交，避免第 N 个失败时留下部分结果。
4. 为配置引入 `schema_version` 和显式 migration，而不是长期依赖 serde 默认值。
5. 将 `src-tauri/src/lib.rs` 拆分为 profiles / webviews / settings / downloads / diagnostics 模块。
6. 将 App.vue 中账号生命周期、标签生命周期和设置加载拆成 composables。
7. 诊断中心继续增加 WebView2 Runtime 检测、代理真实连通性检测、数据目录占用检测和错误码分类。
8. 增加账号标签、收藏、最近使用、批量选择与批量修改主页模式。

---

## 完成情况（截至 0.7.0 / v23）

以上 8 条**已全部完成**。

| # | 内容 | 版本 | 说明 |
| --- | --- | --- | --- |
| 1 | 模板迁入 Rust store | v15 | 随 `profiles.json` 统一迁移，localStorage 数据幂等导入一次后清除 |
| 2 | 导入导出增强 | v18 | 原生文件选择器 + CSV + 冲突策略 + 逐行错误报告 |
| 3 | 批量创建原子提交 | v16 | `create_profiles_bulk`：整批校验通过才落盘 |
| 4 | `schema_version` + 显式迁移 | v17 | 当前 `SCHEMA_VERSION = 2` |
| 5 | 拆分 `lib.rs` | v19 | 4326 行 → 9 个领域模块，`lib.rs` 剩约 1.1k 行 |
| 6 | 拆分 `App.vue` | v20 | 817 行 → 477 行，逻辑进 `useProfiles` / `useTabs` / `useSettings` |
| 7 | 诊断中心增强 | v21 | 错误码分类 + 修复建议、WebView2 运行时检测、数据目录占用、代理延迟 |
| 8 | 标签 / 收藏 / 最近 / 批量 | v22 | 含 `update_profile_batch` 原子批量修改 |

### 过程中额外修掉的（不在原清单里，但都是真问题）

| 问题 | 版本 | 影响 |
| --- | --- | --- |
| `Cargo.toml` 缺 `[features] custom-protocol` | v22 | **发布构建根本没打包前端**，打开白屏。此前多轮"release 构建通过"只证明了编译通过 |
| 批量操作的成功提示在异步写入前就弹出 | v22 | 失败时会先看到"已修改"再看到报错 |
| 标签被删光后 `activeTag` 残留 | v22 | 列表永久停在空状态 |
| 批量选择不随筛选收敛 | v23 | 批量操作会打到用户看不见的账号上 |
| 批量路径重复解析全局主页 | v23 | N 个账号反序列化 N 次设置，且跑在主线程 |
| 标签筛选/批量逻辑无测试 | v22/v23 | 已补 `update_profile_batch_*` 等用例，共 43 个 |

### 发布相关（务必看 README 4.1 / 4.2）

- 发布命令：`bash scripts/tauri-msvc.sh build`（本机唯一验证可用的路径）。
- 发布后校验：`npm run verify:assets` —— 解压 `tauri-codegen-assets/` 与 `dist/` 逐字节比对。
- **不要**用"在 exe 里 grep 前端字符串"验证：资源是 brotli 压缩嵌入的，明文搜不到，会误判。
- 完整安装包本机出不了（缺 NSIS / WiX，而 `bundle.targets = "all"`）。
