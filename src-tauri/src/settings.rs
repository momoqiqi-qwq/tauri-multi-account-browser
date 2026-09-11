//! 应用设置的读取、写入与下载配置下发。

use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use crate::*;

pub(crate) fn resolved_download_dir(app: &AppHandle, settings: &AppSettings) -> PathBuf {
    let trimmed = settings.download_dir.trim();
    if !trimmed.is_empty() {
        return PathBuf::from(trimmed);
    }
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("downloads")
}

/// 把当前下载配置推送到所有账号页面（自动下载开关 / 扩展名白名单）。
/// 通过自定义事件下发，避免在页面 window 上留全局变量痕迹。
pub(crate) fn download_cfg_eval_script(app: &AppHandle) -> String {
    let settings = load_app_settings(app);
    let normalized_exts = normalize_ai_exts(&settings.ai_exts);
    let exts: Vec<String> = normalized_exts
        .split(',')
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .collect();
    let downloaded_names: Vec<String> = load_download_ledger(app).names.into_iter().collect();
    let cfg = serde_json::json!({
        "auto": settings.auto_ai_download,
        "skipDownloaded": true,
        "downloadedNames": downloaded_names,
        "blockFirstSeconds": settings.download_guard_seconds.min(60),
        "guardAutoRetry": settings.download_guard_auto_retry,
        "exts": exts
    });
    format!(
        "try {{ document.dispatchEvent(new CustomEvent('mb-dl-cfg', {{ detail: {cfg} }})); }} catch (e) {{ }}"
    )
}

pub(crate) fn push_download_cfg(app: &AppHandle) {
    let script = download_cfg_eval_script(app);
    for (label, webview) in app.webviews() {
        if label.starts_with(PROFILE_PREFIX) {
            let _ = webview.eval(&script);
        }
    }
}

#[tauri::command]
pub(crate) fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    let mut settings = load_app_settings(&app);
    settings.download_history_limit = settings.download_history_limit.clamp(50, 2000);
    settings.download_guard_seconds = settings.download_guard_seconds.min(60);
    settings.download_guard_rules = normalized_guard_rules(settings.download_guard_rules);
    settings.skip_downloaded_files = true;
    settings.ai_exts = normalize_ai_exts(&settings.ai_exts);
    settings.global_default_url = normalize_url(&settings.global_default_url)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| DEFAULT_PROFILE_URL.to_string());
    if settings.download_dir.trim().is_empty() {
        settings.download_dir = resolved_download_dir(&app, &settings)
            .to_string_lossy()
            .to_string();
    }
    Ok(settings)
}

#[tauri::command]
pub(crate) fn set_app_settings(app: AppHandle, mut settings: AppSettings) -> Result<(), String> {
    let previous_global = normalized_global_default_url(&app).to_string();
    settings.download_history_limit = settings.download_history_limit.clamp(50, 2000);
    settings.download_guard_seconds = settings.download_guard_seconds.min(60);
    settings.download_guard_rules = normalized_guard_rules(settings.download_guard_rules);
    settings.skip_downloaded_files = true;
    settings.ai_exts = normalize_ai_exts(&settings.ai_exts);
    settings.global_default_url = normalize_url(&settings.global_default_url)?.to_string();
    if let Ok(store) = app.store(STORE_FILE) {
        store.set(
            SETTINGS_KEY,
            serde_json::to_value(&settings).map_err(|e| e.to_string())?,
        );
        store.save().map_err(|e| e.to_string())?;
    }
    // 继承账号若仍停在旧全局主页，跟随到新主页；若用户已经浏览到其他页面则保留 last_url。
    if previous_global != settings.global_default_url {
        if let Ok(mut profiles) = load_profiles(&app) {
            let mut changed = false;
            for profile in &mut profiles {
                if profile.url_mode == "inherit"
                    && (profile.last_url.trim().is_empty() || profile.last_url == previous_global)
                {
                    profile.last_url = settings.global_default_url.clone();
                    changed = true;
                }
            }
            if changed {
                save_profiles(&app, &profiles)?;
            }
        }
    }
    // 配置即时下发到所有已打开的账号页面（自动下载开关 / 扩展名白名单）。
    push_download_cfg(&app);
    Ok(())
}
