//! 설정 파일의 [hooks]에 적힌 명령을 실행하고 출력 패널로 보낸다.

use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const HOOK_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn run(app: &AppHandle, root: &Path, source: &str, commands: &[String]) {
    for cmd in commands {
        let _ = app.emit("output", json!({ "source": source, "text": format!("$ {cmd}\n") }));
        let text = match crate::tools::run_shell(root, cmd, HOOK_TIMEOUT).await {
            Ok(out) => out,
            Err(e) => format!("실행 실패: {e:#}\n"),
        };
        let _ = app.emit("output", json!({ "source": source, "text": text }));
    }
}
