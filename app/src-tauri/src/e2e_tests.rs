//! 모의 HTTP 서버로 모델 스트리밍과 도구 실행 경로를 끝까지 검증한다 (API 키 없이).

use crate::config::{self, ModelConfig};
use crate::llm::{self, Block, ChatRequest, Message, StopReason, StreamEvent, ToolSpec};
use crate::state::Project;
use crate::tools::{Approver, BoxFuture, ToolCtx};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 요청 하나를 받아 SSE 본문으로 답하고, 받은 요청(헤더, 본문)을 돌려준다.
async fn mock_server(sse: String) -> (String, tokio::task::JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let (head, body) = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..p]).to_string();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                    .unwrap_or(0);
                while buf.len() < p + 4 + len {
                    let n = sock.read(&mut tmp).await.unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                }
                break (head, serde_json::from_slice(&buf[p + 4..p + 4 + len]).unwrap());
            }
        };
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{sse}");
        // 청크 경계를 흔들기 위해 잘게 나눠 보낸다.
        for part in resp.as_bytes().chunks(37) {
            sock.write_all(part).await.unwrap();
        }
        sock.shutdown().await.unwrap();
        (head, body)
    });
    (format!("http://{addr}"), handle)
}

fn sse(events: &[(&str, Value)]) -> String {
    events.iter().map(|(e, d)| format!("event: {e}\ndata: {d}\n\n")).collect()
}

fn tools() -> Vec<ToolSpec> {
    crate::tools::specs(&["read_file".to_string()])
}

#[tokio::test]
async fn anthropic_stream_with_thinking_text_and_tool() {
    let body = sse(&[
        ("message_start", json!({"type":"message_start","message":{"model":"claude-opus-5","usage":{"input_tokens":120,"cache_read_input_tokens":1000,"cache_creation_input_tokens":0}}})),
        ("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}})),
        ("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"생각"}})),
        ("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig123"}})),
        ("content_block_stop", json!({"type":"content_block_stop","index":0})),
        ("content_block_start", json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}})),
        ("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"파일을 "}})),
        ("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"읽겠습니다."}})),
        ("content_block_stop", json!({"type":"content_block_stop","index":1})),
        ("content_block_start", json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}})),
        ("content_block_delta", json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"src/"}})),
        ("content_block_delta", json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"main.rs\"}"}})),
        ("content_block_stop", json!({"type":"content_block_stop","index":2})),
        ("message_delta", json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}})),
        ("message_stop", json!({"type":"message_stop"})),
    ]);
    let (url, server) = mock_server(body).await;
    let mut m: ModelConfig = config::load(None).unwrap().models["smart"].clone();
    m.base_url = Some(url);
    m.api_key = Some("test-key".into());
    m.api_key_env = None;

    let mut streamed = String::new();
    let mut tool_started = Vec::new();
    let msgs = vec![Message::user_text("main.rs 보여줘")];
    let resp = llm::stream(
        &reqwest::Client::new(),
        &m,
        &ChatRequest { system: "sys", messages: &msgs, tools: &tools() },
        &Arc::new(AtomicBool::new(false)),
        &mut |ev| match ev {
            StreamEvent::Text(t) => streamed.push_str(t),
            StreamEvent::ToolStart { name } => tool_started.push(name.to_string()),
        },
    )
    .await
    .unwrap();

    assert_eq!(streamed, "파일을 읽겠습니다.");
    assert_eq!(tool_started, ["read_file"]);
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(resp.usage.input_tokens, 120);
    assert_eq!(resp.usage.cache_read_tokens, 1000);
    assert_eq!(resp.usage.output_tokens, 42);
    assert_eq!(resp.blocks.len(), 3);
    match &resp.blocks[0] {
        Block::Raw { value } => {
            assert_eq!(value["thinking"], "생각");
            assert_eq!(value["signature"], "sig123");
        }
        b => panic!("thinking 블록이어야 함: {b:?}"),
    }
    match &resp.blocks[2] {
        Block::ToolUse { id, input, invalid_json: None, .. } => {
            assert_eq!(id, "toolu_1");
            assert_eq!(input["path"], "src/main.rs");
        }
        b => panic!("tool_use 블록이어야 함: {b:?}"),
    }
    // 비용: 입력 120 + 캐시 읽기 1000×0.1 = 220토큰 × $5/M + 출력 42 × $25/M
    let cost = resp.usage.cost(&m).unwrap();
    assert!((cost - (220.0 * 5.0 + 42.0 * 25.0) / 1e6).abs() < 1e-12);

    let (head, sent) = server.await.unwrap();
    let head = head.to_ascii_lowercase();
    assert!(head.starts_with("post /v1/messages"));
    assert!(head.contains("x-api-key: test-key"));
    assert!(head.contains("anthropic-version: 2023-06-01"));
    assert!(head.contains("anthropic-beta: server-side-fallback-2026-07-01"));
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["tools"][0]["name"], "read_file");
}

