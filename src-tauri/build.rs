fn main() {
    tauri_build::build();

    // 注意：这里**不要**给构建目标注入 Common-Controls v6 manifest。
    //
    // Windows 上有两份 comctl32：System32\comctl32.dll 是 v5.82，不导出
    // TaskDialogIndirect；只有 WinSxS 的 Common-Controls v6 才导出它。
    // 打开 tauri 的 `test` feature 后，测试二进制会链入 tray-icon/muda 从而依赖该导出，
    // 没有 v6 manifest 就会以 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) 启动失败。
    //
    // 但正式 app 的 MANIFEST 资源由 tauri-build 通过 resource.lib 注入，
    // 若这里再叠加一份，链接器会报 CVT1100「资源重复（类型 MANIFEST，名称 1）」。
    // 而 build script 又**拿不到** CARGO_CFG_TEST（那是给被编译 crate 用的），
    // 无法在 build.rs 里区分构建类型；`cargo:rustc-link-arg-tests` 也只作用于
    // tests/ 下的集成测试，覆盖不到 `--lib` 的单元测试 harness。
    //
    // 因此改为只在那一次调用里注入：见 scripts/cargo-test.sh（用 --config 传 rustflags）。
}
