//! 导入导出：文件对话框、CSV、冲突策略、逐行错误报告。

use std::{collections::HashSet, fs, path::Path};
use tauri::{AppHandle, Runtime};

use crate::*;

/// 导入账号：支持冲突策略与逐行错误报告。
///
/// 与 `create_profiles_bulk`（原子、用于批量创建表单）不同，这里默认**尽力而为**：
/// 单行校验失败只记入 errors，其余行照常导入，因为导入文件里个别行坏掉很常见，
/// 整批回滚反而更难受。
#[tauri::command]
pub(crate) fn import_profiles(
    app: AppHandle,
    drafts: Vec<ProfileDraft>,
    options: Option<ImportOptions>,
) -> Result<ImportReport, String> {
    let options = options.unwrap_or(ImportOptions {
        conflict: default_conflict_strategy(),
        dry_run: false,
    });
    import_profiles_impl(&app, drafts, options.conflict, options.dry_run)
}

/// 导入逻辑本体，与命令分离以便单元测试直接调用。
pub(crate) fn import_profiles_impl<R: Runtime>(
    app: &AppHandle<R>,
    drafts: Vec<ProfileDraft>,
    conflict: String,
    dry_run: bool,
) -> Result<ImportReport, String> {
    match conflict.as_str() {
        "rename" | "skip" | "overwrite" => {}
        other => return Err(format!("不支持的冲突策略：{other}")),
    }
    if drafts.is_empty() {
        return Err("没有需要导入的账号".to_string());
    }
    if drafts.len() > MAX_BULK_PROFILES {
        return Err(format!("单次最多导入 {MAX_BULK_PROFILES} 个账号"));
    }

    let mut profiles = load_profiles(app)?;
    let mut taken: HashSet<String> = profiles.iter().map(|p| p.name.clone()).collect();

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();

    for (offset, draft) in drafts.into_iter().enumerate() {
        let row = offset + 1;
        // 先用原始名字校验（昵称非法要如实报出来，不能先改名再校验）。
        if let Err(message) = sanitize_profile_name(&draft.name) {
            errors.push(ImportRowError {
                row,
                name: draft.name.clone(),
                message,
            });
            continue;
        }
        let requested = sanitize_profile_name(&draft.name).unwrap_or_default();

        // 冲突处理。
        if taken.contains(&requested) {
            match conflict.as_str() {
                "skip" => {
                    skipped.push(ImportSkipped {
                        row,
                        name: requested.clone(),
                        reason: "已存在同名账号".to_string(),
                    });
                    continue;
                }
                "rename" => {
                    let renamed = unique_profile_name(&requested, &taken);
                    taken.insert(renamed.clone());
                    let mut draft = draft;
                    draft.name = renamed;
                    match build_profile(app, draft, profiles.len() + created.len()) {
                        Ok(profile) => created.push(profile),
                        Err(message) => errors.push(ImportRowError {
                            row,
                            name: requested,
                            message,
                        }),
                    }
                }
                _ => {
                    // overwrite：就地更新同名账号，保留 id / created_at / order。
                    let mut draft = draft;
                    draft.name = requested.clone();
                    let order = profiles
                        .iter()
                        .find(|p| p.name == requested)
                        .map(|p| p.order)
                        .unwrap_or(profiles.len());
                    match build_profile(app, draft, order) {
                        Ok(mut incoming) => {
                            if let Some(existing) =
                                profiles.iter_mut().find(|p| p.name == requested)
                            {
                                incoming.id = existing.id.clone();
                                incoming.created_at = existing.created_at.clone();
                                incoming.order = existing.order;
                                *existing = incoming.clone();
                                updated.push(incoming);
                            } else {
                                created.push(incoming);
                            }
                        }
                        Err(message) => errors.push(ImportRowError {
                            row,
                            name: requested,
                            message,
                        }),
                    }
                }
            }
            continue;
        }

        taken.insert(requested.clone());
        match build_profile(app, draft, profiles.len() + created.len()) {
            Ok(profile) => created.push(profile),
            Err(message) => errors.push(ImportRowError {
                row,
                name: requested,
                message,
            }),
        }
    }

    if !dry_run {
        profiles.extend(created.iter().cloned());
        save_profiles(app, &profiles)?;
    }

    Ok(ImportReport {
        created,
        updated,
        skipped,
        errors,
    })
}

/// 弹出系统原生"打开文件"对话框，返回用户选中的路径（取消则返回 null）。
#[tauri::command]
pub(crate) async fn pick_import_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .add_filter("账号数据", &["json", "csv"])
        .blocking_pick_file();
    match picked {
        Some(path) => Ok(Some(
            path.into_path()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string(),
        )),
        None => Ok(None),
    }
}

/// 弹出系统原生"保存文件"对话框，返回用户指定的路径（取消则返回 null）。
#[tauri::command]
pub(crate) async fn pick_export_file(
    app: AppHandle,
    default_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("账号数据", &["json", "csv"])
        .blocking_save_file();
    match picked {
        Some(path) => Ok(Some(
            path.into_path()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string(),
        )),
        None => Ok(None),
    }
}

/// 读取导入文件内容。走 Rust 侧 std::fs，不经过前端 fs 插件，因此不受权限 scope 限制。
#[tauri::command]
pub(crate) fn read_import_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{e}"))
}

/// 写出导出内容。
#[tauri::command]
pub(crate) fn write_export_file(path: String, content: String) -> Result<(), String> {
    if let Some(parent) = Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
    }
    fs::write(&path, content).map_err(|e| format!("写入文件失败：{e}"))
}

/// CSV 文本 -> 导入草稿。前端拿到草稿后可在导入前预览。
#[tauri::command]
pub(crate) fn parse_accounts_csv(text: String) -> Result<Vec<ProfileDraft>, String> {
    parse_csv_accounts(&text)
}

/// 草稿 -> CSV 文本（含表头）。
#[tauri::command]
pub(crate) fn build_accounts_csv(drafts: Vec<ProfileDraft>) -> Result<String, String> {
    build_csv_accounts(&drafts)
}
