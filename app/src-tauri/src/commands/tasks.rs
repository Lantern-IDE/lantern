//! Tauri 명령: 작업 저장, 격리 작업 공간, AI 변경 되돌리기

use crate::state::AppState;
use crate::{applog, config, hooks, state, tasks, worktree};
use crate::{anyhow_err, err, CmdResult};
use tauri::{AppHandle, State};

// ── 작업 저장과 격리 ───────────────────────────────────────
#[tauri::command]
pub(crate) fn task_list(state: State<'_, AppState>) -> CmdResult<Vec<serde_json::Value>> {
    let p = state.project().map_err(anyhow_err)?;
    tasks::list(&p.root).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) fn task_save(state: State<'_, AppState>, id: String, data: serde_json::Value) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    tasks::save_ui(&p.root, &id, &data).map_err(anyhow_err)
}

/// 작업을 지운다. 격리 작업 공간이 있으면 함께 지운다.
#[tauri::command]
pub(crate) async fn task_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let w = state.worktrees.lock().unwrap().remove(&id).map(|w| (w.path, w.branch)).or_else(|| tasks::worktree_of(&p.root, &id));
    state.sessions.lock().unwrap().remove(&id);
    let root = p.root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some((path, branch)) = w {
            worktree::remove(&root, &path, &branch);
        }
        tasks::delete(&root, &id)
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)
}

fn worktree_for(state: &State<'_, AppState>, root: &std::path::Path, session: &str) -> CmdResult<worktree::Worktree> {
    if let Some(w) = state.worktrees.lock().unwrap().get(session) {
        return Ok(w.clone());
    }
    let (path, branch) = tasks::worktree_of(root, session).ok_or("격리된 작업이 아닙니다")?;
    if !path.is_dir() {
        return Err("작업 공간이 지워졌습니다".into());
    }
    let w = worktree::Worktree { path, branch };
    state.worktrees.lock().unwrap().insert(session.to_string(), w.clone());
    Ok(w)
}

#[tauri::command]
pub(crate) async fn worktree_create(state: State<'_, AppState>, session: String) -> CmdResult<worktree::Worktree> {
    let p = state.project().map_err(anyhow_err)?;
    if !p.is_trusted() {
        return Err("제한 모드에서는 격리 작업 공간을 만들 수 없습니다".into());
    }
    let root = p.root.clone();
    let id = session.clone();
    let w = tauri::async_runtime::spawn_blocking(move || worktree::create(&root, &id)).await.map_err(err)?.map_err(anyhow_err)?;
    applog::info(&format!("격리 작업 공간: {} ({})", w.branch, w.path.display()));
    state.worktrees.lock().unwrap().insert(session, w.clone());
    Ok(w)
}

#[derive(serde::Serialize)]
pub(crate) struct WorktreeChanges {
    files: Vec<String>,
    diff: String,
}

#[tauri::command]
pub(crate) async fn worktree_changes(state: State<'_, AppState>, session: String) -> CmdResult<WorktreeChanges> {
    let p = state.project().map_err(anyhow_err)?;
    let w = worktree_for(&state, &p.root, &session)?;
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<WorktreeChanges> {
        Ok(WorktreeChanges { files: worktree::changed_files(&w.path)?, diff: worktree::diff(&w.path)? })
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)
}

/// 격리 작업의 변경을 프로젝트에 적용한다. 바뀐 파일 목록을 돌려준다.
#[tauri::command]
pub(crate) async fn worktree_apply(app: AppHandle, state: State<'_, AppState>, session: String) -> CmdResult<Vec<String>> {
    let p = state.project().map_err(anyhow_err)?;
    let w = worktree_for(&state, &p.root, &session)?;
    let root = p.root.clone();
    let files = tauri::async_runtime::spawn_blocking(move || worktree::apply(&root, &w.path)).await.map_err(err)?.map_err(anyhow_err)?;
    if !files.is_empty() && p.is_trusted() {
        let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
        if !cfg.hooks.on_agent_done.is_empty() {
            hooks::run(&app, &p.root, "on_agent_done", &cfg.hooks.on_agent_done).await;
        }
    }
    Ok(files)
}

#[tauri::command]
pub(crate) async fn worktree_discard(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let w = worktree_for(&state, &p.root, &session)?;
    state.worktrees.lock().unwrap().remove(&session);
    let root = p.root.clone();
    tauri::async_runtime::spawn_blocking(move || worktree::remove(&root, &w.path, &w.branch)).await.map_err(err)?;
    Ok(())
}

/// AI 작업 하나를 통째로 되돌린다 (체크포인트는 한 번만 쓸 수 있다).
/// 그 뒤로 다른 곳에서 바뀐 파일이 있으면 `force` 없이는 되돌리지 않고 "CONFLICT:" 목록을 돌려준다.
#[tauri::command]
pub(crate) async fn revert_checkpoint(state: State<'_, AppState>, id: String, force: Option<bool>) -> CmdResult<usize> {
    if !force.unwrap_or(false) {
        if let Ok(p) = state.project() {
            let later = tasks::changed_since(&id, &p.root);
            if !later.is_empty() {
                return Err(format!("CONFLICT:{}", later.join("\n")));
            }
        }
    }
    let mem = state.checkpoints.lock().unwrap().remove(&id);
    let originals = mem.or_else(|| tasks::load_checkpoint(&id)).ok_or("이미 되돌렸거나 없는 작업입니다")?;
    let n = state::restore(&originals).map_err(anyhow_err)?;
    tasks::delete_checkpoint(&id);
    Ok(n)
}
