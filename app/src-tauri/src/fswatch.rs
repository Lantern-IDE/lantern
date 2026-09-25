//! 외부 파일 변경 감지. 다른 프로그램(git, 다른 편집기)이 바꾼 파일을 트리와 탭에 반영한다.
//! 이벤트를 300ms 동안 모아 한 번에 보낸다.

use crate::state::rel_path;
use notify::{recommended_watcher, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const IGNORED: &[&str] = &[".git", "node_modules", "target", "__pycache__", ".venv", "venv", "dist", "build"];
const DEBOUNCE: Duration = Duration::from_millis(300);

fn ignored(rel: &str) -> bool {
    rel.split('/').any(|seg| IGNORED.contains(&seg)) || rel.starts_with(".lantern/index.db")
}

/// 감시를 시작한다. 돌려받은 값을 버리면(drop) 감시가 멈춘다.
pub fn watch(app: AppHandle, root: &Path) -> notify::Result<RecommendedWatcher> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = recommended_watcher(tx)?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    let root: PathBuf = root.to_path_buf();
    std::thread::spawn(move || {
        // 채널이 닫히면(감시자가 drop되면) 끝난다.
        while let Ok(first) = rx.recv() {
            let mut paths = BTreeSet::new();
            let mut collect = |ev: notify::Result<Event>| {
                if let Ok(ev) = ev {
                    for p in ev.paths {
                        let rel = rel_path(&root, &p);
                        if !ignored(&rel) {
                            paths.insert(rel);
                        }
                    }
                }
            };
            collect(first);
            while let Ok(ev) = rx.recv_timeout(DEBOUNCE) {
                collect(ev);
            }
            if !paths.is_empty() {
                let _ = app.emit("fs-change", json!({ "paths": paths }));
            }
        }
    });
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_noisy_paths() {
        assert!(ignored("node_modules/x/index.js"));
        assert!(ignored("app/target/debug/a.exe"));
        assert!(ignored(".git/HEAD"));
        assert!(ignored(".lantern/index.db-wal"));
        assert!(!ignored("src/main.rs"));
        assert!(!ignored(".lantern/config.toml"));
    }
}
