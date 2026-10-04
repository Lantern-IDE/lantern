//! 의미 검색 (선택): config.toml에 [embeddings]가 있으면 맥락 조립에 곁들이고, 벡터는 뒤에서 조금씩 만든다.
//! 임베딩(HTTP)은 인덱스 잠금 밖에서 해서, 느린 로컬 서버라도 에이전트의 맥락 조립을 막지 않는다.

use crate::config;
use crate::state::{AppState, Project};
use lantern_context::assemble::{ContextRequest, ContextResult};
use lantern_context::semantic::{self, Embedder};
use lantern_context::Engine;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// 한 번에 임베딩하는 조각 수 (잠금을 오래 잡지 않게 작게)
const BATCH: usize = 16;

/// 질문 벡터. 설정이 없거나 서버에 닿지 않으면 None (키워드·그래프만으로 조립)
pub fn query_vector(emb: Option<&Embedder>, query: &str) -> Option<(String, Vec<f32>)> {
    let emb = emb?;
    match emb.embed_query(query) {
        Ok(v) => Some((emb.model.clone(), v)),
        Err(e) => {
            crate::applog::error(&format!("의미 검색 건너뜀: {e:#}"));
            None
        }
    }
}

pub fn context(e: &Engine, req: &ContextRequest, q: Option<&(String, Vec<f32>)>) -> anyhow::Result<ContextResult> {
    match q {
        Some((model, v)) => e.context_with_vector(req, model, v),
        None => e.context(req),
    }
}

/// 프로젝트를 여는 동안 빠진 벡터를 조금씩 만든다. 프로젝트가 바뀌면 끝난다.
/// 진행은 `semantic` 이벤트로 알린다: { done, total } 또는 { error }
pub fn spawn_background(app: AppHandle, project: Arc<Project>) {
    std::thread::spawn(move || {
        let mut last_error = String::new();
        loop {
            let current = app.state::<AppState>().project.lock().unwrap().clone();
            if !current.is_some_and(|p| Arc::ptr_eq(&p, &project)) {
                return;
            }
            let Some(emb) = config::load(project.config_root()).ok().and_then(|c| c.embedder()) else {
                std::thread::sleep(Duration::from_secs(30));
                continue;
            };
            let todo = {
                let e = project.engine.lock().unwrap();
                semantic::pending(&e.store, &e.root, &emb, BATCH)
            };
            let todo = match todo {
                Ok(t) => t,
                Err(e) => {
                    crate::applog::error(&format!("의미 검색 조각을 준비하지 못함: {e:#}"));
                    std::thread::sleep(Duration::from_secs(60));
                    continue;
                }
            };
            if todo.is_empty() {
                std::thread::sleep(Duration::from_secs(15));
                continue;
            }
            match emb.embed(&todo.iter().map(|t| t.1.clone()).collect::<Vec<_>>()) {
                Ok(vecs) => {
                    last_error.clear();
                    let e = project.engine.lock().unwrap();
                    let items: Vec<(String, Vec<f32>)> = todo.into_iter().map(|t| t.0).zip(vecs).collect();
                    let r = semantic::save(&e.store, &emb.model, &items).and_then(|_| e.store.count_chunks(&emb.model));
                    drop(e);
                    match r {
                        Ok((done, total)) => {
                            let _ = app.emit("semantic", json!({ "done": done, "total": total }));
                        }
                        Err(e) => crate::applog::error(&format!("의미 검색 벡터를 저장하지 못함: {e:#}")),
                    }
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    if msg != last_error {
                        crate::applog::error(&format!("의미 검색 임베딩 실패: {msg}"));
                        let _ = app.emit("semantic", json!({ "error": msg }));
                        last_error = msg;
                    }
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
        }
    });
}
