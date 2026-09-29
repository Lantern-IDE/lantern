// E2E용 가짜 외부 에이전트 (Agent Client Protocol, 줄 단위 JSON-RPC).
// 실제 Gemini CLI·Codex와 같은 순서로 움직인다: 로그인 전 session/new는 -32000 → authenticate →
// 프롬프트: 파일 읽기 요청 → 수정 권한 요청(diff) → 승인되면 쓰기 → 프로젝트 밖 쓰기 시도(거절돼야 함).
// 받은 요청은 ACP_LOG 파일에 한 줄씩 남겨 run.mjs가 확인한다.
// 환경변수: ACP_FILE(프로젝트 기준 경로), ACP_FROM, ACP_TO(바꿀 글자)
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const { ACP_LOG, ACP_FILE = "src/a.ts", ACP_FROM = "1", ACP_TO = "2" } = process.env;
const log = (o) => ACP_LOG && fs.appendFileSync(ACP_LOG, JSON.stringify(o) + "\n");
const sessions = new Map();
const waiting = new Map();
let authed = false;
let nextId = 1000;
const send = (o) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...o }) + "\n");
const ask = (method, params) => new Promise((r) => { const id = nextId++; waiting.set(id, r); send({ id, method, params }); });
const update = (sessionId, u) => send({ method: "session/update", params: { sessionId, update: u } });

async function prompt(id, p) {
  const sid = p.sessionId;
  const cwd = sessions.get(sid);
  const file = path.join(cwd, ACP_FILE);
  log({ hasContext: p.prompt[0].text.includes("<project_context>") });
  update(sid, { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "파일을 읽겠습니다. " } });
  update(sid, { sessionUpdate: "tool_call", toolCallId: "t1", title: "Read", kind: "read", status: "in_progress", locations: [{ path: file }] });
  const read = await ask("fs/read_text_file", { sessionId: sid, path: file });
  log({ read: read.result?.content ?? null });
  update(sid, { sessionUpdate: "tool_call_update", toolCallId: "t1", status: "completed" });
  const old = read.result.content;
  const next = old.replace(ACP_FROM, ACP_TO);
  update(sid, { sessionUpdate: "tool_call", toolCallId: "t2", title: "Edit", kind: "edit", status: "pending", locations: [{ path: file }] });
  const perm = await ask("session/request_permission", {
    sessionId: sid,
    toolCall: { toolCallId: "t2", title: "Edit", kind: "edit", content: [{ type: "diff", path: file, oldText: old, newText: next }] },
    options: [{ optionId: "yes", name: "Allow", kind: "allow_once" }, { optionId: "no", name: "Reject", kind: "reject_once" }],
  });
  log({ permission: perm.result?.outcome?.optionId ?? "cancelled" });
  if (perm.result?.outcome?.optionId === "yes") {
    await ask("fs/write_text_file", { sessionId: sid, path: file, content: next });
    const outside = await ask("fs/write_text_file", { sessionId: sid, path: path.join(cwd, "..", "acp-escape.txt"), content: "x" });
    log({ outside: outside.error ? "rejected" : "written" });
    update(sid, { sessionUpdate: "tool_call_update", toolCallId: "t2", status: "completed" });
    update(sid, { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "고쳤습니다." } });
  } else {
    update(sid, { sessionUpdate: "tool_call_update", toolCallId: "t2", status: "failed" });
  }
  send({ id, result: { stopReason: "end_turn" } });
}

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return;
  const m = JSON.parse(line);
  if (m.method === undefined) {
    waiting.get(m.id)?.(m);
    waiting.delete(m.id);
    return;
  }
  switch (m.method) {
    case "initialize":
      log({ clientCapabilities: m.params.clientCapabilities });
      return send({ id: m.id, result: { protocolVersion: 1, agentInfo: { name: "e2e", title: "E2E Agent", version: "1" }, authMethods: [{ id: "browser", name: "Log in with E2E" }, { type: "env_var", id: "key", name: "Use E2E_KEY" }], agentCapabilities: {} } });
    case "authenticate":
      authed = m.params.methodId === "browser";
      log({ authenticate: m.params.methodId });
      return send({ id: m.id, result: {} });
    case "session/new":
      log({ mcp: m.params.mcpServers?.[0]?.args?.[0] ?? null });
      if (!authed) return send({ id: m.id, error: { code: -32000, message: "Authentication required" } });
      sessions.set("s1", m.params.cwd);
      return send({ id: m.id, result: { sessionId: "s1" } });
    case "session/prompt":
      return void prompt(m.id, m.params);
    default:
      if (m.id !== undefined) send({ id: m.id, error: { code: -32601, message: "no" } });
  }
});
