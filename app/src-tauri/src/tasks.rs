//! 작업 저장. 앱을 다시 켜도 작업(대화 기록, 발자취, 되돌리기)이 남는다.
//!
//! 대화에는 코드와 (가려도 새어 나갈 수 있는) 민감한 내용이 섞이므로 프로젝트 폴더가 아니라
//! 앱 데이터 폴더에 둔다: `tasks/<프로젝트 키>/<작업>.ui.json`(화면 기록)과 `<작업>.history.json`(모델 대화).
//! 되돌리기용 원본은 `checkpoints/<id>.json`.

use crate::llm::Message;
use crate::state::Originals;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 프로젝트마다 보관하는 작업 수
const MAX_TASKS: usize = 60;
/// 화면 기록 한 개의 최대 크기
const MAX_UI_BYTES: usize = 4 * 1024 * 1024;
/// 되돌리기 원본을 보관하는 기간
const CHECKPOINT_DAYS: u64 = 14;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 경로가 같으면 같은 키. 사람이 알아보게 폴더 이름을 붙인다.
pub fn project_key(root: &Path) -> String {
    let norm = root.to_string_lossy().replace('\\', "/").to_lowercase();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    norm.hash(&mut h);
    // 이름은 정규화한 경로에서 뽑는다 (Path::file_name은 macOS·Linux에서 `\`를 구분자로 보지 않는다)
    let name: String = norm
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .take(24)
        .collect();
    format!("{name}-{:016x}", h.finish())
}

fn check_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 80 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("잘못된 작업 id: {id}");
    }
    Ok(())
}

fn dir_in(base: &Path, root: &Path) -> PathBuf {
    base.join("tasks").join(project_key(root))
}

fn dir(root: &Path) -> Result<PathBuf> {
    Ok(dir_in(&crate::state::data_dir().context("데이터 폴더를 찾지 못했습니다")?, root))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ── 화면 기록 ────────────────────────────────────────────

pub fn save_ui(root: &Path, id: &str, data: &Value) -> Result<()> {
    save_ui_in(&dir(root)?, id, data)
}

fn save_ui_in(dir: &Path, id: &str, data: &Value) -> Result<()> {
    check_id(id)?;
    let bytes = serde_json::to_vec(data)?;
    if bytes.len() > MAX_UI_BYTES {
        bail!("작업 기록이 너무 큽니다 ({}MB)", bytes.len() / 1024 / 1024);
    }
    write_atomic(&dir.join(format!("{id}.ui.json")), &bytes)
}

/// 최근에 바뀐 것부터. 오래된 작업은 정리한다.
pub fn list(root: &Path) -> Result<Vec<Value>> {
    let d = dir(root)?;
    // 넘쳐서 지울 작업의 격리 작업 공간도 함께 정리한다
    for v in overflow_in(&d) {
        if let (Some(path), Some(branch)) = (v.pointer("/isolated/path").and_then(Value::as_str), v.pointer("/isolated/branch").and_then(Value::as_str)) {
            crate::worktree::remove(root, Path::new(path), branch);
        }
    }
    list_in(&d)
}

/// 보관 한도를 넘는 (지워질) 작업들
fn overflow_in(dir: &Path) -> Vec<Value> {
    let mut items: Vec<(u64, Value)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".ui.json"))
        .filter_map(|e| serde_json::from_str::<Value>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .map(|v| (v.get("updated").and_then(Value::as_u64).unwrap_or(0), v))
        .collect();
    items.sort_by_key(|x| std::cmp::Reverse(x.0));
    items.into_iter().skip(MAX_TASKS).map(|x| x.1).collect()
}

fn list_in(dir: &Path) -> Result<Vec<Value>> {
    let mut items: Vec<(u64, String, Value)> = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".ui.json") else { continue };
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let updated = v.get("updated").and_then(Value::as_u64).unwrap_or(0);
        items.push((updated, id.to_string(), v));
    }
    items.sort_by_key(|x| std::cmp::Reverse(x.0));
    for (_, id, _) in items.iter().skip(MAX_TASKS) {
        delete_in(dir, id);
    }
    Ok(items.into_iter().take(MAX_TASKS).map(|x| x.2).collect())
}

pub fn delete(root: &Path, id: &str) -> Result<()> {
    check_id(id)?;
    delete_in(&dir(root)?, id);
    Ok(())
}

fn delete_in(dir: &Path, id: &str) {
    for ext in ["ui.json", "history.json"] {
        let _ = std::fs::remove_file(dir.join(format!("{id}.{ext}")));
    }
}

/// 격리 작업 공간 정보 (화면 기록에 들어 있다)
pub fn worktree_of(root: &Path, id: &str) -> Option<(PathBuf, String)> {
    check_id(id).ok()?;
    let text = std::fs::read_to_string(dir(root).ok()?.join(format!("{id}.ui.json"))).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let w = v.get("isolated")?;
    Some((PathBuf::from(w.get("path")?.as_str()?), w.get("branch")?.as_str()?.to_string()))
}

