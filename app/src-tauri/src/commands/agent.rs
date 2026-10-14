//! Tauri 명령: 에이전트 대화·승인·외부 에이전트 로그인·Ctrl+K·자동 완성

use crate::state::AppState;
use crate::{acp, agent, agents, completion, config, inline, llm, mentions, semantic, state};
use crate::{anyhow_err, assemble_rel, err, project_root, with_engine, CmdResult};
use lantern_context::assemble::ContextRequest;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) async fn list_agents(state: State<'_, AppState>) -> CmdResult<Vec<agents::AgentDef>> {
    let root = project_root(&state);
    let mut list = agents::load(root.as_deref());
    // 외부 에이전트 (Gemini CLI, Codex 등). 설정이 깨졌으면 내장 목록만
    let cfg = config::load(root.as_deref()).unwrap_or_else(|_| toml::from_str(config::DEFAULT_CONFIG).expect("기본 설정"));
    list.extend(acp::agent_defs(&cfg));
    Ok(list)
}

/// 외부 에이전트 로그인. 대개 에이전트가 브라우저를 열고, 로그인이 끝나면 돌아온다.
#[tauri::command]
pub(crate) async fn acp_authenticate(app: AppHandle, session: String, agent: String, method: String) -> CmdResult<()> {
    acp::authenticate(&app, &session, &agent, &method).await.map_err(anyhow_err)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn agent_send(
    app: AppHandle,
    session: String,
    agent: String,
    text: String,
    images: Option<Vec<llm::ImageInput>>,
    mentions: Option<Vec<mentions::Mention>>,
    plan: Option<bool>,
    file: Option<String>,
    line: Option<u32>,
) -> CmdResult<()> {
    let images = images.unwrap_or_default();
    llm::check_images(&images).map_err(err)?;
    let mentions = mentions.unwrap_or_default();
    if agent.starts_with(acp::PREFIX) {
        tauri::async_runtime::spawn(acp::run(app, session, agent, text, images, mentions, file, line));
    } else {
        tauri::async_runtime::spawn(agent::run(app, session, agent, text, images, mentions, plan.unwrap_or(false), file, line));
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn agent_cancel(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    if let Some(c) = state.cancels.lock().unwrap().get(&session) {
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn agent_reset(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    state.sessions.lock().unwrap().remove(&session);
    acp::close(&state, Some(&session));
    Ok(())
}

#[tauri::command]
pub(crate) async fn resolve_approval(app: AppHandle, state: State<'_, AppState>, session: String, id: String, approved: bool) -> CmdResult<()> {
    if let Some(tx) = state.approvals.lock().unwrap().remove(&id) {
        let _ = tx.send(approved);
    }
    let _ = app.emit("agent", agent::AgentEvent::ApprovalResolved { session, id, approved });
    Ok(())
}

/// 인라인 자동 완성. 꺼져 있으면 None
#[tauri::command]
pub(crate) async fn inline_complete(state: State<'_, AppState>, path: String, prefix: String, suffix: String) -> CmdResult<Option<String>> {
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

/// 편집기 안 즉시 수정 (Ctrl+K). `before`·`selection`·`after`는 편집기 내용(저장 전 포함)을 선택 기준으로 나눈 것
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn inline_edit(
    state: State<'_, AppState>,
    path: String,
    before: String,
    selection: String,
    after: String,
    instruction: String,
    line: u32,
) -> CmdResult<inline::InlineEdit> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    agent::check_budget(&cfg).map_err(anyhow_err)?;
    let rel = assemble_rel(&p.root, &path);
    // 지시에 맞는 코드를 맥락 엔진이 조금 붙인다 (예산은 에이전트보다 작게)
    let engine = p.engine.clone();
    let proj = p.clone();
    let req = ContextRequest { query: instruction.clone(), file: Some(rel.clone()), line: Some(line), budget_tokens: (cfg.context.budget_tokens / 3).max(1500) };
    let emb = cfg.embedder();
    let context = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let q = semantic::query_vector(emb.as_ref(), &req.query);
        let mut e = engine.lock().unwrap();
        proj.refresh_index(&mut e)?;
        Ok(semantic::context(&e, &req, q.as_ref())?.to_markdown())
    })
    .await
    .map_err(err)?
    .unwrap_or_default();
    inline::edit(&state.http, &cfg, &rel, &before, &selection, &after, &instruction, &context).await.map_err(anyhow_err)
}

/// 테스트 만들기에 붙일 예시: 같은 언어 계열의 기존 테스트 파일 중 대상 파일과 경로가 가까운 것부터 3개
#[tauri::command]
pub(crate) async fn test_examples(state: State<'_, AppState>, path: String) -> CmdResult<Vec<String>> {
    with_engine(&state, move |e, root| {
        let rel = assemble_rel(root, &path);
        use lantern_context::lang::Lang;
        let Some(target) = Lang::from_path(std::path::Path::new(&rel)).map(|l| Lang::family(l.name()).to_string()) else {
            return Ok(Vec::new());
        };
        let shared = |p: &str| rel.split('/').zip(p.split('/')).take_while(|(a, b)| a == b).count();
        let mut tests: Vec<String> = e
            .store
            .file_summaries()?
            .into_iter()
            .filter(|(p, lang, _)| Lang::family(lang) == target && lantern_context::graph::is_test_path(p))
            .map(|(p, _, _)| p)
            .collect();
        tests.sort_by(|a, b| shared(b).cmp(&shared(a)).then(a.len().cmp(&b.len())).then(a.cmp(b)));
        tests.truncate(3);
        Ok(tests)
    })
    .await
}
