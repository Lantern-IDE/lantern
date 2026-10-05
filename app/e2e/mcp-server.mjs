// E2E용 가짜 MCP 서버 (stdio, 줄 단위 JSON-RPC). 이슈 트래커처럼 티켓을 찾아 준다.
// 받은 도구 호출은 MCP_LOG 파일에 한 줄씩 남겨 run.mjs가 확인한다.
import fs from "node:fs";
import readline from "node:readline";

const { MCP_LOG } = process.env;
const log = (o) => MCP_LOG && fs.appendFileSync(MCP_LOG, JSON.stringify(o) + "\n");
const send = (o) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...o }) + "\n");
const TICKETS = { "T-1": "로그인 쿠키 만료가 너무 짧음 (30분 → 7일로)" };

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return;
  const m = JSON.parse(line);
  if (m.id === undefined) return; // 알림
  switch (m.method) {
    case "initialize":
      log({ initialize: m.params.clientInfo?.name });
      return send({ id: m.id, result: { protocolVersion: "2025-06-18", capabilities: { tools: {} }, serverInfo: { name: "tracker", version: "1" } } });
    case "tools/list":
      return send({ id: m.id, result: { tools: [
        { name: "lookup_ticket", description: "Look up an issue ticket by id", inputSchema: { type: "object", properties: { id: { type: "string" } }, required: ["id"] } },
        { name: "list_tickets", description: "List open tickets", inputSchema: { type: "object", properties: {} } },
      ] } });
    case "tools/call": {
      log({ call: m.params.name, args: m.params.arguments });
      if (m.params.name === "lookup_ticket") {
        const t = TICKETS[m.params.arguments?.id];
        return send({ id: m.id, result: t ? { content: [{ type: "text", text: `${m.params.arguments.id}: ${t}` }] } : { content: [{ type: "text", text: "없는 티켓" }], isError: true } });
      }
      return send({ id: m.id, result: { content: [{ type: "text", text: Object.keys(TICKETS).join(", ") }] } });
    }
    default:
      return send({ id: m.id, error: { code: -32601, message: "no" } });
  }
});
