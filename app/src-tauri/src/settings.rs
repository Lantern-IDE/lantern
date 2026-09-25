//! 설정 화면용: 설정 읽기·쓰기(주석 보존), API 키 저장, 모델 연결 확인, 로컬 모델 서버 찾기.
//!
//! 설정의 원본은 언제나 TOML 파일이다. 화면은 그 파일을 편하게 고치는 수단이고,
//! 사용자가 직접 적은 주석과 순서는 그대로 둔다 (toml_edit).

use crate::config::{self, KEYRING_SERVICE};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use toml_edit::{Array, DocumentMut, Item, Table};

pub fn scope_path(scope: &str, root: Option<&Path>) -> Result<PathBuf> {
    match scope {
        "global" => config::global_path().context("홈 폴더를 찾을 수 없습니다"),
        "project" => Ok(config::project_path(root.context("열린 프로젝트가 없습니다")?)),
        other => bail!("알 수 없는 범위: {other}"),
    }
}

fn json_to_item(v: &Value) -> Result<Option<Item>> {
    Ok(Some(match v {
        Value::Null => return Ok(None),
        Value::Bool(b) => toml_edit::value(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml_edit::value(i),
            None => toml_edit::value(n.as_f64().context("숫자 형식 오류")?),
        },
        Value::String(s) => toml_edit::value(s.as_str()),
        Value::Array(items) => {
            let mut arr = Array::new();
            for it in items {
                match it {
                    Value::String(s) => arr.push(s.as_str()),
                    Value::Number(n) if n.is_i64() => arr.push(n.as_i64().unwrap()),
                    Value::Number(n) => arr.push(n.as_f64().unwrap_or(0.0)),
                    Value::Bool(b) => arr.push(*b),
                    _ => bail!("배열에는 문자열·숫자·참거짓만 넣을 수 있습니다"),
                }
            }
            toml_edit::value(arr)
        }
        Value::Object(_) => bail!("객체 값은 경로를 나눠서 저장하세요"),
    }))
}

/// `models.local.base_url` 같은 점 경로에 값을 쓴다. `null`이면 지운다.
pub fn set_in_doc(doc: &mut DocumentMut, path: &str, value: &Value) -> Result<()> {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    let Some((last, parents)) = parts.split_last() else { bail!("빈 경로") };
    let mut table: &mut Table = doc.as_table_mut();
    for (i, p) in parents.iter().enumerate() {
        let entry = table.entry(p).or_insert_with(|| {
            let mut t = Table::new();
            // [models] 같은 빈 상위 표 머리말을 만들지 않는다.
            t.set_implicit(i + 1 < parents.len());
            Item::Table(t)
        });
        table = entry.as_table_mut().with_context(|| format!("'{p}'는 표가 아닙니다"))?;
    }
    match json_to_item(value)? {
        Some(item) => {
            table[last] = item;
        }
        None => {
            table.remove(last);
        }
    }
    Ok(())
}

/// 설정 파일 하나를 고친다. 고친 결과가 유효한 설정이 아니면 되돌린다.
pub fn set_setting(scope: &str, root: Option<&Path>, changes: &[(String, Value)]) -> Result<()> {
    let path = scope_path(scope, root)?;
    let original = std::fs::read_to_string(&path).ok();
    let mut doc: DocumentMut = original
        .as_deref()
        .unwrap_or("")
        .parse()
        .with_context(|| format!("{} 형식 오류", path.display()))?;
    for (k, v) in changes {
        set_in_doc(&mut doc, k, v)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = doc.to_string();
    if original.is_none() {
        let header = if scope == "global" { "# Lantern 전역 설정 (설정 화면에서 만든 파일)\n\n" } else { "# Lantern 프로젝트 설정 (설정 화면에서 만든 파일)\n\n" };
        text = format!("{header}{text}");
    }
    std::fs::write(&path, text)?;
    if let Err(e) = config::load(root) {
        match original {
            Some(o) => std::fs::write(&path, o)?,
            None => std::fs::remove_file(&path)?,
        }
        return Err(e.context("저장하지 않았습니다"));
    }
    Ok(())
}

/// 화면에 보낼 설정. API 키 값은 절대 내보내지 않고 출처만 알린다.
pub fn snapshot(root: Option<&Path>) -> Result<Value> {
    let cfg = config::load(root)?;
    let mut v = serde_json::to_value(&cfg)?;
    if let Some(models) = v.get_mut("models").and_then(Value::as_object_mut) {
        for (key, m) in models.iter_mut() {
            let src = cfg.models.get(key).and_then(|mc| mc.api_key_with_source()).map(|(_, s)| s);
            m["api_key"] = Value::Null;
            m["key_source"] = json!(src);
            m["needs_key"] = json!(cfg.models[key].provider == "anthropic" || cfg.models[key].api_key_env.is_some());
        }
    }
    let global = config::global_path();
    Ok(json!({
        "config": v,
        "global_path": global.as_ref().map(|p| p.to_string_lossy().to_string()),
        "global_exists": global.as_ref().is_some_and(|p| p.exists()),
        "project_path": root.map(|r| config::project_path(r).to_string_lossy().to_string()),
        "project_exists": root.is_some_and(|r| config::project_path(r).exists()),
    }))
}

pub fn set_api_key(name: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        bail!("API 키가 비어 있습니다");
    }
    keyring::Entry::new(KEYRING_SERVICE, name)?
        .set_password(key)
        .context("자격 증명 저장소에 저장하지 못했습니다")
}

