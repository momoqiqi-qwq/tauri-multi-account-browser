//! 下载：去重账本、历史记录、移动/删除/打开。

use chrono::Utc;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;
use url::Url;

use crate::*;

pub(crate) fn load_download_ledger(app: &AppHandle) -> DownloadLedger {
    let Ok(store) = app.store(STORE_FILE) else {
        return DownloadLedger::default();
    };
    let mut ledger: DownloadLedger = store
        .get(DOWNLOAD_LEDGER_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let before_names = ledger.names.len();
    let history: Vec<DownloadEntry> = store
        .get(HISTORY_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    for entry in history.into_iter().filter(|entry| entry.success) {
        let name = entry.file_name.trim().to_ascii_lowercase();
        if !name.is_empty() {
            ledger.names.insert(name);
        }
    }
    if ledger.names.len() != before_names || store.get(DOWNLOAD_LEDGER_KEY).is_none() {
        if let Ok(value) = serde_json::to_value(&ledger) {
            store.set(DOWNLOAD_LEDGER_KEY, value);
            let _ = store.save();
        }
    }
    ledger
}

pub(crate) fn download_seen(app: &AppHandle, url: &Url) -> bool {
    let ledger = load_download_ledger(app);
    ledger.urls.contains(&normalized_download_url(url))
        || inferred_download_name(url)
            .map(|n| ledger.names.contains(&n))
            .unwrap_or(false)
}

pub(crate) fn remember_download(app: &AppHandle, source_url: Option<&str>, file_name: &str) {
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    let mut ledger: DownloadLedger = store
        .get(DOWNLOAD_LEDGER_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let name = file_name.trim().to_ascii_lowercase();
    if !name.is_empty() && name != "未知文件" {
        ledger.names.insert(name);
    }
    if let Some(raw) = source_url {
        if let Ok(url) = Url::parse(raw) {
            ledger.urls.insert(normalized_download_url(&url));
            if let Some(name) = inferred_download_name(&url) {
                ledger.names.insert(name);
            }
        }
    }
    if let Ok(value) = serde_json::to_value(&ledger) {
        store.set(DOWNLOAD_LEDGER_KEY, value);
        let _ = store.save();
    }
}

/// 下载完成：追加历史（新记录在最前）并通过 profile:download 事件广播。
pub(crate) fn record_download(
    app: &AppHandle,
    id: &str,
    path: Option<String>,
    source_url: Option<String>,
    success: bool,
) {
    let file_name = path
        .as_ref()
        .and_then(|p| {
            Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "未知文件".to_string());
    let size = path
        .as_ref()
        .and_then(|p| fs::metadata(p).ok().map(|m| m.len()))
        .unwrap_or(0);
    let profile_name = load_profiles(app)
        .ok()
        .and_then(|profiles| profiles.iter().find(|p| p.id == id).map(|p| p.name.clone()))
        .unwrap_or_default();
    if success {
        remember_download(app, source_url.as_deref(), &file_name);
    }
    let entry = DownloadEntry {
        id: id.to_string(),
        profile_name,
        file_name,
        path: path.unwrap_or_default(),
        size,
        finished_at: Utc::now().to_rfc3339(),
        success,
    };
    if let Ok(store) = app.store(STORE_FILE) {
        let mut history: Vec<DownloadEntry> = store
            .get(HISTORY_KEY)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        history.insert(0, entry.clone());
        let history_limit = load_app_settings(app)
            .download_history_limit
            .clamp(50, 2000);
        history.truncate(history_limit);
        if let Ok(value) = serde_json::to_value(&history) {
            store.set(HISTORY_KEY, value);
            let _ = store.save();
        }
    }
    let _ = app.emit("profile:download", entry);
    if success {
        // 新完成的文件立即加入所有页面的“已下载”集合，避免同一链接再次被自动点击。
        push_download_cfg(app);
    }
}

/// 下载落盘路径：统一重定向到设置的下载目录，重名自动追加 (1)、(2)…
pub(crate) fn download_destination(app: &AppHandle, url: &Url, suggested: &mut PathBuf) {
    let settings = load_app_settings(app);
    let requested_dir = resolved_download_dir(app, &settings);
    // 用户配置的目录不可创建/不可用时，不要让整个下载静默失败；退回应用目录，
    // 再不行则退回系统临时目录。on_download 要求 destination 必须是绝对路径。
    let dir = if fs::create_dir_all(&requested_dir).is_ok() {
        requested_dir
    } else {
        let fallback = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("downloads");
        if fs::create_dir_all(&fallback).is_ok() {
            fallback
        } else {
            let temp = std::env::temp_dir().join("multi-account-browser-downloads");
            let _ = fs::create_dir_all(&temp);
            temp
        }
    };

    let raw_name = suggested
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty() && n != ".")
        .or_else(|| {
            url.path_segments()
                .and_then(|mut segs| segs.next_back())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "download".to_string());
    // URL 里可能带 query 或编码字符，清理成合法文件名。
    let safe_name: String = raw_name
        .split(['?', '#'])
        .next()
        .unwrap_or("download")
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Windows 不允许文件名以空格/点结尾，也不允许 CON/PRN/AUX/NUL/COM1 等设备名。
    let mut safe_name = safe_name.trim().trim_end_matches([' ', '.']).to_string();
    if safe_name.is_empty() {
        safe_name = "download".to_string();
    }
    let stem_upper = Path::new(&safe_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_uppercase())
        .unwrap_or_default();
    let reserved = matches!(stem_upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem_upper.len() == 4
            && (stem_upper.starts_with("COM") || stem_upper.starts_with("LPT"))
            && stem_upper.as_bytes()[3].is_ascii_digit()
            && stem_upper.as_bytes()[3] != b'0');
    if reserved {
        safe_name.insert(0, '_');
    }

    let mut candidate = dir.join(&safe_name);
    let stem = PathBuf::from(&safe_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| safe_name.clone());
    let ext = PathBuf::from(&safe_name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    let mut index = 1;
    while candidate.exists() {
        let name = match &ext {
            Some(e) => format!("{stem} ({index}){e}"),
            None => format!("{stem} ({index})"),
        };
        candidate = dir.join(name);
        index += 1;
    }
    *suggested = candidate;
}

/// 派生 curl 时必须带 CREATE_NO_WINDOW：GUI 程序在 Windows 上启动
/// 控制台子进程会各自弹出终端窗口（图标拉取、代理检测都是每次账号一次）。
/// 构造一个"不弹控制台窗口"的子进程（Windows 下加 CREATE_NO_WINDOW）。
/// **任何调用系统命令的地方都应该走这里**，否则启动时会闪一个黑框。
pub(crate) fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub(crate) fn hidden_curl() -> Command {
    hidden_command("curl")
}

pub(crate) fn icon_failed_memo() -> &'static Mutex<HashSet<String>> {
    ICON_FAILED.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(crate) fn icon_data_url(bytes: Vec<u8>) -> String {
    format!(
        "data:image/x-icon;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// 账号站点图标：从该账号自己的默认站点 /favicon.ico 拉取（走该账号配置的
/// 代理），本地缓存后以 data URL 返回。绝不请求第三方 favicon 服务
///（之前用 google.com/s2/favicons 会把全部账号域名以直连 IP 告知谷歌）。
#[tauri::command]
pub(crate) async fn get_profile_icon(app: AppHandle, id: String) -> Result<Option<String>, String> {
    let profile = load_profiles(&app)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let host = Url::parse(&profile.default_url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()));
    let Some(host) = host else {
        return Ok(None);
    };

    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("icons");
    let safe_host: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cache_path = cache_dir.join(format!("{safe_host}.icon"));
    if cache_path.exists() {
        let bytes = fs::read(&cache_path).map_err(|e| e.to_string())?;
        return Ok(Some(icon_data_url(bytes)));
    }
    if icon_failed_memo().lock().unwrap().contains(&host) {
        return Ok(None);
    }

    let proxy = profile.proxy.trim().to_string();
    let url = format!("https://{host}/favicon.ico");
    let tmp_path = cache_path.with_extension("tmp");
    let fetch_path = tmp_path.clone();
    let fetch = tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = hidden_curl();
        cmd.args([
            "-sL",
            "--max-time",
            "8",
            "-A",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
        ]);
        if !proxy.is_empty() {
            cmd.args(["-x", &proxy]);
        }
        cmd.arg("-o").arg(&fetch_path).arg(&url);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?;

    let fetched_ok = fetch.map(|o| o.status.success()).unwrap_or(false);
    let bytes = fs::read(&tmp_path).unwrap_or_default();
    let _ = fs::remove_file(&tmp_path);
    // 起码 100 字节、不超过 500KB、开头不是 "<"（HTML 页面说明 404 落地页）。
    let valid =
        fetched_ok && bytes.len() >= 100 && bytes.len() <= 500_000 && !bytes.starts_with(b"<");
    if !valid {
        icon_failed_memo().lock().unwrap().insert(host);
        return Ok(None);
    }
    fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    fs::write(&cache_path, &bytes).map_err(|e| e.to_string())?;
    Ok(Some(icon_data_url(bytes)))
}

#[tauri::command]
pub(crate) fn get_download_history(app: AppHandle) -> Result<Vec<DownloadEntry>, String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    Ok(match store.get(HISTORY_KEY) {
        Some(value) => serde_json::from_value(value).map_err(|e| e.to_string())?,
        None => vec![],
    })
}

#[tauri::command]
pub(crate) fn clear_download_history(app: AppHandle) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(HISTORY_KEY, serde_json::json!([]));
    store.save().map_err(|e| e.to_string())?;
    push_download_cfg(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn remove_download_history_entries(
    app: AppHandle,
    entries: Vec<DownloadIdentity>,
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let keys: HashSet<(String, String, String)> = entries
        .into_iter()
        .map(|entry| (entry.id, entry.path, entry.finished_at))
        .collect();
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    let mut history: Vec<DownloadEntry> = store
        .get(HISTORY_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    history.retain(|entry| {
        !keys.contains(&(
            entry.id.clone(),
            entry.path.clone(),
            entry.finished_at.clone(),
        ))
    });
    store.set(
        HISTORY_KEY,
        serde_json::to_value(&history).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    push_download_cfg(&app);
    Ok(())
}

pub(crate) fn update_download_history_path(
    app: &AppHandle,
    old_path: &str,
    new_path: Option<&str>,
) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    let mut history: Vec<DownloadEntry> = store
        .get(HISTORY_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    if let Some(new_path) = new_path {
        for entry in &mut history {
            if entry.path == old_path {
                entry.path = new_path.to_string();
                entry.file_name = Path::new(new_path)
                    .file_name()
                    .map(|v| v.to_string_lossy().to_string())
                    .unwrap_or_else(|| entry.file_name.clone());
            }
        }
    } else {
        history.retain(|entry| entry.path != old_path);
    }
    store.set(
        HISTORY_KEY,
        serde_json::to_value(&history).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    push_download_cfg(app);
    Ok(())
}

#[tauri::command]
pub(crate) fn delete_download(app: AppHandle, path: String) -> Result<(), String> {
    let file = PathBuf::from(&path);
    if file.exists() {
        if !file.is_file() {
            return Err("目标不是普通文件".to_string());
        }
        fs::remove_file(&file).map_err(|e| e.to_string())?;
    }
    update_download_history_path(&app, &path, None)
}

/// 打开系统原生文件夹选择器，供“移动下载文件”直接选择目标目录。
#[tauri::command]
pub(crate) fn pick_move_directory(initial_dir: String) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '选择移动目标文件夹'
$dialog.ShowNewFolderButton = $true
if ($args.Count -gt 0 -and (Test-Path -LiteralPath $args[0] -PathType Container)) {
  $dialog.SelectedPath = $args[0]
}
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::Write($dialog.SelectedPath)
}
"#;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", script])
            .arg(initial_dir.trim())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("无法打开文件夹选择器：{e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let selected = String::from_utf8_lossy(&output.stdout)
            .trim_start_matches('\u{feff}')
            .trim()
            .to_string();
        return Ok((!selected.is_empty()).then_some(selected));
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .args([
                "-e",
                "set chosenFolder to choose folder with prompt \"选择移动目标文件夹\"",
                "-e",
                "POSIX path of chosenFolder",
            ])
            .output()
            .map_err(|e| format!("无法打开文件夹选择器：{e}"))?;
        if !output.status.success() {
            return Ok(None);
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!selected.is_empty()).then_some(selected));
    }

    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("zenity");
        command.args([
            "--file-selection",
            "--directory",
            "--title=选择移动目标文件夹",
        ]);
        let initial = initial_dir.trim();
        if !initial.is_empty() {
            command.arg(format!("--filename={}/", initial.trim_end_matches('/')));
        }
        let output = command
            .output()
            .map_err(|e| format!("无法打开文件夹选择器：{e}"))?;
        if !output.status.success() {
            return Ok(None);
        }
        let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!selected.is_empty()).then_some(selected));
    }
}

#[tauri::command]
pub(crate) fn move_download(
    app: AppHandle,
    path: String,
    destination_dir: String,
) -> Result<String, String> {
    let source = PathBuf::from(&path);
    if !source.exists() || !source.is_file() {
        return Err("文件已被移动或删除".to_string());
    }
    let dir = PathBuf::from(destination_dir.trim());
    if destination_dir.trim().is_empty() {
        return Err("目标文件夹不能为空".to_string());
    }
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目标文件夹：{e}"))?;
    let file_name = source
        .file_name()
        .ok_or_else(|| "无法识别文件名".to_string())?
        .to_string_lossy()
        .to_string();
    let stem = Path::new(&file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.clone());
    let ext = Path::new(&file_name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    let mut target = dir.join(&file_name);
    let mut index = 1;
    while target.exists() {
        let candidate = match &ext {
            Some(ext) => format!("{stem} ({index}){ext}"),
            None => format!("{stem} ({index})"),
        };
        target = dir.join(candidate);
        index += 1;
    }
    if let Err(rename_err) = fs::rename(&source, &target) {
        fs::copy(&source, &target)
            .map_err(|copy_err| format!("移动失败：{rename_err}；复制也失败：{copy_err}"))?;
        fs::remove_file(&source).map_err(|e| format!("复制成功但删除原文件失败：{e}"))?;
    }
    let new_path = target.to_string_lossy().to_string();
    update_download_history_path(&app, &path, Some(&new_path))?;
    Ok(new_path)
}

/// 在资源管理器里定位已下载的文件（Windows：explorer /select）。
#[tauri::command]
pub(crate) fn reveal_download(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if !Path::new(&path).exists() {
            return Err("文件已被移动或删除".to_string());
        }
        Command::new("explorer")
            .arg(format!("/select,{path}"))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    let _ = path;
    Ok(())
}
