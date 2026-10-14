// E2E용 모의 모델 서버 (OpenAI 호환 스트리밍) · 가짜 문서 · 가짜 임베딩과, 그 서버를 가리키는 E2E 전용 사용자 설정.

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { DIRS, HERE, TMP } from "./harness.mjs";

// ── 모의 모델 (OpenAI 호환, 스트리밍) ────────────────────────
// 1차: read_file → 2차: edit_file → 3차: 완료. 무엇을 읽고 고칠지는 시나리오가 정한다.
export const mock = { read: "src/auth/login.ts", edit: "src/auth/session.ts", old: "v + '.sig'", new: "v + '.signed'", requests: 0, testPrompt: "", image: null, embedded: 0, semanticPrompt: "", toolNames: [], verifyFailure: "", fetched: [], fetchResult: "", mentionPrompt: "", planTools: [], planSystem: "" };
export const mockServer = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    // fetch_url이 읽을 가짜 문서
    if (req.method === "GET" && req.url.startsWith("/docs/")) {
      mock.fetched.push(req.url);
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      return res.end(`<html><head><script>steal()</script></head><body><h1>${req.url === "/docs/cookie" ? "Cookie signing" : "Session lifetime"}</h1><p>Sign cookies with an HMAC; keep sessions for 7 days.</p></body></html>`);
    }
    if (req.method === "GET") {
      res.writeHead(200, { "content-type": "application/json" });
      return res.end(JSON.stringify({ data: [{ id: "e2e-model" }] }));
    }
    const j = JSON.parse(body);
    // 의미 검색 임베딩: 글자에서 정한 고정 벡터 (뜻은 없지만 늘 같은 값)
    if (req.url.endsWith("/embeddings")) {
      const input = Array.isArray(j.input) ? j.input : [j.input];
      mock.embedded += input.length;
      const vec = (t) => Array.from({ length: 8 }, (_, i) => [...t].reduce((n, c, k) => n + ((c.charCodeAt(0) * (k + 1) * (i + 3)) % 97), 1));
      res.writeHead(200, { "content-type": "application/json" });
      return res.end(JSON.stringify({ data: input.map((t, index) => ({ index, embedding: vec(t) })) }));
    }
    mock.requests++;
    // 일시적 오류: 처음 한 번은 과부하로 거절한다
    if (JSON.stringify(j.messages).includes("재시도 확인") && !mock.overloaded) {
      mock.overloaded = true;
      res.writeHead(529, { "content-type": "application/json", "retry-after": "0" });
      return res.end(JSON.stringify({ error: { message: "Overloaded" } }));
    }
    res.writeHead(200, { "content-type": "text/event-stream" });
    const send = (o) => res.write(`data: ${JSON.stringify(o)}\n\n`);
    const done = (reason) => {
      send({ choices: [{ index: 0, delta: {}, finish_reason: reason }] });
      send({ choices: [], usage: { prompt_tokens: 800, completion_tokens: 20 } });
      res.end("data: [DONE]\n\n");
    };
    const sys = String(j.messages.find((m) => m.role === "system")?.content ?? "");
    if (sys.includes("code completion engine")) {
      send({ choices: [{ index: 0, delta: { content: "a * b;" } }] });
      return done("stop");
    }
    if (sys.includes("You edit code inside an editor")) {
      mock.inlinePrompt = String(j.messages.at(-1)?.content ?? "");
      send({ choices: [{ index: 0, delta: { content: "v + '.inline'" } }] });
      return done("stop");
    }
    if (sys.includes("You write git commit messages")) {
      mock.commitPrompt = String(j.messages.at(-1)?.content ?? "");
      send({ choices: [{ index: 0, delta: { content: "FIX: 로그인 경로 이름 변경" } }] });
      return done("stop");
    }
    const tools = j.messages.filter((m) => m.role === "tool").length;
    const parts = j.messages.flatMap((m) => (m.role === "user" && Array.isArray(m.content) ? m.content : []));
    const img = parts.find((p) => p.type === "image_url");
    if (img) {
      const png = Buffer.from(img.image_url.url.split(",")[1], "base64");
      mock.image = { prefix: img.image_url.url.split(",")[0], width: png.readUInt32BE(16), height: png.readUInt32BE(20), text: parts.find((p) => p.type === "text")?.text ?? "" };
      send({ choices: [{ index: 0, delta: { content: "이미지를 봤습니다." } }] });
      return done("stop");
    }
    const first = String(j.messages.find((m) => m.role === "user")?.content ?? "");
    // 계획 먼저: 첫 요청은 읽기 도구만, 진행하면 다시 모든 도구
    if (first.includes("계획 시나리오")) {
      const names = (j.tools ?? []).map((t) => t.function?.name);
      const lastUser = String([...j.messages].reverse().find((m) => m.role === "user")?.content ?? "");
      if (lastUser.includes("계획대로 진행")) {
        mock.planTools.push(names);
        send({ choices: [{ index: 0, delta: { content: "계획대로 진행했습니다." } }] });
        return done("stop");
      }
      mock.planTools.push(names);
      mock.planSystem = String(j.messages.find((m) => m.role === "system")?.content ?? "");
      send({ choices: [{ index: 0, delta: { content: "계획: 1) src/auth/session.ts의 signCookie를 고칩니다 2) 테스트로 확인합니다" } }] });
      return done("stop");
    }
    // @로 집어 넣은 파일·심볼이 맥락에 붙었는지
    if (first.includes("멘션 확인")) {
      mock.mentionPrompt = first;
      send({ choices: [{ index: 0, delta: { content: "지정한 코드를 받았습니다." } }] });
      return done("stop");
    }
    // 긴 작업: 큰 파일을 여러 번 읽어 모델 입력 한도(로컬 모델 짐작 32k)를 넘긴다
    if (first.includes("긴 작업 확인")) {
      const all = JSON.stringify(j.messages);
      mock.compactSeen ||= all.includes("줄였습니다");
      mock.compactMax = Math.max(mock.compactMax ?? 0, all.length);
      if (tools < 8) {
        send({ choices: [{ index: 0, delta: { tool_calls: [{ index: 0, id: `call_big${tools}`, type: "function", function: { name: "read_file", arguments: JSON.stringify({ path: "big.txt" }) } }] } }] });
        return done("tool_calls");
      }
      send({ choices: [{ index: 0, delta: { content: "다 읽었습니다." } }] });
      return done("stop");
    }
    // 고친 뒤 확인: 일부러 틀리게 고치고, 테스트 실패를 받으면 되돌려 고친다
    if (first.includes("검증 시나리오")) {
      const last = j.messages.at(-1);
      const lastText = typeof last.content === "string" ? last.content : "";
      const edit = (from, to) => {
        send({ choices: [{ index: 0, delta: { tool_calls: [{ index: 0, id: `call_v${j.messages.length}`, type: "function", function: { name: "edit_file", arguments: JSON.stringify({ path: "src/auth/session.ts", old_string: from, new_string: to }) } }] } }] });
        done("tool_calls");
      };
      if (last.role === "tool") {
        send({ choices: [{ index: 0, delta: { content: "고쳤습니다." } }] });
        return done("stop");
      }
      if (lastText.includes("Lantern ran the tests")) {
        mock.verifyFailure = lastText;
        return edit("v + '.bad'", "v + '.sig'");
      }
      return edit("v + '.sig'", "v + '.bad'");
    }
    // 웹 문서: 같은 사이트의 두 페이지를 읽고 요약한다
    if (first.includes("문서를 찾아봐")) {
      const fetchCall = (url) => {
        send({ choices: [{ index: 0, delta: { tool_calls: [{ index: 0, id: `call_f${tools}`, type: "function", function: { name: "fetch_url", arguments: JSON.stringify({ url }) } }] } }] });
        done("tool_calls");
      };
      if (tools === 0) return fetchCall(`http://127.0.0.1:${MOCK_PORT}/docs/cookie`);
      if (tools === 1) return fetchCall(`http://127.0.0.1:${MOCK_PORT}/docs/session`);
      const result = String(j.messages.filter((m) => m.role === "tool").at(0)?.content ?? "");
      mock.fetchResult = result;
      send({ choices: [{ index: 0, delta: { content: "문서 요약: HMAC으로 서명" } }] });
      return done("stop");
    }
    // 외부 MCP 도구: 티켓을 찾아 그 내용으로 답한다
    if (first.includes("티켓 T-1")) {
      mock.toolNames = (j.tools ?? []).map((t) => t.function?.name);
      if (tools === 0) {
        send({ choices: [{ index: 0, delta: { tool_calls: [{ index: 0, id: "call_mcp", type: "function", function: { name: "mcp__tracker__lookup_ticket", arguments: JSON.stringify({ id: "T-1" }) } }] } }] });
        return done("tool_calls");
      }
      const result = String(j.messages.filter((m) => m.role === "tool").at(-1)?.content ?? "");
      send({ choices: [{ index: 0, delta: { content: `티켓 내용: ${result}` } }] });
      return done("stop");
    }
    if (first.includes("뜻으로 찾기 확인")) {
      mock.semanticPrompt = first;
      send({ choices: [{ index: 0, delta: { content: "맥락을 받았습니다." } }] });
      return done("stop");
    }
    if (first.includes("테스트를 만들어줘")) {
      mock.testPrompt = first;
      send({ choices: [{ index: 0, delta: { content: "테스트를 만들었습니다." } }] });
      return done("stop");
    }
    const call = (name, args) => {
      send({ choices: [{ index: 0, delta: { tool_calls: [{ index: 0, id: `call_${tools}`, type: "function", function: { name, arguments: JSON.stringify(args) } }] } }] });
      done("tool_calls");
    };
    if (tools === 0) {
      send({ choices: [{ index: 0, delta: { content: "관련 코드를 읽겠습니다." } }] });
      return call("read_file", { path: mock.read });
    }
    if (tools === 1) return call("edit_file", { path: mock.edit, old_string: mock.old, new_string: mock.new });
    send({ choices: [{ index: 0, delta: { content: "바꿨습니다." } }] });
    done("stop");
  });
});
await new Promise((r) => mockServer.listen(0, "127.0.0.1", r));
export const MOCK_PORT = mockServer.address().port;
fs.writeFileSync(path.join(DIRS.home, "config.toml"), `[models.mock]
provider = "openai"
base_url = "http://127.0.0.1:${MOCK_PORT}/v1"
model = "e2e-model"
max_tokens = 1000
price_input = 0.0
price_output = 0.0

[routing]
default = "mock"

[embeddings]
base_url = "http://127.0.0.1:${MOCK_PORT}/v1"
model = "e2e-embed"

[mcp.tracker]
command = "node"
args = [${JSON.stringify(path.join(HERE, "mcp-server.mjs"))}]

[mcp.tracker.env]
MCP_LOG = ${JSON.stringify(path.join(TMP, "mcp.log"))}

[acp.e2e]
name = "E2E"
command = "node"
args = [${JSON.stringify(path.join(HERE, "acp-agent.mjs"))}]

[acp.e2e.env]
ACP_LOG = ${JSON.stringify(path.join(TMP, "acp.log"))}
ACP_FILE = "src/api.ts"
ACP_FROM = "'/login'"
ACP_TO = "'/signin'"
`);
export const mcpLog = () => (fs.existsSync(path.join(TMP, "mcp.log")) ? fs.readFileSync(path.join(TMP, "mcp.log"), "utf8").trim().split("\n").map((l) => JSON.parse(l)) : []);
export const acpLog = () => (fs.existsSync(path.join(TMP, "acp.log")) ? fs.readFileSync(path.join(TMP, "acp.log"), "utf8").trim().split("\n").map((l) => JSON.parse(l)) : []);
