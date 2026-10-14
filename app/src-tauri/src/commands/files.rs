//! Tauri 명령: 파일 탐색기·검색·바꾸기·읽고 쓰기

use crate::state::{resolve_in_root, AppState};
use crate::{config, hooks, state, tasks, workspace};
use crate::{anyhow_err, err, CmdResult};
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) async fn list_dir(state: State<'_, AppState>, path: String) -> CmdResult<Vec<workspace::Entry>> {
    let p = state.project().map_err(anyhow_err)?;
    let dir = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    workspace::list_dir(&p.root, &dir).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn list_files(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    let p = state.project().map_err(anyhow_err)?;
    tauri::async_runtime::spawn_blocking(move || workspace::list_files(&p.root)).await.map_err(err)
}

#[tauri::command]
pub(crate) async fn create_path(state: State<'_, AppState>, path: String, dir: bool) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    if dir { workspace::create_dir(&abs) } else { workspace::create_file(&abs) }.map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn rename_path(state: State<'_, AppState>, from: String, to: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let a = resolve_in_root(&p.root, &from).map_err(anyhow_err)?;
    let b = resolve_in_root(&p.root, &to).map_err(anyhow_err)?;
    workspace::rename(&a, &b).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn delete_path(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    if abs == p.root {
        return Err("프로젝트 폴더 자체는 지울 수 없습니다".into());
    }
    workspace::delete_to_trash(&abs).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn reveal_path(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    workspace::reveal_in_os(&abs).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn search_text(state: State<'_, AppState>, query: String, case_sensitive: bool) -> CmdResult<Vec<workspace::FileMatches>> {
    let p = state.project().map_err(anyhow_err)?;
    tauri::async_runtime::spawn_blocking(move || workspace::search_text(&p.root, &query, case_sensitive))
        .await
        .map_err(err)
}

#[derive(serde::Serialize)]
pub(crate) struct ReplaceResult {
    count: usize,
    files: Vec<String>,
    checkpoint: Option<String>,
}

/// 검색 뷰의 바꾸기. AI 작업처럼 체크포인트를 남겨 한 번에 되돌릴 수 있다.
#[tauri::command]
pub(crate) async fn replace_text(
    state: State<'_, AppState>,
    paths: Vec<String>,
    query: String,
    replacement: String,
    case_sensitive: bool,
) -> CmdResult<ReplaceResult> {
    let p = state.project().map_err(anyhow_err)?;
    let root = p.root.clone();
    let (count, originals) = tauri::async_runtime::spawn_blocking(move || {
        workspace::replace_text(&root, &paths, &query, &replacement, case_sensitive)
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)?;
    let files = originals.iter().map(|(path, _)| state::rel_path(&p.root, path)).collect();
    let checkpoint = (!originals.is_empty()).then(|| {
        let id = tasks::unique_id("rp");
        let _ = tasks::save_checkpoint(&id, &originals);
        state.checkpoints.lock().unwrap().insert(id.clone(), originals);
        id
    });
    Ok(ReplaceResult { count, files, checkpoint })
}

#[tauri::command]
pub(crate) async fn read_file(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    workspace::read_text(&abs).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn write_file(app: AppHandle, state: State<'_, AppState>, path: String, content: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    std::fs::write(&abs, content).map_err(err)?;

    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    if p.is_trusted() && !cfg.hooks.on_save.is_empty() {
        let root = p.root.clone();
        tauri::async_runtime::spawn(async move {
            hooks::run(&app, &root, "on_save", &cfg.hooks.on_save).await;
        });
    }
    Ok(())
}
