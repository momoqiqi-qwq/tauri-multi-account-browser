use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

mod diagnostics;
mod downloads;
mod import_export;
mod profiles;
mod settings;
mod status;
mod store;
mod validation;
mod webviews;

// 函数按领域拆到各子模块后，这里做一次 crate 内扁平再导出：
// 文件末尾的 `mod tests` 用扁平名字调用它们，不必关心函数落在哪个文件。
// 生产代码调用子模块函数时请直接写模块路径（如 `profiles::list_profiles`），
// 这样调用点自带归属信息，重构时更容易定位。
#[allow(unused_imports)]
pub(crate) use {
    diagnostics::*, downloads::*, import_export::*, profiles::*, settings::*, status::*, store::*,
    validation::*, webviews::*,
};

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
/// - 2：账号新增 tags / favorite / last_used_at。
const SCHEMA_VERSION: u64 = 2;

const SCHEMA_VERSION_KEY: &str = "schema_version";

/// 账号模板上限：防止store无限膨胀。
const MAX_PROFILE_TEMPLATES: usize = 200;

/// 单次批量创建上限：既是性能保护，也避免误操作刷出上千个账号。
const MAX_BULK_PROFILES: usize = 200;

/// 单个账号最多打几个标签：防止标签区把账号卡片撑爆。
const MAX_PROFILE_TAGS: usize = 12;

/// 单个标签的最大字符数。
const MAX_TAG_LEN: usize = 24;

/// 诊断时统计账号数据目录占用的文件数上限。
/// 浏览数据目录动辄几万个小文件，全量遍历会让诊断对话框卡住几秒，
/// 超过上限就停止并把 `truncated` 置为 true —— 用户只需要量级，不需要精确值。
const MAX_DIAG_FILES: u64 = 20_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DownloadLedger {
    #[serde(default)]
    names: HashSet<String>,
    #[serde(default)]
    urls: HashSet<String>,
}

static ACTIVE_PROFILE_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// 账号页面回传状态用的假导航 scheme，on_navigation 里拦截并取消，
/// 从而不需要给远程页面开放任何 IPC 权限。
const STATUS_SCHEME: &str = "mbstatus";

/// 账号页要求「用独立小窗看这张图」用的假导航 scheme。
/// 站点里的图片几乎都是 `<a target="_blank" href=".../x.png">`，注入脚本会摘掉
/// target 让导航留在账号 WebView —— 结果整个聊天页被图片顶掉，用户丢会话。
/// 改成回传 mbimage:// 由宿主另开小窗，账号页的网址一动不动。
const IMAGE_SCHEME: &str = "mbimage";

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
    /// 自由标签，用于跨分组归类（如"客服""投放"）。写入前经 `normalize_tags` 规范化。
    #[serde(default)]
    tags: Vec<String>,
    /// 收藏：侧边栏可只看收藏。
    #[serde(default)]
    favorite: bool,
    /// 最近一次被打开的时间（RFC3339）。未打开过为 None。
    #[serde(default)]
    last_used_at: Option<String>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// 标签；None 表示"不改动"，Some([]) 表示清空。
    #[serde(default)]
    tags: Option<Vec<String>>,
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

fn default_true() -> bool {
    true
}

fn default_history_limit() -> usize {
    200
}

fn default_download_guard_seconds() -> u64 {
    3
}

