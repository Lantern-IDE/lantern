//! Tauri 명령: 코드 지도·영향 반경·맥락 미리보기

use crate::state::AppState;
use crate::{config, semantic};
use crate::{anyhow_err, err, with_engine, CmdResult};
use lantern_context::assemble::ContextRequest;
use serde_json::json;
use tauri::State;

#[tauri::command]
pub(crate) async fn graph_overview(state: State<'_, AppState>, max_nodes: Option<usize>) -> CmdResult<lantern_context::graph::Graph> {
    let n = max_nodes.unwrap_or(400);
    with_engine(&state, move |e, _| lantern_context::graph::overview(&e.store, n)).await
}

#[tauri::command]
pub(crate) async fn graph_neighborhood(state: State<'_, AppState>, center: String, limit: Option<usize>) -> CmdResult<lantern_context::graph::Graph> {
    let n = limit.unwrap_or(30);
    with_engine(&state, move |e, _| lantern_context::graph::neighborhood(&e.store, &center, n)).await
}

#[tauri::command]
pub(crate) async fn graph_impact(state: State<'_, AppState>, path: String, ranges: Vec<(u32, u32)>) -> CmdResult<lantern_context::graph::Impact> {
    with_engine(&state, move |e, _| lantern_context::graph::impact(&e.store, &path, &ranges)).await
}

#[tauri::command]
pub(crate) async fn graph_memory(state: State<'_, AppState>) -> CmdResult<(Vec<lantern_context::graph::MemoryNote>, lantern_context::graph::Graph)> {
    with_engine(&state, move |e, root| lantern_context::graph::memory(&e.store, root)).await
}

/// 언어 서버 참조 위치들 → 각 위치를 감싼 심볼
#[tauri::command]
pub(crate) async fn graph_enclosing(state: State<'_, AppState>, locations: Vec<(String, u32)>) -> CmdResult<Vec<Option<lantern_context::graph::Node>>> {
    with_engine(&state, move |e, _| locations.iter().map(|(p, l)| lantern_context::graph::enclosing(&e.store, p, *l)).collect()).await
}

#[tauri::command]
pub(crate) async fn graph_find(state: State<'_, AppState>, query: String) -> CmdResult<Vec<lantern_context::graph::Node>> {
    with_engine(&state, move |e, _| lantern_context::graph::find(&e.store, &query, 12)).await
}

#[tauri::command]
pub(crate) async fn context_preview(
    state: State<'_, AppState>,
    query: String,
    file: Option<String>,
    line: Option<u32>,
) -> CmdResult<serde_json::Value> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let req = ContextRequest { query, file, line, budget_tokens: cfg.context.budget_tokens };
    let emb = cfg.embedder();
    tauri::async_runtime::spawn_blocking(move || {
        let q = semantic::query_vector(emb.as_ref(), &req.query);
        let mut e = p.engine.lock().unwrap();
        p.refresh_index(&mut e)?;
        let r = semantic::context(&e, &req, q.as_ref())?;
        Ok::<_, anyhow::Error>(json!({ "markdown": r.to_markdown(), "used_tokens": r.used_tokens, "elapsed_ms": r.elapsed_ms, "items": r.items.len() }))
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)
}
