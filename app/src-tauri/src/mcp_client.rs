//! 외부 MCP 서버를 내장 에이전트의 도구로 쓴다 (config.toml의 `[mcp.<이름>]`, stdio).
//!
//! 서버는 처음 쓸 때 띄워 두고 계속 쓴다. 설정(명령·인자·환경변수)이 바뀌면 다시 띄운다.
//! 도구 이름은 `mcp__<서버>__<도구>`로 모델에 보이고, 부를 때마다 승인 카드를 거친다
//! (서버 설정의 `auto_approve`에 있는 도구는 바로 실행).

use crate::config::{Config, McpServerConfig};
use crate::llm::ToolSpec;
use crate::state::AppState;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

pub const PREFIX: &str = "mcp__";
const PROTOCOL_VERSION: &str = "2025-06-18";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// 공급자들의 도구 이름 한도
const MAX_TOOL_NAME: usize = 64;

type Reply = std::result::Result<Value, String>;

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(skip)]
    pub schema: Value,
}

pub struct Conn {
    /// 설정 지문: 바뀌면 다시 띄운다
    fingerprint: String,
    stdin: tokio::sync::Mutex<ChildStdin>,
    child: Mutex<Option<Child>>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    alive: AtomicBool,
    tools: Mutex<Vec<McpTool>>,
    /// tools/list_changed를 받으면 다음에 다시 받는다
    stale: AtomicBool,
    stderr: Mutex<Vec<String>>,
    pub server_name: String,
}

fn fingerprint(c: &McpServerConfig) -> String {
    format!("{}\0{}\0{:?}", c.command, c.args.join("\0"), c.env)
}

impl Conn {
    async fn start(name: &str, cfg: &McpServerConfig, root: &Path) -> Result<Arc<Conn>> {
        let exe = which::which(&cfg.command).map_err(|_| anyhow!("MCP 서버 '{name}'의 명령 '{}'을(를) 찾지 못했습니다", cfg.command))?;
        let mut cmd = Command::new(exe);
        cmd.args(&cfg.args)
            .envs(&cfg.env)
            .current_dir(root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // 콘솔 창을 띄우지 않는다
        let mut child = cmd.spawn().with_context(|| format!("MCP 서버 '{name}'을(를) 실행하지 못했습니다"))?;
        let stdin = child.stdin.take().context("stdin")?;
        let stdout = child.stdout.take().context("stdout")?;
        let stderr = child.stderr.take().context("stderr")?;
        let conn = Arc::new(Conn {
            fingerprint: fingerprint(cfg),
            stdin: tokio::sync::Mutex::new(stdin),
            child: Mutex::new(Some(child)),
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
            tools: Mutex::new(Vec::new()),
            stale: AtomicBool::new(true),
            stderr: Mutex::new(Vec::new()),
            server_name: name.to_string(),
        });
        tokio::spawn(read_loop(conn.clone(), stdout, root.to_path_buf()));
        let c = conn.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut q = c.stderr.lock().unwrap();
                q.push(l);
                if q.len() > 20 {
                    q.remove(0);
                }
            }
        });
        conn.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "roots": {} },
                "clientInfo": { "name": "lantern", "title": "Lantern IDE", "version": env!("CARGO_PKG_VERSION") },
            }),
            START_TIMEOUT,
        )
        .await
        .map_err(|e| conn.fail(&format!("MCP 서버 '{name}' 연결 실패: {e}")))?;
        conn.notify("notifications/initialized", json!({})).await;
        Ok(conn)
    }

    fn fail(&self, msg: &str) -> anyhow::Error {
        let tail = self.stderr.lock().unwrap().iter().rev().take(5).rev().cloned().collect::<Vec<_>>().join("\n");
        if tail.is_empty() {
            anyhow!("{msg}")
        } else {
            anyhow!("{msg}\n{tail}")
        }
    }

    async fn write(&self, msg: Value) -> std::io::Result<()> {
        let mut w = self.stdin.lock().await;
        w.write_all(format!("{msg}\n").as_bytes()).await?;
        w.flush().await
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        if !self.alive.load(Ordering::Relaxed) {
            return Err("서버가 종료되었습니다".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("서버에 보내지 못했습니다: {e}"));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("서버가 종료되었습니다".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("{method} 응답이 {}초 안에 오지 않았습니다", timeout.as_secs()))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) {
        let _ = self.write(json!({ "jsonrpc": "2.0", "method": method, "params": params })).await;
    }

    pub fn shutdown(&self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.start_kill();
        }
    }

    /// 도구 목록 (처음이거나 서버가 바뀌었다고 알렸으면 다시 받는다)
    pub async fn tools(&self) -> Result<Vec<McpTool>> {
        if !self.stale.load(Ordering::Relaxed) {
            return Ok(self.tools.lock().unwrap().clone());
        }
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let r = self.request("tools/list", params, START_TIMEOUT).await.map_err(|e| anyhow!("{e}"))?;
            for t in r["tools"].as_array().into_iter().flatten() {
                let Some(name) = t["name"].as_str() else { continue };
                out.push(McpTool {
                    name: name.to_string(),
                    description: t["description"].as_str().unwrap_or("").to_string(),
                    schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                });
            }
            cursor = r["nextCursor"].as_str().map(str::to_string);
            if cursor.is_none() || out.len() > 500 {
                break;
            }
        }
        *self.tools.lock().unwrap() = out.clone();
        self.stale.store(false, Ordering::Relaxed);
        Ok(out)
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<String> {
        let r = self
            .request("tools/call", json!({ "name": tool, "arguments": args }), CALL_TIMEOUT)
            .await
            .map_err(|e| self.fail(&format!("MCP 도구 {tool} 실패: {e}")))?;
        let text = result_text(&r);
        if r["isError"].as_bool().unwrap_or(false) {
            bail!("{text}");
        }
        Ok(text)
    }
}

