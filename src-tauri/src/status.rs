//! 账号状态追踪（登录态/封禁/回答就绪等）。

use chrono::Utc;
use std::{collections::HashMap, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;

use crate::*;

pub(crate) fn active_profile_id() -> &'static Mutex<Option<String>> {
    ACTIVE_PROFILE_ID.get_or_init(|| Mutex::new(None))
}

pub(crate) fn set_active_profile_id(id: &str) {
    if let Ok(mut active) = active_profile_id().lock() {
        *active = Some(id.to_string());
    }
}

pub(crate) fn is_active_profile(id: &str) -> bool {
    active_profile_id()
        .lock()
        .ok()
        .and_then(|active| active.clone())
        .as_deref()
        == Some(id)
}

pub(crate) fn profile_activity_eval_script(active: bool) -> String {
    format!(
        "try {{ document.dispatchEvent(new CustomEvent('mb-profile-active', {{ detail: {{ active: {} }} }})); }} catch (e) {{}}",
        if active { "true" } else { "false" }
    )
}

/// 页面 alt / aria-label 里的界面用词（如 Google 图标的 "logo"）不能当作账号名。
pub(crate) fn is_junk_account(value: &str) -> bool {
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
pub(crate) fn clean_status(mut status: ProfileStatus) -> ProfileStatus {
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

pub(crate) fn save_profile_status(app: &AppHandle, id: &str, mut status: ProfileStatus) {
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
            // answer_finished_at 现在是「最近一次 AI 完成回答的时间」，要持久保留，
            // 用于侧边栏显示上次 AI 使用时间。仅在 answer_ready 从 false 翻到 true
            // （即真正的「刚刚答完」瞬间）时才覆盖；其它情况（仍在生成 / 反复扫描）
            // 都沿用上一次的值，避免时间戳随每次扫描往前漂。
            if status.answer_ready && !previous.answer_ready {
                status.answer_finished_at = Some(Utc::now().to_rfc3339());
            } else {
                status.answer_finished_at = previous.answer_finished_at.clone();
            }
        } else if status.answer_ready {
            // 首次上报且已经 ready：当作刚答完，记下时间。
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

/// 用户切回账号即视为已查看"回答完成"提醒。页面端也同步清除，
/// 防止下一次状态上报把提醒重新置回。
///
/// 注意：只清 `answer_ready`（未读提醒），**保留** `answer_finished_at`——
/// 它现在表示「最近一次 AI 完成回答的时间」，要持久用于侧边栏展示。
pub(crate) fn clear_profile_answer_ready(app: &AppHandle, id: &str) {
    if let Ok(store) = app.store(STORE_FILE) {
        let mut map: HashMap<String, ProfileStatus> = store
            .get(STATUS_KEY)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        if let Some(status) = map.get_mut(id) {
            if status.answer_ready {
                status.answer_ready = false;
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
        let _ = webview
            .eval("try { document.dispatchEvent(new Event('mb-answer-seen')); } catch (e) {}");
    }
}

/// 返回所有账号最近一次上报的状态（积分 / 登录账号），随 store 持久化。
/// 读取时顺带清理旧版本缓存的脏账号名（如 "logo"）。
#[tauri::command]
pub(crate) fn get_profile_statuses(
    app: AppHandle,
) -> Result<HashMap<String, ProfileStatus>, String> {
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
pub(crate) fn refresh_profile_statuses(app: AppHandle) -> Result<(), String> {
    for (label, webview) in app.webviews() {
        if label.starts_with(PROFILE_PREFIX) {
            let _ = webview
                .eval("try { document.dispatchEvent(new Event('mb-status-scan')); } catch (e) {}");
        }
    }
    Ok(())
}
