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
