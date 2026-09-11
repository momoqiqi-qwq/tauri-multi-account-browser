//! 启动诊断、自救修复与代理连通性测试。
//!
//! 这里的原则是：**诊断要能自己说清楚原因**。
//! 后端抛出的原始错误通常是一长串英文（来自 WebView2 / 系统 IO），
//! 用户看不懂，所以统一走 `classify_error` 归类 + `hint` 给建议。

use chrono::Utc;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

use crate::*;

/// WebView2 Runtime 在 EdgeUpdate 里的客户端 ID（Evergreen 版）。
#[cfg(target_os = "windows")]
const WEBVIEW2_CLSID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// 把后端抛出的原始错误串归类成 `DiagnosticCode`。
///
/// 判定顺序很关键：越具体的越靠前。比如 "proxy" 的错误里也可能出现
/// "connection"，但显然应该归到代理而不是网络。
pub(crate) fn classify_error(message: &str) -> DiagnosticCode {
    let lower = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));

    if has(&["webview2", "webview"]) {
        DiagnosticCode::Webview2Runtime
    } else if has(&["proxy", "socks", "代理"]) {
        DiagnosticCode::Proxy
    } else if has(&[
        "permission",
        "denied",
        "os error 5",
        "os error 32",
        "os error 28",
        "read-only",
        "权限",
        "占用",
    ]) {
        DiagnosticCode::DataDir
    } else if has(&[
        "timed out",
        "timeout",
        "dns",
        "connection",
        "network",
        "超时",
        "无法连接",
    ]) {
        DiagnosticCode::Network
    } else if has(&["last_url", "last url"]) {
        DiagnosticCode::BadLastUrl
    } else if has(&["url", "scheme", "网址"]) {
        DiagnosticCode::InvalidUrl
    } else {
        DiagnosticCode::Unknown
    }
}

/// 检测系统是否装了 WebView2 Runtime。
/// 先查注册表（最准，能拿到版本号），查不到再退回去扫安装目录。
pub(crate) fn detect_webview2_runtime() -> WebView2Status {
    #[cfg(target_os = "windows")]
    {
        if let Some((version, source)) = webview2_from_registry() {
            return WebView2Status {
                installed: true,
                version,
                source,
            };
        }
        if let Some((version, source)) = webview2_from_filesystem() {
            return WebView2Status {
                installed: true,
                version,
                source,
            };
        }
    }
    WebView2Status {
        installed: false,
        version: String::new(),
        source: "none".to_string(),
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn webview2_from_registry() -> Option<(String, String)> {
    const HIVES: [(&str, &str); 3] = [
        (
            "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients",
            "registry-hklm",
        ),
        (
            "HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients",
            "registry-hklm",
        ),
        (
            "HKCU\\Software\\Microsoft\\EdgeUpdate\\Clients",
            "registry-hkcu",
        ),
    ];
    for (base, source) in HIVES {
        let key = format!("{base}\\{WEBVIEW2_CLSID}");
        let Ok(output) = hidden_command("reg")
            .args(["query", &key, "/v", "pv"])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(version) = parse_reg_pv(&text) {
            return Some((version, source.to_string()));
        }
    }
    None
}

/// `reg query /v pv` 的输出形如：
/// ```text
///     pv    REG_SZ    131.0.2903.86
/// ```
/// 取第三个字段；再校验一下首位是数字，避免把 REG_SZ 之类读成版本号。
#[cfg(target_os = "windows")]
pub(crate) fn parse_reg_pv(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.trim_start().starts_with("pv"))
        .and_then(|line| line.split_whitespace().nth(2))
        .filter(|value| value.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|value| value.to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn webview2_from_filesystem() -> Option<(String, String)> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(
            PathBuf::from(local)
                .join("Microsoft")
                .join("EdgeWebView")
                .join("Application"),
        );
    }
    roots.push(PathBuf::from(
        r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application",
    ));
    roots.push(PathBuf::from(
        r"C:\Program Files\Microsoft\EdgeWebView\Application",
    ));

    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        // 版本号目录名形如 131.0.2903.86，字典序够用（位数相同时）。
        let mut versions: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().join("msedgewebview2.exe").exists())
            .filter_map(|entry| entry.file_name().to_str().map(|s| s.to_string()))
            .filter(|name| name.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .collect();
        versions.sort();
        if let Some(version) = versions.pop() {
            return Some((version, "filesystem".to_string()));
        }
    }
    None
}

/// 统计目录占用。超过 `max_files` 就停止，把 `truncated` 置为 true ——
/// 用户只需要量级（几十 MB 还是几个 GB），不需要精确到字节。
pub(crate) fn measure_dir(path: &Path, max_files: u64) -> DataDirUsage {
    let mut usage = DataDirUsage {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
        bytes: 0,
        file_count: 0,
        truncated: false,
    };
    if !usage.exists {
        return usage;
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if usage.file_count >= max_files {
                usage.truncated = true;
                return usage;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                usage.bytes += meta.len();
                usage.file_count += 1;
            }
        }
    }
    usage
}

