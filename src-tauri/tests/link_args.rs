//! 集成测试入口。
//!
//! 存在这个文件的主要目的，是让 cargo 认为本包拥有 **test target** ——
//! 只有这样 `build.rs` 里的 `cargo:rustc-link-arg-tests` 才会被接受。
//!
//! 背景：Windows 上 `System32\comctl32.dll` 是 v5.82，不导出 `TaskDialogIndirect`；
//! 该导出只在 Common-Controls v6（WinSxS）里。打开 tauri 的 `test` feature 后，
//! 测试二进制会链入 tray-icon/muda，从而依赖这个导出。没有 v6 manifest 时进程
//! 会以 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) 启动失败。
//!
//! 正式 app 由 tauri-build 注入 manifest，而 `cargo test` 的 harness 不会，
//! 所以必须靠 build.rs 补上。注意：那个 link-arg **只能**作用于 test target，
//! 否则会与 app 自己的 MANIFEST 资源冲突（CVT1100 资源重复）。
//!
//! 真正的单元测试都写在 `src/lib.rs` 的 `#[cfg(test)] mod tests` 里，
//! 通过 `cargo test --lib` 运行。这个文件本身不需要断言。

#[test]
fn test_target_exists_so_link_args_apply() {
    // 占位：确保存在至少一个集成测试，让 test target 合法。
    // 真实断言在 src/lib.rs 的单元测试模块中。
    assert_eq!(1 + 1, 2);
}
