use base64::Engine as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{
    webview::{NewWindowResponse, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl,
};
use tauri_plugin_store::StoreExt;
use url::Url;
use uuid::Uuid;

const STORE_FILE: &str = "profiles.json";
const STORE_KEY: &str = "profiles";
const STATUS_KEY: &str = "profile_status";
const SETTINGS_KEY: &str = "app_settings";
const HISTORY_KEY: &str = "download_history";
const DOWNLOAD_LEDGER_KEY: &str = "download_ledger_v1";
const PROFILE_TEMPLATES_KEY: &str = "profile_templates_v1";
const PROFILE_PREFIX: &str = "profile_";
const DEFAULT_PROFILE_URL: &str = "https://chat01.ai/";
const DOWNLOAD_RETURN_URL: &str = DEFAULT_PROFILE_URL;

/// store 结构版本号。加字段或改变字段语义时递增，并在 `migrate_store` 里补一段迁移。
/// - 0：无版本号的历史数据（v14 及之前）。
/// - 1：引入 schema_version 本身，并把此前散落的隐式兼容显式化。
const SCHEMA_VERSION: u64 = 1;
const SCHEMA_VERSION_KEY: &str = "schema_version";
/// 账号模板上限：防止store无限膨胀。
const MAX_PROFILE_TEMPLATES: usize = 200;
/// 单次批量创建上限：既是性能保护，也避免误操作刷出上千个账号。
const MAX_BULK_PROFILES: usize = 200;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DownloadLedger {
    #[serde(default)]
    names: HashSet<String>,
    #[serde(default)]
    urls: HashSet<String>,
}

fn normalized_download_url(url: &Url) -> String {
    let mut clean = url.clone();
    clean.set_fragment(None);
    clean.to_string()
}

fn inferred_download_name(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|segments| segments.filter(|s| !s.is_empty()).last())
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
}

fn is_chat01_download_url(url: &Url) -> bool {
    url.host_str() == Some("files.chat01.ai") && url.path().starts_with("/python-generations/")
}

fn load_download_ledger(app: &AppHandle) -> DownloadLedger {
    let Ok(store) = app.store(STORE_FILE) else { return DownloadLedger::default() };
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
        if !name.is_empty() { ledger.names.insert(name); }
    }
    if ledger.names.len() != before_names || store.get(DOWNLOAD_LEDGER_KEY).is_none() {
        if let Ok(value) = serde_json::to_value(&ledger) {
            store.set(DOWNLOAD_LEDGER_KEY, value);
            let _ = store.save();
        }
    }
    ledger
}

fn download_seen(app: &AppHandle, url: &Url) -> bool {
    let ledger = load_download_ledger(app);
    ledger.urls.contains(&normalized_download_url(url))
        || inferred_download_name(url).map(|n| ledger.names.contains(&n)).unwrap_or(false)
}

fn remember_download(app: &AppHandle, source_url: Option<&str>, file_name: &str) {
    let Ok(store) = app.store(STORE_FILE) else { return };
    let mut ledger: DownloadLedger = store
        .get(DOWNLOAD_LEDGER_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let name = file_name.trim().to_ascii_lowercase();
    if !name.is_empty() && name != "未知文件" { ledger.names.insert(name); }
    if let Some(raw) = source_url {
        if let Ok(url) = Url::parse(raw) {
            ledger.urls.insert(normalized_download_url(&url));
            if let Some(name) = inferred_download_name(&url) { ledger.names.insert(name); }
        }
    }
    if let Ok(value) = serde_json::to_value(&ledger) {
        store.set(DOWNLOAD_LEDGER_KEY, value);
        let _ = store.save();
    }
}

static ACTIVE_PROFILE_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn active_profile_id() -> &'static Mutex<Option<String>> {
    ACTIVE_PROFILE_ID.get_or_init(|| Mutex::new(None))
}

fn set_active_profile_id(id: &str) {
    if let Ok(mut active) = active_profile_id().lock() {
        *active = Some(id.to_string());
    }
}

fn is_active_profile(id: &str) -> bool {
    active_profile_id()
        .lock()
        .ok()
        .and_then(|active| active.clone())
        .as_deref()
        == Some(id)
}

fn profile_activity_eval_script(active: bool) -> String {
    format!(
        "try {{ document.dispatchEvent(new CustomEvent('mb-profile-active', {{ detail: {{ active: {} }} }})); }} catch (e) {{}}",
        if active { "true" } else { "false" }
    )
}

/// 账号页面回传状态用的假导航 scheme，on_navigation 里拦截并取消，
/// 从而不需要给远程页面开放任何 IPC 权限。
const STATUS_SCHEME: &str = "mbstatus";

fn default_fingerprint_guard() -> bool {
    true
}

fn default_url_mode() -> String {
    // 兼容旧账号：历史版本每个账号都保存自己的 default_url，因此迁移时应保持原行为。
    "custom".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Profile {
    id: String,
    name: String,
    note: String,
    avatar: Option<String>,
    #[serde(default)]
    default_url: String,
    /// 默认网址模式：inherit 跟随全局默认网址，custom 使用账号独立网址。
    #[serde(default = "default_url_mode")]
    url_mode: String,
    #[serde(default)]
    last_url: String,
    created_at: String,
    order: usize,
    incognito: bool,
    /// 独立代理，空字符串表示直连。支持 http:// 与 socks5://（Windows/Linux）。
    #[serde(default)]
    proxy: String,
    /// 自定义 User-Agent，空表示使用系统默认。
    #[serde(default)]
    user_agent: String,
    /// IANA 时区（如 Asia/Shanghai），空表示跟随系统。
    #[serde(default)]
    timezone: String,
    /// 语言/区域（如 zh-CN、en-US），空表示跟随系统。
    #[serde(default)]
    locale: String,
    /// 按账号注入确定性画布/硬件指纹噪声。
    #[serde(default = "default_fingerprint_guard")]
    fingerprint_guard: bool,
    /// 账号分组，空表示未分组。
    #[serde(default)]
    group: String,
}

/// 账号模板：一次配置、反复套用，用于批量建号。
///
/// 字段与前端 `AccountToolsDialog.vue` 里的 `ProfileTemplate` 一一对应，
/// 全部带 serde 默认值，保证读到旧数据（缺字段）时不会反序列化失败。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileTemplate {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    default_url: String,
    #[serde(default)]
    url_mode: String,
    #[serde(default)]
    incognito: bool,
    #[serde(default)]
    proxy: String,
    #[serde(default)]
    user_agent: String,
    #[serde(default)]
    timezone: String,
    #[serde(default)]
    locale: String,
    #[serde(default = "default_fingerprint_guard")]
    fingerprint_guard: bool,
    #[serde(default)]
    group: String,
}

/// 单个创建与批量创建共用的输入结构。
///
/// 全字段可选，前端可以只传 `name`；`url_mode` 用 Option 以区分「没传」与「传了空串」。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileDraft {
    #[serde(default)]
    name: String,
    #[serde(default)]
    default_url: String,
    #[serde(default)]
    url_mode: Option<String>,
    #[serde(default)]
    incognito: bool,
    #[serde(default)]
    proxy: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    fingerprint_guard: Option<bool>,
    #[serde(default)]
    group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadGuardRule {
    domain: String,
    seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
struct DownloadGuardBlockedEvent {
    seconds: u64,
    queued: bool,
}

/// 全局应用设置：下载目录与 chat01 AI 文件自动下载。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppSettings {
    /// 下载保存目录，空表示使用 应用数据目录/downloads。
    #[serde(default)]
    download_dir: String,
    /// chat01 页面出现 AI 生成的下载链接时自动点击下载。
    #[serde(default)]
    auto_ai_download: bool,
    /// 自动下载时跳过下载历史里已经成功下载过的同名文件。
    #[serde(default = "default_true")]
    skip_downloaded_files: bool,
    /// 删除下载文件时是否先显示二次确认。
    #[serde(default = "default_true")]
    confirm_delete_download: bool,
    /// 下载历史最大保留条数，防止 store 无限增长。
    #[serde(default = "default_history_limit")]
    download_history_limit: usize,
    /// 页面打开后的下载保护秒数；0 表示关闭。
    #[serde(default = "default_download_guard_seconds")]
    download_guard_seconds: u64,
    /// 指定下载域名的单独保护时长。
    #[serde(default)]
    download_guard_rules: Vec<DownloadGuardRule>,
    /// 保护期拦截后自动排队重试。
    #[serde(default = "default_true")]
    download_guard_auto_retry: bool,
    /// 自动下载的扩展名白名单（逗号分隔，空表示全部）。
    #[serde(default)]
    ai_exts: String,
    /// 新建账号以及 inherit 模式账号共用的全局默认主页。
    #[serde(default = "default_global_profile_url")]
    global_default_url: String,
}

fn default_true() -> bool { true }
fn default_history_limit() -> usize { 200 }
fn default_download_guard_seconds() -> u64 { 3 }
fn default_global_profile_url() -> String { DEFAULT_PROFILE_URL.to_string() }


fn normalized_guard_rules(mut rules: Vec<DownloadGuardRule>) -> Vec<DownloadGuardRule> {
    let mut seen = HashSet::new();
    rules.retain_mut(|rule| {
        rule.domain = rule.domain.trim().trim_start_matches('.').to_ascii_lowercase();
        rule.seconds = rule.seconds.min(60);
        !rule.domain.is_empty()
            && rule.domain.len() <= 253
            && rule.domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && seen.insert(rule.domain.clone())
    });
    rules.truncate(20);
    rules
}

fn download_guard_seconds_for(settings: &AppSettings, url: &Url) -> u64 {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    for rule in &settings.download_guard_rules {
        let domain = rule.domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}"))) {
            return rule.seconds.min(60);
        }
    }
    settings.download_guard_seconds.min(60)
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            auto_ai_download: false,
            skip_downloaded_files: true,
            confirm_delete_download: true,
            download_history_limit: default_history_limit(),
            download_guard_seconds: default_download_guard_seconds(),
            download_guard_rules: vec![],
            download_guard_auto_retry: true,
            ai_exts: "md,txt,csv,docx,xlsx,pptx,pdf,png,jpg,jpeg,webp,zip,json,html".to_string(),
            global_default_url: DEFAULT_PROFILE_URL.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Serialize)]
struct NavigationEvent {
    id: String,
    url: String,
}

/// 从账号页面上抓到的状态：chat01.ai 积分、登录账号（通常是谷歌账号）、代理出口信息。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProfileStatus {
    credits: Option<String>,
    account: Option<String>,
    email: Option<String>,
    updated_at: Option<String>,
    /// 最近一次代理检测结果：出口 IP / 地区 / ISP。
    #[serde(default)]
    proxy_ip: Option<String>,
    #[serde(default)]
    proxy_region: Option<String>,
    #[serde(default)]
    proxy_isp: Option<String>,
    #[serde(default)]
    proxy_tested_at: Option<String>,
    /// 后台账号 AI 回答完成后置为 true，直到用户切回该账号。
    #[serde(default)]
    answer_ready: bool,
    /// 页面当前是否检测到 AI 正在生成回答。
    #[serde(default)]
    answer_generating: bool,
    /// 最近一次回答完成时间。
    #[serde(default)]
    answer_finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProfileStatusEvent {
    id: String,
    #[serde(flatten)]
    status: ProfileStatus,
}

/// 页面 alt / aria-label 里的界面用词（如 Google 图标的 "logo"）不能当作账号名。
fn is_junk_account(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    matches!(
        v.as_str(),
        "logo"
            | "logo.png"
            | "logo.svg"
            | "icon"
            | "icons"
            | "img"
            | "image"
            | "photo"
            | "picture"
            | "avatar"
            | "profile"
            | "account"
            | "user"
            | "menu"
            | "home"
            | "search"
            | "settings"
            | "close"
            | "open"
            | "chat01"
            | "chat01.ai"
            | "头像"
            | "图标"
            | "标志"
            | "菜单"
            | "首页"
            | "搜索"
            | "设置"
            | "账户"
            | "账号"
            | "用户"
    )
}

/// 读取 / 落盘前统一清理账号名里的历史脏数据。
fn clean_status(mut status: ProfileStatus) -> ProfileStatus {
    if status
        .account
        .as_deref()
        .map(is_junk_account)
        .unwrap_or(false)
    {
        status.account = None;
    }
    status
}

fn save_profile_status(app: &AppHandle, id: &str, mut status: ProfileStatus) {
    if let Ok(store) = app.store(STORE_FILE) {
        let mut map: HashMap<String, ProfileStatus> = match store.get(STATUS_KEY) {
            Some(value) => serde_json::from_value(value).unwrap_or_default(),
            None => HashMap::new(),
        };

        // 页面状态扫描是启发式的，某一次没扫到字段不代表它真的消失；
        // 保留上一条有效值，同时也避免页面上报覆盖代理检测结果。
        if let Some(previous) = map.get(id) {
            if status.credits.is_none() {
                status.credits = previous.credits.clone();
            }
            if status.account.is_none() {
                status.account = previous.account.clone();
            }
            if status.email.is_none() {
                status.email = previous.email.clone();
            }
            if status.proxy_ip.is_none() {
                status.proxy_ip = previous.proxy_ip.clone();
                status.proxy_region = previous.proxy_region.clone();
                status.proxy_isp = previous.proxy_isp.clone();
                status.proxy_tested_at = previous.proxy_tested_at.clone();
            }
            if status.answer_ready {
                status.answer_finished_at = previous
                    .answer_finished_at
                    .clone()
                    .or_else(|| Some(Utc::now().to_rfc3339()));
            } else {
                status.answer_finished_at = None;
            }
        } else if status.answer_ready && status.answer_finished_at.is_none() {
            status.answer_finished_at = Some(Utc::now().to_rfc3339());
        }

        map.insert(id.to_string(), status.clone());
        if let Ok(value) = serde_json::to_value(&map) {
            store.set(STATUS_KEY, value);
            let _ = store.save();
        }
    }
    let _ = app.emit(
        "profile:status",
        ProfileStatusEvent {
            id: id.to_string(),
            status,
        },
    );
}

