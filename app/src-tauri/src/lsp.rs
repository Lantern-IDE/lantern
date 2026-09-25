//! 언어 서버 중계. 서버를 stdio로 띄우고 JSON 메시지를 화면의 CodeMirror LSP 클라이언트와 주고받는다.
//! 메시지 해석은 화면 쪽이 하고, 여기서는 `Content-Length` 틀만 씌우고 벗긴다.

use crate::config::LspConfig;
use anyhow::{Context, Result};
use serde_json::json;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

const STDERR_TAIL: usize = 20;

pub struct Lsp {
    stdin: ChildStdin,
    child: Child,
    /// 앱이 일부러 끈 경우. 이때는 종료 이벤트를 보내지 않는다.
    /// (늦게 도착한 이전 서버의 종료 이벤트가 새로 띄운 같은 언어 클라이언트를 지우지 않게)
    stopping: Arc<AtomicBool>,
    /// initialize 요청에 덧붙일 initializationOptions (한 번 쓰고 버린다)
    init_options: Option<serde_json::Value>,
}

pub fn start(app: AppHandle, lang: &str, cfg: &LspConfig, root: &Path, root_uri: &str) -> Result<Lsp> {
    let exe = which::which(&cfg.command)
        .ok()
        .or_else(|| crate::lspinstall::installed_bin(&cfg.command))
        .with_context(|| format!("언어 서버 '{}'가 설치되어 있지 않습니다", cfg.command))?;
    let mut cmd = Command::new(exe);
    cmd.args(&cfg.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    lantern_context::git::no_window(&mut cmd);
    let mut child = cmd.spawn().with_context(|| format!("{} 실행 실패", cfg.command))?;
    let stdin = child.stdin.take().context("stdin 없음")?;
    let stdout = child.stdout.take().context("stdout 없음")?;
    let stderr = child.stderr.take().context("stderr 없음")?;

    // 서버가 죽었을 때 이유를 보여주려고 stderr 마지막 줄들을 보관한다.
    let tail: Arc<Mutex<VecDeque<String>>> = Default::default();
    {
        let tail = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut t = tail.lock().unwrap();
                if t.len() >= STDERR_TAIL {
                    t.pop_front();
                }
                t.push_back(line);
            }
        });
    }
    let stopping = Arc::new(AtomicBool::new(false));
    let stopping_flag = stopping.clone();
    let exit = move |app: &AppHandle, lang: &str| {
        if stopping_flag.load(Ordering::Relaxed) {
            return;
        }
        // stderr 스레드가 마지막 줄을 읽을 틈을 준다.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let reason = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
        let _ = app.emit("lsp-exit", json!({ "lang": lang, "reason": reason }));
    };

    let lang = lang.to_string();
    let variants = uri_variants(root_uri);
    let canonical = root_uri.to_string();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut len: Option<usize> = None;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => return exit(&app, &lang),
                    Ok(_) => {}
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some(v) = line.strip_prefix("Content-Length:") {
                    len = v.trim().parse().ok();
                }
            }
            let Some(len) = len else { continue };
            let mut body = vec![0u8; len];
            if reader.read_exact(&mut body).is_err() {
                return exit(&app, &lang);
            }
            let mut msg = String::from_utf8_lossy(&body).into_owned();
            for v in &variants {
                if msg.contains(v.as_str()) {
                    msg = msg.replace(v.as_str(), &canonical);
                }
            }
            let _ = app.emit("lsp-msg", json!({ "lang": lang, "msg": msg }));
        }
    });

    let init_options = crate::lspinstall::init_options(&cfg.command, root);
    Ok(Lsp { stdin, child, stopping, init_options })
}

/// initialize 요청이면 params.initializationOptions에 옵션을 합친다 (클라이언트가 준 값이 우선)
fn add_init_options(msg: &str, extra: &serde_json::Value) -> Option<String> {
    let mut v: serde_json::Value = serde_json::from_str(msg).ok()?;
    if v.get("method")?.as_str()? != "initialize" {
        return None;
    }
    let params = v.get_mut("params")?.as_object_mut()?;
    let opts = params.entry("initializationOptions").or_insert_with(|| json!({}));
    if !opts.is_object() {
        *opts = json!({});
    }
    let opts = opts.as_object_mut()?;
    for (k, val) in extra.as_object()? {
        opts.entry(k.clone()).or_insert_with(|| val.clone());
    }
    serde_json::to_string(&v).ok()
}

/// 서버마다 루트 URI를 다르게 표기한다 (`c:` / `C:` / `c%3A`).
/// 화면 쪽 문서 URI와 맞추려고 받은 메시지의 루트 부분을 우리가 보낸 표기로 되돌린다.
fn uri_variants(root_uri: &str) -> Vec<String> {
    let Some(rest) = root_uri.strip_prefix("file:///") else { return vec![] };
    let mut chars = rest.chars();
    let (Some(drive), Some(':')) = (chars.next(), chars.next()) else { return vec![] };
    let tail: String = chars.collect();
    let mut out = Vec::new();
    for d in [drive.to_ascii_lowercase(), drive.to_ascii_uppercase()] {
        for colon in [":", "%3A", "%3a"] {
            let v = format!("file:///{d}{colon}{tail}");
            if v != root_uri {
                out.push(v);
            }
        }
    }
    out
}

impl Lsp {
    pub fn send(&mut self, msg: &str) -> Result<()> {
        if self.init_options.is_some() && msg.contains("\"initialize\"") {
            if let Some(patched) = add_init_options(msg, self.init_options.as_ref().unwrap()) {
                self.init_options = None;
                return self.write(&patched);
            }
        }
        self.write(msg)
    }

    fn write(&mut self, msg: &str) -> Result<()> {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", msg.len(), msg)?;
        self.stdin.flush()?;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_init_options_into_initialize_only() {
        let extra = json!({ "tsserver": { "path": "C:/x/tsserver.js" } });
        let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"rootUri":"file:///c:/p","initializationOptions":{"a":1}}}"#;
        let out: serde_json::Value = serde_json::from_str(&add_init_options(init, &extra).unwrap()).unwrap();
        assert_eq!(out["params"]["initializationOptions"]["a"], 1);
        assert_eq!(out["params"]["initializationOptions"]["tsserver"]["path"], "C:/x/tsserver.js");
        let bare = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#;
        assert!(add_init_options(bare, &extra).unwrap().contains("tsserver"));
        assert!(add_init_options(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#, &extra).is_none());
    }

    #[test]
    fn variants_cover_drive_forms() {
        let v = uri_variants("file:///C:/proj");
        assert!(v.contains(&"file:///c:/proj".to_string()));
        assert!(v.contains(&"file:///c%3A/proj".to_string()));
        assert!(!v.contains(&"file:///C:/proj".to_string()));
        assert!(uri_variants("file:///home/x").is_empty());
    }
}
