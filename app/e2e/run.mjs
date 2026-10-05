// Lantern 실제 앱 E2E (Windows, WebView2).
// 빌드한 앱을 격리된 폴더로 띄우고(사용자 설정·데이터를 건드리지 않음), 모의 모델 서버와 임시 git 프로젝트로
// 핵심 흐름을 자동으로 조작해 확인한다.
//
// 사용:  cd app && npx tauri build --no-bundle && npm run e2e
//        npm run e2e -- --only 격리      (이름에 "격리"가 든 시나리오만)
//        npm run e2e -- --keep           (끝나도 임시 폴더를 남김)
//        npm run e2e -- --trace          (지도 그리기 단계를 콘솔로 받아, 멈추면 마지막 기록을 보여 줌)
//        npm run e2e -- --only "격리|바꾸기"  (여러 시나리오는 |로)
// 실패하면 e2e/results/<시나리오>.png 에 화면을 남긴다.

import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const EXE = path.resolve(HERE, "..", "..", "target", "release", "lantern-app.exe");
const RESULTS = path.join(HERE, "results");
const argv = process.argv.slice(2);
const ONLY = argv.includes("--only") ? argv[argv.indexOf("--only") + 1] : null;
const KEEP = argv.includes("--keep");
const HOLD = argv.includes("--hold"); // 진단용: 끝나도 앱을 띄워 둔다

if (process.platform !== "win32") {
  console.log("E2E는 지금 Windows(WebView2)에서만 돈다. 다른 OS는 단위 테스트와 빌드만 확인한다.");
  process.exit(0);
}
if (!fs.existsSync(EXE)) {
  console.error(`앱이 없습니다: ${EXE}\n먼저 cd app && npx tauri build --no-bundle`);
  process.exit(2);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── 임시 작업 폴더 ─────────────────────────────────────────
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "lantern-e2e-"));
const DIRS = { data: path.join(TMP, "data"), home: path.join(TMP, "home"), wv: path.join(TMP, "wv"), index: path.join(TMP, "index"), proj: path.join(TMP, "proj") };
for (const d of Object.values(DIRS)) fs.mkdirSync(d, { recursive: true });

function git(...args) {
  return execFileSync("git", ["-C", DIRS.proj, ...args], { encoding: "utf8" });
}

function makeProject() {
  const w = (rel, text) => {
    fs.mkdirSync(path.dirname(path.join(DIRS.proj, rel)), { recursive: true });
    fs.writeFileSync(path.join(DIRS.proj, rel), text);
  };
  w("src/auth/session.ts", "export function issueSession(user: string) {\n  return signCookie(user);\n}\n\nexport function signCookie(v: string) {\n  return v + '.sig';\n}\n");
  w("src/auth/login.ts", "import { issueSession } from './session';\n\nexport function handleLogin(name: string) {\n  const s = issueSession(name);\n  return s;\n}\n");
  w("src/api.ts", "import { handleLogin } from './auth/login';\n\nexport function route(path: string) {\n  if (path === '/login') return handleLogin('x');\n}\n");
  w("tests/session.test.ts", "import { signCookie } from '../src/auth/session';\n\nexport function testSign() {\n  return signCookie('a');\n}\n");
  w(".lantern/memory/conventions.md", "# conventions\n\n- 쿠키 서명은 signCookie 한 곳에서만 한다\n");
  git("init", "-q", "-b", "main");
  git("config", "user.email", "e2e@example.com");
  git("config", "user.name", "e2e");
  git("config", "core.autocrlf", "false");
  git("add", "-A");
  git("commit", "-qm", "init");
  for (const n of [2, 3]) {
    fs.appendFileSync(path.join(DIRS.proj, "src/auth/session.ts"), `// v${n}\n`);
    fs.appendFileSync(path.join(DIRS.proj, "tests/session.test.ts"), `// v${n}\n`);
    git("commit", "-qam", `v${n}`);
  }
  // 이 폴더를 신뢰한 것으로 (신뢰 대화상자 없이 시작)
  const key = DIRS.proj.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();
  fs.writeFileSync(path.join(DIRS.data, "trusted.json"), JSON.stringify([key]));
}

const file = (rel) => fs.readFileSync(path.join(DIRS.proj, rel), "utf8");

// ── 모의 모델 (OpenAI 호환, 스트리밍) ────────────────────────
// 1차: read_file → 2차: edit_file → 3차: 완료. 무엇을 읽고 고칠지는 시나리오가 정한다.
const mock = { read: "src/auth/login.ts", edit: "src/auth/session.ts", old: "v + '.sig'", new: "v + '.signed'", requests: 0, testPrompt: "", image: null, embedded: 0, semanticPrompt: "", toolNames: [], verifyFailure: "" };
const mockServer = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
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
const MOCK_PORT = mockServer.address().port;
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
const mcpLog = () => (fs.existsSync(path.join(TMP, "mcp.log")) ? fs.readFileSync(path.join(TMP, "mcp.log"), "utf8").trim().split("\n").map((l) => JSON.parse(l)) : []);
const acpLog = () => (fs.existsSync(path.join(TMP, "acp.log")) ? fs.readFileSync(path.join(TMP, "acp.log"), "utf8").trim().split("\n").map((l) => JSON.parse(l)) : []);

// ── 앱 띄우기와 CDP ────────────────────────────────────────
let app = null;
let cdpPort = 0;
let ws = null;
let msgId = 0;
const pending = new Map();