#[tokio::test]
async fn openai_stream_with_split_tool_call() {
    let chunks = [
        json!({"model":"qwen","choices":[{"index":0,"delta":{"role":"assistant","content":"확인"}}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read_file","arguments":"{\"pa"}}]}}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.py\"}"}}]}}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":50,"completion_tokens":9}}),
    ];
    let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    let (url, server) = mock_server(body).await;
    let mut m = config::load(None).unwrap().models["local"].clone();
    m.base_url = Some(format!("{url}/v1"));

    let mut streamed = String::new();
    let msgs = vec![Message::user_text("a.py")];
    let resp = llm::stream(
        &reqwest::Client::new(),
        &m,
        &ChatRequest { system: "sys", messages: &msgs, tools: &tools() },
        &Arc::new(AtomicBool::new(false)),
        &mut |ev| {
            if let StreamEvent::Text(t) = ev {
                streamed.push_str(t)
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(streamed, "확인");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!((resp.usage.input_tokens, resp.usage.output_tokens), (50, 9));
    assert!(matches!(&resp.blocks[1], Block::ToolUse { id, input, .. } if id == "call_a" && input["path"] == "a.py"));
    assert_eq!(resp.usage.cost(&m), Some(0.0));

    let (head, sent) = server.await.unwrap();
    assert!(head.to_ascii_lowercase().starts_with("post /v1/chat/completions"));
    assert_eq!(sent["tools"][0]["type"], "function");
    assert_eq!(sent["messages"][0]["role"], "system");
}

#[tokio::test]
async fn http_errors_are_readable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut tmp = [0u8; 8192];
        let _ = sock.read(&mut tmp).await;
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        let resp = format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        sock.write_all(resp.as_bytes()).await.unwrap();
    });
    let mut m = config::load(None).unwrap().models["smart"].clone();
    m.base_url = Some(format!("http://{addr}"));
    m.api_key = Some("bad".into());
    m.api_key_env = None;
    let msgs = vec![Message::user_text("hi")];
    let err = llm::stream(
        &reqwest::Client::new(),
        &m,
        &ChatRequest { system: "s", messages: &msgs, tools: &[] },
        &Arc::new(AtomicBool::new(false)),
        &mut |_| {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("401") && err.contains("invalid x-api-key") && err.contains("API 키"), "{err}");
}

struct FakeApprover {
    answer: bool,
    asked: Mutex<Vec<(String, String)>>,
}

impl Approver for FakeApprover {
    fn ask<'a>(&'a self, kind: &'a str, _title: &'a str, detail: &'a str) -> BoxFuture<'a, bool> {
        self.asked.lock().unwrap().push((kind.to_string(), detail.to_string()));
        Box::pin(async move { self.answer })
    }
    fn output(&self, _text: &str) {}
}

fn project(dir: &std::path::Path) -> Arc<Project> {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "/// 인사말을 만든다\npub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n").unwrap();
    let engine = lantern_context::Engine::open(dir).unwrap();
    Arc::new(Project {
        root: dir.to_path_buf(),
        engine: Arc::new(Mutex::new(engine)),
        trusted: std::sync::atomic::AtomicBool::new(true),
    })
}