// ── 모델 대화 ────────────────────────────────────────────

pub fn save_history(root: &Path, id: &str, history: &[Message]) -> Result<()> {
    check_id(id)?;
    write_atomic(&dir(root)?.join(format!("{id}.history.json")), &serde_json::to_vec(history)?)
}

pub fn load_history(root: &Path, id: &str) -> Option<Vec<Message>> {
    check_id(id).ok()?;
    let text = std::fs::read_to_string(dir(root).ok()?.join(format!("{id}.history.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

// ── 되돌리기 원본 ────────────────────────────────────────

fn checkpoint_path(id: &str) -> Result<PathBuf> {
    check_id(id)?;
    Ok(crate::state::data_dir().context("데이터 폴더를 찾지 못했습니다")?.join("checkpoints").join(format!("{id}.json")))
}

pub fn save_checkpoint(id: &str, originals: &Originals) -> Result<()> {
    write_atomic(&checkpoint_path(id)?, &serde_json::to_vec(originals)?)?;
    // 바꾼 직후의 내용 지문: 되돌리기 전에 그 뒤로 다른 곳에서 바뀌었는지 본다
    let after: Vec<(PathBuf, Option<u64>)> = originals.iter().map(|(p, _)| (p.clone(), fingerprint(p))).collect();
    write_atomic(&checkpoint_path(id)?.with_extension("after.json"), &serde_json::to_vec(&after)?)
}

fn fingerprint(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    Some(h.finish())
}

/// 이 체크포인트 뒤로 다른 곳(다른 작업, 사용자)에서 바뀐 파일. 되돌리면 그 변경도 사라진다.
pub fn changed_since(id: &str, root: &Path) -> Vec<String> {
    let Ok(p) = checkpoint_path(id) else { return vec![] };
    let Ok(text) = std::fs::read_to_string(p.with_extension("after.json")) else { return vec![] };
    let Ok(after) = serde_json::from_str::<Vec<(PathBuf, Option<u64>)>>(&text) else { return vec![] };
    after
        .into_iter()
        .filter(|(path, fp)| fingerprint(path) != *fp)
        .map(|(path, _)| crate::state::rel_path(root, &path))
        .collect()
}

pub fn load_checkpoint(id: &str) -> Option<Originals> {
    let text = std::fs::read_to_string(checkpoint_path(id).ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn delete_checkpoint(id: &str) {
    if let Ok(p) = checkpoint_path(id) {
        let _ = std::fs::remove_file(p.with_extension("after.json"));
        let _ = std::fs::remove_file(p);
    }
}

/// 오래된 되돌리기 원본을 지운다 (시작할 때 한 번)
pub fn prune_checkpoints() {
    let Some(dir) = crate::state::data_dir().map(|d| d.join("checkpoints")) else { return };
    let limit = Duration::from_secs(CHECKPOINT_DAYS * 24 * 3600);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 다시 시작해도 겹치지 않는 id
pub fn unique_id(prefix: &str) -> String {
    format!("{prefix}{}-{}", now_ms(), crate::agent::next_id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_stable_and_readable() {
        let a = project_key(Path::new("C:/Work/My App"));
        assert_eq!(a, project_key(Path::new("c:\\work\\my app")), "대소문자·구분자 무시");
        assert!(a.starts_with("my-app-"));
        assert_ne!(a, project_key(Path::new("C:/Work/Other")));
    }

    #[test]
    fn saves_lists_and_prunes() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        save_ui_in(d, "t1", &json!({"id": "t1", "updated": 10})).unwrap();
        save_ui_in(d, "t2", &json!({"id": "t2", "updated": 20})).unwrap();
        let l = list_in(d).unwrap();
        assert_eq!(l[0]["id"], "t2", "최근 것부터");
        assert!(save_ui_in(d, "../evil", &json!({})).is_err(), "경로 탈출 거부");
        for i in 0..(MAX_TASKS + 3) {
            save_ui_in(d, &format!("x{i}"), &json!({"updated": 100 + i})).unwrap();
        }
        assert_eq!(list_in(d).unwrap().len(), MAX_TASKS);
        assert_eq!(std::fs::read_dir(d).unwrap().count(), MAX_TASKS, "넘치는 오래된 작업은 지운다");
    }

    #[test]
    fn fingerprints_detect_later_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, "after").unwrap();
        let before = fingerprint(&file);
        assert_eq!(before, fingerprint(&file), "같은 내용이면 같은 지문");
        std::fs::write(&file, "changed later").unwrap();
        assert_ne!(before, fingerprint(&file), "다른 곳에서 바뀌면 다른 지문");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(fingerprint(&file), None, "지워진 파일");
    }

    #[test]
    fn history_roundtrip() {
        let h = vec![Message::user_text("안녕")];
        let text = serde_json::to_string(&h).unwrap();
        let back: Vec<Message> = serde_json::from_str(&text).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }
}