/** 앱을 띄운다. dir: 열 폴더 (기본은 시나리오용 프로젝트) */
async function launch(dir = DIRS.proj) {
  cdpPort = 9400 + Math.floor(Math.random() * 500);
  app = spawn(EXE, [dir], {
    env: {
      ...process.env,
      LANTERN_HOME: DIRS.home,
      LANTERN_DATA_DIR: DIRS.data,
      LANTERN_WEBVIEW_DATA_DIR: DIRS.wv,
      LANTERN_INDEX_DIR: DIRS.index,
      LANTERN_E2E_CDP_PORT: String(cdpPort),
    },
    stdio: "ignore",
  });
  let page = null;
  let seen = "연결 포트 응답 없음";
  // CI 가상 머신은 첫 실행(WebView2 준비)이 느리다
  for (let i = 0; i < (process.env.CI ? 180 : 60) && !page && app.exitCode === null; i++) {
    await sleep(500);
    try {
      const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json`)).json();
      seen = targets.map((t) => `${t.type} ${t.url}`).join(", ") || "화면 없음";
      page = targets.find((t) => t.type === "page" && t.url.includes("tauri.localhost"));
    } catch { /* 아직 안 뜸 */ }
  }
  if (!page) throw new Error(`앱 화면에 연결하지 못했습니다 (앱 ${app.exitCode === null ? "실행 중" : `종료 코드 ${app.exitCode}`}, 본 것: ${seen})${appLogTail()}`);
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => {
    ws.addEventListener("open", r, { once: true });
    ws.addEventListener("error", j, { once: true });
  });
  ws.addEventListener("message", (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) {
      pending.get(m.id)(m);
      pending.delete(m.id);
    }
    if (m.method === "Debugger.paused") onPaused?.(m.params);
    if (m.method === "Runtime.consoleAPICalled") {
      const text = (m.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ");
      if (text.startsWith("[lantern-trace]")) {
        traceLog.push(`${new Date().toISOString().slice(11, 23)} ${text.slice(14)}`);
        if (traceLog.length > 30) traceLog.shift();
      }
    }
    if (m.method === "Debugger.scriptParsed") scripts.set(m.params.scriptId, m.params.url);
  });
  hangReported = false;
  if (argv.includes("--trace")) {
    await cdp("Runtime.enable");
    await cdp("Runtime.evaluate", { expression: "localStorage.setItem('lantern.trace','1')", returnByValue: true });
    await sleep(1200);
  }
  // 시나리오는 한국어 화면 기준이다. 처음 켜면 시스템 언어를 따르므로(CI는 영어) 한국어로 고정한다.
  // 앱 문서가 열리기 전(빈 페이지)에는 localStorage에 접근할 수 없으니 먼저 기다린다.
  await waitFor(() => location.hostname === "tauri.localhost" && document.readyState !== "loading" && !!document.documentElement.lang, 30000, "앱 문서");
  if ((await run(() => document.documentElement.lang)) !== "ko") {
    await run(() => {
      localStorage.setItem("lang", JSON.stringify("ko"));
      setTimeout(() => location.reload(), 50);
      return true;
    });
    await sleep(1500);
    await waitFor(() => document.documentElement.lang === "ko", 20000, "한국어 화면");
  }
  // 프로젝트를 열고 지도를 그릴 때까지
  await waitFor((name) => document.querySelector("#cc-label")?.textContent === name && !!document.querySelector("#map-view:not(.hidden)") && !!document.querySelector("#map-crumb .cur"), 30000, "프로젝트 열림", path.basename(dir));
}

/** 앱이 뜨지 않았을 때 원인을 볼 수 있게 앱 로그와 충돌 보고서의 끝부분 */
function appLogTail() {
  const dir = path.join(DIRS.data, "logs");
  if (!fs.existsSync(dir)) return "\n      (앱 로그 없음: 앱이 로그를 쓰기 전에 멈춤)";
  return fs.readdirSync(dir).filter((f) => f.endsWith(".log") || f.startsWith("crash-")).map((f) => {
    const lines = fs.readFileSync(path.join(dir, f), "utf8").trim().split("\n").slice(-15);
    return `\n      ── ${f}\n        ${lines.join("\n        ")}`;
  }).join("");
}

let onPaused = null;
const traceLog = [];
let hangReported = false;
const scripts = new Map();

/** 자바스크립트가 멈췄을 때: 디버거로 멈추게 해서 어디서 돌고 있는지 기록한다 */
async function diagnoseHang() {
  if (hangReported) return "";
  hangReported = true;
  const paused = new Promise((r) => {
    onPaused = r;
    setTimeout(() => r(null), 5000);
  });
  ws.send(JSON.stringify({ id: ++msgId, method: "Debugger.enable" }));
  ws.send(JSON.stringify({ id: ++msgId, method: "Debugger.pause" }));
  const p = await paused;
  onPaused = null;
  if (!p) return `(디버거로도 멈추지 못함: 자바스크립트 밖에서 막혔을 수 있음)${traceLog.length ? `\n      마지막 지도 기록:\n        ${traceLog.slice(-12).join("\n        ")}` : ""}`;
  const frames = p.callFrames.slice(0, 8).map((f) => `${f.functionName || "(익명)"} @ ${(scripts.get(f.location.scriptId) || f.url || "").split("/").pop()}:${f.location.lineNumber + 1}:${f.location.columnNumber + 1}`);
  ws.send(JSON.stringify({ id: ++msgId, method: "Debugger.resume" }));
  const text = `멈춘 곳:\n        ${frames.join("\n        ")}`;
  fs.mkdirSync(RESULTS, { recursive: true });
  fs.writeFileSync(path.join(RESULTS, "hang-stack.txt"), JSON.stringify(p.callFrames.slice(0, 8).map((f) => ({ fn: f.functionName, url: scripts.get(f.location.scriptId) || f.url, line: f.location.lineNumber, col: f.location.columnNumber })), null, 2));
  return text;
}

/** 앱이 멈추면(자바스크립트가 돌지 않으면) 영원히 기다리지 않고 실패로 끝낸다 */
function cdp(method, params = {}, timeout = 20000) {
  return new Promise((resolve, reject) => {
    const id = ++msgId;
    const timer = setTimeout(async () => {
      pending.delete(id);
      const where = await diagnoseHang();
      reject(new Error(`앱이 ${timeout / 1000}초 동안 응답하지 않음 (${method}) — 화면이 멈췄을 수 있음${where ? `\n      ${where}` : ""}`));
    }, timeout);
    pending.set(id, (m) => {
      clearTimeout(timer);
      resolve(m);
    });
    ws.send(JSON.stringify({ id, method, params }));
  });
}

/** 앱 안에서 함수를 실행하고 값을 돌려받는다 (함수는 문자열로 옮겨지므로 바깥 변수를 쓸 수 없다. 인자로 넘긴다) */
async function run(fn, ...args) {
  const expression = `(async () => (${fn.toString()})(...${JSON.stringify(args)}))()`;
  const r = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? "앱 안에서 예외");
  return r.result?.result?.value;
}

async function waitFor(fn, timeout, what, ...args) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await run(fn, ...args);
      if (last) return last;
    } catch (e) {
      if (String(e.message).includes("응답하지 않음")) throw e;
      last = e.message;
    }
    await sleep(250);
  }
  throw new Error(`기다림 초과: ${what}${last ? ` (마지막 값: ${JSON.stringify(last).slice(0, 200)})` : ""}`);
}

async function screenshot(name) {
  try {
    const r = await cdp("Page.captureScreenshot", { format: "png" }, 8000);
    fs.mkdirSync(RESULTS, { recursive: true });
    fs.writeFileSync(path.join(RESULTS, `${name}.png`), Buffer.from(r.result.data, "base64"));
  } catch { /* 화면이 없으면 건너뜀 */ }
}

/** 네이티브 대화상자(되돌리기 확인 등)의 단추를 누른다. index: 단추 순서 (0부터) */
function clickDialog(index) {
  const ps = path.join(HERE, "dialog.ps1");
  return execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps, "-ProcId", String(app.pid), "-Index", String(index)], { encoding: "utf8" });
}

async function waitDialog(index, timeout = 8000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const out = clickDialog(index);
    if (out.includes("클릭")) return out;
    await sleep(300);
  }
  throw new Error("대화상자가 뜨지 않았습니다");
}

async function stop() {
  try {
    ws?.close();
  } catch { /* 이미 닫힘 */ }
  if (app && app.exitCode === null) app.kill();
  await sleep(500);
}

// ── 앱 안에서 쓰는 조작 (문자열로 옮겨 실행) ───────────────────
const ui = {
  sendTask: async (text, agent, isolate) => {
    const sel = document.querySelector("#agent-select");
    sel.value = agent;
    sel.dispatchEvent(new Event("change"));
    const iso = document.querySelector("#isolate-task");
    if (!iso.disabled) iso.checked = isolate;
    const ta = document.querySelector("#prompt");
    ta.value = text;
    ta.dispatchEvent(new Event("input"));
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    return true;
  },
  approve: () => {
    const b = [...document.querySelectorAll(".task-log:not(.hidden) .approval .actions button")].find((x) => x.textContent.includes("적용"));
    b?.click();
    return !!b;
  },
};

// ── 시나리오 ──────────────────────────────────────────────
const scenarios = [];
const scenario = (name, fn) => scenarios.push({ name, fn });

scenario("프로젝트를 열면 코드 지도와 빈 작업 목록", async () => {
  const s = await run(() => ({
    crumb: document.querySelector("#map-crumb")?.textContent ?? "",
    nodes: Number(document.querySelector(".map-count")?.textContent?.replace(/\D/g, "") || 0),
    empty: !!document.querySelector("#task-list .empty-view"),
  }));
  if (!s.crumb.includes("전체 구조")) throw new Error(`지도 제목: ${s.crumb}`);
  if (s.nodes < 4) throw new Error(`지도 노드 ${s.nodes}개`);
  if (!s.empty) throw new Error("작업 목록이 비어 있지 않음");
});

scenario("제목 표시줄의 빈 곳은 어디든 창을 끌 수 있다", async () => {
  // 창 테두리를 직접 그려서, data-tauri-drag-region이 붙은 요소를 직접 눌러야 끌린다(자식은 해당 없음).
  // 제목 표시줄을 가로로 훑어 단추·입력이 아닌 곳이 모두 끌기 영역인지 본다.
  const bad = await run(() => {
    const bar = document.querySelector("#titlebar").getBoundingClientRect();
    const out = [];
    for (let x = 1; x < bar.width - 1; x += 4) {
      for (const y of [bar.top + 3, bar.top + bar.height / 2, bar.bottom - 3]) {
        const el = document.elementFromPoint(x, y);
        if (!el || el.closest("button, input, select, a, [role=tab], [role=tablist], [role=menuitem]")) continue;
        if (!el.hasAttribute("data-tauri-drag-region")) out.push(`x=${Math.round(x)} ${el.tagName.toLowerCase()}${el.id ? "#" + el.id : ""}${typeof el.className === "string" && el.className ? "." + el.className.split(" ")[0] : ""}`);
      }
    }
    return [...new Set(out)].slice(0, 8);
  });
  if (bad.length) throw new Error(`끌리지 않는 빈 곳: ${bad.join(", ")}`);
});

scenario("에이전트 수정: 승인 카드의 영향 반경 → 적용 → 발자취", async () => {
  mock.old = "v + '.sig'";
  mock.new = "v + '.signed'";
  await run(ui.sendTask, "서명 접미사를 바꿔줘", "code", false);
  const impact = await waitFor(() => document.querySelector(".task-log:not(.hidden) .approval .impact:not(.loading)")?.innerText, 20000, "영향 반경");
  if (!impact.includes("signCookie") || !impact.includes("직접 호출")) throw new Error(`영향 반경: ${impact}`);
  if (file("src/auth/session.ts").includes(".signed")) throw new Error("승인 전에 파일이 바뀜");
  await run(ui.approve);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .changed"), 15000, "완료");
  if (!file("src/auth/session.ts").includes("'.signed'")) throw new Error("적용 후 파일이 그대로");
  const meta = await run(() => document.querySelector("#task-list .task.active .task-meta")?.textContent ?? "");
  if (!meta.includes("수정 1") || !meta.includes("읽음 1")) throw new Error(`발자취: ${meta}`);
});

scenario("되돌리기: 확인 후 원래 파일", async () => {
  await run(() => {
    setTimeout(() => document.querySelector(".task-log:not(.hidden) .ch-undo")?.click(), 50);
    return true;
  });
  await waitDialog(0);
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .changed")?.classList.contains("reverted"), 8000, "되돌림 표시");
  if (!file("src/auth/session.ts").includes("'.sig'")) throw new Error("파일이 되돌아가지 않음");
});

scenario("격리 작업: 작업 공간에서만 바뀌고, 적용해야 프로젝트에 반영", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  mock.old = "v + '.sig'";
  mock.new = "v + '.iso'";
  await run(ui.sendTask, "격리해서 바꿔줘", "code", true);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .approval .actions"), 20000, "승인 카드");
  await run(ui.approve);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .iso-box .iso-actions"), 20000, "격리 결과 상자");
  if (file("src/auth/session.ts").includes(".iso")) throw new Error("적용 전에 프로젝트가 바뀜");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .iso-box button")].find((b) => b.textContent.includes("프로젝트에 적용"))?.click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .iso-box")?.classList.contains("done"), 15000, "적용 완료");
  if (!file("src/auth/session.ts").includes("'.iso'")) throw new Error("적용 후 프로젝트가 그대로");
  git("checkout", "-q", "--", ".");
});

scenario("파일에서 바꾸기와 되돌리기", async () => {
  await run(() => {
    document.querySelector('.ab-item[data-view="search"]').click();
    const q = document.querySelector("#search-input");
    q.value = "handleLogin";
    q.dispatchEvent(new Event("input"));
    return true;
  });
  await waitFor(() => /2개 파일/.test(document.querySelector("#search-summary")?.textContent ?? ""), 8000, "검색 결과");
  await run(() => {
    if (document.querySelector("#replace-row").classList.contains("hidden")) document.querySelector("#search-expand").click();
    document.querySelector("#replace-input").value = "handleSignIn";
    setTimeout(() => document.querySelector("#replace-all").click(), 50);
    return true;
  });
  await waitDialog(0);
  // "곳을 바꿨습니다"로 찾는다. "바꿨습니다"만으로는 에이전트 수정 알림("Lantern이 파일 N개를 바꿨습니다")도 걸려서,
  // CI처럼 느린 환경에서 앞 시나리오의 알림이 남아 있으면 되돌리기 단추가 없는 알림을 고른다.
  try {
    await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("곳을 바꿨습니다")), process.env.CI ? 20000 : 8000, "바꾸기 알림");
  } catch (e) {
    const dialog = execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(HERE, "dialog.ps1"), "-ProcId", String(app.pid)], { encoding: "utf8" }).trim();
    const toasts = await run(() => [...document.querySelectorAll(".toast")].map((t) => t.innerText.replace(/\s+/g, " ")).join(" | "));
    throw new Error(`${e.message}\n      알림: ${toasts || "없음"}\n      파일: ${file("src/api.ts").includes("handleSignIn") ? "바뀜" : "그대로"}\n      ${dialog.replace(/\n/g, "\n      ")}`);
  }
  if (!file("src/api.ts").includes("handleSignIn") || !file("src/auth/login.ts").includes("handleSignIn")) throw new Error("바꾸지 않음");
  const clicked = await run(() => {
    const t = [...document.querySelectorAll(".toast")].find((x) => x.innerText.includes("곳을 바꿨습니다"));
    const b = t && [...t.querySelectorAll("button")].find((x) => x.textContent.includes("되돌리기"));
    b?.click();
    return !!b;
  });
  if (!clicked) throw new Error("바꾸기 알림에 되돌리기 단추가 없음");
  try {
    await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("되돌렸습니다")), process.env.CI ? 20000 : 8000, "되돌림 알림");
  } catch (e) {
    // 네이티브 대화상자(예: 충돌 확인)는 화면 캡처에 찍히지 않으니 문구를 남긴다
    const dialog = execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(HERE, "dialog.ps1"), "-ProcId", String(app.pid)], { encoding: "utf8" }).trim();
    const toasts = await run(() => [...document.querySelectorAll(".toast")].map((t) => t.innerText.replace(/\s+/g, " ")).join(" | "));
    throw new Error(`${e.message}\n      알림: ${toasts || "없음"}\n      ${dialog.replace(/\n/g, "\n      ")}`);
  }
  if (file("src/api.ts").includes("handleSignIn")) throw new Error("되돌리지 않음");
});

scenario("지도: 검색해서 주변 보기", async () => {
  await run(() => {
    document.querySelector('#canvas-switch [data-canvas="map"]').click();
    const i = document.querySelector("#map-search");
    i.value = "signCookie";
    i.dispatchEvent(new Event("input"));
    return true;
  });
  await waitFor(() => !!document.querySelector("#map-results .mr-row"), 5000, "지도 검색 결과");
  await run(() => {
    document.querySelector("#map-results .mr-row").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    return true;
  });
  const crumb = await waitFor(() => (document.querySelector("#map-crumb")?.textContent ?? "").includes("signCookie") && document.querySelector("#map-crumb").textContent, 8000, "주변 보기");
  const n = Number(crumb.replace(/.*signCookie/, "").replace(/\D/g, ""));
  if (n < 3) throw new Error(`주변 노드 ${n}개 (호출자 issueSession, testSign과 파일이 있어야 함)`);
});

/** 소스 제어 뷰를 연다 (이미 보이는 뷰의 아이콘을 다시 누르면 사이드바가 접히므로 안 보일 때만 누른다) */
const openScm = () => run(() => {
  if (!document.querySelector("#view-scm")?.offsetParent) document.querySelector('.ab-item[data-view="scm"]').click();
  return true;
});

/** 메뉴 팝업에서 글자가 든 항목을 누른다 */
const pickMenu = (text) => run((t) => {
  const it = [...document.querySelectorAll("#menu-popup .menu-item")].find((x) => x.textContent.includes(t));
  it?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  return !!it;
}, text);

scenario("소스 제어: 원격 연결, 커밋 이력, 푸시, 브랜치 만들기·전환", async () => {
  // 원격 저장소 흉내 (로컬 bare 저장소)
  const bare = path.join(TMP, "remote.git");
  execFileSync("git", ["init", "-q", "--bare", "-b", "main", bare]);
  git("remote", "add", "origin", bare);
  await openScm();
  await waitFor(() => document.querySelector("#scm-sync .scm-branch-btn")?.textContent.includes("main") && document.querySelector("#scm-sync .scm-remote")?.textContent.includes("remote"), 10000, "브랜치와 원격 표시");

  // 커밋 이력: 3개 (init, v2, v3), 커밋을 펼치면 바뀐 파일
  await run(() => {
    [...document.querySelectorAll(".scm-history .scm-group")][0].click();
    return true;
  });
  const subjects = await waitFor(() => {
    const rows = [...document.querySelectorAll(".scm-commit-row .csubject")].map((x) => x.textContent);
    return rows.length >= 3 && rows;
  }, 10000, "커밋 이력");
  if (subjects[0] !== "v3" || !subjects.includes("init")) throw new Error(`커밋 이력: ${subjects.join(", ")}`);
  await run(() => {
    document.querySelector(".scm-commit-row").click();
    return true;
  });
  const files = await waitFor(() => {
    const f = [...document.querySelectorAll(".scm-commit-files .scm-file .fname")].map((x) => x.textContent);
    return f.length && f;
  }, 10000, "커밋의 바뀐 파일");
  if (!files.includes("session.ts")) throw new Error(`v3에서 바뀐 파일: ${files.join(", ")}`);

  // 푸시: 업스트림이 없으면 origin에 올리고 연결
  await run(() => {
    [...document.querySelectorAll("#scm-sync .scm-sync-actions button")].find((b) => b.title.startsWith("푸시")).click();
    return true;
  });
  await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("원격에 올렸습니다")), 20000, "푸시 알림");
  const pushed = execFileSync("git", ["--git-dir", bare, "rev-parse", "main"], { encoding: "utf8" }).trim();
  if (pushed !== git("rev-parse", "HEAD").trim()) throw new Error("원격에 올라간 커밋이 다름");

  // 새 브랜치 만들기 → 상태 표시줄에도 → 다시 main으로
  await run(() => {
    document.querySelector("#scm-sync .scm-branch-btn").click();
    return true;
  });
  await waitFor(() => !!document.querySelector("#menu-popup:not(.hidden) .menu-item"), 5000, "브랜치 메뉴");
  if (!(await pickMenu("새 브랜치 만들기"))) throw new Error("브랜치 메뉴에 '새 브랜치 만들기' 없음");
  await run(() => {
    const i = document.querySelector(".scm-newbranch");
    i.value = "e2e/feature";
    i.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    return true;
  });
  await waitFor(() => document.querySelector("#scm-sync .scm-branch-btn")?.textContent.includes("e2e/feature") && document.querySelector("#sb-branch")?.textContent.includes("e2e/feature"), 10000, "새 브랜치로 전환");
  if (git("branch", "--show-current").trim() !== "e2e/feature") throw new Error("git 브랜치가 바뀌지 않음");
  await run(() => {
    document.querySelector("#scm-sync .scm-branch-btn").click();
    return true;
  });
  await waitFor(() => !!document.querySelector("#menu-popup:not(.hidden) .menu-item"), 5000, "브랜치 메뉴");
  if (!(await pickMenu("main"))) throw new Error("브랜치 메뉴에 main 없음");
  await waitFor(() => document.querySelector("#scm-sync .scm-branch-btn")?.textContent.includes("main"), 10000, "main으로 전환");
  if (git("branch", "--show-current").trim() !== "main") throw new Error("main으로 돌아가지 않음");
});

scenario("외부 에이전트(ACP): 로그인 카드 → 로그인 → 읽기 → 승인 카드 → 적용", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "로그인 경로를 /signin으로 바꿔줘", "acp:e2e", false);
  // 로그인 전: 로그인 카드 (환경변수 방법은 버튼이 아니라 안내)
  const card = await waitFor(() => {
    const c = document.querySelector(".task-log:not(.hidden) .auth-card");
    return c && { buttons: [...c.querySelectorAll("button")].map((b) => b.textContent), text: c.innerText };
  }, 30000, "로그인 카드");
  if (card.buttons.length !== 1 || !card.buttons[0].includes("Log in with E2E") || !card.text.includes("E2E_KEY")) throw new Error(`로그인 카드: ${JSON.stringify(card)}`);
  const init = acpLog().find((l) => l.clientCapabilities);
  if (!init?.clientCapabilities?.fs?.writeTextFile || init.clientCapabilities.terminal !== false) throw new Error(`클라이언트 기능: ${JSON.stringify(init)}`);
  if (acpLog().find((l) => "mcp" in l)?.mcp !== "--mcp") throw new Error("Lantern MCP 서버를 넘기지 않음");
  await run(() => {
    document.querySelector(".task-log:not(.hidden) .auth-card button").click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden) .auth-card .auth-status")?.textContent.includes("로그인되었습니다"), 15000, "로그인");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .auth-card button")].find((b) => b.textContent.includes("다시 보내기")).click();
    return true;
  });
  // 수정 권한 요청 → 기존과 같은 승인 카드와 영향 반경
  const impact = await waitFor(() => document.querySelector(".task-log:not(.hidden) .approval .impact:not(.loading)")?.innerText, 30000, "영향 반경");
  if (!impact.includes("직접 호출") && !impact.includes("찾지 못했습니다")) throw new Error(`영향 반경: ${impact}`);
  if (file("src/api.ts").includes("/signin")) throw new Error("승인 전에 파일이 바뀜");
  if (!acpLog().some((l) => l.hasContext) || !acpLog().some((l) => typeof l.read === "string" && l.read.includes("/login"))) throw new Error("맥락 또는 파일 읽기가 에이전트에 가지 않음");
  await run(ui.approve);
  await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .changed"), 15000, "완료");
  if (!file("src/api.ts").includes("'/signin'")) throw new Error("적용 후 파일이 그대로");
  if (!acpLog().some((l) => l.outside === "rejected") || fs.existsSync(path.join(TMP, "acp-escape.txt"))) throw new Error("프로젝트 밖 쓰기를 막지 못함");
  const meta = await run(() => document.querySelector("#task-list .task.active .task-meta")?.textContent ?? "");
  if (!meta.includes("수정 1")) throw new Error(`발자취: ${meta}`);
});

scenario("커밋 전 영향 검토와 AI 커밋 메시지", async () => {
  // 사람이 직접 고친 변경: signCookie 본문 (호출자 issueSession, 테스트 session.test.ts)
  fs.writeFileSync(path.join(DIRS.proj, "src/auth/session.ts"), file("src/auth/session.ts").replace("v + '.sig'", "v + '.sig2'"));
  await run(() => {
    document.querySelector('.ab-item[data-view="scm"]').click();
    return true;
  });
  await waitFor(() => !document.querySelector("#scm-commit").classList.contains("hidden"), 15000, "소스 제어");
  await run(() => {
    document.querySelector("#scm-review-btn").click();
    return true;
  });
  const rv = await waitFor(() => {
    const box = document.querySelector("#scm-review:not(.hidden)");
    const rows = box ? [...box.querySelectorAll(".rv-file")].map((r) => r.innerText.replace(/\s+/g, " ")) : [];
    return rows.length ? { head: box.querySelector(".rv-head").innerText, rows } : null;
  }, 20000, "영향 검토 결과");
  const row = rv.rows.find((r) => r.includes("session.ts"));
  // signCookie를 부르는 곳: issueSession, 테스트의 testSign
  if (!row || !/직접 2/.test(row) || !/테스트 1/.test(row)) throw new Error(`영향 검토: ${JSON.stringify(rv)}`);
  // 파일을 누르면 지도에서 영향 범위
  await run(() => {
    [...document.querySelectorAll("#scm-review .rv-file")].find((r) => r.innerText.includes("session.ts")).click();
    return true;
  });
  await waitFor(() => document.querySelector("#map-crumb")?.textContent.includes("영향"), 10000, "지도의 영향 범위");
  // AI 커밋 메시지
  await run(() => {
    document.querySelector("#scm-gen-msg").click();
    return true;
  });
  const msg = await waitFor(() => document.querySelector("#scm-message").value, 15000, "커밋 메시지");
  if (msg !== "FIX: 로그인 경로 이름 변경") throw new Error(`커밋 메시지: ${msg}`);
  if (!mock.commitPrompt.includes(".sig2") || !mock.commitPrompt.includes("Recent commit subjects")) throw new Error("커밋 메시지 요청에 diff·최근 커밋이 없음");
  // 다음 시나리오를 위해 되돌린다
  fs.writeFileSync(path.join(DIRS.proj, "src/auth/session.ts"), file("src/auth/session.ts").replace("'.sig2'", "'.sig'"));
  await run(() => {
    document.querySelector("#scm-message").value = "";
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("테스트 없는 변경 → 테스트 만들기: 기존 테스트를 예시로 새 작업", async () => {
  // handleLogin은 route가 부르지만 테스트가 없다
  fs.writeFileSync(path.join(DIRS.proj, "src/auth/login.ts"), file("src/auth/login.ts").replace("return s;", "return s.trim();"));
  await run(() => {
    document.querySelector('.ab-item[data-view="scm"]').click();
    return true;
  });
  await waitFor(() => !document.querySelector("#scm-commit").classList.contains("hidden"), 15000, "소스 제어");
  await run(() => {
    document.querySelector("#scm-review-btn").click();
    return true;
  });
  await waitFor(() => {
    const b = [...document.querySelectorAll("#scm-review .rv-actions button")].find((x) => x.textContent.includes("테스트 만들기"));
    if (!b) return false;
    b.click();
    b.click(); // 두 번 눌러도 작업은 하나
    return true;
  }, 20000, "테스트 만들기 버튼");
  const prompt = await waitFor(() => document.querySelector("#messages")?.innerText.includes("테스트를 만들었습니다") && true, 20000, "테스트 작업의 응답");
  if (!prompt || !mock.testPrompt.includes("src/auth/login.ts: handleLogin") || !mock.testPrompt.includes("tests/session.test.ts")) throw new Error(`테스트 요청: ${mock.testPrompt}`);
  if (mock.testPrompt.includes("session.ts:")) throw new Error("테스트가 있는 파일까지 요청함");
  const made = await run(() => [...document.querySelectorAll("#task-list .task .task-title")].filter((x) => x.textContent.includes("테스트를 만들어줘")).length);
  if (made !== 1) throw new Error(`테스트 작업 ${made}개`);
  git("checkout", "--", "src/auth/login.ts");
  await run(() => {
    document.querySelector("#scm-review .rv-head .codicon-close")?.closest("button")?.click();
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("채팅에 이미지 붙여넣기: 미리보기 → 줄여서 모델에 보냄", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  // 3000×2000 PNG를 붙여 넣는다 (긴 변 1568로 줄어야 한다)
  await run(async () => {
    const c = document.createElement("canvas");
    c.width = 3000;
    c.height = 2000;
    const g = c.getContext("2d");
    g.fillStyle = "#22d3ee";
    g.fillRect(0, 0, 3000, 2000);
    const blob = await new Promise((r) => c.toBlob(r, "image/png"));
    const dt = new DataTransfer();
    dt.items.add(new File([blob], "shot.png", { type: "image/png" }));
    document.querySelector("#prompt").dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    return true;
  });
  await waitFor(() => document.querySelectorAll("#attachments .attach-thumb").length === 1, 10000, "미리보기");
  await run(ui.sendTask, "이 화면에서 버튼 색을 알려줘", "ask", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("이미지를 봤습니다"), 20000, "이미지에 대한 답");
  if (!mock.image || mock.image.prefix !== "data:image/png;base64" || mock.image.width !== 1568 || mock.image.height !== 1045) throw new Error(`보낸 이미지: ${JSON.stringify(mock.image)}`);
  if (!mock.image.text.includes("버튼 색")) throw new Error("이미지와 함께 글이 가지 않음");
  const ui2 = await run(() => ({
    thumbs: document.querySelectorAll(".task-log:not(.hidden) .user-images img").length,
    pending: document.querySelectorAll("#attachments .attach-thumb").length,
  }));
  if (ui2.thumbs !== 1 || ui2.pending !== 0) throw new Error(`화면: ${JSON.stringify(ui2)}`);
});

scenario("의미 검색: 뒤에서 코드 조각을 임베딩하고, 질문에 뜻이 가까운 코드를 붙인다", async () => {
  // 프로젝트를 열면 인덱싱 뒤에 조각 임베딩이 돈다 (코드 파일 4개 → 조각 4개 이상)
  const end = Date.now() + 20000;
  while (mock.embedded < 4 && Date.now() < end) await sleep(250);
  if (mock.embedded < 4) throw new Error(`임베딩한 조각 ${mock.embedded}개`);
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "뜻으로 찾기 확인", "ask", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("맥락을 받았습니다"), 20000, "답");
  // 키워드가 하나도 안 맞는 질문인데도 뜻이 가까운 코드가 맥락에 들어간다
  if (!/뜻이 가까움 \(\d+위\)/.test(mock.semanticPrompt)) throw new Error(`맥락: ${mock.semanticPrompt.slice(0, 600)}`);
  const hidden = await run(() => document.querySelector("#sb-semantic").classList.contains("hidden"));
  if (!hidden) throw new Error("다 만들었는데 진행률이 남아 있음");
});

scenario("설정 화면에서 의미 검색: 연결 시험 → 끄기 → 직접 입력으로 다시 켜기", async () => {
  const cfgPath = path.join(DIRS.home, "config.toml");
  await run(() => {
    document.querySelector("#ab-settings").click();
    return true;
  });
  // 기본 포트가 아닌 로컬 서버는 '직접 입력'으로 보여야 한다 (로컬 찾기 결과로 덮어쓰지 않게)
  const shown = await waitFor(() => {
    const box = document.querySelector("#set-embed");
    const on = box?.querySelector(".segmented button.on")?.getAttribute("data-kind");
    const url = box?.querySelector('input[aria-label="주소"]')?.value;
    return on && url ? { on, url } : null;
  }, 10000, "의미 검색 설정");
  if (shown.on !== "custom" || !shown.url.includes("127.0.0.1")) throw new Error(`설정 표시: ${JSON.stringify(shown)}`);
  await run(() => {
    [...document.querySelectorAll("#set-embed button")].find((b) => b.textContent.includes("연결 시험")).click();
    return true;
  });
  const ok = await waitFor(() => document.querySelector("#set-embed .test-result.ok")?.textContent, 10000, "연결 시험");
  if (!ok.includes("8차원")) throw new Error(`연결 시험: ${ok}`);
  // 끄면 설정 파일에서 [embeddings]가 빠진다
  await run(() => {
    document.querySelector('#set-embed [data-kind="off"]').click();
    return true;
  });
  const end = Date.now() + 5000;
  while (fs.readFileSync(cfgPath, "utf8").includes("[embeddings]") && Date.now() < end) await sleep(100);
  if (fs.readFileSync(cfgPath, "utf8").includes("[embeddings]")) throw new Error("끈 뒤에도 [embeddings]가 남음");
  // 직접 입력으로 다시 저장
  await run(() => {
    document.querySelector('#set-embed [data-kind="custom"]').click();
    return true;
  });
  await run(() => {
    [...document.querySelectorAll("#set-embed button")].find((b) => b.textContent.trim() === "저장").click();
    return true;
  });
  const end2 = Date.now() + 5000;
  while (!fs.readFileSync(cfgPath, "utf8").includes("[embeddings]") && Date.now() < end2) await sleep(100);
  const cfg = fs.readFileSync(cfgPath, "utf8");
  if (!cfg.includes("[embeddings]") || !cfg.includes(`127.0.0.1:${MOCK_PORT}/v1`) || !cfg.includes('model = "e2e-embed"')) throw new Error(`다시 켠 설정:
${cfg}`);
});

scenario("외부 MCP 서버: 에이전트 도구로 보이고, 승인 카드 → 실행 → 결과로 답", async () => {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "티켓 T-1 확인해줘", "code", false);
  const card = await waitFor(() => {
    const c = [...document.querySelectorAll(".task-log:not(.hidden) .approval:not(.resolved)")].at(-1);
    return c && { title: c.querySelector(".atitle")?.textContent ?? "", plug: !!c.querySelector(".ahead .codicon-plug"), detail: c.querySelector("pre")?.textContent ?? "" };
  }, 30000, "MCP 승인 카드");
  if (!card.title.includes("tracker · lookup_ticket") || !card.plug || !card.detail.includes("T-1")) throw new Error(`승인 카드: ${JSON.stringify(card)}`);
  if (!mock.toolNames.includes("mcp__tracker__lookup_ticket") || !mock.toolNames.includes("mcp__tracker__list_tickets")) throw new Error(`모델에 보인 도구: ${mock.toolNames}`);
  if (mcpLog().some((l) => l.call)) throw new Error("승인 전에 도구가 실행됨");
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .approval:not(.resolved) .actions button")].find((b) => b.textContent.includes("실행")).click();
    return true;
  });
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("티켓 내용: T-1: 로그인 쿠키 만료가 너무 짧음"), 20000, "도구 결과로 답");
  const calls = mcpLog().filter((l) => l.call);
  if (calls.length !== 1 || calls[0].call !== "lookup_ticket" || calls[0].args?.id !== "T-1") throw new Error(`MCP 호출: ${JSON.stringify(calls)}`);
  // 설정 화면: 서버 상태와 도구, 도구를 눌러 '승인 없이'로
  await run(() => {
    document.querySelector("#ab-settings").click();
    return true;
  });
  const row = await waitFor(() => {
    const r = document.querySelector('[data-mcp="tracker"]');
    return r?.querySelector(".pill.ok") && r.innerText;
  }, 15000, "MCP 서버 상태");
  if (!row.includes("도구 2개") || !row.includes("lookup_ticket")) throw new Error(`서버 상태: ${row}`);
  await run(() => {
    [...document.querySelectorAll('[data-mcp="tracker"] .tool-chip')].find((b) => b.textContent.includes("lookup_ticket")).click();
    return true;
  });
  const cfgPath = path.join(DIRS.home, "config.toml");
  const end = Date.now() + 5000;
  while (!/auto_approve = \["lookup_ticket"\]/.test(fs.readFileSync(cfgPath, "utf8")) && Date.now() < end) await sleep(100);
  if (!/auto_approve = \["lookup_ticket"\]/.test(fs.readFileSync(cfgPath, "utf8"))) throw new Error(`auto_approve 저장 안 됨:\n${fs.readFileSync(cfgPath, "utf8")}`);
  await run(() => {
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("고친 뒤 관련 테스트로 확인: 실패 → 출력을 받아 다시 고침 → 통과", async () => {
  // 이 프로젝트만: 테스트 명령과, 승인 없이 실행할 node
  const projCfg = path.join(DIRS.proj, ".lantern", "config.toml");
  const check = path.join(HERE, "check-test.mjs").replace(/\\/g, "/");
  fs.writeFileSync(projCfg, `[agent]\nallowed_commands = ["node"]\ntest_command = "node ${check} {files}"\n`);
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, "검증 시나리오: 서명 접미사를 바꿔줘", "code", false);
  // 틀린 수정 → (테스트 실패) → 되돌리는 수정: 승인 카드가 하나씩 차례로 온다
  for (let i = 0; i < 2; i++) {
    await waitFor(() => !!document.querySelector(".task-log:not(.hidden) .approval:not(.resolved)"), 30000, `수정 승인 카드 ${i + 1}`);
    await run(() => {
      [...document.querySelectorAll(".task-log:not(.hidden) .approval:not(.resolved) .actions button")].find((b) => b.textContent.includes("적용")).click();
      return true;
    });
    await waitFor((n) => document.querySelectorAll(".task-log:not(.hidden) .approval.resolved").length === n, 10000, `승인 반영 ${i + 1}`, i + 1);
  }
  const cards = await waitFor(() => {
    const c = [...document.querySelectorAll(".task-log:not(.hidden) details.tool")].filter((d) => d.querySelector(".tname")?.textContent.includes("관련 테스트로 확인"));
    return c.length >= 2 && !c.some((d) => d.querySelector(".tstate.run")) && c.map((d) => ({ ok: !!d.querySelector(".tstate.ok"), text: d.querySelector("pre")?.textContent ?? "" }));
  }, 30000, "테스트 확인 두 번");
  if (cards.length !== 2 || cards[0].ok || !cards[1].ok) throw new Error(`테스트 카드: ${JSON.stringify(cards)}`);
  if (!cards[1].text.includes("PASS") || !cards[0].text.includes("tests/session.test.ts")) throw new Error(`테스트 출력: ${JSON.stringify(cards)}`);
  if (!mock.verifyFailure.includes("FAIL testSign")) throw new Error("실패 출력이 모델에 가지 않음");
  if (!file("src/auth/session.ts").includes("v + '.sig'")) throw new Error("다시 고친 결과가 파일에 없음");
  fs.rmSync(projCfg);
  git("checkout", "--", "src/auth/session.ts");
});

scenario("편집기 안 즉시 수정 (Ctrl+K): diff·영향 반경 → 적용 → 되돌리기", async () => {
  await run(() => {
    document.querySelector('.ab-item[data-view="explorer"]').click();
    document.querySelector("#canvas-switch [data-canvas='editor']").click();
    return true;
  });
  // session.ts를 열고 signCookie 본문의 v + '.sig'를 선택
  await run(async () => {
    for (const p of ["src", "src/auth"]) {
      const n = [...document.querySelectorAll("#tree .node[data-path]")].find((e) => e.dataset.path === p);
      if (n && n.getAttribute("aria-expanded") !== "true") n.click();
      await new Promise((r) => setTimeout(r, 400));
    }
    const f = [...document.querySelectorAll("#tree .node[data-path]")].find((e) => e.dataset.path === "src/auth/session.ts");
    f.click();
    f.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    return true;
  });
  await waitFor(() => [...document.querySelectorAll(".cm-editor")].some((e) => e.offsetParent && e.textContent.includes("signCookie")), 10000, "편집기");
  await run(() => {
    const ed = [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent && e.textContent.includes("signCookie"));
    const view = ed.querySelector(".cm-content").cmTile.root.view;
    const text = view.state.doc.toString();
    const from = text.indexOf("v + '.sig'");
    view.dispatch({ selection: { anchor: from, head: from + "v + '.sig'".length } });
    view.focus();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "k", code: "KeyK", ctrlKey: true, bubbles: true }));
    return true;
  });
  await waitFor(() => !!document.querySelector(".inline-edit .ie-input"), 5000, "Ctrl+K 패널");
  await run(() => {
    const i = document.querySelector(".inline-edit .ie-input");
    i.value = "접미사를 .inline으로";
    i.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    return true;
  });
  const panel = await waitFor(() => {
    const p = document.querySelector(".inline-edit");
    const impact = p?.querySelector(".impact:not(.loading)");
    return impact && { diff: p.querySelector(".diff")?.innerText ?? "", impact: impact.innerText };
  }, 20000, "diff와 영향 반경");
  if (!panel.diff.includes(".inline") || !panel.impact.includes("signCookie")) throw new Error(`Ctrl+K 결과: ${JSON.stringify(panel)}`);
  if (!mock.inlinePrompt.includes("<selection>v + '.sig'</selection>")) throw new Error("선택 영역이 프롬프트에 없음");
  await run(() => {
    [...document.querySelectorAll(".inline-edit button")].find((b) => b.textContent.includes("적용")).click();
    return true;
  });
  const doc = () => run(() => [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent && e.textContent.includes("signCookie")).querySelector(".cm-content").cmTile.root.view.state.doc.toString());
  if (!(await doc()).includes("v + '.inline'")) throw new Error("적용 후 편집기에 반영되지 않음");
  if (file("src/auth/session.ts").includes(".inline")) throw new Error("저장 전에 파일이 바뀜");
  // Ctrl+Z
  await run(() => {
    const view = [...document.querySelectorAll(".cm-editor")].find((e) => e.offsetParent && e.textContent.includes("signCookie")).querySelector(".cm-content").cmTile.root.view;
    view.focus();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "z", code: "KeyZ", ctrlKey: true, bubbles: true }));
    return true;
  });
  await sleep(300);
  if ((await doc()).includes(".inline")) throw new Error("Ctrl+Z로 되돌아가지 않음");
  // 되돌린 내용(원래와 같음)을 저장해 탭의 '저장 안 됨' 표시를 지운다 (다음 시나리오가 창을 닫는다)
  await run(() => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", code: "KeyS", ctrlKey: true, bubbles: true }));
    return true;
  });
  await sleep(500);
  if (file("src/auth/session.ts").includes(".inline")) throw new Error("되돌린 뒤 저장했는데 파일에 남음");
  await run(() => {
    document.querySelector('.ab-item[data-view="tasks"]').click();
    return true;
  });
});

scenario("다시 켜면 작업이 복원된다", async () => {
  await run(() => {
    setTimeout(() => document.querySelector("#win-close").click(), 50);
    return true;
  });
  const end = Date.now() + 10000;
  while (app.exitCode === null && Date.now() < end) await sleep(200);
  if (app.exitCode === null) throw new Error("닫기(X)로 창이 닫히지 않음");
  await launch();
  const titles = await waitFor(() => {
    const t = [...document.querySelectorAll("#task-list .task .task-title")].map((x) => x.textContent);
    return t.length >= 2 && t;
  }, 10000, "복원된 작업");
  if (!titles.includes("서명 접미사를 바꿔줘") || !titles.includes("격리해서 바꿔줘")) throw new Error(`작업 목록: ${titles.join(", ")}`);
});

scenario("외부 에이전트: 다시 켠 뒤에도 같은 대화를 이어간다 (session/load)", async () => {
  await run(() => {
    [...document.querySelectorAll("#task-list .task")].find((t) => t.textContent.includes("/signin으로 바꿔줘")).click();
    return true;
  });
  await run(ui.sendTask, "이어서 확인해줘", "acp:e2e", false);
  await waitFor(() => document.querySelector(".task-log:not(.hidden)")?.innerText.includes("앞의 대화를 기억합니다"), 30000, "이어진 대화의 답");
  const log = acpLog();
  const loads = log.filter((l) => "load" in l);
  if (loads.length !== 1 || loads[0].load !== "s1") throw new Error(`session/load: ${JSON.stringify(loads)}`);
  if (log.filter((l) => "mcp" in l).length !== 2) throw new Error("다시 켠 뒤 새 세션을 만듦"); // 로그인 전·후 두 번뿐
  if (!log.some((l) => l.followUp === "s1")) throw new Error("이어간 세션에 보내지 않음");
  const text = await run(() => document.querySelector(".task-log:not(.hidden)").innerText);
  if (text.includes("REPLAYED") || text.includes("새 대화로 시작")) throw new Error("다시 보낸 지난 대화가 화면에 쌓이거나 이어가지 못했다고 나옴");
});

scenario("영어 화면: 보이는 한국어가 없다", async () => {
  await run(() => {
    localStorage.setItem("lang", JSON.stringify("en"));
    setTimeout(() => location.reload(), 50);
    return true;
  });
  await sleep(1500);
  await waitFor(() => document.documentElement.lang === "en" && !!document.querySelector("#map-view:not(.hidden)"), 20000, "영어로 다시 불러옴");
  const left = await run(() => {
    const hangul = /[가-힣]/;
    const out = [];
    const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      const el = n.parentElement;
      if (!hangul.test(n.data) || !el || el.closest(".cm-editor, .xterm, .md, .user-text, .diff, pre, code, #tree, [data-no-i18n], .task-log, .task-title, .th-title, .fp-title, .mem-text, .mem-topic-head, .mc-title, .mc-note")) continue;
      if (!el.offsetParent) continue;
      out.push(n.data.trim().slice(0, 40));
    }
    return out;
  });
  await run(() => {
    localStorage.setItem("lang", JSON.stringify("ko"));
    return true;
  });
  if (left.length) throw new Error(`번역 안 된 글자: ${left.slice(0, 5).join(" | ")}`);
});

scenario("여러 저장소를 담은 폴더: 저장소를 모두 찾고 골라서 본다", async () => {
  // 저장소 두 개를 담은 작업 폴더 (폴더 자체는 저장소가 아님)
  const ws = path.join(TMP, "ws");
  for (const [name, file] of [["api", "server.ts"], ["web", "app.ts"]]) {
    const dir = path.join(ws, name);
    fs.mkdirSync(dir, { recursive: true });
    const g = (...a) => execFileSync("git", ["-C", dir, ...a], { encoding: "utf8" });
    g("init", "-q", "-b", "main");
    g("config", "user.email", "e2e@example.com");
    g("config", "user.name", "e2e");
    fs.writeFileSync(path.join(dir, file), `export const ${name} = 1;\n`);
    g("add", "-A");
    g("commit", "-qm", `${name} 시작`);
  }
  fs.writeFileSync(path.join(ws, "web", "app.ts"), "export const web = 2;\n"); // web에만 변경 1개
  const trusted = JSON.parse(fs.readFileSync(path.join(DIRS.data, "trusted.json"), "utf8"));
  fs.writeFileSync(path.join(DIRS.data, "trusted.json"), JSON.stringify([...trusted, ws.replace(/\\/g, "/").toLowerCase()]));

  await stop();
  await launch(ws);
  await openScm();
  const names = await waitFor(() => {
    const r = [...document.querySelectorAll("#scm-repos .scm-repo .rname")].map((x) => x.textContent);
    return r.length === 2 && r;
  }, 15000, "저장소 목록");
  if (names.join(",") !== "api,web") throw new Error(`저장소 목록: ${names.join(", ")}`);
  if (await run(() => !!document.querySelector("#scm-body .btn-primary"))) throw new Error("하위에 저장소가 있는데 '저장소 만들기'를 권함");
  // 변경이 있는 저장소(web)를 먼저 고른다
  await waitFor(() => document.querySelector("#scm-repos .scm-repo.active .rname")?.textContent === "web" && document.querySelectorAll("#scm-body .scm-file").length === 1, 10000, "web 저장소의 변경");
  if (!(await run(() => document.querySelector("#sb-branch")?.textContent.includes("web: main")))) throw new Error("상태 표시줄에 저장소·브랜치가 없음");
  await run(() => {
    [...document.querySelectorAll("#scm-repos .scm-repo")].find((r) => r.textContent.includes("api")).click();
    return true;
  });
  await waitFor(() => document.querySelector("#scm-repos .scm-repo.active .rname")?.textContent === "api" && document.querySelectorAll("#scm-body .scm-file").length === 0, 10000, "api 저장소로 전환");
});

// ── 실행 ──────────────────────────────────────────────────
makeProject();
const results = [];
let launched = false;
try {
  await launch();
  launched = true;
} catch (e) {
  console.error(`앱을 띄우지 못했습니다: ${e.message}`);
}
for (const s of scenarios) {
  if (!launched) break;
  if (ONLY && !ONLY.split("|").some((k) => s.name.includes(k))) continue;
  const started = Date.now();
  try {
    await s.fn();
    results.push({ name: s.name, ok: true, ms: Date.now() - started });
    console.log(`  ✓ ${s.name} (${((Date.now() - started) / 1000).toFixed(1)}초)`);
  } catch (e) {
    results.push({ name: s.name, ok: false, error: e.message });
    console.log(`  ✗ ${s.name}\n      ${e.message}`);
    await screenshot(s.name.replace(/[^\p{L}\p{N}]+/gu, "_"));
  }
}
if (HOLD) {
  console.log(`앱을 띄워 둠: pid ${app.pid}, CDP ${cdpPort}. 끝내려면 Ctrl+C`);
  await new Promise(() => {});
}
await stop();
mockServer.close();
if (!KEEP) {
  try {
    execFileSync("git", ["-C", DIRS.proj, "worktree", "prune"]);
  } catch { /* 없으면 넘어감 */ }
  try {
    fs.rmSync(TMP, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
  } catch (e) {
    // 꺼진 뒤에도 WebView2가 잠깐 파일을 쥐고 있을 때가 있다. 결과와는 무관하다
    console.warn(`임시 폴더를 지우지 못함: ${e.message}`);
  }
} else console.log(`임시 폴더: ${TMP}`);

const failed = results.filter((r) => !r.ok).length;
console.log(`\nE2E ${results.length - failed}/${results.length} 통과${failed ? ` · 실패 ${failed} (화면: e2e/results/)` : ""}`);
process.exit(launched && !failed ? 0 : 1);
