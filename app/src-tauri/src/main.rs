// 릴리스 빌드에서 콘솔 창을 띄우지 않는다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod agents;
mod config;
mod applog;
mod tasks;
mod worktree;
mod completion;
mod fswatch;
mod lspinstall;
mod gitops;
mod hooks;
mod llm;
mod lsp;
mod settings;
mod state;
mod terminal;
mod tools;
mod workspace;

#[cfg(test)]
mod e2e_tests;

use lantern_context::assemble::ContextRequest;
use lantern_context::Engine;
use serde::Serialize;
use serde_json::json;
use state::{file_uri, resolve_in_root, AppState, Project};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn anyhow_err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[derive(Serialize)]
struct ProjectInfo {
    root: String,
    root_uri: String,
    name: String,
    trusted: bool,
    /// 예전 설정 폴더(.khala)를 .lantern으로 옮겼으면 true
    migrated: bool,
}

#[tauri::command]
async fn open_project(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<ProjectInfo> {
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

    let trusted = state::is_trusted(&root);
    let project = Arc::new(Project {
        root: root.clone(),
        engine: Arc::new(Mutex::new(engine)),
        trusted: std::sync::atomic::AtomicBool::new(trusted),
    });
    *state.project.lock().unwrap() = Some(project.clone());

    // 외부 변경 감시 (실패해도 앱은 계속 쓴다)
    *state.watcher.lock().unwrap() = match fswatch::watch(app.clone(), &root) {
        Ok(w) => Some(w),
        Err(e) => {
            applog::error(&format!("파일 감시를 시작하지 못했습니다: {e}"));
            None
        }
    };

    // 인덱싱은 뒤에서. 끝나면 알린다.
    let _ = app.emit("index", json!({ "status": "running" }));
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = project.engine.lock().unwrap().refresh();
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
async fn set_trust(state: State<'_, AppState>, trusted: bool) -> CmdResult<()> {
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

/// `lantern-app <폴더>`로 실행했을 때 그 폴더
#[tauri::command]
fn startup_path() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-') && std::path::Path::new(a).is_dir())
        .and_then(|a| std::path::absolute(a).ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn list_dir(state: State<'_, AppState>, path: String) -> CmdResult<Vec<workspace::Entry>> {
    let p = state.project().map_err(anyhow_err)?;
    let dir = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    workspace::list_dir(&p.root, &dir).map_err(anyhow_err)
}

#[tauri::command]
async fn list_files(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    let p = state.project().map_err(anyhow_err)?;
    tauri::async_runtime::spawn_blocking(move || workspace::list_files(&p.root)).await.map_err(err)
}

#[tauri::command]
async fn create_path(state: State<'_, AppState>, path: String, dir: bool) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    if dir { workspace::create_dir(&abs) } else { workspace::create_file(&abs) }.map_err(anyhow_err)
}

#[tauri::command]
async fn rename_path(state: State<'_, AppState>, from: String, to: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let a = resolve_in_root(&p.root, &from).map_err(anyhow_err)?;
    let b = resolve_in_root(&p.root, &to).map_err(anyhow_err)?;
    workspace::rename(&a, &b).map_err(anyhow_err)
}

#[tauri::command]
async fn delete_path(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    if abs == p.root {
        return Err("프로젝트 폴더 자체는 지울 수 없습니다".into());
    }
    workspace::delete_to_trash(&abs).map_err(anyhow_err)
}

#[tauri::command]
async fn reveal_path(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    workspace::reveal_in_os(&abs).map_err(anyhow_err)
}

/// 소스 제어가 쓸 저장소 폴더. git은 저장소 설정(훅, fsmonitor 등)으로 명령을 실행할 수 있어서
/// 신뢰하지 않은 폴더(제한 모드)에서는 쓰지 않는다 (VS Code와 같음).
fn git_repo(state: &AppState, repo: &str) -> anyhow::Result<std::path::PathBuf> {
    let p = state.project()?;
    if !p.is_trusted() {
        anyhow::bail!("제한 모드에서는 소스 제어를 쓸 수 없습니다. 이 폴더를 신뢰하면 켜집니다");
    }
    gitops::resolve(&p.root, repo)
}

/// git 작업을 백그라운드 스레드에서 돈다 (네트워크 작업은 오래 걸릴 수 있다)
async fn git_blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?.map_err(anyhow_err)
}

#[tauri::command]
async fn git_repos(state: State<'_, AppState>) -> CmdResult<Vec<gitops::RepoSummary>> {
    let p = state.project().map_err(anyhow_err)?;
    if !p.is_trusted() {
        return Err(anyhow_err(anyhow::anyhow!("제한 모드에서는 소스 제어를 쓸 수 없습니다. 이 폴더를 신뢰하면 켜집니다")));
    }
    git_blocking(move || Ok(gitops::repos(&p.root))).await
}

#[tauri::command]
async fn git_status(state: State<'_, AppState>, repo: Option<String>) -> CmdResult<gitops::GitStatus> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    git_blocking(move || gitops::status(&dir)).await
}

#[tauri::command]
async fn git_diff(state: State<'_, AppState>, repo: Option<String>, path: String, staged: bool) -> CmdResult<String> {
    let dir = git_repo(&state, repo.as_deref().unwrap_or("")).map_err(anyhow_err)?;
    git_blocking(move || gitops::diff(&dir, &path, staged)).await
}

/// action: stage | unstage | discard | commit | init | fetch | pull | push
#[tauri::command]
async fn git_action(state: State<'_, AppState>, repo: Option<String>, action: String, paths: Vec<String>, message: Option<String>) -> CmdResult<String> {
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
async fn git_log(state: State<'_, AppState>, repo: String, skip: u32, limit: u32) -> CmdResult<Vec<gitops::Commit>> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::log(&dir, skip, limit)).await
}

#[tauri::command]
async fn git_commit_files(state: State<'_, AppState>, repo: String, hash: String) -> CmdResult<Vec<gitops::CommitFile>> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::commit_files(&dir, &hash)).await
}

#[tauri::command]
async fn git_commit_diff(state: State<'_, AppState>, repo: String, hash: String, path: String) -> CmdResult<String> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::commit_diff(&dir, &hash, &path)).await
}

#[tauri::command]
async fn git_branches(state: State<'_, AppState>, repo: String) -> CmdResult<Vec<gitops::Branch>> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::branches(&dir)).await
}

#[tauri::command]
async fn git_checkout(state: State<'_, AppState>, repo: String, name: String, create: bool, remote: bool) -> CmdResult<String> {
    let dir = git_repo(&state, &repo).map_err(anyhow_err)?;
    git_blocking(move || gitops::checkout(&dir, &name, create, remote)).await
}

#[tauri::command]
async fn search_text(state: State<'_, AppState>, query: String, case_sensitive: bool) -> CmdResult<Vec<workspace::FileMatches>> {
    let p = state.project().map_err(anyhow_err)?;
    tauri::async_runtime::spawn_blocking(move || workspace::search_text(&p.root, &query, case_sensitive))
        .await
        .map_err(err)
}

#[derive(serde::Serialize)]
struct ReplaceResult {
    count: usize,
    files: Vec<String>,
    checkpoint: Option<String>,
}

/// 검색 뷰의 바꾸기. AI 작업처럼 체크포인트를 남겨 한 번에 되돌릴 수 있다.
#[tauri::command]
async fn replace_text(
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
async fn read_file(state: State<'_, AppState>, path: String) -> CmdResult<String> {
    let p = state.project().map_err(anyhow_err)?;
    let abs = resolve_in_root(&p.root, &path).map_err(anyhow_err)?;
    workspace::read_text(&abs).map_err(anyhow_err)
}

#[tauri::command]
async fn write_file(app: AppHandle, state: State<'_, AppState>, path: String, content: String) -> CmdResult<()> {
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

/// 프로젝트 설정 파일과 예시(에이전트, 메모리)를 만들고 설정 파일 경로를 돌려준다.
#[tauri::command]
async fn ensure_config(state: State<'_, AppState>) -> CmdResult<String> {
    let p = state.project().map_err(anyhow_err)?;
    config::ensure_project_files(&p.root).map_err(anyhow_err)?;
    Ok(".lantern/config.toml".into())
}

/// 프로젝트 설정을 읽어도 되는 루트. 제한 모드면 None (전역 설정만 쓴다).
fn project_root(state: &State<'_, AppState>) -> Option<std::path::PathBuf> {
    state.project().ok().and_then(|p| p.config_root().map(|r| r.to_path_buf()))
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    settings::snapshot(project_root(&state).as_deref()).map_err(anyhow_err)
}

/// changes: [[점 경로, 값], ...]. 값이 null이면 그 항목을 지운다.
#[tauri::command]
async fn set_settings(state: State<'_, AppState>, scope: String, changes: Vec<(String, serde_json::Value)>) -> CmdResult<()> {
    settings::set_setting(&scope, project_root(&state).as_deref(), &changes).map_err(anyhow_err)
}

fn key_name_for(state: &State<'_, AppState>, model_key: &str) -> Result<String, String> {
    let cfg = config::load(project_root(state).as_deref()).map_err(anyhow_err)?;
    let m = cfg.models.get(model_key).ok_or_else(|| format!("모델 '{model_key}'이 없습니다"))?;
    m.api_key_env.clone().ok_or_else(|| "이 모델은 API 키 이름(api_key_env)이 없습니다".to_string())
}

#[tauri::command]
async fn set_api_key(state: State<'_, AppState>, model_key: String, key: String) -> CmdResult<()> {
    let name = key_name_for(&state, &model_key)?;
    settings::set_api_key(&name, &key).map_err(anyhow_err)
}

#[tauri::command]
async fn delete_api_key(state: State<'_, AppState>, model_key: String) -> CmdResult<()> {
    let name = key_name_for(&state, &model_key)?;
    settings::delete_api_key(&name).map_err(anyhow_err)
}

#[tauri::command]
async fn test_model(state: State<'_, AppState>, model_key: String) -> CmdResult<settings::TestResult> {
    let cfg = config::load(project_root(&state).as_deref()).map_err(anyhow_err)?;
    let m = cfg.models.get(&model_key).ok_or_else(|| format!("모델 '{model_key}'이 없습니다"))?;
    Ok(settings::test_model(&state.http, m).await)
}

#[tauri::command]
async fn probe_local(state: State<'_, AppState>) -> CmdResult<Vec<settings::LocalServer>> {
    Ok(settings::probe_local(&state.http).await)
}

#[tauri::command]
async fn list_agents(state: State<'_, AppState>) -> CmdResult<Vec<agents::AgentDef>> {
    let root = project_root(&state);
    Ok(agents::load(root.as_deref()))
}

#[tauri::command]
async fn model_info(state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    let root = project_root(&state);
    let cfg = config::load(root.as_deref()).map_err(anyhow_err)?;
    let models: Vec<_> = cfg
        .models
        .iter()
        .map(|(k, m)| json!({ "key": k, "provider": m.provider, "model": m.model, "has_key": m.provider != "anthropic" || m.resolve_api_key().is_some() }))
        .collect();
    let month = state::this_month_usage();
    Ok(json!({
        "default": cfg.routing.default,
        "models": models,
        "month": state::current_month(),
        "usage": month,
        "limit_usd": cfg.budget.monthly_usd_limit,
        "warn_at_percent": cfg.budget.warn_at_percent,
    }))
}

#[tauri::command]
async fn agent_send(
    app: AppHandle,
    session: String,
    agent: String,
    text: String,
    file: Option<String>,
    line: Option<u32>,
) -> CmdResult<()> {
    tauri::async_runtime::spawn(agent::run(app, session, agent, text, file, line));
    Ok(())
}

#[tauri::command]
async fn agent_cancel(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    if let Some(c) = state.cancels.lock().unwrap().get(&session) {
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
async fn agent_reset(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    state.sessions.lock().unwrap().remove(&session);
    Ok(())
}

#[tauri::command]
async fn resolve_approval(app: AppHandle, state: State<'_, AppState>, session: String, id: String, approved: bool) -> CmdResult<()> {
    if let Some(tx) = state.approvals.lock().unwrap().remove(&id) {
        let _ = tx.send(approved);
    }
    let _ = app.emit("agent", agent::AgentEvent::ApprovalResolved { session, id, approved });
    Ok(())
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// 새 버전이 있으면 알려준다. 배포 주소가 아직 없거나 오프라인이면 오류 대신 None.
#[tauri::command]
async fn check_update(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(err)?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo { version: u.version.clone(), notes: u.body.clone() })),
        Ok(None) => Ok(None),
        Err(e) => {
            applog::info(&format!("업데이트 확인 실패: {e}"));
            Err(format!("업데이트 서버에 연결하지 못했습니다: {e}"))
        }
    }
}

/// 내려받아 설치하고 다시 시작한다 (서명이 맞지 않으면 설치하지 않는다).
#[tauri::command]
async fn install_update(app: AppHandle) -> CmdResult<()> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(err)?.check().await.map_err(err)?.ok_or("설치할 새 버전이 없습니다")?;
    applog::info(&format!("업데이트 설치: {}", update.version));
    update.download_and_install(|_, _| {}, || {}).await.map_err(err)?;
    app.restart();
}

/// 프런트엔드 오류·경고를 앱 로그에 남긴다
#[tauri::command]
fn log_frontend(level: String, message: String) {
    let level = if level == "error" { "UI-ERROR" } else { "UI" };
    applog::write(level, &message.chars().take(4000).collect::<String>());
}

#[tauri::command]
fn problem_report() -> String {
    applog::report()
}

#[tauri::command]
fn pending_crash() -> Option<String> {
    applog::take_pending_crash()
}

#[tauri::command]
fn open_log_dir() -> CmdResult<()> {
    let dir = applog::log_dir().ok_or("데이터 폴더를 찾지 못했습니다")?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let target = if dir.join("lantern.log").exists() { dir.join("lantern.log") } else { dir };
    workspace::reveal_in_os(&target).map_err(anyhow_err)
}

fn keybindings_file() -> CmdResult<std::path::PathBuf> {
    state::data_dir().map(|d| d.join("keybindings.json")).ok_or_else(|| "데이터 폴더를 찾지 못했습니다".to_string())
}

/// (파일 경로, 내용). 파일이 없으면 빈 배열
#[tauri::command]
fn get_keybindings() -> CmdResult<(String, String)> {
    let path = keybindings_file()?;
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "[]".into());
    Ok((path.to_string_lossy().into_owned(), text))
}

#[tauri::command]
fn set_keybindings(json: String) -> CmdResult<()> {
    let v: serde_json::Value = serde_json::from_str(&json).map_err(err)?;
    if !v.is_array() {
        return Err("단축키 설정은 배열이어야 합니다".into());
    }
    let path = keybindings_file()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    std::fs::write(path, json).map_err(err)
}

/// 인라인 자동 완성. 꺼져 있으면 None
#[tauri::command]
async fn inline_complete(state: State<'_, AppState>, path: String, prefix: String, suffix: String) -> CmdResult<Option<String>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let Some(model) = completion::model(&cfg) else { return Ok(None) };
    let limit = cfg.budget.monthly_usd_limit;
    if limit > 0.0 && state::this_month_usage().cost_usd >= limit {
        return Ok(None);
    }
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(prev) = state.completion_cancel.lock().unwrap().replace(cancel.clone()) {
        prev.store(true, Ordering::Relaxed);
    }
    let (text, usage) = completion::complete(&state.http, &model, &path, &prefix, &suffix, &cancel).await.map_err(anyhow_err)?;
    state::record_usage(usage.cost(&model).unwrap_or(0.0), usage.input_tokens, usage.output_tokens);
    Ok(if cancel.load(Ordering::Relaxed) { None } else { Some(text) })
}

// ── 작업 저장과 격리 ───────────────────────────────────────

#[tauri::command]
fn task_list(state: State<'_, AppState>) -> CmdResult<Vec<serde_json::Value>> {
    let p = state.project().map_err(anyhow_err)?;
    tasks::list(&p.root).map_err(anyhow_err)
}

#[tauri::command]
fn task_save(state: State<'_, AppState>, id: String, data: serde_json::Value) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    tasks::save_ui(&p.root, &id, &data).map_err(anyhow_err)
}

/// 작업을 지운다. 격리 작업 공간이 있으면 함께 지운다.
#[tauri::command]
async fn task_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
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
async fn worktree_create(state: State<'_, AppState>, session: String) -> CmdResult<worktree::Worktree> {
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
struct WorktreeChanges {
    files: Vec<String>,
    diff: String,
}

#[tauri::command]
async fn worktree_changes(state: State<'_, AppState>, session: String) -> CmdResult<WorktreeChanges> {
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
async fn worktree_apply(app: AppHandle, state: State<'_, AppState>, session: String) -> CmdResult<Vec<String>> {
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
async fn worktree_discard(state: State<'_, AppState>, session: String) -> CmdResult<()> {
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
async fn revert_checkpoint(state: State<'_, AppState>, id: String, force: Option<bool>) -> CmdResult<usize> {
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

// ── 지식그래프 ──────────────────────────────────────────────

/// 그래프 질의를 색인 잠금 안에서 실행한다 (먼저 증분 갱신)
async fn with_engine<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&lantern_context::Engine, &std::path::Path) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let p = state.project().map_err(anyhow_err)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut e = p.engine.lock().unwrap();
        let _ = e.refresh();
        f(&e, &p.root)
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)
}

#[tauri::command]
async fn graph_overview(state: State<'_, AppState>, max_nodes: Option<usize>) -> CmdResult<lantern_context::graph::Graph> {
    let n = max_nodes.unwrap_or(400);
    with_engine(&state, move |e, _| lantern_context::graph::overview(&e.store, n)).await
}

#[tauri::command]
async fn graph_neighborhood(state: State<'_, AppState>, center: String, limit: Option<usize>) -> CmdResult<lantern_context::graph::Graph> {
    let n = limit.unwrap_or(30);
    with_engine(&state, move |e, _| lantern_context::graph::neighborhood(&e.store, &center, n)).await
}

#[tauri::command]
async fn graph_impact(state: State<'_, AppState>, path: String, ranges: Vec<(u32, u32)>) -> CmdResult<lantern_context::graph::Impact> {
    with_engine(&state, move |e, _| lantern_context::graph::impact(&e.store, &path, &ranges)).await
}

#[tauri::command]
async fn graph_memory(state: State<'_, AppState>) -> CmdResult<(Vec<lantern_context::graph::MemoryNote>, lantern_context::graph::Graph)> {
    with_engine(&state, move |e, root| lantern_context::graph::memory(&e.store, root)).await
}

/// 언어 서버 참조 위치들 → 각 위치를 감싼 심볼
#[tauri::command]
async fn graph_enclosing(state: State<'_, AppState>, locations: Vec<(String, u32)>) -> CmdResult<Vec<Option<lantern_context::graph::Node>>> {
    with_engine(&state, move |e, _| locations.iter().map(|(p, l)| lantern_context::graph::enclosing(&e.store, p, *l)).collect()).await
}

#[tauri::command]
async fn graph_find(state: State<'_, AppState>, query: String) -> CmdResult<Vec<lantern_context::graph::Node>> {
    with_engine(&state, move |e, _| lantern_context::graph::find(&e.store, &query, 12)).await
}

#[tauri::command]
async fn context_preview(
    state: State<'_, AppState>,
    query: String,
    file: Option<String>,
    line: Option<u32>,
) -> CmdResult<serde_json::Value> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let req = ContextRequest { query, file, line, budget_tokens: cfg.context.budget_tokens };
    tauri::async_runtime::spawn_blocking(move || {
        let mut e = p.engine.lock().unwrap();
        e.refresh()?;
        let r = e.context(&req)?;
        Ok::<_, anyhow::Error>(json!({ "markdown": r.to_markdown(), "used_tokens": r.used_tokens, "elapsed_ms": r.elapsed_ms, "items": r.items.len() }))
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)
}

#[tauri::command]
async fn term_spawn(app: AppHandle, state: State<'_, AppState>, cols: u16, rows: u16) -> CmdResult<u32> {
    let root = state
        .project()
        .map(|p| p.root.clone())
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default());
    let (id, term) = terminal::spawn(app, &root, cols, rows).map_err(anyhow_err)?;
    state.terminals.lock().unwrap().insert(id, term);
    Ok(id)
}

#[tauri::command]
async fn term_write(state: State<'_, AppState>, id: u32, data: String) -> CmdResult<()> {
    if let Some(t) = state.terminals.lock().unwrap().get_mut(&id) {
        t.write(&data).map_err(anyhow_err)?;
    }
    Ok(())
}

#[tauri::command]
async fn term_resize(state: State<'_, AppState>, id: u32, cols: u16, rows: u16) -> CmdResult<()> {
    if let Some(t) = state.terminals.lock().unwrap().get(&id) {
        t.resize(cols, rows).map_err(anyhow_err)?;
    }
    Ok(())
}

#[tauri::command]
async fn term_kill(state: State<'_, AppState>, id: u32) -> CmdResult<()> {
    if let Some(mut t) = state.terminals.lock().unwrap().remove(&id) {
        t.kill();
    }
    Ok(())
}

/// 언어 서버 시작. 이미 떠 있으면 그대로 둔다. 설정이 없거나 설치되지 않았으면 오류.
#[tauri::command]
async fn lsp_start(app: AppHandle, state: State<'_, AppState>, lang: String) -> CmdResult<()> {
    if state.lsps.lock().unwrap().contains_key(&lang) {
        return Ok(());
    }
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let l = cfg.lsp.get(&lang).ok_or_else(|| format!("[lsp.{lang}] 설정이 없습니다"))?;
    let server = lsp::start(app, &lang, l, &p.root, &file_uri(&p.root)).map_err(anyhow_err)?;
    state.lsps.lock().unwrap().insert(lang, server);
    Ok(())
}

/// 언어 서버를 찾지 못했을 때 설치 방법. 이미 있으면 None (ignore_found면 있어도 계획을 준다: rustup 셈처럼 껍데기만 있는 경우)
#[tauri::command]
async fn lsp_install_plan(state: State<'_, AppState>, lang: String, ignore_found: bool) -> CmdResult<Option<lspinstall::InstallPlan>> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let Some(l) = cfg.lsp.get(&lang) else { return Ok(None) };
    let found = which::which(&l.command).is_ok() || lspinstall::installed_bin(&l.command).is_some();
    if found && !ignore_found {
        return Ok(None);
    }
    Ok(lspinstall::plan(&l.command))
}

#[tauri::command]
async fn lsp_install(state: State<'_, AppState>, lang: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let command = cfg.lsp.get(&lang).ok_or_else(|| format!("[lsp.{lang}] 설정이 없습니다"))?.command.clone();
    tauri::async_runtime::spawn_blocking(move || lspinstall::install(&command))
        .await
        .map_err(err)?
        .map_err(|e| {
            applog::error(&format!("언어 서버 설치 실패: {e:#}"));
            anyhow_err(e)
        })
}

#[tauri::command]
async fn lsp_send(state: State<'_, AppState>, lang: String, msg: String) -> CmdResult<()> {
    let mut lsps = state.lsps.lock().unwrap();
    let l = lsps.get_mut(&lang).ok_or("언어 서버가 실행 중이 아닙니다")?;
    l.send(&msg).map_err(anyhow_err)
}

fn main() {
    applog::install_panic_hook();
    // 예전 이름(KHALA)의 데이터를 먼저 옮긴다 (로그도 새 자리에 쌓이도록)
    let moved = state::migrate_legacy_data();
    tasks::prune_checkpoints();
    applog::info(&format!("Lantern {} 시작", env!("CARGO_PKG_VERSION")));
    for m in moved {
        applog::info(&format!("예전 데이터 옮김 — {m}"));
    }
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .expect("HTTP 클라이언트");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState { http, ..Default::default() })
        .setup(|app| {
            // 창은 코드에서 만든다. LANTERN_WEBVIEW_DATA_DIR을 주면 WebView 데이터(localStorage 등)를
            // 별도 폴더에 두어, 이미 실행 중인 Lantern과 충돌 없이 두 번째 인스턴스를 띄울 수 있다.
            let mut builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Lantern")
                .inner_size(1440.0, 900.0)
                .min_inner_size(900.0, 560.0)
                .shadow(true)
                .background_color(tauri::window::Color(0x18, 0x18, 0x18, 0xff));
            // Windows·Linux는 제목 표시줄을 직접 그린다 (창 조작 버튼 포함).
            // macOS는 시스템 신호등 버튼을 제목 표시줄 위에 겹쳐 두고, 그 자리만 비운다 (styles.css의 .mac).
            #[cfg(not(target_os = "macos"))]
            {
                builder = builder.decorations(false);
            }
            #[cfg(target_os = "macos")]
            {
                builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
            }
            if let Some(dir) = std::env::var_os("LANTERN_WEBVIEW_DATA_DIR") {
                builder = builder.data_directory(dir.into());
            }
            // E2E 전용: 원격 디버깅 포트를 연다. WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 환경변수는
            // WebView2 런타임 버전에 따라 Tauri가 넘기는 인자에 밀려 무시돼서(CI) 인자로 직접 준다.
            // Tauri 기본 인자를 대신하므로 그 값(--disable-features=...)도 함께 넘긴다.
            #[cfg(windows)]
            if let Some(port) = std::env::var("LANTERN_E2E_CDP_PORT").ok().filter(|p| p.parse::<u16>().is_ok()) {
                builder = builder.additional_browser_args(&format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
                ));
            }
            builder.build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_project,
            set_trust,
            startup_path,
            list_dir,
            list_files,
            create_path,
            rename_path,
            delete_path,
            reveal_path,
            search_text,
            replace_text,
            git_status,
            git_diff,
            git_action,
            git_repos,
            git_log,
            git_commit_files,
            git_commit_diff,
            git_branches,
            git_checkout,
            read_file,
            write_file,
            ensure_config,
            get_settings,
            set_settings,
            set_api_key,
            delete_api_key,
            test_model,
            probe_local,
            list_agents,
            model_info,
            agent_send,
            agent_cancel,
            agent_reset,
            resolve_approval,
            revert_checkpoint,
            task_list,
            task_save,
            task_delete,
            worktree_create,
            worktree_changes,
            worktree_apply,
            worktree_discard,
            graph_overview,
            graph_neighborhood,
            graph_impact,
            graph_memory,
            graph_find,
            graph_enclosing,
            inline_complete,
            get_keybindings,
            log_frontend,
            check_update,
            install_update,
            problem_report,
            pending_crash,
            open_log_dir,
            set_keybindings,
            context_preview,
            term_spawn,
            term_write,
            term_resize,
            term_kill,
            lsp_start,
            lsp_send,
            lsp_install_plan,
            lsp_install,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let st = window.app_handle().state::<AppState>();
                for (_, mut t) in st.terminals.lock().unwrap().drain() {
                    t.kill();
                }
                for (_, mut l) in st.lsps.lock().unwrap().drain() {
                    l.stop();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Lantern 실행 실패");
}