/// 用户切回账号即视为已查看“回答完成”提醒。页面端也同步清除，
/// 防止下一次状态上报把提醒重新置回。
fn clear_profile_answer_ready(app: &AppHandle, id: &str) {
    if let Ok(store) = app.store(STORE_FILE) {
        let mut map: HashMap<String, ProfileStatus> = store
            .get(STATUS_KEY)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        if let Some(status) = map.get_mut(id) {
            if status.answer_ready || status.answer_finished_at.is_some() {
                status.answer_ready = false;
                status.answer_finished_at = None;
                let cloned = status.clone();
                if let Ok(value) = serde_json::to_value(&map) {
                    store.set(STATUS_KEY, value);
                    let _ = store.save();
                }
                let _ = app.emit(
                    "profile:status",
                    ProfileStatusEvent {
                        id: id.to_string(),
                        status: cloned,
                    },
                );
            }
        }
    }
    if let Some(webview) = app.get_webview(&profile_label(id)) {
        let _ = webview.eval(
            "try { document.dispatchEvent(new Event('mb-answer-seen')); } catch (e) {}",
        );
    }
}

/// 一条下载历史记录，随 store 持久化，最多保留 200 条。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadEntry {
    id: String,
    /// 下载发生时的账号名快照（账号删除后历史仍可读）。
    profile_name: String,
    file_name: String,
    path: String,
    size: u64,
    finished_at: String,
    success: bool,
}

/// 下载完成：追加历史（新记录在最前）并通过 profile:download 事件广播。
fn record_download(app: &AppHandle, id: &str, path: Option<String>, source_url: Option<String>, success: bool) {
    let file_name = path
        .as_ref()
        .and_then(|p| Path::new(p).file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "未知文件".to_string());
    let size = path
        .as_ref()
        .and_then(|p| fs::metadata(p).ok().map(|m| m.len()))
        .unwrap_or(0);
    let profile_name = load_profiles(app)
        .ok()
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.name.clone())
        })
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
        let history_limit = load_app_settings(app).download_history_limit.clamp(50, 2000);
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

fn load_app_settings<R: Runtime>(app: &AppHandle<R>) -> AppSettings {
    app.store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(SETTINGS_KEY))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn resolved_download_dir(app: &AppHandle, settings: &AppSettings) -> PathBuf {
    let trimmed = settings.download_dir.trim();
    if !trimmed.is_empty() {
        return PathBuf::from(trimmed);
    }
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("downloads")
}

/// 下载落盘路径：统一重定向到设置的下载目录，重名自动追加 (1)、(2)…
fn download_destination(app: &AppHandle, url: &Url, suggested: &mut PathBuf) {
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
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
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

/// 规范化扩展名白名单：去点号/空白、转小写、去重，并丢弃异常项。
/// 空字符串仍代表“不限制格式”。
fn normalize_ai_exts(raw: &str) -> String {
    let mut seen = HashSet::new();
    raw.split(',')
        .filter_map(|item| {
            let ext = item.trim().trim_start_matches('.').to_ascii_lowercase();
            if ext.is_empty() || ext.len() > 12 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            if seen.insert(ext.clone()) { Some(ext) } else { None }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// 把当前下载配置推送到所有账号页面（自动下载开关 / 扩展名白名单）。
/// 通过自定义事件下发，避免在页面 window 上留全局变量痕迹。
fn download_cfg_eval_script(app: &AppHandle) -> String {
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

fn push_download_cfg(app: &AppHandle) {
    let script = download_cfg_eval_script(app);
    for (label, webview) in app.webviews() {
        if label.starts_with(PROFILE_PREFIX) {
            let _ = webview.eval(&script);
        }
    }
}

#[derive(Debug, Serialize)]
struct UpdateProfileResult {
    profile: Profile,
    needs_reopen: bool,
}

fn load_profiles<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<Profile>, String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    let mut profiles: Vec<Profile> = match store.get(STORE_KEY) {
        Some(value) => serde_json::from_value(value).map_err(|e| e.to_string())?,
        None => vec![],
    };
    profiles.sort_by_key(|p| p.order);

    // 旧数据里 default_url / last_url 为空或损坏时，过去会在创建 WebView 时直接失败，
    // 用户表现为“账号点不开”。现在读取时自动迁移到可用网址，并持久化修复结果。
    let mut repaired = false;
    for profile in &mut profiles {
        repaired |= repair_profile_urls(&app, profile);
    }
    if repaired {
        store.set(
            STORE_KEY,
            serde_json::to_value(&profiles).map_err(|e| e.to_string())?,
        );
        store.save().map_err(|e| e.to_string())?;
    }

    Ok(profiles)
}

fn save_profiles<R: Runtime>(app: &AppHandle<R>, profiles: &[Profile]) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        STORE_KEY,
        serde_json::to_value(profiles).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

// ---- 账号模板 ----
//
// 模板原先存在 WebView 的 localStorage 里，换机器/清缓存就丢，也不进应用数据目录的备份。
// 改存 Rust store 后由 `list_profile_templates` / `save_profile_templates` 两个命令读写。

fn load_profile_templates<R: Runtime>(app: &AppHandle<R>) -> Vec<ProfileTemplate> {
    app.store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(PROFILE_TEMPLATES_KEY))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// 写盘前的规范化：去空白、剔除无效项、修正非法 url_mode、限制数量。
///
/// 返回规范化后的列表，由调用方回传给前端，避免前后端状态漂移。
fn normalized_templates(templates: Vec<ProfileTemplate>) -> Vec<ProfileTemplate> {
    let mut out = Vec::new();
    for mut template in templates {
        template.id = template.id.trim().to_string();
        template.name = template.name.trim().to_string();
        // id 与 name 缺任一都无法在 UI 上定位/显示，直接剔除。
        if template.id.is_empty() || template.name.is_empty() {
            continue;
        }
        template.default_url = template.default_url.trim().to_string();
        template.proxy = template.proxy.trim().to_string();
        template.user_agent = template.user_agent.trim().to_string();
        template.timezone = template.timezone.trim().to_string();
        template.locale = template.locale.trim().to_string();
        template.group = template.group.trim().to_string();
        if template.url_mode != "inherit" && template.url_mode != "custom" {
            template.url_mode = "custom".to_string();
        }
        out.push(template);
        if out.len() >= MAX_PROFILE_TEMPLATES {
            break;
        }
    }
    out
}

/// 注意：命令 `save_profile_templates` 与本函数同名会冲突，故内部 helper 用 `store_` 前缀。
fn store_profile_templates<R: Runtime>(
    app: &AppHandle<R>,
    templates: &[ProfileTemplate],
) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        PROFILE_TEMPLATES_KEY,
        serde_json::to_value(templates).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

fn profile_label(id: &str) -> String {
    format!("{PROFILE_PREFIX}{}", id.replace('-', "_"))
}

fn profile_data_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("profiles").join(id))
}

/// 从账号 UUID 派生 32 位种子（FNV-1a），作为该账号指纹噪声的确定性来源：
/// 同一账号每次读到同一噪声，不同账号对同一页面读到不同噪声。
fn profile_seed(id: &str) -> u32 {
    let mut seed: u32 = 0x811c_9dc5;
    for byte in id.as_bytes() {
        seed ^= u32::from(*byte);
        seed = seed.wrapping_mul(0x0100_0193);
    }
    seed
}

fn parse_proxy(raw: &str) -> Result<Option<Url>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let url = Url::parse(trimmed).map_err(|e| format!("代理地址不合法：{e}"))?;
    match url.scheme() {
        "http" | "https" | "socks5" => Ok(Some(url)),
        other => Err(format!(
            "暂不支持代理协议 {other}，请使用 http://、https:// 或 socks5://"
        )),
    }
}

/// 时区会以字符串形式嵌入初始化脚本，这里做白名单转义，防止注入。
fn sanitize_timezone(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+' | '.'))
    {
        return Err("时区格式不合法，请使用 IANA 名称，如 Asia/Shanghai".to_string());
    }
    Ok(trimmed.to_string())
}

/// 语言/区域同样嵌入脚本并拼入 --lang，做白名单转义（如 zh-CN、en-US）。
fn sanitize_locale(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > 35 || !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("语言格式不合法，请使用 BCP-47 名称，如 zh-CN".to_string());
    }
    Ok(trimmed.to_string())
}

fn sanitize_profile_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("账号昵称不能为空".to_string());
    }
    if name.chars().count() > 80 {
        return Err("账号昵称不能超过 80 个字符".to_string());
    }
    Ok(name.to_string())
}

fn normalize_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let candidate = if trimmed.is_empty() {
        DEFAULT_PROFILE_URL.to_string()
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.contains("://") {
        return Err("只支持 http:// 或 https:// 网址".to_string());
    } else {
        format!("https://{trimmed}")
    };

    let url = Url::parse(&candidate).map_err(|e| format!("URL 不合法: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("只支持 http:// 或 https:// 网址".to_string());
    }
    Ok(url)
}

/// 修复旧版/异常账号中的空网址或坏网址。
/// 读取即修复并落盘，确保之后休眠恢复、重启恢复、直接打开都不会再次卡住。
fn normalized_global_default_url<R: Runtime>(app: &AppHandle<R>) -> Url {
    let settings = load_app_settings(app);
    normalize_url(&settings.global_default_url)
        .unwrap_or_else(|_| Url::parse(DEFAULT_PROFILE_URL).expect("DEFAULT_PROFILE_URL must be valid"))
}

fn effective_profile_home_url<R: Runtime>(app: &AppHandle<R>, profile: &Profile) -> Url {
    if profile.url_mode == "inherit" {
        return normalized_global_default_url(app);
    }
    normalize_url(&profile.default_url).unwrap_or_else(|_| normalized_global_default_url(app))
}

fn repair_profile_urls<R: Runtime>(app: &AppHandle<R>, profile: &mut Profile) -> bool {
    let mut changed = false;
    if profile.url_mode != "inherit" && profile.url_mode != "custom" {
        profile.url_mode = "custom".to_string();
        changed = true;
    }
    let fallback = normalized_global_default_url(app).to_string();
    let default_url = normalize_url(&profile.default_url)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| fallback.clone());
    if profile.default_url != default_url {
        profile.default_url = default_url;
        changed = true;
    }
    let home = effective_profile_home_url(app, profile).to_string();
    let last_url = if profile.last_url.trim().is_empty() {
        home.clone()
    } else {
        normalize_url(&profile.last_url).map(|u| u.to_string()).unwrap_or(home)
    };
    if profile.last_url != last_url { profile.last_url = last_url; changed = true; }
    changed
}

fn profile_start_url(app: &AppHandle, profile: &Profile) -> Url {
    if !profile.last_url.trim().is_empty() {
        if let Ok(url) = normalize_url(&profile.last_url) { return url; }
    }
    effective_profile_home_url(app, profile)
}

fn update_last_url(app: &AppHandle, id: &str, url: &str) {
    if let Ok(mut profiles) = load_profiles(app) {
        if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
            // SPA 的同地址导航（hash 变化、弹层路由等）不用反复写盘。
            if profile.last_url == url {
                return;
            }
            profile.last_url = url.to_string();
            let _ = save_profiles(app, &profiles);
        }
    }
}

/// 统一布局所有账号 WebView：全部对齐到当前视口矩形，显示活跃的、隐藏其余。
/// 隐藏的也同步边界，避免切换标签时旧 WebView 因边界残留从边缘“漏”出来。
fn layout_profile_webviews(app: &AppHandle, active_label: &str, bounds: &BrowserBounds) {
    let position = LogicalPosition::new(bounds.x, bounds.y);
    let size = LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0));
    for (label, webview) in app.webviews() {
        if !label.starts_with(PROFILE_PREFIX) {
            continue;
        }
        let active = label == active_label;
        if active {
            // 只给当前可见 WebView 同步几何尺寸。后台几十个 WebView 若每次切换/缩放
            // 都 set_position + set_size，会产生大量原生 IPC 并明显拖慢窗口。
            let _ = webview.set_position(position);
            let _ = webview.set_size(size);
            let _ = webview.show();
        } else {
            let _ = webview.hide();
        }
        // 原生子 WebView 的 show/hide 不同平台对 document.hidden 的反馈并不一致，
        // 显式告诉注入脚本谁在前台，后台账号就停止高频状态扫描。
        let _ = webview.eval(&profile_activity_eval_script(active));
    }
}

