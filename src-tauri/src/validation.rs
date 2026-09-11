//! 输入校验与规范化：昵称、网址、代理、时区、语言、CSV、模板。

use chrono::Utc;
use std::collections::HashSet;
use tauri::{AppHandle, Runtime};
use url::Url;
use uuid::Uuid;

use crate::*;

/// 规范化标签列表：去空白、丢空串、去重（大小写不敏感）、限制单个长度与总个数。
///
/// 保留首次出现的顺序，这样"已有的标签不会因为再次输入而跳位置"。
/// 去重用小写比较但保留原样大小写 —— 用户写 VIP 就显示 VIP。
pub(crate) fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut tags: Vec<String> = Vec::new();
    for item in raw {
        let tag: String = item.trim().chars().take(MAX_TAG_LEN).collect();
        if tag.is_empty() {
            continue;
        }
        let key = tag.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        tags.push(tag);
        if tags.len() >= MAX_PROFILE_TAGS {
            break;
        }
    }
    tags
}

pub(crate) fn normalized_download_url(url: &Url) -> String {
    let mut clean = url.clone();
    clean.set_fragment(None);
    clean.to_string()
}

pub(crate) fn inferred_download_name(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
}

pub(crate) fn is_chat01_download_url(url: &Url) -> bool {
    url.host_str() == Some("files.chat01.ai") && url.path().starts_with("/python-generations/")
}

pub(crate) fn normalized_guard_rules(mut rules: Vec<DownloadGuardRule>) -> Vec<DownloadGuardRule> {
    let mut seen = HashSet::new();
    rules.retain_mut(|rule| {
        rule.domain = rule
            .domain
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        rule.seconds = rule.seconds.min(60);
        !rule.domain.is_empty()
            && rule.domain.len() <= 253
            && rule
                .domain
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && seen.insert(rule.domain.clone())
    });
    rules.truncate(20);
    rules
}

pub(crate) fn download_guard_seconds_for(settings: &AppSettings, url: &Url) -> u64 {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    for rule in &settings.download_guard_rules {
        let domain = rule
            .domain
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}"))) {
            return rule.seconds.min(60);
        }
    }
    settings.download_guard_seconds.min(60)
}