/// tools/call 결과를 모델에 보낼 글로
fn result_text(r: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in r["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_string()),
            Some("image") => parts.push(format!("[이미지 {}]", c["mimeType"].as_str().unwrap_or(""))),
            Some("audio") => parts.push(format!("[오디오 {}]", c["mimeType"].as_str().unwrap_or(""))),
            Some("resource") => parts.push(
                c["resource"]["text"].as_str().map(str::to_string).unwrap_or_else(|| format!("[리소스 {}]", c["resource"]["uri"].as_str().unwrap_or(""))),
            ),
            Some("resource_link") => parts.push(format!("[링크 {}]", c["uri"].as_str().unwrap_or(""))),
            _ => {}
        }
    }
    if parts.is_empty() {
        if let Some(s) = r.get("structuredContent") {
            return serde_json::to_string_pretty(s).unwrap_or_default();
        }
        return "(결과 없음)".into();
    }
    parts.join("\n")
}

async fn read_loop(conn: Arc<Conn>, stdout: tokio::process::ChildStdout, root: std::path::PathBuf) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let method = msg.get("method").and_then(Value::as_str);
        match (method, msg.get("id").cloned()) {
            // 서버 → Lantern 요청: 작업 폴더(roots)만 알려 주고 나머지(sampling 등)는 지원하지 않는다
            (Some(m), Some(id)) => {
                let reply = match m {
                    "roots/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "roots": [{ "uri": crate::file_uri(&root), "name": "project" }] } }),
                    "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                    _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Lantern은 {m}을(를) 지원하지 않습니다") } }),
                };
                let _ = conn.write(reply).await;
            }
            (Some("notifications/tools/list_changed"), None) => conn.stale.store(true, Ordering::Relaxed),
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(id) = id.as_u64() else { continue };
                if let Some(tx) = conn.pending.lock().unwrap().remove(&id) {
                    let r = match msg.get("error") {
                        Some(e) => Err(e["message"].as_str().unwrap_or("알 수 없는 오류").to_string()),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = tx.send(r);
                }
            }
            _ => {}
        }
    }
    conn.alive.store(false, Ordering::Relaxed);
    for (_, tx) in conn.pending.lock().unwrap().drain() {
        let _ = tx.send(Err("서버가 종료되었습니다".into()));
    }
}

/// 서버 연결을 가져오거나 띄운다
async fn connection(st: &AppState, name: &str, cfg: &McpServerConfig, root: &Path) -> Result<Arc<Conn>> {
    let existing = st.mcp.lock().unwrap().get(name).cloned();
    if let Some(c) = existing {
        if c.alive.load(Ordering::Relaxed) && c.fingerprint == fingerprint(cfg) {
            return Ok(c);
        }
        c.shutdown();
    }
    let c = Conn::start(name, cfg, root).await?;
    st.mcp.lock().unwrap().insert(name.to_string(), c.clone());
    Ok(c)
}

/// 프로젝트를 바꾸거나 앱을 닫을 때
pub fn close_all(st: &AppState) {
    for (_, c) in st.mcp.lock().unwrap().drain() {
        c.shutdown();
    }
}