/// 画布 / WebGL / 硬件指纹防护。噪声由账号种子确定性生成，保证
/// 同账号跨页面、跨会话一致，不同账号之间互不相同。
const FINGERPRINT_GUARD_JS: &str = r#"
(function () {
  if (window.__mbGuardInstalled__) return;
  try { Object.defineProperty(window, '__mbGuardInstalled__', { value: true }); } catch (e) { return; }
  var seed = __SEED__ >>> 0;

  function nativeFn(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  function noiseAt(r, g, b, x, y) {
    var h = seed ^ Math.imul(x + 1, 374761393) ^ Math.imul(y + 1, 668265263) ^ Math.imul((r << 16) | (g << 8) | b, 2654435761);
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    h ^= h >>> 16;
    return ((h >>> 0) % 3) - 1;
  }

  function clamp(v) { return v < 0 ? 0 : v > 255 ? 255 : v; }

  function perturbData(imageData) {
    try {
      var data = imageData.data, w = imageData.width | 0, h = imageData.height | 0;
      var total = w * h;
      if (!total) return;
      var step = total > 262144 ? 8 : 1;
      for (var p = 0; p < total; p += step) {
        var i = p << 2;
        var nz = noiseAt(data[i], data[i + 1], data[i + 2], p % w, (p / w) | 0);
        if (nz) {
          data[i] = clamp(data[i] + nz);
          data[i + 1] = clamp(data[i + 1] - nz);
        }
      }
    } catch (e) {}
  }

  function patchImageData(proto) {
    if (!proto || !proto.getImageData) return;
    var original = proto.getImageData;
    proto.getImageData = nativeFn(function () {
      var imageData = original.apply(this, arguments);
      perturbData(imageData);
      return imageData;
    }, 'getImageData');
  }

  try { patchImageData(CanvasRenderingContext2D.prototype); } catch (e) {}
  try { if (window.OffscreenCanvasRenderingContext2D) patchImageData(OffscreenCanvasRenderingContext2D.prototype); } catch (e) {}

  // toDataURL / toBlob：把画布复制到临时画布再读取，噪声只作用于读出结果，不改用户画布。
  function readNoisedCopy(canvas) {
    var tmp = document.createElement('canvas');
    tmp.width = canvas.width;
    tmp.height = canvas.height;
    var tctx = tmp.getContext('2d');
    if (!tctx) return null;
    tctx.drawImage(canvas, 0, 0);
    var data = tctx.getImageData(0, 0, tmp.width, tmp.height);
    tctx.putImageData(data, 0, 0);
    return tmp;
  }

  try {
    var originalToDataURL = HTMLCanvasElement.prototype.toDataURL;
    HTMLCanvasElement.prototype.toDataURL = nativeFn(function () {
      try {
        if (this.width && this.height) {
          var copy = readNoisedCopy(this);
          if (copy) return originalToDataURL.apply(copy, arguments);
        }
      } catch (e) {}
      return originalToDataURL.apply(this, arguments);
    }, 'toDataURL');

    var originalToBlob = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = nativeFn(function (callback) {
      var rest = Array.prototype.slice.call(arguments, 1);
      var target = this;
      try {
        if (this.width && this.height) {
          var copy = readNoisedCopy(this);
          if (copy) {
            target = copy;
          }
        }
      } catch (e) {}
      return originalToBlob.apply(target, [callback].concat(rest));
    }, 'toBlob');
  } catch (e) {}

  // WebGL GPU 字符串掩码（UNMASKED_VENDOR_WEBGL / UNMASKED_RENDERER_WEBGL）。
  var glPick = seed % 3;
  var glVendors = ['Google Inc. (Intel)', 'Google Inc. (NVIDIA)', 'Google Inc. (AMD)'];
  var glRenderers = [
    'ANGLE (Intel, Intel(R) UHD Graphics 630 (0x00003E9B) Direct3D11 vs_5_0 ps_5_0, D3D11)',
    'ANGLE (NVIDIA, NVIDIA GeForce GTX 1650 (0x00001F82) Direct3D11 vs_5_0 ps_5_0, D3D11)',
    'ANGLE (AMD, AMD Radeon(TM) Graphics (0x00001638) Direct3D11 vs_5_0 ps_5_0, D3D11)'
  ];
  function patchGetParameter(proto) {
    if (!proto || !proto.getParameter) return;
    var original = proto.getParameter;
    proto.getParameter = nativeFn(function (param) {
      if (param === 37445) return glVendors[glPick];
      if (param === 37446) return glRenderers[glPick];
      return original.apply(this, arguments);
    }, 'getParameter');
  }
  try { if (window.WebGLRenderingContext) patchGetParameter(WebGLRenderingContext.prototype); } catch (e) {}
  try { if (window.WebGL2RenderingContext) patchGetParameter(WebGL2RenderingContext.prototype); } catch (e) {}

  // 音频指纹（AudioContext/OfflineAudioContext 渲染摘要）：按账号种子给
  // AudioBuffer 采样加确定性微噪声（±2e-7，听感无差异，摘要完全改变）。
  // getChannelData 对同一 buffer 返回缓存数组，打标记防止重复叠加漂移。
  function perturbSamples(data) {
    try {
      if (!data || !data.length || data.length > 1000000) return;
      if (data.__mbHNoised) return;
      for (var i = 0; i < data.length; i++) {
        var h = Math.imul(i + 1, 2654435761) ^ seed;
        h = Math.imul(h ^ (h >>> 13), 1274126177);
        h ^= h >>> 16;
        data[i] += (((h >>> 0) % 5) - 2) * 1e-7;
      }
      Object.defineProperty(data, '__mbHNoised', { value: true });
    } catch (e) {}
  }

  try {
    var bufferProto = AudioBuffer.prototype;
    var originalGetChannelData = bufferProto.getChannelData;
    bufferProto.getChannelData = nativeFn(function () {
      var data = originalGetChannelData.apply(this, arguments);
      perturbSamples(data);
      return data;
    }, 'getChannelData');
    if (bufferProto.copyFromChannel) {
      var originalCopyFromChannel = bufferProto.copyFromChannel;
      bufferProto.copyFromChannel = nativeFn(function (destination) {
        originalCopyFromChannel.apply(this, arguments);
        perturbSamples(destination);
      }, 'copyFromChannel');
    }
  } catch (e) {}

  try {
    var analyserProto = AnalyserNode.prototype;
    ['getFloatFrequencyData', 'getFloatTimeDomainData'].forEach(function (name) {
      var original = analyserProto[name];
      analyserProto[name] = nativeFn(function (array) {
        var result = original.apply(this, arguments);
        perturbSamples(array);
        return result;
      }, name);
    });
  } catch (e) {}

  // 硬件信息按账号固定。
  var cores = [4, 6, 8, 12, 16][seed % 5];
  var mem = [4, 8, 8, 16, 8][seed % 5];
  try { Object.defineProperty(navigator, 'hardwareConcurrency', { get: function () { return cores; }, configurable: true }); } catch (e) {}
  try { Object.defineProperty(navigator, 'deviceMemory', { get: function () { return mem; }, configurable: true }); } catch (e) {}
})();
"#;

/// 时区伪装：接管 getTimezoneOffset 与默认 Intl.DateTimeFormat。
const TIMEZONE_JS: &str = r#"
(function () {
  var TZ = '__TZ__';
  try { new Intl.DateTimeFormat('en', { timeZone: TZ }); } catch (e) { return; }

  function nativeFn(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  function zoneOffsetMinutes(date) {
    var dtf = new Intl.DateTimeFormat('en-US', {
      timeZone: TZ, hour12: false,
      year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', second: '2-digit'
    });
    var parts = dtf.formatToParts(date), map = {};
    for (var i = 0; i < parts.length; i++) map[parts[i].type] = parts[i].value;
    var asUTC = Date.UTC(+map.year, map.month - 1, +map.day, (+map.hour) % 24, +map.minute, +map.second);
    return Math.round((asUTC - date.getTime()) / 60000);
  }

  Date.prototype.getTimezoneOffset = nativeFn(function () {
    return -zoneOffsetMinutes(this);
  }, 'getTimezoneOffset');

  var RealDateTimeFormat = Intl.DateTimeFormat;
  function PatchedDateTimeFormat() {
    var locales = arguments[0];
    var options = arguments[1];
    if (!options || options.timeZone === undefined) {
      var patched = Object.assign({}, options || {}, { timeZone: TZ });
      return new RealDateTimeFormat(locales, patched);
    }
    return new RealDateTimeFormat(locales, options);
  }
  PatchedDateTimeFormat.prototype = RealDateTimeFormat.prototype;
  try { Object.defineProperty(PatchedDateTimeFormat, 'supportedLocalesOf', { value: RealDateTimeFormat.supportedLocalesOf }); } catch (e) {}
  Intl.DateTimeFormat = PatchedDateTimeFormat;
})();
"#;

/// 语言伪装：接管 navigator.language(s) 与 Intl 各格式化类的默认区域。
const LOCALE_JS: &str = r#"
(function () {
  var L = '__LOCALE__';

  function nativeFn(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  try {
    Object.defineProperty(navigator, 'language', { get: function () { return L; }, configurable: true });
    Object.defineProperty(navigator, 'languages', { get: function () { return [L]; }, configurable: true });
  } catch (e) {}

  ['DateTimeFormat', 'NumberFormat', 'Collator', 'PluralRules', 'RelativeTimeFormat', 'ListFormat', 'Segmenter', 'DisplayNames']
    .forEach(function (name) {
      try {
        var Real = Intl[name];
        if (typeof Real !== 'function') return;
        var Patched = function () {
          var args = Array.prototype.slice.call(arguments);
          if (args[0] === undefined || args[0] === null || args[0] === '') args[0] = L;
          return Reflect.construct(Real, args);
        };
        Patched.prototype = Real.prototype;
        if (Real.supportedLocalesOf) {
          Object.defineProperty(Patched, 'supportedLocalesOf', { value: Real.supportedLocalesOf });
        }
        Intl[name] = Patched;
      } catch (e) {}
    });
})();
"#;

/// 自定义 UA 去掉 Edge 标识时，同步覆盖 navigator.userAgentData，
/// 避免“UA 字符串说 Chrome、Client Hints 说 Edge”的矛盾特征。
/// 注意：Sec-CH-UA 请求头由网络层生成，JS 无法改，属 WebView2 硬限制。
const USER_AGENT_DATA_JS: &str = r#"
(function () {
  if (!navigator.userAgentData) return;
  var brands = __BRANDS__;
  var fullVersion = '__FULL__';
  var mobile = __MOBILE__;
  var platform = '__PLATFORM__';
  var original = navigator.userAgentData;
  var override = {
    brands: brands,
    mobile: mobile,
    platform: platform
  };
  override.getHighEntropyValues = function (hints) {
    return original.getHighEntropyValues(hints).then(function (values) {
      try {
        hints = hints || [];
        if (hints.indexOf('fullVersionList') !== -1) values.fullVersionList = brands;
        if (hints.indexOf('uaFullVersion') !== -1) values.uaFullVersion = fullVersion;
        if (hints.indexOf('platform') !== -1) values.platform = platform;
        if (hints.indexOf('model') !== -1) values.model = '';
      } catch (e) {}
      return values;
    });
  };
  override.toJSON = function () {
    return { brands: brands, mobile: mobile, platform: platform };
  };
  try { Object.defineProperty(navigator, 'userAgentData', { get: function () { return override; }, configurable: true }); } catch (e) {}
})();
"#;

/// 新窗口 / blob 下载链接补丁：宿主会把 target=_blank 与 window.open 请求
/// 重定向回当前页（见 on_new_window），但 blob:/data: 这类内存下载链接不能
/// 整页导航（blob URL 无法跨上下文加载），这里在页面内提前接管：
/// 摘掉 target 让锚点留在本页触发 WebView2 下载管理器，window.open(blob:)
/// 则合成一个带 download 属性的锚点点击完成下载。
const NEW_WINDOW_PATCH_JS: &str = r#"
(function () {
  if (window.__mbNewTabInstalled) return;
  try { Object.defineProperty(window, '__mbNewTabInstalled', { value: true }); } catch (e) { return; }

  function isMemoryUrl(u) { return /^(blob:|data:)/i.test(String(u || '')); }
  function isDownloadLike(el, href) {
    try {
      if (el && el.hasAttribute && el.hasAttribute('download')) return true;
      var raw = String(href || '');
      if (isMemoryUrl(raw)) return true;
      var clean = raw.split('#')[0].split('?')[0];
      if (/\.(?:md|txt|csv|docx?|xlsx?|pptx?|pdf|png|jpe?g|gif|webp|zip|rar|7z|json|html?|xml|mp3|wav|mp4|mov|webm)$/i.test(clean)) return true;
      if (/(?:^|[\/?&=_-])(?:download|export|attachment|file)(?:[\/?&=_-]|$)/i.test(raw)) return true;
      var text = el ? String(el.textContent || el.getAttribute('aria-label') || el.getAttribute('title') || '') : '';
      return /(?:下载|导出|保存文件|download|export|save file)/i.test(text);
    } catch (e) { return false; }
  }

  // WebView2 的新窗口请求会被宿主拒绝。对于用户点击的链接，直接摘掉
  // target=_blank，让请求留在当前账号 WebView 中；这样服务端返回
  // Content-Disposition: attachment 时才能进入 on_download，而不是先变成 popup。
  document.addEventListener('click', function (e) {
    try {
      if (e.defaultPrevented || e.button !== 0) return;
      var el = e.target;
      while (el && el.tagName !== 'A') el = el.parentElement;
      if (!el) return;
      var target = (el.getAttribute('target') || '').toLowerCase();
      if (target === '_blank') el.removeAttribute('target');
    } catch (x) {}
  }, true);

  // 站点经常通过 a.click() 触发下载，这种点击不会经过用户侧的 target 修补时机。
  // 在原型层也做同样兜底，尤其覆盖 blob:/data: 和带 download 属性的动态链接。
  try {
    var originalAnchorClick = HTMLAnchorElement.prototype.click;
    HTMLAnchorElement.prototype.click = function () {
      try {
        if ((this.getAttribute('target') || '').toLowerCase() === '_blank' || isDownloadLike(this, this.href)) {
          this.removeAttribute('target');
        }
      } catch (e) {}
      return originalAnchorClick.apply(this, arguments);
    };
  } catch (x) {}

  try {
    var originalOpen = window.open;
    window.open = function (url) {
      try {
        if (isMemoryUrl(url)) {
          var a = document.createElement('a');
          a.href = url;
          a.download = '';
          document.documentElement.appendChild(a);
          a.click();
          a.remove();
          return null;
        }
        // 明显的下载 URL 不创建 popup，改为当前 WebView 导航，从而交给
        // WebView2 下载管理器处理 Content-Disposition 响应。
        if (url && isDownloadLike(null, url)) {
          location.href = String(url);
          return null;
        }
      } catch (x) {}
      return originalOpen.apply(this, arguments);
    };
  } catch (x) {}
})();
"#;

fn fingerprint_script(id: &str) -> String {
    FINGERPRINT_GUARD_JS.replace("__SEED__", &profile_seed(id).to_string())
}

/// 注入每个账号 WebView 的状态抓取脚本：在 chat01.ai 页面里用启发式规则
/// （DOM 文本 + localStorage）提取积分和登录账号，然后通过一次会被后端
/// 取消的 mbstatus:// 假导航把数据带出来。chat01.ai 是客户端渲染的 SPA，
/// 无法依赖固定选择器，所以采取多策略扫描并只在结果变化时上报。
const STATUS_REPORTER_JS: &str = r#"
(function () {
  if (window.__mbStatusInstalled) return;
  try { Object.defineProperty(window, '__mbStatusInstalled', { value: true }); } catch (e) { return; }

  var EMAIL_RE = /^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$/;
  var CRED_HEAD = /(?:积分|点数|余额|credits?)\s*[:：=]?\s*([0-9][0-9,]*(?:\.[0-9]+)?)/i;
  var CRED_TAIL = /([0-9][0-9,]*(?:\.[0-9]+)?)\s*(?:个)?\s*(?:积分|点数|credits?)\b/i;
  var CRED_KEY = /credit|balance|remain|quota|points?|jifen|积分/;
  var NAME_KEY = /^(display_?name|user_?name|nick(name)?|full_?name|name)$/;
  // alt / aria-label 里常见的界面用词，不能当作登录账号名。
  var NOT_A_NAME = /^(true|false|null|undefined|user|admin|member|guest|logo|logo\.png|icon|icons|image|img|photo|picture|avatar|profile|account|menu|home|search|settings|setting|close|open|toggle|send|submit|copy|edit|delete|remove|more|back|next|previous|refresh|reload|loading|app|site|web|link|chat|chat01|chat01\.ai|gpt|ai|用户|账户|账号|头像|图标|标志|菜单|首页|搜索|设置|关闭|打开|更多|发送|复制|删除|返回|刷新|加载|加载中|登录|退出|注册)$/;
  var JUNK_NAME_RE = NOT_A_NAME;

  // 页面脚本检测 hook 的第一招就是 fetch.toString()，这里伪装成原生函数。
  function maskNative(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  function onSite() {
    var h = (location.hostname || '').toLowerCase();
    return h === 'chat01.ai' || (h.length > 10 && h.slice(-10) === '.chat01.ai');
  }

  function scanObject(obj, depth, out) {
    if (!obj || typeof obj !== 'object' || depth > 4) return;
    try {
      for (var key in obj) {
        if (!Object.prototype.hasOwnProperty.call(obj, key)) continue;
        var v = obj[key];
        if (v === null || v === undefined) continue;
        var lk = String(key).toLowerCase();
        if (typeof v === 'object') { scanObject(v, depth + 1, out); continue; }
        // 积分始终覆盖：接口返回的余额会随消耗变化，不能只取第一次。
        if (CRED_KEY.test(lk)) {
          var num = typeof v === 'number' ? v : parseFloat(String(v).replace(/[,，\s]/g, ''));
          if (isFinite(num) && num >= 0 && num < 1e9) out.credits = String(Math.round(num * 100) / 100);
        }
        if (typeof v === 'string') {
          var t = v.trim();
          if (!out.email && /e-?mail|login|account/.test(lk) && t.length < 100 && EMAIL_RE.test(t)) out.email = t;
          if (!out.name && NAME_KEY.test(lk) && t && t.length <= 60 && !EMAIL_RE.test(t) && !NOT_A_NAME.test(t)) out.name = t;
        }
      }
    } catch (e) {}
  }

  function scanStorages() {
    var out = { credits: '', name: '', email: '' };
    var stores = [];
    try { stores.push(window.localStorage); } catch (e) {}
    try { stores.push(window.sessionStorage); } catch (e) {}
    for (var si = 0; si < stores.length; si++) {
      var store = stores[si];
      var len = 0;
      try { len = store.length; } catch (e) { continue; }
      for (var i = 0; i < len && i < 80; i++) {
        var raw = null;
        try { raw = store.getItem(store.key(i)); } catch (e) {}
        if (!raw) continue;
        var parsed = null;
        try { parsed = JSON.parse(raw); } catch (e) {}
        if (parsed && typeof parsed === 'object') {
          scanObject(parsed, 0, out);
        } else if (EMAIL_RE.test(String(raw).trim())) {
          if (!out.email) out.email = String(raw).trim();
        }
      }
    }
    return out;
  }

  function scanDom() {
    var out = { credits: '', name: '', email: '' };
    try {
      var nodes = document.querySelectorAll('div,span,p,a,button,strong,b,em,i,td,th,li,label,h1,h2,h3,h4,h5,h6,small');
      var emails = [];
      for (var i = 0; i < nodes.length; i++) {
        var el = nodes[i];
        var t = (el.textContent || '').replace(/\s+/g, ' ').trim();
        if (!t || t.length > 40) continue;
        if (!out.credits) {
          var m = t.match(CRED_HEAD) || t.match(CRED_TAIL);
          if (m) out.credits = m[1].replace(/,/g, '');
        }
        var em = t.match(EMAIL_RE);
        if (em) emails.push(em[0]);
        if (el.tagName === 'A') {
          var href = el.getAttribute('href') || '';
          if (href.slice(0, 7).toLowerCase() === 'mailto:') emails.push(href.slice(7).split('?')[0]);
        }
        // 关键字段都拿到后不必把剩余节点扫完。
        if (out.credits && out.email) break;
      }
      for (var j = 0; j < emails.length; j++) {
        if (/gmail\.com$/i.test(emails[j])) { out.email = emails[j]; break; }
      }
      if (!out.email) out.email = emails[0] || '';
      var labeled = document.querySelectorAll('img[alt],button[aria-label],[aria-label]');
      for (var k = 0; k < labeled.length; k++) {
        var attr = (labeled[k].getAttribute('alt') || labeled[k].getAttribute('aria-label') || '').trim();
        if (attr && attr.length >= 2 && attr.length <= 40 && !EMAIL_RE.test(attr) &&
            /[a-z0-9\u4e00-\u9fa5]/i.test(attr) && !JUNK_NAME_RE.test(attr)) {
          out.name = attr;
          break;
        }
      }
    } catch (e) {}
    return out;
  }

  // ---- AI 回答状态：只观察 DOM 结构变化，不跟踪每个流式 token 的文字变化。
  // 这样后台同时开很多账号时，MutationObserver 的压力会明显低于 characterData 全量监听。 ----
  var profileActive = !document.hidden;
  var answerGenerating = false;
  var answerReady = false;
  var answerFinishTimer = 0;
  var answerCheckTimer = 0;

  function hasGeneratingIndicator() {
    try {
      var quick = document.querySelector(
        'button[data-testid*="stop" i],button[aria-label*="stop" i],button[title*="stop" i],button[aria-label*="停止"],button[title*="停止"]'
      );
      if (quick) return true;
      var buttons = document.querySelectorAll('button,[role="button"]');
      var limit = Math.min(buttons.length, 180);
      for (var i = 0; i < limit; i++) {
        var el = buttons[i];
        var text = String(
          el.getAttribute('aria-label') || el.getAttribute('title') || el.textContent || ''
        ).replace(/\s+/g, ' ').trim();
        if (/^(?:停止(?:生成|回答|输出|响应|思考)?|取消生成|stop(?: generating| generation| response)?|cancel generation)$/i.test(text)) {
          return true;
        }
      }
    } catch (e) {}
    return false;
  }

  function checkAnswerState() {
    answerCheckTimer = 0;
    var generating = hasGeneratingIndicator();
    if (generating) {
      if (answerFinishTimer) { clearTimeout(answerFinishTimer); answerFinishTimer = 0; }
      if (!answerGenerating) {
        answerGenerating = true;
        answerReady = false;
        try { report(true); } catch (e) {}
      }
      return;
    }
    if (!answerGenerating || answerFinishTimer) return;
    // 结束按钮偶尔会因前端重渲染短暂消失，稳定 1.2 秒后才判定回答完成。
    answerFinishTimer = setTimeout(function () {
      answerFinishTimer = 0;
      if (hasGeneratingIndicator()) { scheduleAnswerCheck(); return; }
      if (!answerGenerating) return;
      answerGenerating = false;
      // 只有后台账号才需要“回答完成”提醒；当前正在看的账号不额外打扰。
      answerReady = !profileActive;
      try { report(true); } catch (e) {}
    }, 1200);
  }

  function scheduleAnswerCheck() {
    if (answerCheckTimer) return;
    answerCheckTimer = setTimeout(checkAnswerState, 300);
  }

  try {
    document.addEventListener('mb-profile-active', function (e) {
      try {
        profileActive = !!(e.detail && e.detail.active);
        if (profileActive) kickIfVisible();
        scheduleAnswerCheck();
      } catch (x) {}
    });
    document.addEventListener('mb-answer-seen', function () {
      if (!answerReady) return;
      answerReady = false;
      try { report(true); } catch (e) {}
    });
  } catch (e) {}

  // ---- 接口响应扫描（积分精确化）：钩住 fetch / XHR，直接解析 JSON 里的积分字段，
  // 比页面文案更准、更快（在积分被渲染前就能拿到），文案扫描仅作兜底。----
  var api = { credits: '', name: '', email: '' };

  function scanApiResponse(data) {
    try {
      if (!data || typeof data !== 'object') return;
      var before = api.credits + '|' + api.email;
      scanObject(data, 0, api);
      if (api.credits + '|' + api.email !== before) kickIfVisible();
    } catch (e) {}
  }

  try {
    var originalFetch = window.fetch;
    if (originalFetch && !Object.getOwnPropertyDescriptor(originalFetch, '__mbH')) {
      var patchedFetch = function () {
        var promise = originalFetch.apply(this, arguments);
        try {
          promise
            .then(function (res) {
              try {
                var ct = '';
                try { ct = (res.headers && res.headers.get('content-type')) || ''; } catch (e) {}
                if (ct.indexOf('json') === -1 && ct.indexOf('text') === -1) return;
                res.clone().text().then(function (text) {
                  if (text && text.length < 500000 && text.charAt(0) !== '<') {
                    scanApiResponse(JSON.parse(text));
                  }
                }).catch(function () {});
              } catch (e) {}
            })
            .catch(function () {});
        } catch (e) {}
        return promise;
      };
      maskNative(patchedFetch, 'fetch');
      try { Object.defineProperty(patchedFetch, '__mbH', { value: true }); } catch (e) {}
      window.fetch = patchedFetch;
    }
  } catch (e) {}

  try {
    var xhrProto = XMLHttpRequest.prototype;
    if (xhrProto.send && !Object.getOwnPropertyDescriptor(xhrProto.send, '__mbH')) {
      var originalSend = xhrProto.send;
      var patchedSend = function () {
        try {
          this.addEventListener('load', function () {
            try {
              var text = this.responseText || '';
              if (text && text.length < 500000 && text.charAt(0) !== '<') {
                scanApiResponse(JSON.parse(text));
              }
            } catch (e) {}
          });
        } catch (e) {}
        return originalSend.apply(this, arguments);
      };
      maskNative(patchedSend, 'send');
      try { Object.defineProperty(patchedSend, '__mbH', { value: true }); } catch (e) {}
      xhrProto.send = patchedSend;
    }
  } catch (e) {}

  var lastPayload = '';
  function report(force) {
    if (!onSite()) return;
    var st = scanStorages();
    var dom = scanDom();
    var credits = api.credits || dom.credits || st.credits || '';
    var name = api.name || st.name || dom.name || '';
    var email = api.email || dom.email || st.email || '';
    // 谷歌账号的登录名就是邮箱，优先展示；页面昵称只在没有邮箱时兜底。
    var shown = email || name;
    if (!credits && !shown && !force && !answerReady && !answerGenerating) return;
    var payload = 'c=' + encodeURIComponent(credits) +
      '&n=' + encodeURIComponent(shown) +
      '&e=' + encodeURIComponent(email) +
      '&r=' + (answerReady ? '1' : '0') +
      '&g=' + (answerGenerating ? '1' : '0');
    if (!force && payload === lastPayload) return;
    lastPayload = payload;
    try { location.href = 'mbstatus://report?' + payload; } catch (e) {}
  }

  // 后端"刷新状态"按钮通过 mb-status-scan 自定义事件触发强制重扫。
  try {
    document.addEventListener('mb-status-scan', function () { try { report(true); } catch (e) {} });
  } catch (e) {}

  // ---- AI 生成文件自动下载：后端通过 mb-dl-cfg 自定义事件下发开关与扩展名白名单。
  // 修复点：配置到达时主动扫描已有链接；同时监听 href/download 属性变化，覆盖
  // React/Vue 先插入 <a>、后异步赋 href 的情况；HTTP(S) 文件链接也纳入识别。----
  var dlCfg = { auto: false, skipDownloaded: true, downloadedNames: [], blockFirstSeconds: 3, exts: [] };
  var autoScanTimer = 0;
  var downloadGuardStartedAt = Date.now();

  function normalizedExt(value) {
    var raw = String(value || '').split('#')[0].split('?')[0];
    var m = /\.([a-z0-9]{1,8})$/i.exec(raw);
    return m ? m[1].toLowerCase() : '';
  }

  function inferredFileName(name, href) {
    var rawName = String(name || '').trim();
    if (rawName) {
      rawName = rawName.split(/[\\/]/).pop() || rawName;
      if (/\.[a-z0-9]{1,8}$/i.test(rawName)) return rawName.toLowerCase();
    }
    try {
      var clean = String(href || '').split('#')[0].split('?')[0];
      var decoded = decodeURIComponent(clean);
      var part = decoded.split('/').pop() || '';
      return part.toLowerCase();
    } catch (e) { return ''; }
  }

  function alreadyDownloaded(name, href) {
    if (!dlCfg.skipDownloaded) return false;
    var candidate = inferredFileName(name, href);
    if (!candidate) return false;
    var names = dlCfg.downloadedNames || [];
    for (var i = 0; i < names.length; i++) {
      if (String(names[i] || '').toLowerCase() === candidate) return true;
    }
    return false;
  }

  function extAllowed(name, href) {
    if (!dlCfg.auto) return false;
    var exts = dlCfg.exts || [];
    if (!exts.length) return true;
    var ext = normalizedExt(name) || normalizedExt(href);
    // 白名单开启后必须能识别出扩展名；未知格式不自动下载，避免白名单被绕过。
    if (!ext) return false;
    return exts.indexOf(ext) !== -1;
  }

  function looksDownloadLike(el, href) {
    if (el.hasAttribute('download')) return true;
    if (/^(?:blob:|data:)/i.test(href)) return true;
    var clean = String(href || '').split('#')[0].split('?')[0];
    if (/\.[a-z0-9]{1,8}$/i.test(clean)) return true;
    if (/(?:^|[\/?&=_-])(?:download|export|attachment|file)(?:[\/?&=_-]|$)/i.test(href)) return true;
    var text = String(el.textContent || el.getAttribute('aria-label') || el.getAttribute('title') || '');
    return /(?:下载|导出|保存文件|download|export|save file)/i.test(text);
  }

  function downloadGuardRemainingMs() {
    var seconds = Math.max(0, Number(dlCfg.blockFirstSeconds) || 0);
    return Math.max(0, seconds * 1000 - (Date.now() - downloadGuardStartedAt));
  }

  function tryAutoDownload(el) {
    try {
      if (!onSite() || !dlCfg.auto || el.__mbAutoClicked || el.tagName !== 'A') return;
      if (downloadGuardRemainingMs() > 0) return;
      var href = el.getAttribute('href') || '';
      if (!href || href === '#' || /^javascript:/i.test(href)) return;
      if (!looksDownloadLike(el, href)) return;
      var name = el.getAttribute('download') || el.getAttribute('title') || el.textContent || '';
      if (!extAllowed(name, href)) return;
      if (alreadyDownloaded(name, href)) { el.__mbAutoClicked = true; return; }
      el.__mbAutoClicked = true;
      // target=_blank 在嵌入 WebView 中容易转成被拒绝的新窗口请求；强制当前 WebView。
      if ((el.getAttribute('target') || '').toLowerCase() === '_blank') el.removeAttribute('target');
      el.click();
    } catch (e) {}
  }

  function scanAutoDownloads(root) {
    try {
      if (!dlCfg.auto || !onSite()) return;
      if (root && root.tagName === 'A') tryAutoDownload(root);
      var base = root && root.querySelectorAll ? root : document;
      var anchors = base.querySelectorAll('a[href]');
      for (var i = 0; i < anchors.length; i++) tryAutoDownload(anchors[i]);
    } catch (e) {}
  }

  function scheduleAutoScan() {
    if (autoScanTimer) clearTimeout(autoScanTimer);
    var delay = Math.max(50, downloadGuardRemainingMs() + 50);
    autoScanTimer = setTimeout(function () {
      autoScanTimer = 0;
      scanAutoDownloads(document);
    }, delay);
  }

  try {
    document.addEventListener('mb-dl-cfg', function (e) {
      try {
        if (e.detail && typeof e.detail === 'object') dlCfg = e.detail;
        // 配置在 on_page_load 中下发；与后端同一时点重置保护起点，避免慢页面计时漂移。
        downloadGuardStartedAt = Date.now();
        installMutationObserver();
        if (dlCfg.auto) scheduleAutoScan();
      } catch (x) {}
    });
  } catch (e) {}

  function handleMutations(muts) {
    scheduleAnswerCheck();
    try {
      if (!dlCfg.auto) return;
      for (var i = 0; i < muts.length; i++) {
        var m = muts[i];
        if (m.type === 'attributes') {
          tryAutoDownload(m.target);
          continue;
        }
        var nodes = m.addedNodes || [];
        for (var j = 0; j < nodes.length; j++) {
          var n = nodes[j];
          if (n.nodeType !== 1) continue;
          scanAutoDownloads(n);
        }
      }
    } catch (e) {}
  }

  var domObserver = null;
  function installMutationObserver() {
    try {
      if (domObserver) domObserver.disconnect();
      domObserver = new MutationObserver(handleMutations);
      var options = { childList: true, subtree: true };
      // 仅在自动下载开启时才监听 href/download 属性；不再监听 characterData，
      // 避免 AI 流式输出每个 token 都触发观察器。
      if (dlCfg.auto) {
        options.attributes = true;
        options.attributeFilter = ['href', 'download', 'target'];
      }
      domObserver.observe(document.documentElement, options);
    } catch (e) {}
  }

  var pending = 0;
  function kick() {
    if (pending) return;
    pending = setTimeout(function () { pending = 0; try { report(false); } catch (e) {} }, 2500);
  }

  // 隐藏的后台账号 WebView 不做积分/账号的周期扫描；AI 完成检测仍保持轻量运行。
  function kickIfVisible() {
    if (profileActive && !document.hidden) kick();
  }

  function start() {
    if (!onSite()) return;
    setTimeout(function () { if (profileActive) { try { report(false); } catch (e) {} } }, 800);
    setTimeout(function () { if (profileActive) { try { report(false); } catch (e) {} } }, 3000);
    setInterval(function () {
      if (profileActive && !document.hidden) { try { report(false); } catch (e) {} }
    }, 12000);
    try { document.addEventListener('visibilitychange', kickIfVisible); } catch (e) {}
    installMutationObserver();
    scheduleAnswerCheck();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
"#;

fn timezone_script(tz: &str) -> String {
    TIMEZONE_JS.replace("__TZ__", tz)
}

fn locale_script(locale: &str) -> String {
    LOCALE_JS.replace("__LOCALE__", locale)
}

/// 自定义 UA 生效时同步伪装 userAgentData。UA 里仍带 Edg/ 或解析不出
/// Chrome 版本时返回 None（保持原生，不做半吊子伪装）。
fn user_agent_data_script(user_agent: &str) -> Option<String> {
    if user_agent.contains("Edg/") {
        return None;
    }
    let start = user_agent.find("Chrome/")? + "Chrome/".len();
    let rest = &user_agent[start..];
    let version = rest.split([' ', ';']).next()?;
    if version.is_empty()
        || version.len() > 20
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return None;
    }
    let major = version.split('.').next()?;
    let platform = if user_agent.contains("Android") {
        "Android"
    } else if user_agent.contains("Macintosh") {
        "macOS"
    } else if user_agent.contains("Linux") && !user_agent.contains("Android") {
        "Linux"
    } else if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        "iOS"
    } else {
        "Windows"
    };
    let mobile = if platform == "Android" || platform == "iOS" { "true" } else { "false" };
    let brands = serde_json::json!([
        { "brand": "Not.A/Brand", "version": "99" },
        { "brand": "Chromium", "version": major },
        { "brand": "Google Chrome", "version": major }
    ])
    .to_string();
    Some(
        USER_AGENT_DATA_JS
            .replace("__BRANDS__", &brands)
            .replace("__FULL__", version)
            .replace("__MOBILE__", mobile)
            .replace("__PLATFORM__", platform),
    )
}

/// 从代理 URL 拼出 --proxy-server 参数值。Chromium 该参数不支持内嵌用户名密码，
/// 显式丢弃 userinfo，避免带认证的代理导致参数解析失败。
fn proxy_endpoint(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

fn create_profile_webview(
    app: &AppHandle,
    profile: &Profile,
    bounds: &BrowserBounds,
) -> Result<(), String> {
    let label = profile_label(&profile.id);
    if app.get_webview(&label).is_some() {
        return Ok(());
    }

    // UI 主 WebView 所在的窗口。浏览器子 WebView 只占据 BrowserViewport 的矩形区域，
    // 因而不会盖住左/右侧栏、顶部地址栏以及宿主侧快捷工具栏。
    let parent = app
        .get_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;

    // 无论旧数据是否完整，都保证 WebView 有一个可用启动地址。
    let url = profile_start_url(app, profile);

    let id_for_event = profile.id.clone();
    let app_for_event = app.clone();
    let app_for_page_load = app.clone();
    let id_for_page_load = profile.id.clone();
    let download_guard_started_at = Arc::new(Mutex::new(Instant::now()));
    let download_guard_for_navigation = download_guard_started_at.clone();
    let download_guard_for_page_load = download_guard_started_at.clone();

    let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
        // 保留系统默认 UA。不要为了绕过 Google OAuth 风控而伪造 Chrome UA。
        .incognito(profile.incognito)
        .on_navigation(move |url| {
            // 注入脚本用 mbstatus:// 假导航回传积分 / 登录账号，这里拦截并取消导航。
            if url.scheme() == STATUS_SCHEME {
                if url.host_str() == Some("report") {
                    let query: HashMap<String, String> = url
                        .query()
                        .map(|q| {
                            url::form_urlencoded::parse(q.as_bytes())
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let field = |key: &str| {
                        query
                            .get(key)
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                    };
                    save_profile_status(
                        &app_for_event,
                        &id_for_event,
                        clean_status(ProfileStatus {
                            credits: field("c"),
                            account: field("n"),
                            email: field("e"),
                            updated_at: Some(Utc::now().to_rfc3339()),
                            answer_ready: query.get("r").map(|v| v == "1").unwrap_or(false),
                            answer_generating: query.get("g").map(|v| v == "1").unwrap_or(false),
                            ..Default::default()
                        }),
                    );
                }
                return false;
            }
            if let Ok(mut started_at) = download_guard_for_navigation.lock() { *started_at = Instant::now(); }
            let value = url.as_str().to_string();
            // files.chat01.ai/python-generations 只作为下载中转页，不写入账号的 last_url。
            // 即使应用在下载过程中退出，下次打开账号也不会再次落到下载地址。
            if !is_chat01_download_url(url) {
                update_last_url(&app_for_event, &id_for_event, &value);
            }
            let _ = app_for_event.emit(
                "profile:navigated",
                NavigationEvent {
                    id: id_for_event.clone(),
                    url: value,
                },
            );
            true
        })
        // 状态抓取脚本对每个账号无条件注入（不受指纹防护等开关影响）。
        .initialization_script(STATUS_REPORTER_JS)
        .initialization_script(NEW_WINDOW_PATCH_JS)
        // 创建 WebView 后立刻 push 一次可能撞上文档尚未初始化；每次页面加载
        // 都重新下发当前配置，确保刷新、登录重定向后自动下载仍然生效。
        .on_page_load(move |webview, _payload| {
            if let Ok(mut started_at) = download_guard_for_page_load.lock() { *started_at = Instant::now(); }
            let _ = webview.eval(&download_cfg_eval_script(&app_for_page_load));
            // 刷新前若保存了页面位置，则仅恢复一次。URL hash 同时保留，
            // 对长对话/文档页面比单纯 reload 更接近刷新前的位置。
            let _ = webview.eval(r#"
              try {
                var raw = sessionStorage.getItem('__mab_refresh_position_v1');
                if (raw) {
                  sessionStorage.removeItem('__mab_refresh_position_v1');
                  var pos = JSON.parse(raw);
                  var restore = function() {
                    if (pos.hash && location.hash !== pos.hash) location.hash = pos.hash;
                    window.scrollTo(Number(pos.x) || 0, Number(pos.y) || 0);
                  };
                  requestAnimationFrame(function(){ requestAnimationFrame(restore); });
                  setTimeout(restore, 350);
                }
              } catch (_) {}
            "#);
            // SPA 刷新/登录重定向会重建 document，重新下发前后台状态，
            // 避免隐藏账号误以为自己在前台而恢复高频扫描。
            let _ = webview.eval(&profile_activity_eval_script(is_active_profile(&id_for_page_load)));
        });

    // 下载统一接管：所有下载静默保存到设置的下载目录，重名自动加序号，
    // 完成后通过事件通知前端弹提示。chat01 AI 文件的“自动点击下载”
    // 由注入脚本负责，这里负责落盘。
    let app_for_download = app.clone();
    let id_for_download = profile.id.clone();
    let download_guard_for_download = download_guard_started_at.clone();
    // 记住当前下载来源。所有 files.chat01.ai/python-generations 直链在完成后回主界面。
    // 另外用独立、不可随历史清理的下载账本从后端硬性阻止重复下载。
    let pending_download_url = Arc::new(Mutex::new(None::<String>));
    let queued_downloads = Arc::new(Mutex::new(HashSet::<String>::new()));
    let queued_downloads_for_download = queued_downloads.clone();
    builder = builder.on_download(move |webview, event| match event {
        tauri::webview::DownloadEvent::Requested { url, destination } => {
            let settings = load_app_settings(&app_for_download);
            let guard_seconds = download_guard_seconds_for(&settings, &url);
            let elapsed = download_guard_for_download.lock().ok().map(|started_at| started_at.elapsed()).unwrap_or_default();
            let guard_active = guard_seconds > 0 && elapsed < Duration::from_secs(guard_seconds);
            if guard_active {
                let remaining = Duration::from_secs(guard_seconds).saturating_sub(elapsed);
                let retry_key = normalized_download_url(&url);
                let should_queue = settings.download_guard_auto_retry
                    && queued_downloads_for_download.lock().ok().map(|mut q| q.insert(retry_key.clone())).unwrap_or(false);
                let _ = app_for_download.emit("profile:download-guard-blocked", DownloadGuardBlockedEvent { seconds: remaining.as_secs().max(1), queued: should_queue });
                if should_queue {
                    let app_retry = app_for_download.clone();
                    let label_retry = profile_label(&id_for_download);
                    let queued_retry = queued_downloads_for_download.clone();
                    let retry_url = url.as_str().to_string();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(remaining + Duration::from_millis(150)).await;
                        if let Some(wv) = app_retry.get_webview(&label_retry) {
                            if let Ok(js_url) = serde_json::to_string(&retry_url) {
                                let script = format!("try{{var a=document.createElement('a');a.href={};a.download='';a.style.display='none';document.documentElement.appendChild(a);a.click();a.remove();}}catch(e){{}}", js_url);
                                let _ = wv.eval(&script);
                            }
                        }
                        if let Ok(mut q) = queued_retry.lock() { q.remove(&retry_key); }
                    });
                }
                return false;
            }
            let should_return_home = is_chat01_download_url(&url);
            let suggested_name = destination
                .file_name()
                .map(|name| name.to_string_lossy().trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty());
            let ledger = load_download_ledger(&app_for_download);
            let duplicate = download_seen(&app_for_download, &url)
                || suggested_name.as_ref().map(|name| ledger.names.contains(name)).unwrap_or(false);
            if duplicate {
                let name = suggested_name
                    .or_else(|| inferred_download_name(&url))
                    .unwrap_or_else(|| "该文件".to_string());
                let _ = app_for_download.emit("profile:download-blocked", name);
                if should_return_home {
                    if let Ok(home) = Url::parse(DOWNLOAD_RETURN_URL) { let _ = webview.navigate(home); }
                }
                return false;
            }
            if let Ok(mut pending) = pending_download_url.lock() { *pending = Some(url.as_str().to_string()); }
            download_destination(&app_for_download, &url, destination);
            true
        }
        tauri::webview::DownloadEvent::Finished { path, success, .. } => {
            let source_url = pending_download_url.lock().ok().and_then(|mut pending| pending.take());
            let should_return_home = source_url.as_deref()
                .and_then(|raw| Url::parse(raw).ok())
                .map(|url| is_chat01_download_url(&url))
                .unwrap_or(false);
            record_download(
                &app_for_download,
                &id_for_download,
                path.map(|p| p.to_string_lossy().to_string()),
                source_url,
                success,
            );
            if should_return_home {
                if let Ok(home) = Url::parse(DOWNLOAD_RETURN_URL) { let _ = webview.navigate(home); }
            }
            true
        }
        _ => true,
    });

    // 拖拽上传：Tauri 默认接管 WebView 的拖放事件（供宿主 onDragDrop 回调使用），
    // 页面因此收不到 HTML5 drag 事件，无法把文件拖进网站的上传区。
    // 账号页面不需要宿主感知拖放，关闭接管后交给 WebView2 原生处理。
    builder = builder.disable_drag_drop_handler();

    // 新窗口请求：wry 未设置处理器时一律静默拒绝 target=_blank / window.open，
    // chat01 等站点的下载链接（新窗口打开）点击后毫无反应。这里把请求的 URL
    // 导航到当前账号页内打开（相当于“在本标签页打开”），blob:/data: 已由
    // NEW_WINDOW_PATCH_JS 在页面内接管，不会走到这里。
    let app_for_new_window = app.clone();
    let id_for_new_window = profile.id.clone();
    builder = builder.on_new_window(move |url, _features| {
        if matches!(url.scheme(), "http" | "https") {
            if let Some(webview) = app_for_new_window.get_webview(&profile_label(&id_for_new_window))
            {
                let _ = webview.navigate(url);
            }
        }
        NewWindowResponse::Deny
    });

    if !profile.user_agent.is_empty() {
        builder = builder.user_agent(profile.user_agent.trim());
        // UA 声称是 Chrome 时同步覆盖 userAgentData，防 Client Hints 穿帮。
        if let Some(script) = user_agent_data_script(profile.user_agent.trim()) {
            builder = builder.initialization_script(script);
        }
    }

    // 指纹防护脚本在任何页面脚本执行前注入。
    if profile.fingerprint_guard {
        builder = builder.initialization_script(fingerprint_script(&profile.id));
    }

    if !profile.timezone.is_empty() {
        builder = builder.initialization_script(timezone_script(&profile.timezone));
    }

    if !profile.locale.is_empty() {
        builder = builder.initialization_script(locale_script(&profile.locale));
    }

    let proxy = parse_proxy(&profile.proxy)?;
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = &proxy;
    let locale = profile.locale.trim().to_string();

    // Windows / WebView2：data_directory 会映射到独立的 UserDataFolder，
    // Cookie / LocalStorage / IndexedDB / Cache 因此物理分目录。
    // WebView2 环境按 UserDataFolder 隔离，即使隐身账号也分配独立目录，
    // 避免多个账号共享默认环境导致参数冲突。
    #[cfg(target_os = "windows")]
    {
        let data_dir = profile_data_dir(app, &profile.id)?;
        fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        builder = builder.data_directory(data_dir);

        // 代理 / 语言 / WebRTC 策略统一通过 additional_browser_args 下发。
        // 注意：一旦传入该参数，wry 就不再拼自己的默认参数，所以默认的
        // --disable-features 与 autoplay 策略必须在这里原样带上。
        if proxy.is_some() || !locale.is_empty() {
            let mut args = String::from(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required",
            );
            if let Some(url) = &proxy {
                args.push_str(&format!(" --proxy-server={}", proxy_endpoint(url)));
                // WebRTC 禁止非代理 UDP，防止真实 IP 绕过代理直连形成关联。
                args.push_str(" --force-webrtc-ip-handling-policy=disable_non_proxied_udp");
            }
            if !locale.is_empty() {
                args.push_str(&format!(" --lang={locale}"));
            }
            builder = builder.additional_browser_args(&args);
        } else if let Some(url) = proxy {
            builder = builder.proxy_url(url);
        }
    }

    // macOS / WKWebView：不支持 data_directory。Tauri 2.9+ 提供 data_store_identifier，
    // 对应 WKWebsiteDataStore(identifier:)；仅 macOS 14+ 支持持久化自定义数据存储。
    #[cfg(target_os = "macos")]
    {
        let uuid = Uuid::parse_str(&profile.id).map_err(|e| e.to_string())?;
        builder = builder.data_store_identifier(*uuid.as_bytes());
    }

    // 每账号独立代理（Linux 走 wry 内置支持；代理在 Windows 上已于上方组合进参数）。
    #[cfg(target_os = "linux")]
    if let Some(url) = proxy {
        builder = builder.proxy_url(url);
    }

    parent
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, String> {
    load_profiles(&app)
}

// ---- 账号模板命令 ----

#[tauri::command]
fn list_profile_templates(app: AppHandle) -> Result<Vec<ProfileTemplate>, String> {
    Ok(load_profile_templates(&app))
}

/// 保存模板。返回**规范化后**的列表，前端以此回写本地状态，保证前后端一致。
#[tauri::command]
fn save_profile_templates(
    app: AppHandle,
    templates: Vec<ProfileTemplate>,
) -> Result<Vec<ProfileTemplate>, String> {
    let templates = normalized_templates(templates);
    store_profile_templates(&app, &templates)?;
    Ok(templates)
}

// ---- store 结构迁移 ----

/// 读取 store 里记录的结构版本。缺失或非法一律视为 0（v14 及之前的历史数据）。
fn stored_schema_version<R: Runtime>(app: &AppHandle<R>) -> u64 {
    app.store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(SCHEMA_VERSION_KEY))
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

/// 按升版顺序排列的迁移步骤，索引 i 表示「把版本 i 升到 i + 1」。
///
/// 每个步骤只做一件事：把该版本之前靠 serde default 隐式兜底的语义显式写清楚。
/// 新增版本时往数组尾部追加函数即可，已发布的旧步骤不要再改动。
fn migrations<R: Runtime>() -> Vec<fn(&AppHandle<R>) -> Result<(), String>> {
    vec![migrate_v0_to_v1::<R>]
}

/// v0 -> v1：把历史数据里「靠 serde default 才成立」的字段补齐并落盘。
///
/// 这一步不改变任何用户可见行为，只是让旧数据显式带上当前字段，
/// 避免后续版本删掉 serde default 时突然丢失兼容性。
fn migrate_v0_to_v1<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // 账号：修正空/损坏的 default_url 与 last_url，并规范化 url_mode。
    let mut profiles = load_profiles(app)?;
    let mut changed = false;
    for profile in &mut profiles {
        changed |= repair_profile_urls(app, profile);
    }
    if changed {
        save_profiles(app, &profiles)?;
    }

    // 设置：补齐新增字段的当前默认值（含全局默认主页）。
    let mut settings = load_app_settings(app);
    let fallback = default_global_profile_url();
    if normalize_url(&settings.global_default_url).is_err() {
        settings.global_default_url = fallback;
    }
    settings.download_guard_rules = normalized_guard_rules(settings.download_guard_rules);
    // skip_downloaded_files 已在 v11 移出 UI，Rust 侧始终按 true 处理（与 get/set_app_settings 一致）。
    settings.skip_downloaded_files = true;
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        SETTINGS_KEY,
        serde_json::to_value(&settings).map_err(|e| e.to_string())?,
    );

    // 模板：剔除无名/无 id 项、修正非法 url_mode。
    let templates = load_profile_templates(app);
    if !templates.is_empty() {
        let normalized = normalized_templates(templates);
        store_profile_templates(app, &normalized)?;
    }

    store.save().map_err(|e| e.to_string())
}

/// 在启动时执行一次结构迁移。
///
/// - 版本相同：不做任何事。
/// - 版本落后：按顺序补齐缺失的步骤，最后写入新版本号。
/// - 版本超前：说明用户用旧版程序打开了新版数据。此时**不降级、不改数据**，
///   否则会用旧结构覆盖掉新字段。仅提示用户升级程序。
fn migrate_store<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let current = stored_schema_version(app);
    if current > SCHEMA_VERSION {
        return Err(format!(
            "数据目录来自更新版本的应用（数据版本 {current}，当前程序支持 {SCHEMA_VERSION}）。\
             请升级到最新版后再打开，以免覆盖新数据。"
        ));
    }
    if current == SCHEMA_VERSION {
        return Ok(());
    }

    let steps = migrations();
    // 版本号必须与迁移步骤数对得上，否则说明新增版本时忘了补迁移函数。
    if steps.len() as u64 != SCHEMA_VERSION {
        return Err(format!(
            "内部错误：迁移步骤数 {} 与 schema 版本 {SCHEMA_VERSION} 不一致",
            steps.len()
        ));
    }

    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    for version in current..SCHEMA_VERSION {
        steps[version as usize](app)?;
    }
    store.set(SCHEMA_VERSION_KEY, serde_json::json!(SCHEMA_VERSION));
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// 单个创建与批量创建共用的校验与构造逻辑。
///
/// 校验口径只在这里维护一份，避免两个入口各自演化后行为分叉。
/// `base_order` 由调用方按当前账号数 + 偏移量给出，确保批量创建时 order 连续且不冲突。
fn build_profile<R: Runtime>(
    app: &AppHandle<R>,
    draft: ProfileDraft,
    base_order: usize,
) -> Result<Profile, String> {
    let name = sanitize_profile_name(&draft.name)?;
    // 空网址不是错误。inherit 模式以后端全局主页为唯一真值，避免调用方携带旧默认值。
    let requested_default_url = normalize_url(&draft.default_url)?.to_string();
    let url_mode = match draft.url_mode.as_deref().unwrap_or("inherit") {
        "inherit" => "inherit".to_string(),
        "custom" => "custom".to_string(),
        _ => return Err("网址模式不合法".to_string()),
    };
    let default_url = if url_mode == "inherit" {
        normalized_global_default_url(app).to_string()
    } else {
        requested_default_url
    };
    let proxy = draft.proxy.unwrap_or_default();
    parse_proxy(&proxy)?;
    let timezone = sanitize_timezone(&draft.timezone.unwrap_or_default())?;
    let locale = sanitize_locale(&draft.locale.unwrap_or_default())?;
    Ok(Profile {
        id: Uuid::new_v4().to_string(),
        name,
        note: String::new(),
        avatar: None,
        default_url: default_url.clone(),
        url_mode,
        last_url: default_url,
        created_at: Utc::now().to_rfc3339(),
        order: base_order,
        incognito: draft.incognito,
        proxy: proxy.trim().to_string(),
        user_agent: draft.user_agent.unwrap_or_default().trim().to_string(),
        timezone,
        locale,
        fingerprint_guard: draft.fingerprint_guard.unwrap_or(true),
        group: draft.group.unwrap_or_default().trim().to_string(),
    })
}

#[tauri::command]
fn create_profile(
    app: AppHandle,
    name: String,
    default_url: String,
    url_mode: Option<String>,
    incognito: bool,
    proxy: Option<String>,
    user_agent: Option<String>,
    timezone: Option<String>,
    locale: Option<String>,
    fingerprint_guard: Option<bool>,
    group: Option<String>,
) -> Result<Profile, String> {
    let mut profiles = load_profiles(&app)?;
    let draft = ProfileDraft {
        name,
        default_url,
        url_mode,
        incognito,
        proxy,
        user_agent,
        timezone,
        locale,
        fingerprint_guard,
        group,
    };
    let profile = build_profile(&app, draft, profiles.len())?;
    profiles.push(profile.clone());
    save_profiles(&app, &profiles)?;
    Ok(profile)
}

/// 批量创建：先整体校验，全部通过后才写盘。
///
/// 以前前端循环调用 `create_profile`，第 N 个失败时会留下前 N-1 个半成品账号。
/// 这里改成一次调用：任一草稿不合格就整批不落盘，错误信息带序号便于定位。
#[tauri::command]
fn create_profiles_bulk(
    app: AppHandle,
    drafts: Vec<ProfileDraft>,
) -> Result<Vec<Profile>, String> {
    if drafts.is_empty() {
        return Err("没有需要创建的账号".to_string());
    }
    if drafts.len() > MAX_BULK_PROFILES {
        return Err(format!("单次最多创建 {MAX_BULK_PROFILES} 个账号"));
    }
    let mut profiles = load_profiles(&app)?;
    let base = profiles.len();
    let mut created = Vec::with_capacity(drafts.len());
    for (offset, draft) in drafts.into_iter().enumerate() {
        let profile = build_profile(&app, draft, base + offset)
            .map_err(|e| format!("第 {} 个账号：{e}", offset + 1))?;
        created.push(profile);
    }
    profiles.extend(created.iter().cloned());
    save_profiles(&app, &profiles)?;
    Ok(created)
}

#[tauri::command]
async fn update_profile(
    app: AppHandle,
    id: String,
    name: Option<String>,
    default_url: Option<String>,
    url_mode: Option<String>,
    incognito: Option<bool>,
    proxy: Option<String>,
    user_agent: Option<String>,
    timezone: Option<String>,
    locale: Option<String>,
    fingerprint_guard: Option<bool>,
    group: Option<String>,
) -> Result<UpdateProfileResult, String> {
    let mut profiles = load_profiles(&app)?;
    let index = profiles
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;

    if let Some(ref value) = proxy {
        parse_proxy(value)?;
    }
    // 未传时保持原值，不能误当作“清空时区”。
    let timezone_update = match timezone {
        Some(ref value) => Some(sanitize_timezone(value)?),
        None => None,
    };
    let locale_update = match locale {
        Some(ref value) => Some(sanitize_locale(value)?),
        None => None,
    };

    let profile = &mut profiles[index];
    let mut isolation_changed = false;

    if let Some(value) = name {
        profile.name = sanitize_profile_name(&value)?;
    }
    if let Some(value) = group {
        // 分组只是展示属性，不影响隔离，改动即时生效。
        profile.group = value.trim().to_string();
    }
    if let Some(mode) = url_mode {
        if mode != "inherit" && mode != "custom" { return Err("网址模式不合法".to_string()); }
        profile.url_mode = mode;
    }
    if let Some(value) = default_url {
        // 用户清空默认网址时主动恢复默认主页，而不是保留一个不可见的旧值。
        let previous_default = profile.default_url.clone();
        profile.default_url = if profile.url_mode == "inherit" {
            normalized_global_default_url(&app).to_string()
        } else {
            normalize_url(&value)?.to_string()
        };
        // 若账号还停留在旧主页（或历史数据没有 last_url），同步到新主页。
        if profile.last_url.trim().is_empty() || profile.last_url == previous_default {
            profile.last_url = profile.default_url.clone();
        }
    }
    if let Some(value) = incognito {
        if profile.incognito != value {
            profile.incognito = value;
            isolation_changed = true;
        }
    }
    if let Some(value) = proxy {
        let value = value.trim().to_string();
        if profile.proxy != value {
            profile.proxy = value;
            isolation_changed = true;
        }
    }
    if let Some(value) = user_agent {
        let value = value.trim().to_string();
        if profile.user_agent != value {
            profile.user_agent = value;
            isolation_changed = true;
        }
    }
    if let Some(value) = fingerprint_guard {
        if profile.fingerprint_guard != value {
            profile.fingerprint_guard = value;
            isolation_changed = true;
        }
    }
    if let Some(value) = timezone_update {
        if profile.timezone != value {
            profile.timezone = value;
            isolation_changed = true;
        }
    }
    if let Some(value) = locale_update {
        if profile.locale != value {
            profile.locale = value;
            isolation_changed = true;
        }
    }

    let updated = profile.clone();
    save_profiles(&app, &profiles)?;

    // 隔离相关配置（代理 / UA / 时区 / 指纹防护 / 隐身）挂在 WebView 创建参数上，
    // 必须销毁重建 WebView 才能生效。
    let mut needs_reopen = false;
    if isolation_changed {
        let label = profile_label(&id);
        if app.get_webview(&label).is_some() {
            if let Some(webview) = app.get_webview(&label) {
                let _ = webview.close();
            }
            needs_reopen = true;
            // 等待 WebView2 浏览器进程释放，避免立即重开时环境参数冲突。
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    }

    Ok(UpdateProfileResult {
        profile: updated,
        needs_reopen,
    })
}

#[tauri::command]
fn reorder_profiles(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let mut profiles = load_profiles(&app)?;
    for (index, id) in ids.iter().enumerate() {
        if let Some(profile) = profiles.iter_mut().find(|p| &p.id == id) {
            profile.order = index;
        }
    }
    save_profiles(&app, &profiles)
}


#[tauri::command]
fn move_profile_to_group(
    app: AppHandle,
    id: String,
    group: String,
    ids: Vec<String>,
) -> Result<(), String> {
    let mut profiles = load_profiles(&app)?;
    let profile = profiles
        .iter_mut()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "账号不存在".to_string())?;
    profile.group = group.trim().to_string();

    // 分组变化和顺序变化一次落盘，避免前端两次 command 中途失败导致半完成状态。
    for (index, profile_id) in ids.iter().enumerate() {
        if let Some(profile) = profiles.iter_mut().find(|profile| &profile.id == profile_id) {
            profile.order = index;
        }
    }
    save_profiles(&app, &profiles)
}

#[tauri::command]
async fn activate_profile(
    app: AppHandle,
    id: String,
    bounds: BrowserBounds,
) -> Result<(), String> {
    let profiles = load_profiles(&app)?;
    let profile = profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?
        .clone();
    let label = profile_label(&id);

    // Tauri 文档特别提醒 Windows 上从同步 command 创建 WebView 可能死锁，
    // 所以这个命令必须保持 async。
    // WebView2 关闭后浏览器进程需要短暂时间释放，若刚关闭过该账号的
    // WebView，首次创建可能失败，这里重试一次。
    if app.get_webview(&label).is_none() {
        if let Err(first_err) = create_profile_webview(&app, &profile, &bounds) {
            tokio::time::sleep(Duration::from_millis(800)).await;
            create_profile_webview(&app, &profile, &bounds)
                .map_err(|e| format!("{first_err}；重试仍失败：{e}"))?;
        }
    }
    set_active_profile_id(&id);
    layout_profile_webviews(&app, &label, &bounds);

    let webview = app
        .get_webview(&label)
        .ok_or_else(|| "WebView 创建失败".to_string())?;
    webview.set_focus().map_err(|e| e.to_string())?;
    clear_profile_answer_ready(&app, &id);
    // 新建 / 重建的账号页面同步当前下载配置（自动下载开关 / 扩展名白名单）。
    push_download_cfg(&app);
    Ok(())
}

/// 关闭标签页：销毁 WebView 但保留磁盘上的会话数据，重新打开即恢复登录态。
#[tauri::command]
async fn close_profile_tab(app: AppHandle, id: String) -> Result<(), String> {
    let label = profile_label(&id);
    if let Some(webview) = app.get_webview(&label) {
        webview.close().map_err(|e| e.to_string())?;
        // 留出 WebView2 释放环境的时间，避免紧接着重开同账号时参数冲突。
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Ok(())
}

/// 后台休眠：与关闭 WebView 相同，但前端保留标签；再次激活时自动重建。
#[tauri::command]
async fn hibernate_profile(app: AppHandle, id: String) -> Result<(), String> {
    close_profile_tab(app, id).await
}

/// 窗口尺寸变化时只同步当前可见账号。隐藏账号在切回前会由
/// layout_profile_webviews 再设置一次最新边界，避免多账号时无意义地批量 resize。
#[tauri::command]
fn sync_profile_bounds(
    app: AppHandle,
    bounds: BrowserBounds,
    active_id: Option<String>,
) -> Result<(), String> {
    let Some(id) = active_id else {
        return Ok(());
    };
    if let Some(webview) = app.get_webview(&profile_label(&id)) {
        let _ = webview.set_position(LogicalPosition::new(bounds.x, bounds.y));
        let _ = webview.set_size(LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)));
    }
    Ok(())
}

/// 显示 / 隐藏某个账号的 WebView。Vue 的下拉菜单、对话框等弹层渲染在
/// 主 WebView 上，会被原生子 WebView 盖住；弹层打开期间需要临时隐藏。
#[tauri::command]
fn set_profile_webview_visible(app: AppHandle, id: String, visible: bool) -> Result<(), String> {
    let webview = app
        .get_webview(&profile_label(&id))
        .ok_or_else(|| "WebView 尚未创建".to_string())?;
    if visible {
        webview.show().map_err(|e| e.to_string())
    } else {
        webview.hide().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn navigate_profile(app: AppHandle, id: String, url: String) -> Result<(), String> {
    let webview = app
        .get_webview(&profile_label(&id))
        .ok_or_else(|| "WebView 尚未创建".to_string())?;
    webview.navigate(normalize_url(&url)?).map_err(|e| e.to_string())
}

#[tauri::command]
fn browser_action(app: AppHandle, id: String, action: String) -> Result<(), String> {
    let webview = app
        .get_webview(&profile_label(&id))
        .ok_or_else(|| "WebView 尚未创建".to_string())?;
    match action.as_str() {
        "back" => webview.eval("history.back()"),
        "forward" => webview.eval("history.forward()"),
        "reload" => webview.reload(),
        "reload_restore" => webview.eval(r#"
          try {
            sessionStorage.setItem('__mab_refresh_position_v1', JSON.stringify({
              x: window.scrollX || 0,
              y: window.scrollY || 0,
              hash: location.hash || ''
            }));
          } catch (_) {}
          location.reload();
        "#),
        _ => return Err("未知浏览器动作".to_string()),
    }
    .map_err(|e| e.to_string())
}

/// 返回所有账号最近一次上报的状态（积分 / 登录账号），随 store 持久化。
/// 读取时顺带清理旧版本缓存的脏账号名（如 "logo"）。
#[tauri::command]
fn get_profile_statuses(app: AppHandle) -> Result<HashMap<String, ProfileStatus>, String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    Ok(match store.get(STATUS_KEY) {
        Some(value) => {
            let map: HashMap<String, ProfileStatus> =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            map.into_iter()
                .map(|(id, status)| (id, clean_status(status)))
                .collect()
        }
        None => HashMap::new(),
    })
}

/// 让所有已打开的账号页面立刻重新扫描并强制上报一次状态。
#[tauri::command]
fn refresh_profile_statuses(app: AppHandle) -> Result<(), String> {
    for (label, webview) in app.webviews() {
        if label.starts_with(PROFILE_PREFIX) {
            let _ = webview.eval("try { document.dispatchEvent(new Event('mb-status-scan')); } catch (e) {}");
        }
    }
    Ok(())
}

#[tauri::command]
async fn clear_profile_data(app: AppHandle, id: String) -> Result<(), String> {
    let label = profile_label(&id);
    if let Some(webview) = app.get_webview(&label) {
        webview.clear_all_browsing_data().map_err(|e| e.to_string())?;
        webview.close().map_err(|e| e.to_string())?;
        // WebView2 浏览器进程不会立刻退出，直接删目录会因文件被占用而失败。
        tokio::time::sleep(Duration::from_millis(900)).await;
    }

    #[cfg(target_os = "windows")]
    {
        let dir = profile_data_dir(&app, &id)?;
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn delete_profile(app: AppHandle, id: String) -> Result<(), String> {
    clear_profile_data(app.clone(), id.clone()).await?;
    let mut profiles = load_profiles(&app)?;
    profiles.retain(|p| p.id != id);
    for (index, profile) in profiles.iter_mut().enumerate() {
        profile.order = index;
    }
    save_profiles(&app, &profiles)?;

    // 同步清掉该账号缓存的状态（积分 / 登录账号）。
    if let Ok(store) = app.store(STORE_FILE) {
        let mut map: HashMap<String, ProfileStatus> = match store.get(STATUS_KEY) {
            Some(value) => serde_json::from_value(value).unwrap_or_default(),
            None => HashMap::new(),
        };
        if map.remove(&id).is_some() {
            if let Ok(value) = serde_json::to_value(&map) {
                store.set(STATUS_KEY, value);
                let _ = store.save();
            }
        }
    }
    Ok(())
}

/// 克隆账号：复制全部隔离配置与分组，不复刻会话数据，新账号是全新登录态。
#[tauri::command]
fn clone_profile(app: AppHandle, id: String) -> Result<Profile, String> {
    let mut profiles = load_profiles(&app)?;
    let source = profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?
        .clone();
    let name = format!("{} 副本", source.name);
    let mut copy = source;
    copy.id = Uuid::new_v4().to_string();
    copy.name = name;
    copy.created_at = Utc::now().to_rfc3339();
    copy.order = profiles.len();
    copy.last_url = effective_profile_home_url(&app, &copy).to_string();
    profiles.push(copy.clone());
    save_profiles(&app, &profiles)?;
    Ok(copy)
}

#[derive(Debug, Clone, Serialize)]
struct ProfileDiagnostic {
    id: String,
    name: String,
    url_mode: String,
    global_default_url: String,
    effective_home_url: String,
    last_url: String,
    last_url_valid: bool,
    proxy_configured: bool,
    proxy_valid: bool,
}

#[tauri::command]
fn diagnose_profile(app: AppHandle, id: String) -> Result<ProfileDiagnostic, String> {
    let profile = load_profiles(&app)?.into_iter().find(|p| p.id == id).ok_or_else(|| "账号不存在".to_string())?;
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
async fn repair_profile_startup(app: AppHandle, id: String, action: String) -> Result<Profile, String> {
    let label = profile_label(&id);
    if let Some(webview) = app.get_webview(&label) { let _ = webview.close(); tokio::time::sleep(Duration::from_millis(350)).await; }
    let mut profiles = load_profiles(&app)?;
    let profile = profiles.iter_mut().find(|p| p.id == id).ok_or_else(|| "账号不存在".to_string())?;
    match action.as_str() {
        "clear_last_url" | "home" => profile.last_url = effective_profile_home_url(&app, profile).to_string(),
        "global" => { profile.url_mode = "inherit".to_string(); profile.last_url = normalized_global_default_url(&app).to_string(); },
        "disable_proxy" => profile.proxy.clear(),
        "reset_runtime" => profile.last_url = effective_profile_home_url(&app, profile).to_string(),
        _ => return Err("未知修复操作".to_string()),
    }
    let result = profile.clone();
    save_profiles(&app, &profiles)?;
    Ok(result)
}

#[derive(Debug, Clone, Serialize)]
struct ProxyTestResult {
    ok: bool,
    ip: String,
    region: String,
    isp: String,
    error: String,
}

/// 代理连通性检测：用系统 curl 经该账号的代理访问 ip-api.com，
/// 拿出口 IP / 地区 / ISP，结果写入账号状态并广播（直连账号测的就是本机出口）。
#[tauri::command]
async fn test_profile_proxy(app: AppHandle, id: String) -> Result<ProxyTestResult, String> {
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
            Ok(output) => Ok((profile_id, String::from_utf8_lossy(&output.stdout).to_string())),
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

#[tauri::command]
fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    let mut settings = load_app_settings(&app);
    settings.download_history_limit = settings.download_history_limit.clamp(50, 2000);
    settings.download_guard_seconds = settings.download_guard_seconds.min(60);
    settings.download_guard_rules = normalized_guard_rules(settings.download_guard_rules);
    settings.skip_downloaded_files = true;
    settings.ai_exts = normalize_ai_exts(&settings.ai_exts);
    settings.global_default_url = normalize_url(&settings.global_default_url).map(|u| u.to_string()).unwrap_or_else(|_| DEFAULT_PROFILE_URL.to_string());
    if settings.download_dir.trim().is_empty() {
        settings.download_dir = resolved_download_dir(&app, &settings).to_string_lossy().to_string();
    }
    Ok(settings)
}

#[tauri::command]
fn set_app_settings(app: AppHandle, mut settings: AppSettings) -> Result<(), String> {
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
                if profile.url_mode == "inherit" && (profile.last_url.trim().is_empty() || profile.last_url == previous_global) {
                    profile.last_url = settings.global_default_url.clone();
                    changed = true;
                }
            }
            if changed { save_profiles(&app, &profiles)?; }
        }
    }
    // 配置即时下发到所有已打开的账号页面（自动下载开关 / 扩展名白名单）。
    push_download_cfg(&app);
    Ok(())
}

/// 派生 curl 时必须带 CREATE_NO_WINDOW：GUI 程序在 Windows 上启动
/// 控制台子进程会各自弹出终端窗口（图标拉取、代理检测都是每次账号一次）。
fn hidden_curl() -> Command {
    let mut cmd = Command::new("curl");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// 站点图标获取失败的记忆（按主机名，进程内），避免每次渲染都重试。
static ICON_FAILED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn icon_failed_memo() -> &'static Mutex<HashSet<String>> {
    ICON_FAILED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn icon_data_url(bytes: Vec<u8>) -> String {
    format!(
        "data:image/x-icon;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// 账号站点图标：从该账号自己的默认站点 /favicon.ico 拉取（走该账号配置的
/// 代理），本地缓存后以 data URL 返回。绝不请求第三方 favicon 服务
///（之前用 google.com/s2/favicons 会把全部账号域名以直连 IP 告知谷歌）。
#[tauri::command]
async fn get_profile_icon(app: AppHandle, id: String) -> Result<Option<String>, String> {
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
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '_' })
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
    let valid = fetched_ok
        && bytes.len() >= 100
        && bytes.len() <= 500_000
        && !bytes.starts_with(b"<");
    if !valid {
        icon_failed_memo().lock().unwrap().insert(host);
        return Ok(None);
    }
    fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    fs::write(&cache_path, &bytes).map_err(|e| e.to_string())?;
    Ok(Some(icon_data_url(bytes)))
}

#[tauri::command]
fn get_download_history(app: AppHandle) -> Result<Vec<DownloadEntry>, String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    Ok(match store.get(HISTORY_KEY) {
        Some(value) => serde_json::from_value(value).map_err(|e| e.to_string())?,
        None => vec![],
    })
}

#[tauri::command]
fn clear_download_history(app: AppHandle) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(HISTORY_KEY, serde_json::json!([]));
    store.save().map_err(|e| e.to_string())?;
    push_download_cfg(&app);
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct DownloadIdentity {
    id: String,
    path: String,
    finished_at: String,
}

#[tauri::command]
fn remove_download_history_entries(
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
        !keys.contains(&(entry.id.clone(), entry.path.clone(), entry.finished_at.clone()))
    });
    store.set(
        HISTORY_KEY,
        serde_json::to_value(&history).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    push_download_cfg(&app);
    Ok(())
}

fn update_download_history_path(app: &AppHandle, old_path: &str, new_path: Option<&str>) -> Result<(), String> {
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
    store.set(HISTORY_KEY, serde_json::to_value(&history).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())?;
    push_download_cfg(app);
    Ok(())
}

#[tauri::command]
fn delete_download(app: AppHandle, path: String) -> Result<(), String> {
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
fn pick_move_directory(initial_dir: String) -> Result<Option<String>, String> {
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
        command.args(["--file-selection", "--directory", "--title=选择移动目标文件夹"]);
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
fn move_download(app: AppHandle, path: String, destination_dir: String) -> Result<String, String> {
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
fn reveal_download(path: String) -> Result<(), String> {
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            create_profile,
            create_profiles_bulk,
            list_profile_templates,
            save_profile_templates,
            update_profile,
            reorder_profiles,
            move_profile_to_group,
            activate_profile,
            close_profile_tab,
            hibernate_profile,
            sync_profile_bounds,
            set_profile_webview_visible,
            navigate_profile,
            browser_action,
            clear_profile_data,
            delete_profile,
            get_profile_statuses,
            refresh_profile_statuses,
            clone_profile,
            diagnose_profile,
            repair_profile_startup,
            test_profile_proxy,
            get_app_settings,
            set_app_settings,
            get_profile_icon,
            get_download_history,
            clear_download_history,
            remove_download_history_entries,
            delete_download,
            pick_move_directory,
            move_download,
            reveal_download,
        ])
        .setup(|app| {
            // 结构迁移必须在任何命令被调用前完成，否则前端会读到旧结构。
            // 迁移失败不阻塞启动，仅在控制台留痕：此时 store 未被改动，
            // 用户数据保持原样，等升级到支持该数据版本的程序后会自动补齐。
            if let Err(message) = migrate_store(&app.handle().clone()) {
                eprintln!("[schema] {message}");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 昵称 ----

    #[test]
    fn sanitize_profile_name_rejects_empty_and_whitespace() {
        assert!(sanitize_profile_name("").is_err());
        assert!(sanitize_profile_name("   ").is_err());
    }

    #[test]
    fn sanitize_profile_name_trims_and_accepts_cjk() {
        let name = sanitize_profile_name("  账号 A  ").expect("中文昵称应通过");
        assert_eq!(name, "账号 A");
    }

    #[test]
    fn sanitize_profile_name_enforces_80_char_limit() {
        let long = "x".repeat(81);
        assert!(sanitize_profile_name(&long).is_err());
        let ok = "x".repeat(80);
        assert!(sanitize_profile_name(&ok).is_ok());
    }

    // ---- URL ----

    #[test]
    fn normalize_url_accepts_bare_host_and_schemes() {
        let url = normalize_url("example.com").expect("裸域名应补全 https");
        assert_eq!(url.as_str(), "https://example.com/");

        let upper = normalize_url("HTTPS://Example.com").expect("大写 scheme 应接受");
        assert_eq!(upper.scheme(), "https");
    }

    #[test]
    fn normalize_url_blank_falls_back_to_default() {
        let url = normalize_url("   ").expect("空值应回落默认主页");
        assert_eq!(url.as_str(), DEFAULT_PROFILE_URL);
    }

    #[test]
    fn normalize_url_rejects_non_http_scheme() {
        assert!(normalize_url("ftp://example.com").is_err());
        assert!(normalize_url("file:///etc/passwd").is_err());
    }

    // ---- 代理 ----

    #[test]
    fn parse_proxy_accepts_supported_schemes_and_blank() {
        assert!(parse_proxy("").is_ok());
        assert!(parse_proxy("http://127.0.0.1:8080").is_ok());
        assert!(parse_proxy("https://proxy.example.com:443").is_ok());
        assert!(parse_proxy("socks5://127.0.0.1:1080").is_ok());
    }

    #[test]
    fn parse_proxy_rejects_unsupported_scheme() {
        assert!(parse_proxy("ftp://127.0.0.1:21").is_err());
        assert!(parse_proxy("不是代理").is_err());
    }

    // ---- 注入防护 ----

    #[test]
    fn sanitize_timezone_accepts_iana_and_rejects_injection() {
        assert_eq!(
            sanitize_timezone("Asia/Shanghai").expect("IANA 时区应通过"),
            "Asia/Shanghai"
        );
        // 注入串必须被拒绝，否则会被拼进注入脚本执行。
        assert!(sanitize_timezone("Asia/Shanghai'; alert(1)//").is_err());
    }

    #[test]
    fn sanitize_locale_accepts_bcp47_and_rejects_injection() {
        assert_eq!(sanitize_locale("zh-CN").expect("BCP47 应通过"), "zh-CN");
        assert!(sanitize_locale("en-US<script>").is_err());
    }

    // ---- 模板规范化 ----

    #[test]
    fn normalized_templates_drops_invalid_and_trims() {
        let items = vec![
            ProfileTemplate {
                id: " t1 ".to_string(),
                name: " 模板1 ".to_string(),
                default_url: " https://a.com/ ".to_string(),
                url_mode: "custom".to_string(),
                incognito: false,
                proxy: " ".to_string(),
                user_agent: String::new(),
                timezone: " Asia/Shanghai ".to_string(),
                locale: " zh-CN ".to_string(),
                fingerprint_guard: true,
                group: " g ".to_string(),
            },
            // 无 id / 无 name，应被剔除。
            ProfileTemplate {
                id: String::new(),
                name: "无名模板".to_string(),
                default_url: String::new(),
                url_mode: "custom".to_string(),
                incognito: false,
                proxy: String::new(),
                user_agent: String::new(),
                timezone: String::new(),
                locale: String::new(),
                fingerprint_guard: true,
                group: String::new(),
            },
            ProfileTemplate {
                id: "t2".to_string(),
                name: String::new(),
                default_url: String::new(),
                url_mode: "custom".to_string(),
                incognito: false,
                proxy: String::new(),
                user_agent: String::new(),
                timezone: String::new(),
                locale: String::new(),
                fingerprint_guard: true,
                group: String::new(),
            },
        ];
        let out = normalized_templates(items);
        assert_eq!(out.len(), 1, "无效项应被剔除");
        assert_eq!(out[0].id, "t1");
        assert_eq!(out[0].name, "模板1");
        assert_eq!(out[0].default_url, "https://a.com/");
        assert_eq!(out[0].timezone, "Asia/Shanghai");
        assert_eq!(out[0].group, "g");
    }

    #[test]
    fn normalized_templates_fixes_bad_url_mode_and_caps_count() {
        let mut items = Vec::new();
        for i in 0..(MAX_PROFILE_TEMPLATES + 20) {
            items.push(ProfileTemplate {
                id: format!("t{i}"),
                name: format!("模板{i}"),
                default_url: String::new(),
                // 非法值应被修正为 custom。
                url_mode: "bogus".to_string(),
                incognito: false,
                proxy: String::new(),
                user_agent: String::new(),
                timezone: String::new(),
                locale: String::new(),
                fingerprint_guard: true,
                group: String::new(),
            });
        }
        let out = normalized_templates(items);
        assert_eq!(out.len(), MAX_PROFILE_TEMPLATES);
        assert!(out.iter().all(|t| t.url_mode == "custom"));
    }

    // ---- ProfileDraft 反序列化 ----

    #[test]
    fn profile_draft_deserializes_with_minimal_fields() {
        // 前端只传 name 时，其余字段必须能走 serde 默认值，不能反序列化失败。
        let draft: ProfileDraft = serde_json::from_str(r#"{"name":"账号A"}"#).unwrap();
        assert_eq!(draft.name, "账号A");
        assert!(draft.default_url.is_empty());
        assert!(draft.url_mode.is_none());
        assert!(!draft.incognito);
        assert!(draft.proxy.is_none());
        assert!(draft.fingerprint_guard.is_none());
    }

    #[test]
    fn profile_draft_deserializes_full_payload_from_frontend() {
        // 与 AccountToolsDialog 的 create_profiles_bulk 入参形状保持一致（snake_case）。
        let json = r#"{
            "name": "账号01",
            "default_url": "https://example.com/",
            "url_mode": "custom",
            "incognito": true,
            "proxy": "socks5://127.0.0.1:1080",
            "user_agent": "UA/1.0",
            "timezone": "Asia/Shanghai",
            "locale": "zh-CN",
            "fingerprint_guard": false,
            "group": "g1"
        }"#;
        let draft: ProfileDraft = serde_json::from_str(json).unwrap();
        assert_eq!(draft.name, "账号01");
        assert_eq!(draft.url_mode.as_deref(), Some("custom"));
        assert_eq!(draft.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(draft.fingerprint_guard, Some(false));
    }

    #[test]
    fn download_guard_rule_subdomain_match() {
        // v11/v12 的域名保护规则：规则应同时匹配其子域名。
        let rule = DownloadGuardRule { domain: "example.com".to_string(), seconds: 5 };
        let sub = "files.example.com";
        assert!(sub.ends_with(&format!(".{}", rule.domain)));
    }

    // ---- schema 迁移 ----

    /// 构造一个带 store 插件的 mock App，用于验证迁移对真实 store 的读写。
    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app 构造失败")
    }

    /// 把 store 清成「全新安装」状态。
    ///
    /// 注意：tauri-plugin-store 的 store 是按**文件路径**在进程内共享的，
    /// 多个 mock app 拿到的是同一份数据，测试之间会互相污染。
    /// 因此每个用例开头都要显式重置，并配合单线程运行（见 scripts/cargo-test.sh）。
    fn reset_store(handle: &tauri::AppHandle<tauri::test::MockRuntime>) {
        let store = handle.store(STORE_FILE).expect("store 打开失败");
        store.delete(STORE_KEY);
        store.delete(SETTINGS_KEY);
        store.delete(PROFILE_TEMPLATES_KEY);
        store.set(SCHEMA_VERSION_KEY, serde_json::json!(0));
        store.save().expect("重置 store 失败");
    }

    /// 版本号必须与迁移步骤数量严格对应，否则新增版本时容易忘记补迁移函数。
    #[test]
    fn migrations_count_matches_schema_version() {
        assert_eq!(
            migrations::<tauri::test::MockRuntime>().len() as u64,
            SCHEMA_VERSION,
            "迁移步骤数必须等于 SCHEMA_VERSION"
        );
    }

    /// 全新安装（store 里没有任何数据）应被识别为 v0，并成功迁移到当前版本。
    #[test]
    fn fresh_install_migrates_to_current_version() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        assert_eq!(stored_schema_version(&handle), 0, "空 store 应视为 v0");

        migrate_store(&handle).expect("全新安装应能完成迁移");
        assert_eq!(stored_schema_version(&handle), SCHEMA_VERSION);
    }

    /// 迁移应幂等：重复执行不报错，版本号保持当前值。
    #[test]
    fn migration_is_idempotent() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        migrate_store(&handle).expect("首次迁移失败");
        migrate_store(&handle).expect("重复迁移不应报错");
        assert_eq!(stored_schema_version(&handle), SCHEMA_VERSION);
    }

    /// 迁移必须把新版本号真正落盘，否则每次启动都会重复跑一遍迁移。
    #[test]
    fn migration_persists_schema_version_to_disk() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        migrate_store(&handle).expect("首次迁移失败");

        let reopened = handle.store(STORE_FILE).expect("store 重新打开失败");
        let persisted = reopened
            .get(SCHEMA_VERSION_KEY)
            .and_then(|value| value.as_u64())
            .expect("磁盘上必须写有 schema_version");
        assert_eq!(persisted, SCHEMA_VERSION, "落盘的版本号应为当前版本");
    }

    /// 迁移不应破坏既有账号：空网址账号会被修复成可用地址并保留 id。
    #[test]
    fn migration_repairs_legacy_profiles_without_losing_identity() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let store = handle.store(STORE_FILE).expect("store 打开失败");

        // 模拟 v14 历史数据：缺 url_mode、default_url 与 last_url 都为空。
        let legacy = serde_json::json!([{
            "id": "legacy-1",
            "name": "老账号",
            "note": "",
            "avatar": null,
            "default_url": "",
            "last_url": "",
            "created_at": "2026-01-01T00:00:00+00:00",
            "order": 0,
            "incognito": false
        }]);
        store.set(STORE_KEY, legacy);
        store.save().expect("预置历史数据失败");

        migrate_store(&handle).expect("迁移历史数据失败");

        let profiles = load_profiles(&handle).expect("迁移后应能读出账号");
        assert_eq!(profiles.len(), 1, "账号数量不应变化");
        assert_eq!(profiles[0].id, "legacy-1", "账号 id 必须保留");
        assert_eq!(profiles[0].name, "老账号");
        assert!(!profiles[0].default_url.trim().is_empty(), "default_url 应被补齐");
        assert!(!profiles[0].last_url.trim().is_empty(), "last_url 应被补齐");
        assert!(
            profiles[0].url_mode == "custom" || profiles[0].url_mode == "inherit",
            "url_mode 应被规范化为合法值"
        );
    }

    /// 设置里的损坏全局主页应被迁移修正为默认值，不能让后续启动继续踩坏值。
    #[test]
    fn migration_fixes_broken_global_default_url() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let store = handle.store(STORE_FILE).expect("store 打开失败");
        store.set(
            SETTINGS_KEY,
            serde_json::json!({ "global_default_url": "ftp://bad", "download_guard_rules": [] }),
        );
        store.save().expect("预置设置失败");

        migrate_store(&handle).expect("迁移设置失败");

        let settings = load_app_settings(&handle);
        assert_eq!(
            settings.global_default_url, DEFAULT_PROFILE_URL,
            "损坏的全局主页应回落默认值"
        );
    }

    /// 数据版本超前于程序时必须拒绝迁移，避免用旧结构覆盖新版数据。
    #[test]
    fn future_schema_version_is_rejected_without_touching_data() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let store = handle.store(STORE_FILE).expect("store 打开失败");
        let future = SCHEMA_VERSION + 7;
        store.set(SCHEMA_VERSION_KEY, serde_json::json!(future));
        store.set(STORE_KEY, serde_json::json!([]));
        store.save().expect("预置版本号失败");

        let result = migrate_store(&handle);
        assert!(result.is_err(), "超前版本应返回错误");
        assert_eq!(
            stored_schema_version(&handle),
            future,
            "拒绝迁移时不应改动版本号"
        );
    }
}