/// 规范化扩展名白名单：去点号/空白、转小写、去重，并丢弃异常项。
/// 空字符串仍代表“不限制格式”。
pub(crate) fn normalize_ai_exts(raw: &str) -> String {
    let mut seen = HashSet::new();
    raw.split(',')
        .filter_map(|item| {
            let ext = item.trim().trim_start_matches('.').to_ascii_lowercase();
            if ext.is_empty() || ext.len() > 12 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            if seen.insert(ext.clone()) {
                Some(ext)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// 写盘前的规范化：去空白、剔除无效项、修正非法 url_mode、限制数量。
///
/// 返回规范化后的列表，由调用方回传给前端，避免前后端状态漂移。
pub(crate) fn normalized_templates(templates: Vec<ProfileTemplate>) -> Vec<ProfileTemplate> {
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

pub(crate) fn parse_proxy(raw: &str) -> Result<Option<Url>, String> {
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
pub(crate) fn sanitize_timezone(raw: &str) -> Result<String, String> {
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
pub(crate) fn sanitize_locale(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > 35
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("语言格式不合法，请使用 BCP-47 名称，如 zh-CN".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn sanitize_profile_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("账号昵称不能为空".to_string());
    }
    if name.chars().count() > 80 {
        return Err("账号昵称不能超过 80 个字符".to_string());
    }
    Ok(name.to_string())
}

pub(crate) fn normalize_url(raw: &str) -> Result<Url, String> {
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

// ---- 导入 / 导出 ----

/// 导入时遇到同名账号的处理策略。
/// - `rename`：自动改名（追加 " (2)"、" (3)"…），保留双方。
/// - `skip`：跳过，保留现有账号。
/// - `overwrite`：用导入内容覆盖同名账号。
pub(crate) fn default_conflict_strategy() -> String {
    "rename".to_string()
}

pub(crate) fn parse_bool_cell(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y"
    )
}

/// 解析 CSV 文本为导入草稿。
///
/// 用 csv crate 处理引号/逗号/换行转义，避免手写解析在昵称里带逗号时出错。
/// 表头可有可无：首行若以 `name` 开头就当表头跳过。
pub(crate) fn parse_csv_accounts(text: &str) -> Result<Vec<ProfileDraft>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut drafts = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|e| format!("第 {} 行解析失败：{e}", index + 1))?;
        // 跳过完全空行。
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        // 首行是表头（第一列恰好是 name 且后面跟着已知列名）就跳过。
        if index == 0
            && record.len() > 1
            && record[0].trim().eq_ignore_ascii_case("name")
            && record[1].trim().eq_ignore_ascii_case("default_url")
        {
            continue;
        }
        let row: CsvAccountRow = record
            .deserialize(None)
            .map_err(|e| format!("第 {} 行字段读取失败：{e}", index + 1))?;
        let url_mode = if row.url_mode.trim().is_empty() {
            None
        } else {
            Some(row.url_mode.trim().to_string())
        };
        drafts.push(ProfileDraft {
            name: row.name,
            default_url: row.default_url,
            url_mode,
            incognito: parse_bool_cell(&row.incognito),
            proxy: Some(row.proxy),
            user_agent: Some(row.user_agent),
            timezone: Some(row.timezone),
            locale: Some(row.locale),
            fingerprint_guard: if row.fingerprint_guard.trim().is_empty() {
                None
            } else {
                Some(parse_bool_cell(&row.fingerprint_guard))
            },
            group: Some(row.group),
            tags: if row.tags.trim().is_empty() {
                None
            } else {
                Some(
                    row.tags
                        .split(CSV_TAG_SEPARATOR)
                        .map(|item| item.to_string())
                        .collect(),
                )
            },
        });
    }
    Ok(drafts)
}

/// 把草稿序列化成 CSV（含表头）。
pub(crate) fn build_csv_accounts(drafts: &[ProfileDraft]) -> Result<String, String> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer.write_record(CSV_HEADER).map_err(|e| e.to_string())?;
    for draft in drafts {
        writer
            .write_record([
                draft.name.as_str(),
                draft.default_url.as_str(),
                draft.url_mode.as_deref().unwrap_or(""),
                if draft.incognito { "true" } else { "false" },
                draft.proxy.as_deref().unwrap_or(""),
                draft.user_agent.as_deref().unwrap_or(""),
                draft.timezone.as_deref().unwrap_or(""),
                draft.locale.as_deref().unwrap_or(""),
                match draft.fingerprint_guard {
                    Some(true) => "true",
                    Some(false) => "false",
                    None => "",
                },
                draft.group.as_deref().unwrap_or(""),
                &draft
                    .tags
                    .as_ref()
                    .map(|tags| tags.join(&CSV_TAG_SEPARATOR.to_string()))
                    .unwrap_or_default(),
            ])
            .map_err(|e| e.to_string())?;
    }
    let bytes = writer.into_inner().map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// 在已有账号名集合里为 `name` 找一个不冲突的名字。
pub(crate) fn unique_profile_name(name: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(name) {
        return name.to_string();
    }
    for suffix in 2..10_000 {
        let candidate = format!("{name} ({suffix})");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    // 理论上到不了这里；兜底用后缀时间戳避免死循环。
    format!("{name} ({})", Utc::now().timestamp())
}

/// 单个创建与批量创建共用的校验与构造逻辑。
///
/// 校验口径只在这里维护一份，避免两个入口各自演化后行为分叉。
/// `base_order` 由调用方按当前账号数 + 偏移量给出，确保批量创建时 order 连续且不冲突。
pub(crate) fn build_profile<R: Runtime>(
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
        tags: normalize_tags(&draft.tags.unwrap_or_default()),
        favorite: false,
        last_used_at: None,
    })
}