/// 모델에 보일 도구 이름: 공급자 규칙(영문·숫자·_-, 64자)에 맞춘다
fn tool_id(server: &str, tool: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>();
    let id = format!("{PREFIX}{}__{}", clean(server), clean(tool));
    id.chars().take(MAX_TOOL_NAME).collect()
}

/// 에이전트가 쓸 MCP 도구: 도구 설명과, 모델에 보이는 이름 → (서버, 도구) 대응
pub struct Toolset {
    pub specs: Vec<ToolSpec>,
    /// 모델에 보이는 이름 → (서버 연결, 서버의 도구 이름)
    pub map: HashMap<String, (Arc<Conn>, String)>,
    /// 띄우지 못한 서버와 이유 (작업은 계속한다)
    pub errors: Vec<String>,
}

pub async fn toolset(st: &AppState, cfg: &Config, root: &Path) -> Toolset {
    let mut set = Toolset { specs: Vec::new(), map: HashMap::new(), errors: Vec::new() };
    let servers: Vec<(String, McpServerConfig)> = cfg.mcp.iter().filter(|(_, c)| c.enabled).map(|(n, c)| (n.clone(), c.clone())).collect();
    let results = futures_util::future::join_all(servers.iter().map(|(name, c)| async move {
        let conn = connection(st, name, c, root).await?;
        let tools = conn.tools().await?;
        anyhow::Ok((conn, tools))
    }))
    .await;
    for ((name, _), r) in servers.iter().zip(results) {
        match r {
            Ok((conn, tools)) => {
                for t in tools {
                    let id = tool_id(name, &t.name);
                    if set.map.contains_key(&id) {
                        continue;
                    }
                    set.specs.push(ToolSpec {
                        name: id.clone(),
                        description: format!("[MCP server '{name}'] {}", t.description).chars().take(1024).collect(),
                        input_schema: t.schema.clone(),
                    });
                    set.map.insert(id, (conn.clone(), t.name));
                }
            }
            Err(e) => {
                crate::applog::error(&format!("{e:#}"));
                set.errors.push(format!("{name}: {e:#}"));
            }
        }
    }
    set
}

#[derive(Serialize)]
pub struct ServerStatus {
    pub name: String,
    pub command: String,
    pub enabled: bool,
    pub tools: Vec<McpTool>,
    pub error: Option<String>,
}

/// 설정 화면: 서버마다 띄워 보고 도구 목록이나 오류를 알린다
pub async fn status(st: &AppState, cfg: &Config, root: &Path) -> Vec<ServerStatus> {
    let mut out = Vec::new();
    for (name, c) in &cfg.mcp {
        let command = std::iter::once(c.command.clone()).chain(c.args.iter().cloned()).collect::<Vec<_>>().join(" ");
        if !c.enabled {
            out.push(ServerStatus { name: name.clone(), command, enabled: false, tools: vec![], error: None });
            continue;
        }
        let r = async { connection(st, name, c, root).await?.tools().await }.await;
        match r {
            Ok(tools) => out.push(ServerStatus { name: name.clone(), command, enabled: true, tools, error: None }),
            Err(e) => out.push(ServerStatus { name: name.clone(), command, enabled: true, tools: vec![], error: Some(format!("{e:#}")) }),
        }
    }
    out
}

/// 외부 에이전트(ACP)에도 같은 MCP 서버를 넘긴다
pub fn acp_servers(cfg: &Config) -> Vec<Value> {
    cfg.mcp
        .iter()
        .filter(|(_, c)| c.enabled)
        .map(|(name, c)| {
            json!({
                "name": name,
                "command": c.command,
                "args": c.args,
                "env": c.env.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids_fit_provider_rules() {
        assert_eq!(tool_id("github", "create_issue"), "mcp__github__create_issue");
        assert_eq!(tool_id("my server", "do.thing"), "mcp__my_server__do_thing");
        assert!(tool_id("s", &"x".repeat(100)).len() <= MAX_TOOL_NAME);
    }

    #[test]
    fn turns_results_into_text() {
        let r = json!({ "content": [{ "type": "text", "text": "a" }, { "type": "image", "mimeType": "image/png", "data": "x" }, { "type": "resource", "resource": { "uri": "file:///x", "text": "body" } }] });
        assert_eq!(result_text(&r), "a\n[이미지 image/png]\nbody");
        assert_eq!(result_text(&json!({ "content": [], "structuredContent": { "n": 1 } })), "{\n  \"n\": 1\n}");
        assert_eq!(result_text(&json!({})), "(결과 없음)");
    }
}
