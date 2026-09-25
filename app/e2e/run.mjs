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
  git("init", "-q");
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
const mock = { read: "src/auth/login.ts", edit: "src/auth/session.ts", old: "v + '.sig'", new: "v + '.signed'", requests: 0 };
const mockServer = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (req.method === "GET") {
      res.writeHead(200, { "content-type": "application/json" });
      return res.end(JSON.stringify({ data: [{ id: "e2e-model" }] }));
    }
    mock.requests++;
    const j = JSON.parse(body);
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
    const tools = j.messages.filter((m) => m.role === "tool").length;
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
`);

// ── 앱 띄우기와 CDP ────────────────────────────────────────
let app = null;
let cdpPort = 0;
let ws = null;
let msgId = 0;
const pending = new Map();

async function launch() {
  cdpPort = 9400 + Math.floor(Math.random() * 500);
  app = spawn(EXE, [DIRS.proj], {
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
  // 프로젝트를 열고 지도를 그릴 때까지
  await waitFor(() => document.querySelector("#cc-label")?.textContent === "proj" && !!document.querySelector("#map-view:not(.hidden)") && !!document.querySelector("#map-crumb .cur"), 30000, "프로젝트 열림");
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
  await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("바꿨습니다")), 8000, "바꾸기 알림");
  if (!file("src/api.ts").includes("handleSignIn") || !file("src/auth/login.ts").includes("handleSignIn")) throw new Error("바꾸지 않음");
  await run(() => {
    const t = [...document.querySelectorAll(".toast")].find((x) => x.innerText.includes("바꿨습니다"));
    [...t.querySelectorAll("button")].find((b) => b.textContent.includes("되돌리기"))?.click();
    return true;
  });
  await waitFor(() => [...document.querySelectorAll(".toast")].some((t) => t.innerText.includes("되돌렸습니다")), 8000, "되돌림 알림");
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
  fs.rmSync(TMP, { recursive: true, force: true, maxRetries: 5, retryDelay: 300 });
} else console.log(`임시 폴더: ${TMP}`);

const failed = results.filter((r) => !r.ok).length;
console.log(`\nE2E ${results.length - failed}/${results.length} 통과${failed ? ` · 실패 ${failed} (화면: e2e/results/)` : ""}`);
process.exit(launched && !failed ? 0 : 1);