#[tokio::test]
async fn tools_edit_with_approval_and_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(dir.path());
    let mut cfg = config::load(None).unwrap();
    cfg.agent.allowed_commands = vec!["echo".into()];

    // 승인하는 경우
    let yes = FakeApprover { answer: true, asked: Mutex::new(vec![]) };
    let ctx = ToolCtx { project: p.clone(), config: &cfg, approver: &yes, changed: Default::default(), originals: Default::default(), workdir: None };
    let out = ctx
        .run("edit_file", &json!({"path":"src/lib.rs","old_string":"hi {name}","new_string":"안녕 {name}"}))
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap().contains("안녕 {name}"));
    let asked = yes.asked.lock().unwrap().clone();
    assert_eq!(asked[0].0, "edit");
    assert!(asked[0].1.contains("-    format!(\"hi {name}\")") && asked[0].1.contains("+    format!(\"안녕 {name}\")"));
    assert_eq!(ctx.changed.lock().unwrap().as_slice(), ["src/lib.rs"]);

    // 읽기 도구와 맥락 엔진
    let read = ctx.run("read_file", &json!({"path":"src/lib.rs","start_line":2,"end_line":2})).await;
    assert_eq!(read.content.trim(), "2\tpub fn greet(name: &str) -> String {");
    let c = ctx.run("get_context", &json!({"query":"greet 인사말"})).await;
    assert!(!c.is_error && c.content.contains("`greet`"), "{}", c.content);

    // 목록에 있는 명령은 묻지 않고 실행
    let before = yes.asked.lock().unwrap().len();
    let echo = ctx.run("run_command", &json!({"command":"echo lantern"})).await;
    assert!(echo.content.contains("lantern") && echo.content.contains("종료 코드 0"), "{}", echo.content);
    assert_eq!(yes.asked.lock().unwrap().len(), before);

    // 거절하는 경우: 파일이 그대로여야 한다
    let no = FakeApprover { answer: false, asked: Mutex::new(vec![]) };
    let ctx2 = ToolCtx { project: p.clone(), config: &cfg, approver: &no, changed: Default::default(), originals: Default::default(), workdir: None };
    let rejected = ctx2.run("write_file", &json!({"path":"src/new.rs","content":"x"})).await;
    assert!(rejected.is_error && rejected.content.contains("거절"));
    assert!(!dir.path().join("src/new.rs").exists());
    // 목록 밖 명령, 또는 목록 명령이라도 연결 기호가 있으면 승인 필요
    let chained = ctx2.run("run_command", &json!({"command":"echo a && del x"})).await;
    assert!(chained.is_error);
    assert_eq!(no.asked.lock().unwrap().last().unwrap().0, "command");

    // 프로젝트 밖, .git, 모호한 수정은 거부
    assert!(ctx.run("write_file", &json!({"path":"../evil.txt","content":"x"})).await.is_error);
    assert!(ctx.run("write_file", &json!({"path":".git/config","content":"x"})).await.is_error);
    std::fs::write(dir.path().join("dup.txt"), "a\na\n").unwrap();
    let dup = ctx.run("edit_file", &json!({"path":"dup.txt","old_string":"a","new_string":"b"})).await;
    assert!(dup.is_error && dup.content.contains("2군데"));

    // 비밀 파일 읽기는 승인이 필요하고, 허용해도 값은 가려진다
    std::fs::write(dir.path().join(".env"), "API_KEY=\"sk-ant-api03-abcdefghijklmnopqrstuvwx\"\nDEBUG=1\n").unwrap();
    let denied = ctx2.run("read_file", &json!({"path":".env"})).await;
    assert!(denied.is_error && no.asked.lock().unwrap().last().unwrap().0 == "secret");
    let allowed = ctx.run("read_file", &json!({"path":".env"})).await;
    assert!(!allowed.is_error && !allowed.content.contains("sk-ant-api03") && allowed.content.contains("DEBUG=1"), "{}", allowed.content);

    // 제한 모드: 쓰기·실행 도구는 승인과 상관없이 거부
    p.trusted.store(false, std::sync::atomic::Ordering::Relaxed);
    let blocked = ctx.run("write_file", &json!({"path":"x.txt","content":"x"})).await;
    assert!(blocked.is_error && blocked.content.contains("제한 모드"));
    assert!(!ctx.run("read_file", &json!({"path":"src/lib.rs"})).await.is_error, "읽기는 허용");
    p.trusted.store(true, std::sync::atomic::Ordering::Relaxed);

    // 체크포인트: 수정한 파일은 원본으로 되돌린다
    let originals = ctx.originals.lock().unwrap().clone();
    assert!(originals.iter().any(|(path, c)| path.ends_with("lib.rs") && c.as_deref().unwrap().contains("hi {name}")));
    assert!(crate::state::restore(&originals).unwrap() >= 1);
    assert!(std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap().contains("hi {name}"));
    drop(ctx);
    drop(ctx2);

    // 자동 승인 glob
    cfg.agent.auto_approve = vec!["docs/**".into()];
    let never = FakeApprover { answer: false, asked: Mutex::new(vec![]) };
    let ctx3 = ToolCtx { project: p.clone(), config: &cfg, approver: &never, changed: Default::default(), originals: Default::default(), workdir: None };
    let auto = ctx3.run("write_file", &json!({"path":"docs/a.md","content":"# a"})).await;
    assert!(!auto.is_error, "{}", auto.content);
    assert!(never.asked.lock().unwrap().is_empty());

    // 새로 만든 파일은 원본이 없다 (되돌리면 휴지통으로)
    let created = ctx3.originals.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert!(created[0].1.is_none(), "새로 만든 파일은 원본 없음");
}
