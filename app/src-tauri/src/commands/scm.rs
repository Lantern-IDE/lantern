//! Tauri 명령: 소스 제어(git)와 커밋 전·PR 영향 검토

use crate::state::AppState;
use crate::{agent, config, gitops, review};
use crate::{anyhow_err, git_blocking, git_repo, with_engine, CmdResult};
use tauri::State;

#[tauri::command]
pub(crate) async fn git_repos(state: State<'_, AppState>) -> CmdResult<Vec<gitops::RepoSummary>> {
    let p = state.project().map_err(anyhow_err)?;
    if !p.is_trusted() {
        return Err(anyhow_err(anyhow::anyhow!("제한 모드에서는 소스 제어를 쓸 수 없습니다. 이 폴더를 신뢰하면 켜집니다")));
    }
    git_blocking(move || Ok(gitops::repos(&p.root))).await
}

#[tauri::command]
pub(crate) async fn git_status(state: State<'_, AppState>, repo: Option<String>) -> CmdResult<gitops::GitStatus> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    git_blocking(move || gitops::status(&dir)).await
}

#[tauri::command]
pub(crate) async fn git_diff(state: State<'_, AppState>, repo: Option<String>, path: String, staged: bool) -> CmdResult<String> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    git_blocking(move || gitops::diff(&dir, &path, staged)).await
}

/// action: stage | unstage | discard | commit | init | fetch | pull | push
#[tauri::command]
pub(crate) async fn git_action(state: State<'_, AppState>, repo: Option<String>, action: String, paths: Vec<String>, message: Option<String>) -> CmdResult<String> {
    if action == "init" {
        // 저장소가 없는 연 폴더에 새로 만든다
        let p = state.project().map_err(anyhow_err)?;
        if !p.is_trusted() {
            return Err(anyhow_err(anyhow::anyhow!("제한 모드에서는 소스 제어를 쓸 수 없습니다")));
        }
        return git_blocking(move || gitops::init(&p.root).map(|_| String::new())).await;
    }
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    git_blocking(move || match action.as_str() {
        "stage" => gitops::stage(&dir, &paths).map(|_| String::new()),
        "unstage" => gitops::unstage(&dir, &paths).map(|_| String::new()),
        "discard" => gitops::discard(&dir, &paths).map(|_| String::new()),
        "commit" => gitops::commit(&dir, message.as_deref().unwrap_or("")),
        "fetch" => gitops::fetch(&dir),
        "pull" => gitops::pull(&dir),
        "push" => gitops::push(&dir),
        other => anyhow::bail!("알 수 없는 작업: {other}"),
    })
    .await
}

#[tauri::command]
pub(crate) async fn git_log(state: State<'_, AppState>, repo: String, skip: u32, limit: u32) -> CmdResult<Vec<gitops::Commit>> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::log(&dir, skip, limit)).await
}

#[tauri::command]
pub(crate) async fn git_commit_files(state: State<'_, AppState>, repo: String, hash: String) -> CmdResult<Vec<gitops::CommitFile>> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::commit_files(&dir, &hash)).await
}

#[tauri::command]
pub(crate) async fn git_commit_diff(state: State<'_, AppState>, repo: String, hash: String, path: String) -> CmdResult<String> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::commit_diff(&dir, &hash, &path)).await
}

#[tauri::command]
pub(crate) async fn git_branches(state: State<'_, AppState>, repo: String) -> CmdResult<Vec<gitops::Branch>> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::branches(&dir)).await
}

#[tauri::command]
pub(crate) async fn git_checkout(state: State<'_, AppState>, repo: String, name: String, create: bool, remote: bool) -> CmdResult<String> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::checkout(&dir, &name, create, remote)).await
}

/// 커밋 전 영향 검토: 아직 커밋하지 않은 변경 전체의 영향 반경
#[tauri::command]
pub(crate) async fn review_changes(state: State<'_, AppState>, repo: Option<String>) -> CmdResult<review::Review> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    let root = state.project().map_err(anyhow_err)?.root.clone();
    let prefix = gitops::rel_path(&root, &dir);
    // 연 폴더가 저장소의 하위 폴더면, 저장소 기준 경로에서 그 부분을 뗀다
    let strip = if prefix.is_empty() && dir != root {
        gitops::git(&root, &["rev-parse", "--show-prefix"]).unwrap_or_default().trim().trim_end_matches('/').to_string()
    } else {
        String::new()
    };
    with_engine(&state, move |e, _| review::review(&e.store, &dir, &prefix, &strip)).await
}

/// PR(번호)·브랜치의 영향 검토
#[tauri::command]
pub(crate) async fn review_target(state: State<'_, AppState>, repo: Option<String>, target: String) -> CmdResult<review::Review> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    let root = state.project().map_err(anyhow_err)?.root.clone();
    let prefix = gitops::rel_path(&root, &dir);
    let strip = if prefix.is_empty() && dir != root {
        gitops::git(&root, &["rev-parse", "--show-prefix"]).unwrap_or_default().trim().trim_end_matches('/').to_string()
    } else {
        String::new()
    };
    // PR 번호면 원격에서 가져온 뒤(fetch) 지금 인덱스로 영향 반경을 잰다
    with_engine(&state, move |e, _| review::review_target(&e.store, &dir, &prefix, &strip, &target)).await
}

/// 스테이징된 변경(없으면 전체)으로 커밋 메시지를 만든다 (기본 모델)
#[tauri::command]
pub(crate) async fn commit_message(state: State<'_, AppState>, repo: Option<String>) -> CmdResult<String> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    agent::check_budget(&cfg).map_err(anyhow_err)?;
    review::commit_message(&state.http, &cfg, &dir).await.map_err(anyhow_err)
}
