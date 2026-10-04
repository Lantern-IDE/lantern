//! 의미 검색 (선택): 코드를 40줄 조각으로 나눠 임베딩하고, 질문과 뜻이 가까운 파일을 찾는다.
//! 키워드가 겹치지 않는 질문(특히 한국어)에서 맥락 조립의 후보를 보탠다. 임베딩 서버를 설정했을 때만 켜진다.
//!
//! 벡터는 (모델, 조각 내용 해시)로 저장해 브랜치를 바꾸거나 다시 인덱싱해도 바뀐 조각만 새로 만든다.

use crate::store::Store;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// 조각 길이 (줄). 공개 벤치마크로 잰 값 (#67)
pub const CHUNK_LINES: usize = 40;
const CHUNK_CHARS: usize = 1600;
const BATCH: usize = 32;

/// OpenAI 호환 `/embeddings` (OpenAI, Ollama, LM Studio, vLLM 등)
#[derive(Debug, Clone)]
pub struct Embedder {
    /// 예: `https://api.openai.com/v1`, `http://localhost:11434/v1`
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    /// 질문 앞뒤에 붙이는 형식 (`{query}`). 모델이 권하는 지시문이 있으면 넣는다
    pub query_template: String,
    /// 조각 형식 (`{path}`, `{text}`)
    pub doc_template: String,
}

impl Embedder {
    /// 환경변수로 (CLI·벤치마크): LANTERN_EMBED_URL, LANTERN_EMBED_MODEL, LANTERN_EMBED_KEY,
    /// LANTERN_EMBED_QUERY, LANTERN_EMBED_DOC
    pub fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Some(Self {
            base_url: var("LANTERN_EMBED_URL")?,
            model: var("LANTERN_EMBED_MODEL")?,
            api_key: var("LANTERN_EMBED_KEY"),
            query_template: var("LANTERN_EMBED_QUERY").unwrap_or_else(|| "{query}".into()),
            doc_template: var("LANTERN_EMBED_DOC").unwrap_or_else(|| "{path}\n{text}".into()),
        })
    }

    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let mut req = ureq::post(&url).timeout(Duration::from_secs(120));
        if let Some(k) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
        let resp: serde_json::Value = req
            .send_json(serde_json::json!({ "model": self.model, "input": texts }))
            .map_err(|e| match e {
                ureq::Error::Status(code, r) => anyhow::anyhow!("임베딩 서버 오류 {code}: {}", r.into_string().unwrap_or_default().chars().take(300).collect::<String>()),
                e => anyhow::anyhow!("임베딩 서버에 연결하지 못했습니다: {e}"),
            })?
            .into_json()
            .context("임베딩 응답을 읽지 못했습니다")?;
        let data = resp["data"].as_array().context("임베딩 응답에 data가 없습니다")?;
        let mut out = vec![Vec::new(); texts.len()];
        for d in data {
            let i = d["index"].as_u64().unwrap_or(0) as usize;
            let v: Vec<f32> = d["embedding"].as_array().context("embedding이 없습니다")?.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            if i < out.len() {
                out[i] = normalize(v);
            }
        }
        if out.iter().any(|v| v.is_empty()) {
            bail!("임베딩 응답의 개수가 요청과 다릅니다");
        }
        Ok(out)
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        Ok(self.embed(&[self.query_template.replace("{query}", query)])?.remove(0))
    }

    /// 조각 글. 임베딩 서버로 나가므로 비밀로 보이는 값은 가린다
    fn doc(&self, path: &str, text: &str) -> String {
        self.doc_template.replace("{path}", path).replace("{text}", &crate::secrets::redact(text).0)
    }
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// 파일을 조각으로: (시작 줄 1부터, 본문)
pub fn chunks(text: &str) -> Vec<(u32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    lines
        .chunks(CHUNK_LINES)
        .enumerate()
        .filter_map(|(i, c)| {
            let body = c.join("\n");
            let body: String = body.chars().take(CHUNK_CHARS).collect();
            (!body.trim().is_empty()).then(|| ((i * CHUNK_LINES + 1) as u32, body))
        })
        .collect()
}

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex()[..32].to_string()
}

