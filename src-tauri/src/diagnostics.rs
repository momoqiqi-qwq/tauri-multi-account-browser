//! 启动诊断、自救修复与代理连通性测试。

use chrono::Utc;
use std::{collections::HashMap, time::Duration};
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use crate::*;

#[tauri::command]
pub(crate) fn diagnose_profile(app: AppHandle, id: String) -> Result<ProfileDiagnostic, String> {
    let profile = load_profiles(&app)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;
    Ok(ProfileDiagnostic {
        id: profile.id.clone(),
        name: profile.name.clone(),
        url_mode: profile.url_mode.clone(),
        global_default_url: normalized_global_default_url(&app).to_string(),
        effective_home_url: effective_profile_home_url(&app, &profile).to_string(),
        last_url: profile.last_url.clone(),
        last_url_valid: normalize_url(&profile.last_url).is_ok(),
        proxy_configured: !profile.proxy.trim().is_empty(),
        proxy_valid: parse_proxy(&profile.proxy).is_ok(),
    })
}

#[tauri::command]
pub(crate) async fn repair_profile_startup(
    app: AppHandle,
    id: String,
    action: String,
) -> Result<Profile, String> {
    let label = profile_label(&id);
    if let Some(webview) = app.get_webview(&label) {
        let _ = webview.close();
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    let mut profiles = load_profiles(&app)?;
    let profile = profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;
    match action.as_str() {
        "clear_last_url" | "home" => {
            profile.last_url = effective_profile_home_url(&app, profile).to_string()
        }
        "global" => {
            profile.url_mode = "inherit".to_string();
            profile.last_url = normalized_global_default_url(&app).to_string();
        }
        "disable_proxy" => profile.proxy.clear(),
        "reset_runtime" => profile.last_url = effective_profile_home_url(&app, profile).to_string(),
        _ => return Err("未知修复操作".to_string()),
    }
    let result = profile.clone();
    save_profiles(&app, &profiles)?;
    Ok(result)
}

/// 代理连通性检测：用系统 curl 经该账号的代理访问 ip-api.com，
/// 拿出口 IP / 地区 / ISP，结果写入账号状态并广播（直连账号测的就是本机出口）。
#[tauri::command]
pub(crate) async fn test_profile_proxy(
    app: AppHandle,
    id: String,
) -> Result<ProxyTestResult, String> {
    let profile = load_profiles(&app)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;

    let app_handle = app.clone();
    let profile_id = profile.id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = hidden_curl();
        cmd.args(["-s", "--max-time", "12", "-A", "MultiAccountBrowser/0.1"]);
        let proxy = profile.proxy.trim();
        if !proxy.is_empty() {
            cmd.args(["-x", proxy]);
        }
        cmd.arg("http://ip-api.com/json/?lang=zh-CN");
        match cmd.output() {
            Ok(output) => Ok((
                profile_id,
                String::from_utf8_lossy(&output.stdout).to_string(),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err("系统 curl 不可用，请确认 Windows 版本（Win10 1803+ 自带）".to_string())
            }
            Err(e) => Err(format!("启动 curl 失败：{e}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    let (profile_id, stdout) = match result {
        Ok(pair) => pair,
        Err(msg) => {
            return Ok(ProxyTestResult {
                ok: false,
                ip: String::new(),
                region: String::new(),
                isp: String::new(),
                error: msg,
            })
        }
    };
    let mut result = ProxyTestResult {
        ok: false,
        ip: String::new(),
        region: String::new(),
        isp: String::new(),
        error: String::new(),
    };

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&stdout) {
        let get = |key: &str| {
            value
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        if value.get("status").and_then(|v| v.as_str()) == Some("success") {
            result.ok = true;
            result.ip = get("query");
            result.region = format!("{} {}", get("country"), get("city"))
                .trim()
                .to_string();
            result.isp = get("isp");
        } else {
            result.error = get("message");
        }
    }
    if !result.ok && result.error.is_empty() {
        result.error = "检测失败：网络超时或系统 curl 不可用".to_string();
    }

    if result.ok {
        if let Ok(profiles) = load_profiles(&app_handle) {
            if profiles.iter().any(|p| p.id == profile_id) {
                let mut map: HashMap<String, ProfileStatus> = app_handle
                    .store(STORE_FILE)
                    .ok()
                    .and_then(|store| store.get(STATUS_KEY))
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
                let status = map.entry(profile_id.clone()).or_default();
                status.proxy_ip = Some(result.ip.clone());
                status.proxy_region = Some(result.region.clone());
                status.proxy_isp = Some(result.isp.clone());
                status.proxy_tested_at = Some(Utc::now().to_rfc3339());
                let status = status.clone();
                save_profile_status(&app_handle, &profile_id, status);
            }
        }
    }
    Ok(result)
}
