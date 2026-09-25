//! MCP 서버 (JSON-RPC 2.0, 줄 단위 stdio).
//!
//! 외부 SDK 없이 필요한 메서드만 직접 구현한다: initialize, ping, tools/list, tools/call.
//! stdout은 프로토콜 전용이므로 로그는 stderr로만 쓴다.

use crate::assemble::{ContextRequest, DEFAULT_BUDGET};
use crate::Engine;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const SUPPORTED_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const DEFAULT_VERSION: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Lantern indexes this project locally (symbols, call graph, git co-change, project memory). \
Call get_context first for any question about this codebase: it returns the most relevant code within a token budget, \
with the reason each item was included. Use get_symbol to read a full definition and its callers/callees, \
and find_references to locate usages.";

pub fn serve(engine: &mut Engine) -> Result<()> {
    eprintln!("lantern mcp: {} 에서 대기 중", engine.root.display());
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(engine, &msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = response {
            writeln!(stdout, "{r}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// 요청이면 응답을 돌려주고, 알림(id 없음)이면 `None`.
pub fn handle(engine: &mut Engine, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return None; // 클라이언트가 보낸 응답
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => initialize(&params),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call_tool(engine, &params),
        _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(DEFAULT_VERSION);
    let version = if SUPPORTED_VERSIONS.contains(&requested) { requested } else { DEFAULT_VERSION };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "lantern", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "get_context",
            "description": "Assemble the code most relevant to a question about this project, ranked and packed \
into a token budget. Returns markdown grouped by file; each item states why it was included. \
Items marked as signature-only can be expanded with get_symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The question or task, in natural language. Include identifiers if known." },
                    "file": { "type": "string", "description": "Optional file the user is focused on (relative to project root)." },
                    "line": { "type": "integer", "description": "Optional 1-based line in `file`." },
                    "budget_tokens": { "type": "integer", "description": "Approximate token budget (default 8000)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "search_symbols",
            "description": "Full-text search over symbol names, signatures and bodies. Returns name, kind, location and signature.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "description": "Max results (default 20)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_symbol",
            "description": "Full definition of a symbol by exact name, with the symbols that use it and the symbols it uses.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }
        },
        {
            "name": "find_references",
            "description": "All places that call or reference a name, with the enclosing symbol and source line.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }
        }
    ])
}

fn call_tool(engine: &mut Engine, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    // 질의 전에 증분 인덱싱으로 최신 상태를 보장한다.
    if let Err(e) = engine.refresh() {
        eprintln!("lantern mcp: 인덱스 갱신 실패: {e:#}");
    }

    let result: Result<String> = (|| {
        let str_arg = |key: &str| -> Result<String> {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("'{key}' 인자가 필요합니다"))
        };
        match name {
            "get_context" => {
                let req = ContextRequest {
                    query: str_arg("query")?,
                    file: args.get("file").and_then(Value::as_str).map(str::to_string),
                    line: args.get("line").and_then(Value::as_u64).map(|l| l as u32),
                    budget_tokens: args
                        .get("budget_tokens")
                        .and_then(Value::as_u64)
                        .map(|b| b as usize)
                        .unwrap_or(DEFAULT_BUDGET),
                };
                Ok(engine.context(&req)?.to_markdown())
            }
            "search_symbols" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                engine.search_report(&str_arg("query")?, limit)
            }
            "get_symbol" => engine.symbol_report(&str_arg("name")?),
            "find_references" => engine.references_report(&str_arg("name")?),
            _ => Err(anyhow!("알 수 없는 도구: {name}")),
        }
    })();

    match result {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(e) => json!({ "content": [{ "type": "text", "text": format!("오류: {e:#}") }], "isError": true }),
    }
}
