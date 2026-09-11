//! 账号 CRUD 命令与账号模板。

use chrono::Utc;
use std::{
    collections::{HashMap, HashSet},
    fs,
    time::Duration,
};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime};
use tauri_plugin_store::StoreExt;
use uuid::Uuid;

use crate::*;

#[tauri::command]
pub(crate) fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, String> {
    load_profiles(&app)
}

// ---- 账号模板命令 ----

#[tauri::command]
pub(crate) fn list_profile_templates(app: AppHandle) -> Result<Vec<ProfileTemplate>, String> {
    Ok(load_profile_templates(&app))
}

/// 保存模板。返回**规范化后**的列表，前端以此回写本地状态，保证前后端一致。
#[tauri::command]
pub(crate) fn save_profile_templates(
    app: AppHandle,
    templates: Vec<ProfileTemplate>,
) -> Result<Vec<ProfileTemplate>, String> {
    let templates = normalized_templates(templates);
    store_profile_templates(&app, &templates)?;
    Ok(templates)
}

// 参数多是因为 `#[tauri::command]` 会把每个参数摊成 invoke 载荷的一个字段，
// 前端 `invoke('create_profile', {...})` 才能逐字段传。收成 struct 会改变前端调用形状，
// 收益不抵改动面，这里显式放行。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) fn create_profile(
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
    tags: Option<Vec<String>>,
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
        tags,
    };
    let profile = build_profile(&app, draft, profiles.len())?;
    profiles.push(profile.clone());
    save_profiles(&app, &profiles)?;
    Ok(profile)
}

/// 批量修改：先整体校验，全部通过后才写盘。
///
/// 与 `create_profiles_bulk` 同一个原则（见 CHANGES_V16）：不要用前端循环
/// `update_profile`，第 N 个失败时会留下前 N-1 个已改、后面没改的半截状态。
#[tauri::command]
pub(crate) fn update_profile_batch(
    app: AppHandle,
    ids: Vec<String>,
    patch: ProfileBatchPatch,
) -> Result<Vec<Profile>, String> {
    update_profile_batch_impl(&app, ids, patch)
}

pub(crate) fn update_profile_batch_impl<R: Runtime>(
    app: &AppHandle<R>,
    ids: Vec<String>,
    patch: ProfileBatchPatch,
) -> Result<Vec<Profile>, String> {
    if ids.is_empty() {
        return Err("没有选中任何账号".to_string());
    }
    if ids.len() > MAX_BULK_PROFILES {
        return Err(format!("单次最多修改 {MAX_BULK_PROFILES} 个账号"));
    }

    // ---- 校验阶段：任何一项不合法就整批不改 ----
    let mode = match patch.url_mode.as_deref() {
        None => None,
        Some("inherit") => Some("inherit".to_string()),
        Some("custom") => Some("custom".to_string()),
        Some(other) => return Err(format!("网址模式不合法：{other}")),
    };
    let default_url = match patch.default_url.as_deref() {
        None => None,
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(normalize_url(value)?.to_string()),
    };
    let group = patch.group.as_ref().map(|value| value.trim().to_string());

    // 切成继承模式时的全局主页，循环外算一次。
    // normalized_global_default_url 内部会整份反序列化设置，N 个账号就是 N 次，
    // 而这个命令跑在主线程上。
    let global_default = if mode.as_deref() == Some("inherit") {
        Some(normalized_global_default_url(app).to_string())
    } else {
        None
    };

    // ---- 应用阶段 ----
    let mut profiles = load_profiles(app)?;
    let mut updated = Vec::with_capacity(ids.len());
    for id in &ids {
        let index = profiles
            .iter()
            .position(|p| p.id == *id)
            .ok_or_else(|| format!("账号不存在：{id}"))?;
        let profile = &mut profiles[index];

        let previous_default = profile.default_url.clone();
        if let Some(ref mode) = mode {
            profile.url_mode = mode.clone();
        }
        if let Some(ref url) = default_url {
            profile.default_url = url.clone();
        } else if let Some(ref global) = global_default {
            // 切成继承模式但没给新网址：以全局主页为准，而不是留着旧的 custom 值。
            profile.default_url = global.clone();
        }
        if mode.is_some() || default_url.is_some() {
            // 还停在旧主页（或没有 last_url）的账号同步到新主页，避免下次打开还是老地址。
            if profile.last_url.trim().is_empty() || profile.last_url == previous_default {
                profile.last_url = profile.default_url.clone();
            }
        }

        if let Some(ref set) = patch.tags_set {
            profile.tags = normalize_tags(set);
        }
        if let Some(ref add) = patch.tags_add {
            let mut merged = profile.tags.clone();
            merged.extend(add.iter().cloned());
            profile.tags = normalize_tags(&merged);
        }
        if let Some(ref remove) = patch.tags_remove {
            let dropping: HashSet<String> = remove
                .iter()
                .map(|tag| tag.trim().to_lowercase())
                .filter(|tag| !tag.is_empty())
                .collect();
            profile
                .tags
                .retain(|tag| !dropping.contains(&tag.to_lowercase()));
        }
        if let Some(value) = patch.favorite {
            profile.favorite = value;
        }
        if let Some(ref value) = group {
            profile.group = value.clone();
        }

        updated.push(profile.clone());
    }

    save_profiles(app, &profiles)?;
    Ok(updated)
}

