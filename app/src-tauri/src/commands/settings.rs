//! Tauri 명령: 설정·API 키·모델 연결 확인·의미 검색·MCP 상태·단축키

use crate::state::AppState;
use crate::{config, mcp_client, settings, state};
use crate::{anyhow_err, err, key_name_for, project_root, CmdResult};
use serde_json::json;
use tauri::State;

/// 프로젝트 설정 파일과 예시(에이전트, 메모리)를 만들고 설정 파일 경로를 돌려준다.
#[tauri::command]
pub(crate) async fn ensure_config(state: State<'_, AppState>) -> CmdResult<String> {
    let p = state.project().map_err(anyhow_err)?;
    config::ensure_project_files(&p.root).map_err(anyhow_err)?;
    Ok(".lantern/config.toml".into())
}

#[tauri::command]
pub(crate) async fn get_settings(state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    settings::snapshot(project_root(&state).as_deref()).map_err(anyhow_err)
}

/// changes: [[점 경로, 값], ...]. 값이 null이면 그 항목을 지운다.
#[tauri::command]
pub(crate) async fn set_settings(state: State<'_, AppState>, scope: String, changes: Vec<(String, serde_json::Value)>) -> CmdResult<()> {
    settings::set_setting(&scope, project_root(&state).as_deref(), &changes).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn set_api_key(state: State<'_, AppState>, model_key: String, key: String) -> CmdResult<()> {
    let name = key_name_for(&state, &model_key)?;
    settings::set_api_key(&name, &key).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn delete_api_key(state: State<'_, AppState>, model_key: String) -> CmdResult<()> {
    let name = key_name_for(&state, &model_key)?;
    settings::delete_api_key(&name).map_err(anyhow_err)
}

#[tauri::command]
pub(crate) async fn test_model(state: State<'_, AppState>, model_key: String) -> CmdResult<settings::TestResult> {
    let cfg = config::load(project_root(&state).as_deref()).map_err(anyhow_err)?;
    let m = cfg.models.get(&model_key).ok_or_else(|| format!("모델 '{model_key}'이 없습니다"))?;
    Ok(settings::test_model(&state.http, m).await)
}

#[tauri::command]
pub(crate) async fn probe_local(state: State<'_, AppState>) -> CmdResult<Vec<settings::LocalServer>> {
    Ok(settings::probe_local(&state.http).await)
}

/// 설정 화면: 설정한 MCP 서버를 띄워 보고 도구 목록이나 오류를 알린다
#[tauri::command]
pub(crate) async fn mcp_status(state: State<'_, AppState>) -> CmdResult<Vec<mcp_client::ServerStatus>> {
    let p = state.project().map_err(anyhow_err)?;
    if !p.is_trusted() {
        return Err("MCP 서버는 신뢰한 폴더에서만 씁니다".into());
    }
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    Ok(mcp_client::status(&state, &cfg, &p.root).await)
}

/// 의미 검색 연결 시험: 글 하나를 임베딩해 본다
#[tauri::command]
pub(crate) async fn test_embeddings(base_url: String, model: String, api_key_env: Option<String>) -> CmdResult<serde_json::Value> {
    let mut cfg: config::Config = toml::from_str(config::DEFAULT_CONFIG).map_err(err)?;
    cfg.embeddings = Some(config::EmbeddingsConfig {
        base_url,
        model,
        api_key_env: api_key_env.filter(|k| !k.is_empty()),
        api_key: None,
        query_template: "{query}".into(),
        doc_template: "{path}
{text}".into(),
    });
    let emb = cfg.embedder().ok_or("주소와 모델을 적어 주세요")?;
    let started = std::time::Instant::now();
    let v = tauri::async_runtime::spawn_blocking(move || emb.embed_query("where is the session cookie issued?"))
        .await
        .map_err(err)?
        .map_err(anyhow_err)?;
    Ok(json!({ "dim": v.len(), "ms": started.elapsed().as_millis() as u64 }))
}

/// 의미 검색 키를 OS 자격 증명 저장소에 (이름은 환경변수 이름, 모델 키와 같은 방식)
#[tauri::command]
pub(crate) async fn set_embeddings_key(env: String, key: String) -> CmdResult<()> {
    if env.is_empty() || !env.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return Err("키 이름은 OPENAI_API_KEY처럼 영문 대문자·숫자·_만 씁니다".into());
    }
    settings::set_api_key(&env, key.trim()).map_err(anyhow_err)
}

/// 의미 검색 준비 상태 (설정했을 때): 벡터를 만든 조각 / 전체 조각
#[tauri::command]
pub(crate) async fn semantic_status(state: State<'_, AppState>) -> CmdResult<Option<serde_json::Value>> {
    let Ok(p) = state.project() else { return Ok(None) };
    let Some(emb) = config::load(p.config_root()).ok().and_then(|c| c.embedder()) else { return Ok(None) };
    let (done, total) = p.engine.lock().unwrap().store.count_chunks(&emb.model).map_err(anyhow_err)?;
    Ok(Some(json!({ "done": done, "total": total })))
}

#[tauri::command]
pub(crate) async fn model_info(state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    let root = project_root(&state);
    let cfg = config::load(root.as_deref()).map_err(anyhow_err)?;
    let models: Vec<_> = cfg
        .models
        .iter()
        // 키를 쓰지 않는 모델(로컬 서버)은 키가 있는 것으로 친다. 키 이름이 있는 클라우드 모델은 실제로 찾아본다
        .map(|(k, m)| {
            let needs_key = m.provider == "anthropic" || m.api_key_env.is_some();
            json!({ "key": k, "provider": m.provider, "model": m.model, "has_key": !needs_key || m.resolve_api_key().is_some() })
        })
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

fn keybindings_file() -> CmdResult<std::path::PathBuf> {
    state::data_dir().map(|d| d.join("keybindings.json")).ok_or_else(|| "데이터 폴더를 찾지 못했습니다".to_string())
}

/// (파일 경로, 내용). 파일이 없으면 빈 배열
#[tauri::command]
pub(crate) fn get_keybindings() -> CmdResult<(String, String)> {
    let path = keybindings_file()?;
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "[]".into());
    Ok((path.to_string_lossy().into_owned(), text))
}

#[tauri::command]
pub(crate) fn set_keybindings(json: String) -> CmdResult<()> {
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
