//! Tauri 명령: 프로젝트 열기와 폴더 신뢰

use crate::state::{file_uri, AppState, Project};
use crate::{acp, applog, config, fswatch, mcp_client, semantic, state};
use crate::{anyhow_err, err, CmdResult};
use lantern_context::Engine;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize)]
pub(crate) struct ProjectInfo {
    root: String,
    root_uri: String,
    name: String,
    trusted: bool,
    /// 예전 설정 폴더(.khala)를 .lantern으로 옮겼으면 true
    migrated: bool,
}

#[tauri::command]
pub(crate) async fn open_project(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<ProjectInfo> {
    let root = std::path::absolute(&path).map_err(err)?;
    let migrated = lantern_context::migrate_legacy_dir(&root);
    if migrated {
        applog::info(&format!("프로젝트 설정 폴더를 옮김: {}/.khala → .lantern", root.display()));
    }
    let engine = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        move || Engine::open(&root)
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)?;

    // 이전 프로젝트의 세션·언어 서버 정리
    for (_, mut l) in state.lsps.lock().unwrap().drain() {
        l.stop();
    }
    state.sessions.lock().unwrap().clear();
    acp::close(&state, None);
    mcp_client::close_all(&state);

    let trusted = state::is_trusted(&root);
    let project = Arc::new(Project::new(root.clone(), engine, trusted));
    *state.project.lock().unwrap() = Some(project.clone());

    // 외부 변경 감시 (실패해도 앱은 계속 쓴다)
    *state.watcher.lock().unwrap() = match fswatch::watch(app.clone(), &root, project.index_dirty.clone()) {
        Ok(w) => {
            project.watched.store(true, std::sync::atomic::Ordering::Relaxed);
            Some(w)
        }
        Err(e) => {
            applog::error(&format!("파일 감시를 시작하지 못했습니다: {e}"));
            None
        }
    };

    // 인덱싱은 뒤에서. 끝나면 알린다. 의미 검색을 켰으면 그 뒤로 벡터도 조금씩 만든다.
    let _ = app.emit("index", json!({ "status": "running" }));
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = {
            let mut e = project.engine.lock().unwrap();
            project.index_dirty.store(false, std::sync::atomic::Ordering::SeqCst);
            e.refresh()
        };
        semantic::spawn_background(app2.clone(), project.clone());
        let payload = match r {
            Ok(s) => json!({ "status": "done", "stats": s }),
            Err(e) => json!({ "status": "error", "message": format!("{e:#}") }),
        };
        let _ = app2.emit("index", payload);
    });

    Ok(ProjectInfo {
        name: root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        root_uri: file_uri(&root),
        root: root.to_string_lossy().into_owned(),
        trusted,
        migrated,
    })
}

/// 폴더 신뢰 설정. 신뢰하면 프로젝트 설정·에이전트·훅·파일 수정이 허용된다.
#[tauri::command]
pub(crate) async fn set_trust(state: State<'_, AppState>, trusted: bool) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    state::set_trusted(&p.root, trusted).map_err(anyhow_err)?;
    p.trusted.store(trusted, std::sync::atomic::Ordering::Relaxed);
    // 제한 모드로 바뀌면 프로젝트 설정으로 띄웠을 수 있는 언어 서버를 내린다.
    if !trusted {
        for (_, mut l) in state.lsps.lock().unwrap().drain() {
            l.stop();
        }
    }
    Ok(())
}

/// 프로젝트 설정에서 쓰지 않은 값 (신뢰한 폴더에서만 프로젝트 설정을 읽는다)
#[tauri::command]
pub(crate) async fn config_ignored(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    let p = state.project().map_err(anyhow_err)?;
    if !p.is_trusted() {
        return Ok(Vec::new());
    }
    Ok(config::load_with_ignored(Some(&p.root)).map(|r| r.1).unwrap_or_default())
}

/// `lantern-app <폴더>`로 실행했을 때 그 폴더
#[tauri::command]
pub(crate) fn startup_path() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-') && std::path::Path::new(a).is_dir())
        .and_then(|a| std::path::absolute(a).ok())
        .map(|p| p.to_string_lossy().into_owned())
}