pub fn delete_api_key(name: &str) -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    pub ms: u128,
    pub models: Vec<String>,
}

/// 모델 목록 API로 연결과 키를 확인한다. 토큰을 쓰지 않는다.
pub async fn test_model(http: &reqwest::Client, m: &config::ModelConfig) -> TestResult {
    let started = Instant::now();
    let req = match m.provider.as_str() {
        "anthropic" => {
            let Some(key) = m.resolve_api_key() else {
                return TestResult { ok: false, message: "API 키가 없습니다".into(), ms: 0, models: vec![] };
            };
            let base = m.base_url.as_deref().unwrap_or("https://api.anthropic.com").trim_end_matches('/');
            http.get(format!("{base}/v1/models"))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
        }
        _ => {
            let base = m.base_url.as_deref().unwrap_or("https://api.openai.com/v1").trim_end_matches('/');
            let mut r = http.get(format!("{base}/models"));
            if let Some(k) = m.resolve_api_key() {
                r = r.bearer_auth(k);
            }
            r
        }
    };
    let resp = match req.timeout(Duration::from_secs(10)).send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = if e.is_connect() || e.is_timeout() { "서버에 연결할 수 없습니다" } else { "요청 실패" };
            return TestResult { ok: false, message: format!("{msg}: {e}"), ms: started.elapsed().as_millis(), models: vec![] };
        }
    };
    let ms = started.elapsed().as_millis();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("");
        let hint = match status.as_u16() {
            401 | 403 => "API 키가 올바르지 않습니다",
            404 => "주소(base_url)를 확인하세요",
            429 => "요청 한도를 넘었습니다",
            _ => "요청이 거부되었습니다",
        };
        return TestResult { ok: false, message: format!("{hint} ({status}) {detail}").trim().into(), ms, models: vec![] };
    }
    let models: Vec<String> = body["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let found = models.iter().any(|id| id == &m.model);
    let message = if models.is_empty() || found {
        "연결되었습니다".to_string()
    } else {
        format!("연결되었지만 '{}' 모델이 목록에 없습니다", m.model)
    };
    TestResult { ok: true, message, ms, models }
}

#[derive(Serialize)]
pub struct LocalServer {
    pub name: &'static str,
    pub base_url: &'static str,
    pub running: bool,
    pub models: Vec<String>,
}

/// 이 컴퓨터에서 도는 로컬 모델 서버(Ollama, LM Studio)를 찾는다.
pub async fn probe_local(http: &reqwest::Client) -> Vec<LocalServer> {
    let targets = [
        ("Ollama", "http://localhost:11434/v1"),
        ("LM Studio", "http://localhost:1234/v1"),
    ];
    let mut out = Vec::new();
    for (name, base_url) in targets {
        let r = http.get(format!("{base_url}/models")).timeout(Duration::from_millis(1500)).send().await;
        let models = match r {
            Ok(resp) if resp.status().is_success() => resp
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()))
                .unwrap_or_default(),
            _ => {
                out.push(LocalServer { name, base_url, running: false, models: vec![] });
                continue;
            }
        };
        out.push(LocalServer { name, base_url, running: true, models });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_keep_comments_and_create_tables() {
        let mut doc: DocumentMut = "# 내 설정\n[routing]\ndefault = \"smart\" # 기본\n".parse().unwrap();
        set_in_doc(&mut doc, "routing.default", &json!("local")).unwrap();
        set_in_doc(&mut doc, "models.local.base_url", &json!("http://localhost:11434/v1")).unwrap();
        set_in_doc(&mut doc, "agent.allowed_commands", &json!(["git status", "cargo test"])).unwrap();
        set_in_doc(&mut doc, "context.budget_tokens", &json!(12000)).unwrap();
        let text = doc.to_string();
        assert!(text.starts_with("# 내 설정"), "{text}");
        assert!(text.contains("default = \"local\""));
        assert!(text.contains("[models.local]"));
        assert!(!text.contains("[models]\n"), "빈 상위 표 머리말이 없어야 함: {text}");
        assert!(text.contains("allowed_commands = [\"git status\", \"cargo test\"]"));
        assert!(text.contains("budget_tokens = 12000"));

        set_in_doc(&mut doc, "routing.default", &Value::Null).unwrap();
        assert!(!doc.to_string().contains("default ="));
    }

    #[test]
    fn invalid_settings_are_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".lantern")).unwrap();
        let path = config::project_path(root);
        std::fs::write(&path, "[context]\nbudget_tokens = 8000\n").unwrap();
        // 문자열은 숫자 필드에 맞지 않으므로 거부되고 원래 파일이 남아야 한다.
        let err = set_setting("project", Some(root), &[("context.budget_tokens".into(), json!("많이"))]);
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[context]\nbudget_tokens = 8000\n");
        set_setting("project", Some(root), &[("context.budget_tokens".into(), json!(12000))]).unwrap();
        assert_eq!(config::load(Some(root)).unwrap().context.budget_tokens, 12000);
    }
}
