//! store 读写与结构版本迁移。

use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::*;

pub(crate) fn load_app_settings<R: Runtime>(app: &AppHandle<R>) -> AppSettings {
    app.store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(SETTINGS_KEY))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(crate) fn load_profiles<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<Profile>, String> {
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
        repaired |= repair_profile_urls(app, profile);
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

pub(crate) fn save_profiles<R: Runtime>(
    app: &AppHandle<R>,
    profiles: &[Profile],
) -> Result<(), String> {
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

pub(crate) fn load_profile_templates<R: Runtime>(app: &AppHandle<R>) -> Vec<ProfileTemplate> {
    app.store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(PROFILE_TEMPLATES_KEY))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// 注意：命令 `save_profile_templates` 与本函数同名会冲突，故内部 helper 用 `store_` 前缀。
pub(crate) fn store_profile_templates<R: Runtime>(
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

// ---- store 结构迁移 ----

/// 读取 store 里记录的结构版本。缺失或非法一律视为 0（v14 及之前的历史数据）。
pub(crate) fn stored_schema_version<R: Runtime>(app: &AppHandle<R>) -> u64 {
    app.store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(SCHEMA_VERSION_KEY))
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

/// 单个迁移步骤：拿到 AppHandle，就地升版并写盘。
pub(crate) type Migration<R> = fn(&AppHandle<R>) -> Result<(), String>;

/// 按升版顺序排列的迁移步骤，索引 i 表示「把版本 i 升到 i + 1」。
///
/// 每个步骤只做一件事：把该版本之前靠 serde default 隐式兜底的语义显式写清楚。
/// 新增版本时往数组尾部追加函数即可，已发布的旧步骤不要再改动。
pub(crate) fn migrations<R: Runtime>() -> Vec<Migration<R>> {
    vec![migrate_v0_to_v1::<R>, migrate_v1_to_v2::<R>]
}

/// v1 -> v2：账号新增 tags / favorite / last_used_at。
///
/// 三个新字段都带 `#[serde(default)]`，反序列化历史数据不会失败，
/// 所以这里**不是**为了"补默认值"，而是把已存在的脏数据规范化：
/// 历史写入路径没走过 `normalize_tags`，标签可能有空白、重复、超长。
pub(crate) fn migrate_v1_to_v2<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let mut profiles = load_profiles(app)?;
    let mut changed = false;
    for profile in &mut profiles {
        if profile.tags.is_empty() {
            continue;
        }
        let normalized = normalize_tags(&profile.tags);
        if normalized != profile.tags {
            profile.tags = normalized;
            changed = true;
        }
    }
    if changed {
        save_profiles(app, &profiles)?;
    }
    Ok(())
}

/// v0 -> v1：把历史数据里「靠 serde default 才成立」的字段补齐并落盘。
///
/// 这一步不改变任何用户可见行为，只是让旧数据显式带上当前字段，
/// 避免后续版本删掉 serde default 时突然丢失兼容性。
pub(crate) fn migrate_v0_to_v1<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
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
pub(crate) fn migrate_store<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
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
