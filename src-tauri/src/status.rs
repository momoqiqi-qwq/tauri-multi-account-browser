//! 账号状态追踪（登录态/封禁/回答就绪等）。

use chrono::Utc;
use std::{collections::HashMap, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime};
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
    match active_profile_id().lock() {
        Ok(active) => active.as_deref() == Some(id),
        Err(_) => false,
    }
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

pub(crate) fn save_profile_status<R: Runtime>(app: &AppHandle<R>, id: &str, mut status: ProfileStatus) {
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
            // answer_finished_at 是「最近一次 AI 完成回答的时间」，要持久保留。
            //
            // 边沿信号用 **answer_generating 的下降沿**（生成中 → 不在生成），而不是
            // answer_ready 的上升沿。原因：扫描脚本只在「后台账号」上把 answer_ready 翻
            // 成 true（避免对当前正在看的账号弹「回答完成」提醒），所以如果以 ready
            // 为信号，当前正在看的账号永远不会被记时间 —— 这就是 v24 第一版的坑。
            //
            // 其它情况（仍在生成 / 反复扫描 / 前后台）都沿用上一次的值，避免时间戳
            // 随每次扫描往前漂。
            if !status.answer_generating && previous.answer_generating {
                status.answer_finished_at = Some(Utc::now().to_rfc3339());
            } else {
                status.answer_finished_at = previous.answer_finished_at.clone();
            }
            // v28：AI 对话计时。上升沿（停止按钮出现 = 用户刚发送）记下本轮开始
            // 时间，前端生成期间用它实时走表；新一轮开始本身就是「重新从零计算」。
            // 下降沿用 finished_at 记完成时刻，本轮耗时优先用页面脚本实测的 d；
            // 脚本没带 d（典型：生成中途刷新页面，状态复位后补报 g=0）就沿用
            // 上一轮实测值，避免把刷新间隔误当成一轮对话时长。
            if status.answer_generating && !previous.answer_generating {
                status.answer_started_at = Some(Utc::now().to_rfc3339());
            } else {
                status.answer_started_at = previous.answer_started_at.clone();
            }
            if status.answer_duration_ms.is_none() {
                status.answer_duration_ms = previous.answer_duration_ms;
            }
        }
        // 没有 previous 时不主动记时间：可能是刚加载的标签页（脚本状态被重置），
        // 也可能是首次创建账号。等下一次出现生成→不生成的真实边沿再记。

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

/// 就地改写某条状态的个别字段并落盘 + 广播，**不走** `save_profile_status`
/// 的生成/回答边沿检测。
///
/// 宿主侧更新与页面无关的字段（如代理探测结果）时必须走这里：
/// `save_profile_status` 会拿传入值和上一条比较边沿，而宿主手里没有页面状态，
/// 只能给出默认值 —— 于是被当成一次真实的「生成中 → 空闲」下降沿，凭空写下
/// 一轮对话的完成时间，还会顺带清掉未读提醒。
///
/// `patch` 返回 false 表示无需改动，此时既不写盘也不广播（也不会为不存在的
/// 账号留下空状态）。
fn patch_profile_status<R: Runtime, F>(app: &AppHandle<R>, id: &str, patch: F)
where
    F: FnOnce(&mut ProfileStatus) -> bool,
{
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    let mut map: HashMap<String, ProfileStatus> = store
        .get(STATUS_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let patched = {
        let status = map.entry(id.to_string()).or_default();
        patch(status).then(|| status.clone())
    };
    let Some(status) = patched else { return };
    if let Ok(value) = serde_json::to_value(&map) {
        store.set(STATUS_KEY, value);
        let _ = store.save();
    }
    let _ = app.emit(
        "profile:status",
        ProfileStatusEvent {
            id: id.to_string(),
            status,
        },
    );
}

/// 代理探测结果入库：只覆盖 4 个代理字段，其余状态原样保留。
pub(crate) fn save_profile_proxy_result<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    result: &ProxyTestResult,
) {
    patch_profile_status(app, id, |status| {
        status.proxy_ip = Some(result.ip.clone());
        status.proxy_region = Some(result.region.clone());
        status.proxy_isp = Some(result.isp.clone());
        status.proxy_tested_at = Some(Utc::now().to_rfc3339());
        true
    });
}

/// 用户切回账号即视为已查看"回答完成"提醒。页面端也同步清除，
/// 防止下一次状态上报把提醒重新置回。
///
/// 注意：只清 `answer_ready`（未读提醒），**保留** `answer_finished_at`——
/// 它现在表示「最近一次 AI 完成回答的时间」，要持久用于侧边栏展示。
pub(crate) fn clear_profile_answer_ready<R: Runtime>(app: &AppHandle<R>, id: &str) {
    patch_profile_status(app, id, |status| {
        if !status.answer_ready {
            return false;
        }
        status.answer_ready = false;
        true
    });
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