/// 批量创建：先整体校验，全部通过后才写盘。
///
/// 以前前端循环调用 `create_profile`，第 N 个失败时会留下前 N-1 个半成品账号。
/// 这里改成一次调用：任一草稿不合格就整批不落盘，错误信息带序号便于定位。
#[tauri::command]
pub(crate) fn create_profiles_bulk(
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
    // 全局主页只解析一次：build_profile 每次调用都会反序列化一遍设置，N 个草稿就是 N 次。
    let global_default = normalized_global_default_url(&app);
    let mut created = Vec::with_capacity(drafts.len());
    for (offset, draft) in drafts.into_iter().enumerate() {
        let profile = build_profile_with_global(draft, base + offset, &global_default)
            .map_err(|e| format!("第 {} 个账号：{e}", offset + 1))?;
        created.push(profile);
    }
    profiles.extend(created.iter().cloned());
    save_profiles(&app, &profiles)?;
    Ok(created)
}

// 同 create_profile：参数逐个对应 invoke 载荷字段，保持前端调用形状不变。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn update_profile(
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
    tags: Option<Vec<String>>,
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
        if mode != "inherit" && mode != "custom" {
            return Err("网址模式不合法".to_string());
        }
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
    if let Some(value) = tags {
        // 标签只是展示属性，不影响隔离，改动即时生效。
        profile.tags = normalize_tags(&value);
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
pub(crate) fn reorder_profiles(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let mut profiles = load_profiles(&app)?;
    for (index, id) in ids.iter().enumerate() {
        if let Some(profile) = profiles.iter_mut().find(|p| &p.id == id) {
            profile.order = index;
        }
    }
    save_profiles(&app, &profiles)
}

#[tauri::command]
pub(crate) fn move_profile_to_group(
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
        if let Some(profile) = profiles
            .iter_mut()
            .find(|profile| &profile.id == profile_id)
        {
            profile.order = index;
        }
    }
    save_profiles(&app, &profiles)
}

#[tauri::command]
pub(crate) async fn activate_profile(
    app: AppHandle,
    id: String,
    bounds: BrowserBounds,
) -> Result<(), String> {
    let mut profiles = load_profiles(&app)?;
    let profile = profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "账号不存在".to_string())?
        .clone();
    let label = profile_label(&id);

    // 记录"最近使用"。切标签是人工操作，频率不高，直接落盘；
    // 前端也会乐观更新自己的副本（见 useTabs.activate），两边保持同一口径。
    if let Some(target) = profiles.iter_mut().find(|p| p.id == id) {
        target.last_used_at = Some(Utc::now().to_rfc3339());
        save_profiles(&app, &profiles)?;
    }

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
pub(crate) async fn close_profile_tab(app: AppHandle, id: String) -> Result<(), String> {
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
pub(crate) async fn hibernate_profile(app: AppHandle, id: String) -> Result<(), String> {
    close_profile_tab(app, id).await
}

/// 窗口尺寸变化时只同步当前可见账号。隐藏账号在切回前会由
/// layout_profile_webviews 再设置一次最新边界，避免多账号时无意义地批量 resize。
#[tauri::command]
pub(crate) fn sync_profile_bounds(
    app: AppHandle,
    bounds: BrowserBounds,
    active_id: Option<String>,
) -> Result<(), String> {
    let Some(id) = active_id else {
        return Ok(());
    };
    if let Some(webview) = app.get_webview(&profile_label(&id)) {
        let _ = webview.set_position(LogicalPosition::new(bounds.x, bounds.y));
        let _ = webview.set_size(LogicalSize::new(
            bounds.width.max(1.0),
            bounds.height.max(1.0),
        ));
    }
    Ok(())
}

/// 显示 / 隐藏某个账号的 WebView。Vue 的下拉菜单、对话框等弹层渲染在
/// 主 WebView 上，会被原生子 WebView 盖住；弹层打开期间需要临时隐藏。
#[tauri::command]
pub(crate) fn set_profile_webview_visible(
    app: AppHandle,
    id: String,
    visible: bool,
) -> Result<(), String> {
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
pub(crate) fn navigate_profile(app: AppHandle, id: String, url: String) -> Result<(), String> {
    let webview = app
        .get_webview(&profile_label(&id))
        .ok_or_else(|| "WebView 尚未创建".to_string())?;
    webview
        .navigate(normalize_url(&url)?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn browser_action(app: AppHandle, id: String, action: String) -> Result<(), String> {
    let webview = app
        .get_webview(&profile_label(&id))
        .ok_or_else(|| "WebView 尚未创建".to_string())?;
    match action.as_str() {
        "back" => webview.eval("history.back()"),
        "forward" => webview.eval("history.forward()"),
        "reload" => webview.reload(),
        "reload_restore" => webview.eval(
            r#"
          try {
            sessionStorage.setItem('__mab_refresh_position_v1', JSON.stringify({
              x: window.scrollX || 0,
              y: window.scrollY || 0,
              hash: location.hash || ''
            }));
          } catch (_) {}
          location.reload();
        "#,
        ),
        _ => return Err("未知浏览器动作".to_string()),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn clear_profile_data(app: AppHandle, id: String) -> Result<(), String> {
    let label = profile_label(&id);
    if let Some(webview) = app.get_webview(&label) {
        webview
            .clear_all_browsing_data()
            .map_err(|e| e.to_string())?;
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
pub(crate) async fn delete_profile(app: AppHandle, id: String) -> Result<(), String> {
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
pub(crate) fn clone_profile(app: AppHandle, id: String) -> Result<Profile, String> {
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