fn default_global_profile_url() -> String {
    DEFAULT_PROFILE_URL.to_string()
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

#[derive(Debug, Serialize)]
struct UpdateProfileResult {
    profile: Profile,
    needs_reopen: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ImportOptions {
    #[serde(default = "default_conflict_strategy")]
    conflict: String,
    /// 只预检查、不写盘。用于导入前给用户看将要发生什么。
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ImportRowError {
    /// 1 起的行号，便于用户对着原文件定位。
    row: usize,
    name: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct ImportSkipped {
    row: usize,
    name: String,
    reason: String,
}

/// 导入结果。逐行成功/失败都能反馈给用户，而不是只丢一句"导入失败"。
#[derive(Debug, Clone, Serialize)]
struct ImportReport {
    created: Vec<Profile>,
    updated: Vec<Profile>,
    skipped: Vec<ImportSkipped>,
    errors: Vec<ImportRowError>,
}

/// CSV 账号行。列名与导出一致；除 name 外都可为空。
#[derive(Debug, Clone, Deserialize)]
struct CsvAccountRow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    default_url: String,
    #[serde(default)]
    url_mode: String,
    #[serde(default)]
    incognito: String,
    #[serde(default)]
    proxy: String,
    #[serde(default)]
    user_agent: String,
    #[serde(default)]
    timezone: String,
    #[serde(default)]
    locale: String,
    #[serde(default)]
    fingerprint_guard: String,
    #[serde(default)]
    group: String,
    /// 多个标签用 `|` 分隔（标签本身可能含逗号，用逗号分隔会和 CSV 语义混淆）。
    #[serde(default)]
    tags: String,
}

/// CSV 列顺序。**只许在末尾追加**：历史导出的 CSV 靠位置解析，
/// 缺 trailing 列时 serde default 会兜底，旧文件仍可导入。
const CSV_HEADER: [&str; 11] = [
    "name",
    "default_url",
    "url_mode",
    "incognito",
    "proxy",
    "user_agent",
    "timezone",
    "locale",
    "fingerprint_guard",
    "group",
    "tags",
];

/// CSV 里多个标签的分隔符。
const CSV_TAG_SEPARATOR: char = '|';

/// 批量修改账号的补丁。全是 Option：None 表示"这一项不改"。
///
/// 之所以做成一个 patch 而不是给每个操作开一个命令，是因为侧边栏的批量操作条
/// 会同时勾好几项（比如"改主页模式 + 打标签 + 收藏"），
/// 拆成多个命令就要写盘多次，中间失败还会留下半改状态。
#[derive(Debug, Clone, Default, Deserialize)]
struct ProfileBatchPatch {
    /// inherit / custom
    url_mode: Option<String>,
    /// 只有 url_mode = custom 时才有意义；空串表示不改
    default_url: Option<String>,
    /// 追加标签（已存在则忽略）
    tags_add: Option<Vec<String>>,
    /// 删除标签，大小写不敏感
    tags_remove: Option<Vec<String>>,
    /// 覆盖标签；Some([]) 表示清空
    tags_set: Option<Vec<String>>,
    favorite: Option<bool>,
    group: Option<String>,
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
    /// 打开失败原因的归类；诊断对话框据此显示针对性建议
    code: DiagnosticCode,
    /// 给用户的修复建议
    hint: String,
    /// WebView2 Runtime 检测结果（全局，但与"打不开"强相关，一起返回）
    runtime: WebView2Status,
    /// 该账号数据目录的占用
    data_dir: DataDirUsage,
}

/// 诊断错误码：把后端抛出的原始错误串归类，前端据此给出针对性修复建议，
/// 而不是让用户对着一长串英文堆栈猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticCode {
    /// 未能归类
    Unknown,
    /// 系统缺少 WebView2 Runtime，或版本过旧 / 安装损坏
    Webview2Runtime,
    /// 账号数据目录不可读写：权限不足、被其它进程占用、磁盘已满
    DataDir,
    /// 代理格式错误或不可达
    Proxy,
    /// 目标网址不可达 / 超时 / DNS 失败
    Network,
    /// 网址格式非法（非 http/https 等）
    InvalidUrl,
    /// 上次访问的网址指向了坏页面
    BadLastUrl,
}

impl DiagnosticCode {
    /// 一句话修复建议，直接显示在诊断对话框顶部。
    fn hint(self) -> &'static str {
        match self {
            DiagnosticCode::Webview2Runtime => {
                "系统缺少 WebView2 Runtime 或安装已损坏，请安装/修复 Microsoft Edge WebView2 Runtime 后重启本应用。"
            }
            DiagnosticCode::DataDir => {
                "账号数据目录无法读写：请确认磁盘未满、目录未被安全软件锁定，必要时关闭占用进程后重试。"
            }
            DiagnosticCode::Proxy => {
                "代理不可达或格式错误：请检查代理地址与端口，或先禁用代理确认直连是否正常。"
            }
            DiagnosticCode::Network => {
                "目标网址无法访问：请检查网络连通性、DNS 与目标站点状态。"
            }
            DiagnosticCode::InvalidUrl => {
                "网址格式不合法：请改回 http/https 开头的完整网址。"
            }
            DiagnosticCode::BadLastUrl => {
                "上次访问的网址可能导致启动失败，清除后回到主页重试即可。"
            }
            DiagnosticCode::Unknown => "未能自动定位原因，可先尝试下方的修复操作，或查看控制台日志。",
        }
    }
}

/// WebView2 Runtime 检测结果。缺了它一个标签页都开不出来，
/// 所以这是"打开失败"时第一件要确认的事。
#[derive(Debug, Clone, Serialize)]
struct WebView2Status {
    installed: bool,
    /// 形如 131.0.2903.86；检测不到时为空串
    version: String,
    /// 版本来源：registry-hklm / registry-hkcu / filesystem / none
    source: String,
}

/// 账号数据目录占用。目录可能有好几 GB，统计时设文件数上限，
/// 避免打开诊断时把界面卡住。
#[derive(Debug, Clone, Serialize)]
struct DataDirUsage {
    exists: bool,
    path: String,
    bytes: u64,
    file_count: u64,
    /// 为 true 表示文件数超过统计上限，bytes / file_count 只是部分结果
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ProxyTestResult {
    ok: bool,
    ip: String,
    region: String,
    isp: String,
    error: String,
    /// 本次检测往返耗时（毫秒）。既能反映代理质量，也能区分"连不上"和"太慢"。
    latency_ms: u64,
    /// 请求是否真的走了代理；直连账号为 false
    via_proxy: bool,
    /// 实际使用的代理地址，直连时为空串
    endpoint: String,
}

/// 站点图标获取失败的记忆（按主机名，进程内），避免每次渲染都重试。
static ICON_FAILED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
struct DownloadIdentity {
    id: String,
    path: String,
    finished_at: String,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            profiles::list_profiles,
            profiles::create_profile,
            profiles::create_profiles_bulk,
            profiles::list_profile_templates,
            profiles::save_profile_templates,
            import_export::import_profiles,
            import_export::parse_accounts_csv,
            import_export::build_accounts_csv,
            import_export::pick_import_file,
            import_export::pick_export_file,
            import_export::read_import_file,
            import_export::write_export_file,
            profiles::update_profile,
            profiles::update_profile_batch,
            profiles::reorder_profiles,
            profiles::move_profile_to_group,
            profiles::activate_profile,
            profiles::close_profile_tab,
            profiles::hibernate_profile,
            profiles::sync_profile_bounds,
            profiles::set_profile_webview_visible,
            profiles::navigate_profile,
            profiles::browser_action,
            profiles::clear_profile_data,
            profiles::delete_profile,
            status::get_profile_statuses,
            status::refresh_profile_statuses,
            profiles::clone_profile,
            diagnostics::diagnose_profile,
            diagnostics::open_profile_data_dir,
            diagnostics::repair_profile_startup,
            diagnostics::test_profile_proxy,
            settings::get_app_settings,
            settings::set_app_settings,
            downloads::get_profile_icon,
            downloads::get_download_history,
            downloads::clear_download_history,
            downloads::remove_download_history_entries,
            downloads::delete_download,
            downloads::pick_move_directory,
            downloads::move_download,
            downloads::reveal_download,
        ])
        .setup(|app| {
            // 结构迁移必须在任何命令被调用前完成，否则前端会读到旧结构。
            // 迁移失败不阻塞启动，仅在控制台留痕：此时 store 未被改动，
            // 用户数据保持原样，等升级到支持该数据版本的程序后会自动补齐。
            if let Err(message) = store::migrate_store(&app.handle().clone()) {
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
    // 只有测试会直接开 store 预置数据，生产代码都走 store 模块的封装，
    // 所以这个 trait 导入放在这里，避免非 test 编译时报未使用。
    use tauri_plugin_store::StoreExt;
    use uuid::Uuid;

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

    // ---- CSV 与导入 ----

    #[test]
    fn csv_roundtrip_preserves_commas_and_quotes() {
        // 昵称里带逗号/引号是最容易写错的地方，必须能被正确转义再读回来。
        let drafts = vec![
            ProfileDraft {
                name: "账号, 带逗号".to_string(),
                default_url: "https://a.com/".to_string(),
                url_mode: Some("custom".to_string()),
                incognito: true,
                proxy: Some(String::new()),
                user_agent: Some(String::new()),
                timezone: Some("Asia/Shanghai".to_string()),
                locale: Some("zh-CN".to_string()),
                fingerprint_guard: Some(false),
                group: Some("g1".to_string()),
                tags: Some(vec!["客服".to_string(), "VIP".to_string()]),
            },
            ProfileDraft {
                name: "引号\"账号".to_string(),
                default_url: String::new(),
                url_mode: None,
                incognito: false,
                proxy: None,
                user_agent: None,
                timezone: None,
                locale: None,
                fingerprint_guard: None,
                group: None,
                tags: None,
            },
        ];
        let csv = build_csv_accounts(&drafts).expect("序列化 CSV 失败");
        let parsed = parse_csv_accounts(&csv).expect("解析 CSV 失败");
        assert_eq!(parsed.len(), 2, "往返后行数应一致");
        assert_eq!(parsed[0].name, "账号, 带逗号");
        assert_eq!(parsed[0].default_url, "https://a.com/");
        assert_eq!(parsed[0].url_mode.as_deref(), Some("custom"));
        assert!(parsed[0].incognito);
        assert_eq!(parsed[0].fingerprint_guard, Some(false));
        assert_eq!(parsed[0].group.as_deref(), Some("g1"));
        // 标签用 | 分隔，往返后应还原成原列表
        assert_eq!(
            parsed[0].tags.as_deref(),
            Some([("客服".to_string()), ("VIP".to_string())].as_slice())
        );
        assert_eq!(parsed[1].name, "引号\"账号");
        assert_eq!(parsed[1].fingerprint_guard, None, "空单元格应保持未设置");
        assert_eq!(parsed[1].tags, None, "空标签列应保持未设置");
    }

    #[test]
    fn csv_parse_tolerates_old_exports_without_tags_column() {
        // 旧版导出只有 10 列；新解析靠 serde default 兜底，不应报错。
        let text = "name,default_url,url_mode,incognito,proxy,user_agent,timezone,locale,fingerprint_guard,group\n账号A,https://a.com/,inherit,false,,,Asia/Shanghai,zh-CN,true,g1\n";
        let parsed = parse_csv_accounts(text).expect("旧 CSV 应仍能导入");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "账号A");
        assert_eq!(parsed[0].tags, None);
    }

    #[test]
    fn csv_parse_skips_header_and_blank_lines() {
        let text =
            "name,default_url,url_mode\n账号A,https://a.com/,inherit\n\n   \n账号B,,custom\n";
        let parsed = parse_csv_accounts(text).expect("解析失败");
        assert_eq!(parsed.len(), 2, "表头与空行都应被跳过");
        assert_eq!(parsed[0].name, "账号A");
        assert_eq!(parsed[1].name, "账号B");
    }

    #[test]
    fn csv_parse_accepts_headerless_input() {
        // 不带表头的 CSV 也应能读（首行不以 name/default_url 组合开头就是数据）。
        let text = "账号A,https://a.com/,inherit\n";
        let parsed = parse_csv_accounts(text).expect("解析失败");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "账号A");
    }

    #[test]
    fn unique_profile_name_appends_suffix_until_free() {
        let mut taken = HashSet::new();
        taken.insert("账号".to_string());
        taken.insert("账号 (2)".to_string());
        assert_eq!(unique_profile_name("账号", &taken), "账号 (3)");
        assert_eq!(unique_profile_name("没人用", &taken), "没人用");
    }

    #[test]
    fn import_reports_bad_rows_and_imports_the_rest() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);

        let drafts = vec![
            ProfileDraft {
                name: "好账号".to_string(),
                ..Default::default()
            },
            // 空昵称应被逐行记错，而不是让整批失败。
            ProfileDraft {
                name: "   ".to_string(),
                ..Default::default()
            },
            // 非法网址同样只影响这一行。
            ProfileDraft {
                name: "坏网址".to_string(),
                default_url: "ftp://bad".to_string(),
                url_mode: Some("custom".to_string()),
                ..Default::default()
            },
        ];
        let report = import_profiles_impl(&handle, drafts, "rename".to_string(), false)
            .expect("导入不应整体失败");
        assert_eq!(report.created.len(), 1, "只应导入那一行好的");
        assert_eq!(report.created[0].name, "好账号");
        assert_eq!(report.errors.len(), 2, "两行坏数据应逐行报出");
        assert_eq!(report.errors[0].row, 2);
        assert_eq!(report.errors[1].row, 3);
    }

    #[test]
    fn import_conflict_strategies_behave_as_documented() {
        let app = mock_app();
        let handle = app.handle().clone();

        // rename：同名自动改名，双方都保留。
        reset_store(&handle);
        seed_profile(&handle, "同名");
        let report = import_profiles_impl(
            &handle,
            vec![ProfileDraft {
                name: "同名".to_string(),
                ..Default::default()
            }],
            "rename".to_string(),
            false,
        )
        .unwrap();
        assert_eq!(report.created.len(), 1);
        assert_eq!(report.created[0].name, "同名 (2)");
        assert!(load_profiles(&handle)
            .unwrap()
            .iter()
            .any(|p| p.name == "同名"));

        // skip：跳过，不新增。
        reset_store(&handle);
        seed_profile(&handle, "同名");
        let report = import_profiles_impl(
            &handle,
            vec![ProfileDraft {
                name: "同名".to_string(),
                ..Default::default()
            }],
            "skip".to_string(),
            false,
        )
        .unwrap();
        assert_eq!(report.created.len(), 0);
        assert_eq!(report.skipped.len(), 1);

        // overwrite：覆盖同名，账号总数不变，且 id 保留。
        reset_store(&handle);
        let original_id = seed_profile(&handle, "同名");
        let report = import_profiles_impl(
            &handle,
            vec![ProfileDraft {
                name: "同名".to_string(),
                timezone: Some("Asia/Tokyo".to_string()),
                ..Default::default()
            }],
            "overwrite".to_string(),
            false,
        )
        .unwrap();
        assert_eq!(report.updated.len(), 1);
        let profiles = load_profiles(&handle).unwrap();
        assert_eq!(profiles.len(), 1, "覆盖不应新增账号");
        assert_eq!(profiles[0].id, original_id, "覆盖应保留原 id");
        assert_eq!(profiles[0].timezone, "Asia/Tokyo", "字段应被更新");
    }

    #[test]
    fn import_dry_run_does_not_persist() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let report = import_profiles_impl(
            &handle,
            vec![ProfileDraft {
                name: "试试".to_string(),
                ..Default::default()
            }],
            "rename".to_string(),
            true,
        )
        .unwrap();
        assert_eq!(report.created.len(), 1, "预检查应报告将要创建 1 个");
        assert!(
            load_profiles(&handle).unwrap().is_empty(),
            "dry_run 不应写盘"
        );
    }

    #[test]
    fn import_rejects_unknown_conflict_strategy() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let result = import_profiles_impl(
            &handle,
            vec![ProfileDraft {
                name: "x".to_string(),
                ..Default::default()
            }],
            "bogus".to_string(),
            false,
        );
        assert!(result.is_err(), "未知策略应直接报错");
    }

    #[test]
    fn download_guard_rule_subdomain_match() {
        // v11/v12 的域名保护规则：规则应同时匹配其子域名。
        let rule = DownloadGuardRule {
            domain: "example.com".to_string(),
            seconds: 5,
        };
        let sub = "files.example.com";
        assert!(sub.ends_with(&format!(".{}", rule.domain)));
    }

    // ---- 标签与批量修改 ----

    #[test]
    fn normalize_tags_trims_dedupes_and_caps() {
        let tags = normalize_tags(&[
            "  VIP  ".to_string(),
            "vip".to_string(),
            String::new(),
            "   ".to_string(),
            "客服".to_string(),
            "VIP".to_string(),
        ]);
        // 去空白 + 大小写不敏感去重 + 保序
        assert_eq!(tags, vec!["VIP".to_string(), "客服".to_string()]);
    }

    #[test]
    fn normalize_tags_caps_length_and_count() {
        let long = "x".repeat(MAX_TAG_LEN + 10);
        let tags = normalize_tags(&[long]);
        assert_eq!(tags[0].len(), MAX_TAG_LEN, "超长标签应被截断");

        let many: Vec<String> = (0..MAX_PROFILE_TAGS + 5).map(|i| format!("t{i}")).collect();
        assert_eq!(
            normalize_tags(&many).len(),
            MAX_PROFILE_TAGS,
            "标签数量应有上限"
        );
    }

    #[test]
    fn update_profile_batch_is_atomic_and_applies_patch() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let seeded = seed_profiles(&handle, &["A", "B"]);
        let first = seeded[0].clone();
        let second = seeded[1].clone();

        // ---- 整批成功 ----
        let updated = update_profile_batch_impl(
            &handle,
            vec![first.clone(), second.clone()],
            ProfileBatchPatch {
                url_mode: Some("custom".to_string()),
                default_url: Some("https://example.com/".to_string()),
                tags_add: Some(vec!["VIP".to_string(), "vip".to_string()]),
                favorite: Some(true),
                ..Default::default()
            },
        )
        .expect("批量修改应成功");
        assert_eq!(updated.len(), 2);
        let reloaded = load_profiles(&handle).expect("重新读取失败");
        for id in [&first, &second] {
            let profile = reloaded.iter().find(|p| &p.id == id).expect("账号应存在");
            assert_eq!(profile.url_mode, "custom");
            assert_eq!(profile.default_url, "https://example.com/");
            // 追加两个同名（大小写不同）标签应只留一个
            assert_eq!(profile.tags, vec!["VIP".to_string()]);
            assert!(profile.favorite);
            assert_eq!(profile.last_url, "https://example.com/");
        }

        // ---- 非法输入：整批不改 ----
        let before = load_profiles(&handle).expect("读取失败");
        let err = update_profile_batch_impl(
            &handle,
            vec![first.clone(), second.clone()],
            ProfileBatchPatch {
                url_mode: Some("bogus".to_string()),
                ..Default::default()
            },
        )
        .expect_err("非法 url_mode 应报错");
        assert!(err.contains("网址模式不合法"));
        let after = load_profiles(&handle).expect("读取失败");
        assert_eq!(
            before
                .iter()
                .map(|p| p.url_mode.as_str())
                .collect::<Vec<_>>(),
            after
                .iter()
                .map(|p| p.url_mode.as_str())
                .collect::<Vec<_>>(),
            "校验失败时不应改动任何账号"
        );

        // ---- 部分 id 不存在：同样整批不改 ----
        let before = load_profiles(&handle).expect("读取失败");
        assert!(
            update_profile_batch_impl(
                &handle,
                vec![first.clone(), "missing".to_string()],
                ProfileBatchPatch {
                    favorite: Some(false),
                    ..Default::default()
                },
            )
            .is_err(),
            "含不存在的 id 应整批失败"
        );
        let after = load_profiles(&handle).expect("读取失败");
        assert_eq!(
            before.iter().map(|p| p.favorite).collect::<Vec<_>>(),
            after.iter().map(|p| p.favorite).collect::<Vec<_>>()
        );

        // ---- 删除标签 / 清空标签 ----
        update_profile_batch_impl(
            &handle,
            vec![first.clone()],
            ProfileBatchPatch {
                tags_remove: Some(vec!["vip".to_string()]),
                ..Default::default()
            },
        )
        .expect("删除标签应成功");
        let reloaded = load_profiles(&handle).expect("读取失败");
        let profile = reloaded.iter().find(|p| p.id == first).expect("账号应存在");
        assert!(profile.tags.is_empty(), "大小写不敏感删除");
    }

    #[test]
    fn update_profile_batch_inherit_mode_falls_back_to_global_home() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        let seeded = seed_profiles(&handle, &["A", "B"]);

        // 先切成独立网址，才能验证切回 inherit 时确实回落到全局主页。
        update_profile_batch_impl(
            &handle,
            seeded.clone(),
            ProfileBatchPatch {
                url_mode: Some("custom".to_string()),
                default_url: Some("https://example.org/".to_string()),
                ..Default::default()
            },
        )
        .expect("改成独立网址应成功");

        // 只给 url_mode 不给网址：应回落到全局主页，而不是留着旧的 custom 值。
        update_profile_batch_impl(
            &handle,
            seeded.clone(),
            ProfileBatchPatch {
                url_mode: Some("inherit".to_string()),
                ..Default::default()
            },
        )
        .expect("切回继承应成功");

        let reloaded = load_profiles(&handle).expect("读取失败");
        for id in &seeded {
            let profile = reloaded.iter().find(|p| &p.id == id).expect("账号应存在");
            assert_eq!(profile.url_mode, "inherit");
            assert_eq!(profile.default_url, DEFAULT_PROFILE_URL);
            // 原本停在旧主页上的账号应同步到新主页
            assert_eq!(profile.last_url, DEFAULT_PROFILE_URL);
        }
    }

    #[test]
    fn tags_survive_json_roundtrip_on_profile() {
        // 历史数据没有 tags 字段：靠 serde default 兜底，不应反序列化失败。
        let json = r#"{"id":"p1","name":"A","note":"","created_at":"2026-01-01T00:00:00Z","order":0,"incognito":false}"#;
        let profile: Profile = serde_json::from_str(json).expect("缺字段应走 serde default");
        assert!(profile.tags.is_empty());
        assert!(!profile.favorite);
        assert!(profile.last_used_at.is_none());
    }

    // ---- 诊断：错误码分类 ----

    #[test]
    fn classify_error_maps_webview2_failures() {
        assert_eq!(
            classify_error("WebView2 Runtime 未安装"),
            DiagnosticCode::Webview2Runtime
        );
        assert_eq!(
            classify_error("failed to create webview: 0x80070002"),
            DiagnosticCode::Webview2Runtime
        );
    }

    #[test]
    fn classify_error_prefers_proxy_over_network() {
        // 代理错误里几乎必然带 "connection"，但归类必须是代理，
        // 否则用户会被误导去查网络。
        assert_eq!(
            classify_error("proxy connection refused"),
            DiagnosticCode::Proxy
        );
        assert_eq!(
            classify_error("socks5 handshake failed"),
            DiagnosticCode::Proxy
        );
        assert_eq!(classify_error("代理不可达"), DiagnosticCode::Proxy);
    }

    #[test]
    fn classify_error_maps_io_and_network_failures() {
        assert_eq!(
            classify_error("Permission denied (os error 5)"),
            DiagnosticCode::DataDir
        );
        // os error 32 = 文件被占用，Win 上删/改账号数据目录时常见
        assert_eq!(
            classify_error("文件被占用 (os error 32)"),
            DiagnosticCode::DataDir
        );
        assert_eq!(
            classify_error("request timed out after 30s"),
            DiagnosticCode::Network
        );
        assert_eq!(classify_error("DNS 解析失败"), DiagnosticCode::Network);
    }

    #[test]
    fn classify_error_maps_url_failures_and_unknown() {
        assert_eq!(
            classify_error("invalid url scheme: ftp"),
            DiagnosticCode::InvalidUrl
        );
        assert_eq!(
            classify_error("last_url 指向了坏页面"),
            DiagnosticCode::BadLastUrl
        );
        assert_eq!(classify_error("账号不存在"), DiagnosticCode::Unknown);
        assert_eq!(classify_error(""), DiagnosticCode::Unknown);
    }

    #[test]
    fn every_diagnostic_code_has_a_non_empty_hint() {
        // 每个错误码都得有建议，否则前端会显示一个空白的提示条。
        for code in [
            DiagnosticCode::Unknown,
            DiagnosticCode::Webview2Runtime,
            DiagnosticCode::DataDir,
            DiagnosticCode::Proxy,
            DiagnosticCode::Network,
            DiagnosticCode::InvalidUrl,
            DiagnosticCode::BadLastUrl,
        ] {
            assert!(!code.hint().is_empty(), "{code:?} 缺少修复建议");
        }
    }

    #[test]
    fn parse_reg_pv_reads_version_and_rejects_garbage() {
        let output = "\r\n    pv    REG_SZ    131.0.2903.86\r\n\r\n";
        assert_eq!(parse_reg_pv(output).as_deref(), Some("131.0.2903.86"));
        assert_eq!(parse_reg_pv("    pv    REG_SZ    ").as_deref(), None);
        assert_eq!(parse_reg_pv("错误: 系统找不到指定的注册表项"), None);
    }

    #[test]
    fn measure_dir_sums_files_and_stops_at_limit() {
        let root = std::env::temp_dir().join(format!("mab-diag-{}", Uuid::new_v4()));
        let nested = root.join("Default").join("Cache");
        std::fs::create_dir_all(&nested).expect("建临时目录失败");
        std::fs::write(root.join("a.bin"), vec![0u8; 100]).expect("写文件失败");
        std::fs::write(nested.join("b.bin"), vec![0u8; 50]).expect("写文件失败");

        let usage = measure_dir(&root, 1000);
        assert!(usage.exists);
        assert_eq!(usage.file_count, 2);
        assert_eq!(usage.bytes, 150);
        assert!(!usage.truncated);

        // 上限设成 1：只统计到第一个文件就应该停，并标记 truncated。
        let limited = measure_dir(&root, 1);
        assert!(limited.truncated);
        assert_eq!(limited.file_count, 1);

        let missing = measure_dir(&root.join("does-not-exist"), 1000);
        assert!(!missing.exists);
        assert_eq!(missing.file_count, 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- schema 迁移 ----

    /// 构造一个带 store 插件的 mock App，用于验证迁移对真实 store 的读写。
    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app 构造失败")
    }

    /// 往 store 里放若干已有账号，用于测试冲突 / 批量逻辑。返回它们的 id。
    ///
    /// 注意 `save_profiles` 是**整表覆盖**，所以多个账号必须一次性写完，
    /// 不能循环调用 —— 后一次会把前一次的结果抹掉。
    fn seed_profiles(
        handle: &tauri::AppHandle<tauri::test::MockRuntime>,
        names: &[&str],
    ) -> Vec<String> {
        let profiles: Vec<Profile> = names
            .iter()
            .enumerate()
            .map(|(index, name)| Profile {
                id: format!("seed-{name}"),
                name: name.to_string(),
                note: String::new(),
                avatar: None,
                default_url: DEFAULT_PROFILE_URL.to_string(),
                url_mode: "inherit".to_string(),
                last_url: DEFAULT_PROFILE_URL.to_string(),
                created_at: "2026-01-01T00:00:00+00:00".to_string(),
                order: index,
                incognito: false,
                proxy: String::new(),
                user_agent: String::new(),
                timezone: String::new(),
                locale: String::new(),
                fingerprint_guard: true,
                group: String::new(),
                tags: Vec::new(),
                favorite: false,
                last_used_at: None,
            })
            .collect();
        let ids = profiles.iter().map(|p| p.id.clone()).collect();
        save_profiles(handle, &profiles).expect("预置账号失败");
        ids
    }

    /// 往 store 里放一个已有账号，用于测试导入时的冲突处理。返回其 id。
    fn seed_profile(handle: &tauri::AppHandle<tauri::test::MockRuntime>, name: &str) -> String {
        seed_profiles(handle, &[name])
            .pop()
            .expect("预置账号应返回 id")
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
        store.delete(STATUS_KEY);
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
        assert!(
            !profiles[0].default_url.trim().is_empty(),
            "default_url 应被补齐"
        );
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

    // ---- answer_finished_at 持久化（v24） ----

    fn load_status(
        handle: &tauri::AppHandle<tauri::test::MockRuntime>,
        id: &str,
    ) -> Option<ProfileStatus> {
        let store = handle.store(STORE_FILE).expect("store 打开失败");
        let value = store.get(STATUS_KEY)?;
        let map: std::collections::HashMap<String, ProfileStatus> =
            serde_json::from_value(value).ok()?;
        map.get(id).cloned()
    }

    #[test]
    fn answer_finished_at_is_set_on_generating_to_idle_edge() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: true,
                ..Default::default()
            },
        );
        assert!(
            load_status(&handle, "A")
                .unwrap()
                .answer_finished_at
                .is_none(),
            "生成中状态本身不应记时间"
        );

        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                ..Default::default()
            },
        );
        assert!(
            load_status(&handle, "A")
                .unwrap()
                .answer_finished_at
                .is_some(),
            "generating 下降沿应记下完成时间"
        );
    }

    #[test]
    fn clear_profile_answer_ready_preserves_finished_at() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        // 先 generating=true 再下降沿，记下时间。
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: true,
                ..Default::default()
            },
        );
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                answer_ready: true,
                ..Default::default()
            },
        );
        let first = load_status(&handle, "A").unwrap().answer_finished_at;
        assert!(first.is_some(), "前置：下降沿后应有 finished_at");

        clear_profile_answer_ready(&handle, "A");
        let after = load_status(&handle, "A").expect("已读不应删除整条状态");
        assert!(!after.answer_ready, "未读提醒应被清掉");
        assert_eq!(
            after.answer_finished_at, first,
            "已读不应清掉 AI 使用时间"
        );
    }

    #[test]
    fn answer_finished_at_does_not_drift_on_repeated_idle() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: true,
                ..Default::default()
            },
        );
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                ..Default::default()
            },
        );
        let first = load_status(&handle, "A").unwrap().answer_finished_at.unwrap();

        // 反复上报 idle：没有新的下降沿，时间戳不应再被刷新。
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                ..Default::default()
            },
        );
        let second = load_status(&handle, "A").unwrap().answer_finished_at.unwrap();

        assert_eq!(
            first, second,
            "连续两次 generating=false 不应把时间戳往后推"
        );
    }

    #[test]
    fn answer_finished_at_updates_on_subsequent_generating_edge() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: true,
                ..Default::default()
            },
        );
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                ..Default::default()
            },
        );
        let first = load_status(&handle, "A").unwrap().answer_finished_at.unwrap();

        // AI 重新开始：generating=true。
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: true,
                ..Default::default()
            },
        );
        let during = load_status(&handle, "A").unwrap().answer_finished_at;
        assert_eq!(
            during,
            Some(first.clone()),
            "生成中应沿用旧的完成时间"
        );

        // AI 又答完一次：新的下降沿，应覆盖为新时间。
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                ..Default::default()
            },
        );
        let second = load_status(&handle, "A").unwrap().answer_finished_at.unwrap();
        assert_ne!(first, second, "新一轮「刚刚答完」应覆盖为新时间");
    }

    /// v24 修正后的关键回归点：扫描脚本只在后台账号上把 answer_ready 翻 true
    ///（见 webviews.rs `answerReady = !profileActive`），当前正在看的账号
    /// ready 永远不翻。如果还以 ready 边沿为信号，正看的账号的时间就永远不会被更新。
    /// 这条测试用纯 generating 序列模拟「当前正在看的账号」，验证下降沿一样能记时间。
    #[test]
    fn answer_finished_at_updates_for_active_profile_via_generating_edge() {
        let app = mock_app();
        let handle = app.handle().clone();
        reset_store(&handle);
        // 当前正在看的账号：扫描脚本不会把 answer_ready 翻 true。
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: true,
                answer_ready: false,
                ..Default::default()
            },
        );
        assert!(
            load_status(&handle, "A")
                .unwrap()
                .answer_finished_at
                .is_none()
        );

        std::thread::sleep(std::time::Duration::from_millis(3));
        save_profile_status(
            &handle,
            "A",
            ProfileStatus {
                answer_generating: false,
                answer_ready: false,
                ..Default::default()
            },
        );
        assert!(
            load_status(&handle, "A")
                .unwrap()
                .answer_finished_at
                .is_some(),
            "对当前正在看的账号，generating 下降沿同样要记时间"
        );
    }

    // ---- 图片预览小窗（v25） ----

    /// 假导航的解析必须和注入脚本拼出来的完全对上：scheme / host / 编码后的图片地址。
    #[test]
    fn image_preview_navigation_decodes_target_and_hint() {
        let target = "https://files.chat01.ai/a/b/c/图.png?sign=abc%2Bdef";
        let encode = |s: &str| {
            url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>()
        };
        let href = format!(
            "{}://open?u={}&t={}",
            IMAGE_SCHEME,
            encode(target),
            encode("上传的图片")
        );

        let url = url::Url::parse(&href).expect("假导航应能解析");
        assert_eq!(url.scheme(), IMAGE_SCHEME);
        assert_eq!(url.host_str(), Some("open"), "宿主靠 host=open 分流");

        let query = query_map(&url);
        assert_eq!(query.get("u").map(String::as_str), Some(target));
        assert_eq!(query.get("t").map(String::as_str), Some("上传的图片"));
    }

    /// 小窗标题就是图片路径：path 段要可读（百分号解码），查询串要原样保留。
    #[test]
    fn image_window_title_shows_decoded_path_with_query() {
        let title = image_window_title("https://files.chat01.ai/upload/%E5%9B%BE%E7%89%87.png?v=2", "");
        assert_eq!(title, "https://files.chat01.ai/upload/图片.png?v=2");
    }

    /// blob: 没有可读路径，退回链接上的提示（alt / title）当标题。
    #[test]
    fn image_window_title_falls_back_to_hint_for_blob() {
        assert_eq!(
            image_window_title("blob:https://chat01.ai/x-y-z", ""),
            "blob 预览图"
        );
        assert_eq!(
            image_window_title("blob:https://chat01.ai/x-y-z", "  上传的图片  "),
            "上传的图片"
        );
    }

    /// 超长路径保留尾部：文件名和辨识度最高的段都在后面，截掉尾部等于没信息。
    #[test]
    fn clip_tail_keeps_the_tail_and_marks_truncation() {
        assert_eq!(clip_tail("short", 10), "short");
        let clipped = clip_tail(&"a".repeat(200), 10);
        assert_eq!(clipped.chars().count(), 11, "省略号 + 10 个字符");
        assert!(clipped.starts_with('…'));
    }

    /// 标题只做展示，百分号解码要能容错：非法序列、截断的 % 都不能 panic。
    #[test]
    fn percent_decode_handles_valid_and_broken_escapes() {
        assert_eq!(percent_decode("%E5%9B%BE%20a.png"), "图 a.png");
        assert_eq!(percent_decode("a%2Bb"), "a+b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("bad%ZZtail"), "bad%ZZtail");
        assert_eq!(percent_decode("中%41"), "中A");
    }

    /// 注入脚本和宿主必须对齐同一个假导航 scheme；改一边忘另一边就静默失效。
    #[test]
    fn image_preview_script_and_host_agree_on_contract() {
        assert!(
            NEW_WINDOW_PATCH_JS.contains(&format!("{IMAGE_SCHEME}://open")),
            "点击拦截脚本要拼 {IMAGE_SCHEME}://open"
        );
        assert!(
            IMAGE_PREVIEW_JS.contains("__MB_IMG_PATH__"),
            "查看器脚本要留出路径占位符"
        );
    }
}
