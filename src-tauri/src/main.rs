// release 版使用 Windows GUI 子系统，启动时不再弹出后台终端窗口；
// dev 版保留控制台，方便看调试日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri_multi_account_browser_lib::run()
}