#[tauri::command]
pub(crate) async fn diagnose_profile(
    app: AppHandle,
    id: String,
    error: Option<String>,
) -> Result<ProfileDiagnostic, String> {
    let profile = load_profiles(&app)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;

    let data_dir = profile_data_dir(&app, &profile.id).ok();
    let global_default_url = normalized_global_default_url(&app).to_string();
    let effective_home_url = effective_profile_home_url(&app, &profile).to_string();

    // 遍历数据目录 + 跑 reg query 都会阻塞，丢到阻塞线程池里，别卡住界面。
    let (runtime, data_usage) = tauri::async_runtime::spawn_blocking(move || {
        let usage = match data_dir {
            Some(path) => measure_dir(&path, MAX_DIAG_FILES),
            None => DataDirUsage {
                exists: false,
                path: String::new(),
                bytes: 0,
                file_count: 0,
                truncated: false,
            },
        };
        (detect_webview2_runtime(), usage)
    })
    .await
    .map_err(|e| e.to_string())?;

    let code = classify_error(error.as_deref().unwrap_or_default());
    Ok(ProfileDiagnostic {
        id: profile.id.clone(),
        name: profile.name.clone(),
        url_mode: profile.url_mode.clone(),
        global_default_url,
        effective_home_url,
        last_url: profile.last_url.clone(),
        last_url_valid: normalize_url(&profile.last_url).is_ok(),
        proxy_configured: !profile.proxy.trim().is_empty(),
        proxy_valid: parse_proxy(&profile.proxy).is_ok(),
        code,
        hint: code.hint().to_string(),
        runtime,
        data_dir: data_usage,
    })
}

/// 在文件管理器里打开该账号的数据目录。
/// 路径由 Rust 侧根据账号 id 拼出来，**不接受前端传路径** ——
/// 否则这个命令就退化成"让前端指定任意路径去启动进程"了。
#[tauri::command]
pub(crate) fn open_profile_data_dir(app: AppHandle, id: String) -> Result<(), String> {
    let dir = profile_data_dir(&app, &id)?;
    if !dir.exists() {
        return Err("数据目录还不存在（该账号尚未启动过）".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        hidden_command("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    let _ = dir;
    Ok(())
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
/// 拿出口 IP / 地区 / ISP / 往返耗时，结果写入账号状态并广播
///（直连账号测的就是本机出口）。
#[tauri::command]
pub(crate) async fn test_profile_proxy(
    app: AppHandle,
    id: String,
) -> Result<ProxyTestResult, String> {
    let profile = load_profiles(&app)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;

    let endpoint = profile.proxy.trim().to_string();
    let via_proxy = !endpoint.is_empty();
    let profile_id = profile.id.clone();

    let probe_endpoint = endpoint.clone();
    let probe = tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut cmd = hidden_curl();
        cmd.args(["-s", "--max-time", "12", "-A", "MultiAccountBrowser/0.1"]);
        if !probe_endpoint.is_empty() {
            cmd.args(["-x", &probe_endpoint]);
        }
        cmd.arg("http://ip-api.com/json/?lang=zh-CN");
        let output = match cmd.output() {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("系统 curl 不可用，请确认 Windows 版本（Win10 1803+ 自带）".to_string())
            }
            Err(e) => return Err(format!("启动 curl 失败：{e}")),
        };
        Ok((
            String::from_utf8_lossy(&output.stdout).to_string(),
            started.elapsed().as_millis() as u64,
        ))
    })
    .await
    .map_err(|e| e.to_string())?;

    let result = match probe {
        Ok((stdout, latency_ms)) => {
            let mut built = ProxyTestResult {
                ok: false,
                ip: String::new(),
                region: String::new(),
                isp: String::new(),
                error: String::new(),
                latency_ms,
                via_proxy,
                endpoint,
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
                    built.ok = true;
                    built.ip = get("query");
                    built.region = format!("{} {}", get("country"), get("city"))
                        .trim()
                        .to_string();
                    built.isp = get("isp");
                } else {
                    built.error = get("message");
                }
            }
            if !built.ok && built.error.is_empty() {
                built.error = "检测失败：网络超时或系统 curl 不可用".to_string();
            }
            built
        }
        Err(message) => ProxyTestResult {
            ok: false,
            ip: String::new(),
            region: String::new(),
            isp: String::new(),
            error: message,
            latency_ms: 0,
            via_proxy,
            endpoint,
        },
    };

    if result.ok {
        if let Ok(profiles) = load_profiles(&app) {
            if profiles.iter().any(|p| p.id == profile_id) {
                let mut map: HashMap<String, ProfileStatus> = app
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
                save_profile_status(&app, &profile_id, status);
            }
        }
    }
    Ok(result)
}