/// 조각 목록을 인덱스와 맞추고, 벡터가 없는 조각을 최대 `limit`개 돌려준다 (해시, 임베딩할 글).
/// 임베딩(느린 HTTP)은 인덱스 잠금 밖에서 하도록 준비와 저장을 나눈다.
pub fn pending(store: &Store, root: &Path, emb: &Embedder, limit: usize) -> Result<Vec<(String, String)>> {
    // 1. 바뀐 파일만 다시 나눈다
    let files = store.files()?;
    let chunked = store.chunked_files()?;
    store.begin()?;
    let r = (|| -> Result<()> {
        for path in chunked.keys().filter(|p| !files.contains_key(*p)) {
            store.remove_chunks(path)?;
        }
        for (path, f) in &files {
            if chunked.get(path) == Some(&f.hash) {
                continue;
            }
            let Ok(text) = std::fs::read(root.join(path)) else { continue };
            let text = String::from_utf8_lossy(&text);
            let list: Vec<(u32, String)> = chunks(&text).into_iter().map(|(l, b)| (l, hash(&emb.doc(path, &b)))).collect();
            store.replace_chunks(path, &f.hash, &list)?;
        }
        Ok(())
    })();
    match r {
        Ok(()) => store.commit()?,
        Err(e) => {
            store.rollback();
            return Err(e);
        }
    }
    // 2. 벡터가 없는 조각의 글
    let mut sources: HashMap<String, Option<String>> = HashMap::new();
    Ok(store
        .chunks_without_vectors(&emb.model, limit)?
        .into_iter()
        .map(|(path, line, h)| {
            let src = sources.entry(path.clone()).or_insert_with(|| std::fs::read(root.join(&path)).ok().map(|b| String::from_utf8_lossy(&b).into_owned()));
            let body = src.as_deref().and_then(|s| chunks(s).into_iter().find(|(l, _)| *l == line)).map(|(_, b)| b).unwrap_or_default();
            (h, emb.doc(&path, &body))
        })
        .collect())
}

pub fn save(store: &Store, model: &str, items: &[(String, Vec<f32>)]) -> Result<()> {
    store.begin()?;
    for (h, v) in items {
        if let Err(e) = store.put_vector(model, h, v) {
            store.rollback();
            return Err(e);
        }
    }
    store.commit()
}

/// 빠진 벡터를 최대 `max_new`개 만든다 (CLI처럼 잠금이 필요 없을 때). (벡터가 있는 조각, 전체 조각)
pub fn update(store: &Store, root: &Path, emb: &Embedder, max_new: usize) -> Result<(i64, i64)> {
    let mut left = max_new;
    while left > 0 {
        let todo = pending(store, root, emb, left.min(BATCH))?;
        if todo.is_empty() {
            break;
        }
        let vecs = emb.embed(&todo.iter().map(|t| t.1.clone()).collect::<Vec<_>>())?;
        save(store, &emb.model, &todo.into_iter().map(|t| t.0).zip(vecs).collect::<Vec<_>>())?;
        left = left.saturating_sub(BATCH);
    }
    store.count_chunks(&emb.model)
}

/// 질문과 뜻이 가까운 파일 순서 (파일 점수 = 가장 가까운 조각). 각 파일에서 가장 가까운 조각의 시작 줄도 준다
pub fn rank_files(store: &Store, model: &str, query: &[f32], limit: usize) -> Result<Vec<(String, u32, f32)>> {
    let mut best: HashMap<String, (u32, f32)> = HashMap::new();
    for (path, line, v) in store.chunk_vectors(model)? {
        if v.len() != query.len() {
            continue;
        }
        let s: f32 = v.iter().zip(query).map(|(a, b)| a * b).sum();
        let e = best.entry(path).or_insert((line, f32::MIN));
        if s > e.1 {
            *e = (line, s);
        }
    }
    let mut v: Vec<(String, u32, f32)> = best.into_iter().map(|(p, (l, s))| (p, l, s)).collect();
    v.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    v.truncate(limit);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_into_line_chunks() {
        let text: String = (1..=85).map(|i| format!("line {i}\n")).collect();
        let c = chunks(&text);
        assert_eq!(c.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 41, 81]);
        assert!(c[0].1.starts_with("line 1\n") && c[0].1.ends_with("line 40"));
        assert!(chunks("\n\n  \n").is_empty());
        // 긴 줄은 글자 수로 자르되 문자 중간에서 끊지 않는다
        let long = "가".repeat(5000);
        assert_eq!(chunks(&long)[0].1.chars().count(), CHUNK_CHARS);
    }

    #[test]
    fn ranks_files_by_best_chunk() {
        let store = Store::open_in_memory().unwrap();
        store.replace_chunks("a.rs", "h1", &[(1, "x1".into()), (41, "x2".into())]).unwrap();
        store.replace_chunks("b.rs", "h2", &[(1, "y1".into())]).unwrap();
        store.put_vector("m", "x1", &[1.0, 0.0]).unwrap();
        store.put_vector("m", "x2", &[0.6, 0.8]).unwrap();
        store.put_vector("m", "y1", &[0.8, 0.6]).unwrap();
        let r = rank_files(&store, "m", &[0.0, 1.0], 10).unwrap();
        assert_eq!(r[0].0, "a.rs");
        assert_eq!(r[0].1, 41, "가장 가까운 조각의 시작 줄");
        assert_eq!(r[1].0, "b.rs");
        assert!(rank_files(&store, "other", &[0.0, 1.0], 10).unwrap().is_empty(), "다른 모델 벡터는 쓰지 않는다");
        assert_eq!(store.count_chunks("m").unwrap(), (3, 3));
        assert_eq!(store.chunks_without_vectors("other", 10).unwrap().len(), 3);
    }
}
