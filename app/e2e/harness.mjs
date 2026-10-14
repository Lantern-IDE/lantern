// E2E 공통: 격리된 폴더, 시나리오용 git 프로젝트, 앱 띄우기와 CDP 조작, 시나리오 실행.
// 사용자 설정·데이터를 건드리지 않게 모든 폴더를 임시 폴더에 만든다.

import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const HERE = path.dirname(fileURLToPath(import.meta.url));
export const EXE = path.resolve(HERE, "..", "..", "target", "release", "lantern-app.exe");
export const RESULTS = path.join(HERE, "results");
const argv = process.argv.slice(2);
export const ONLY = argv.includes("--only") ? argv[argv.indexOf("--only") + 1] : null;
export const KEEP = argv.includes("--keep");
export const HOLD = argv.includes("--hold"); // 진단용: 끝나도 앱을 띄워 둔다

if (process.platform !== "win32") {
  console.log("E2E는 지금 Windows(WebView2)에서만 돈다. 다른 OS는 단위 테스트와 빌드만 확인한다.");
  process.exit(0);
}
if (!fs.existsSync(EXE)) {
  console.error(`앱이 없습니다: ${EXE}\n먼저 cd app && npx tauri build --no-bundle`);
  process.exit(2);
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── 임시 작업 폴더 ─────────────────────────────────────────
export const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "lantern-e2e-"));
export const DIRS = { data: path.join(TMP, "data"), home: path.join(TMP, "home"), wv: path.join(TMP, "wv"), index: path.join(TMP, "index"), proj: path.join(TMP, "proj") };
for (const d of Object.values(DIRS)) fs.mkdirSync(d, { recursive: true });

export function git(...args) {
  // 앱도 같은 저장소에서 git을 돌리므로(파일 감시 → 상태 새로 고침) index.lock이 겹치면 잠깐 뒤 다시
  for (let i = 0; ; i++) {
    try {
      return execFileSync("git", ["-C", DIRS.proj, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    } catch (e) {
      if (i >= 8 || !String(e.stderr ?? e.message).includes("index.lock")) throw e;
      execFileSync(process.execPath, ["-e", "setTimeout(() => {}, 250)"]);
    }
  }
}

export function makeProject() {
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

export const file = (rel) => fs.readFileSync(path.join(DIRS.proj, rel), "utf8");

// ── 앱 띄우기와 CDP ────────────────────────────────────────
export let app = null;
export let cdpPort = 0;
export let ws = null;
export let msgId = 0;
export const pending = new Map();

/** 앱을 띄운다. dir: 열 폴더 (기본은 시나리오용 프로젝트) */
export async function launch(dir = DIRS.proj) {
  cdpPort = 9400 + Math.floor(Math.random() * 500);
  app = spawn(EXE, [dir], {
    env: {
      ...process.env,
      LANTERN_HOME: DIRS.home,
      LANTERN_DATA_DIR: DIRS.data,
      LANTERN_WEBVIEW_DATA_DIR: DIRS.wv,
      LANTERN_INDEX_DIR: DIRS.index,
      LANTERN_E2E_CDP_PORT: String(cdpPort),
      LANTERN_FETCH_ALLOW_LOCAL: "1", // 가짜 문서 서버가 로컬이라
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
  // 문서의 lang은 i18n이 바꾸기 전에는 index.html의 기본값이라 믿지 않고, 앱과 같은 방식으로 정해질 언어를 본다
  const effective = await run(() => {
    let saved = null;
    try {
      saved = JSON.parse(localStorage.getItem("lang") ?? "null");
    } catch {}
    return saved ?? (navigator.language.toLowerCase().startsWith("ko") ? "ko" : "en");
  });
  if (effective !== "ko") {
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
export function appLogTail() {
  const dir = path.join(DIRS.data, "logs");
  if (!fs.existsSync(dir)) return "\n      (앱 로그 없음: 앱이 로그를 쓰기 전에 멈춤)";
  return fs.readdirSync(dir).filter((f) => f.endsWith(".log") || f.startsWith("crash-")).map((f) => {
    const lines = fs.readFileSync(path.join(dir, f), "utf8").trim().split("\n").slice(-15);
    return `\n      ── ${f}\n        ${lines.join("\n        ")}`;
  }).join("");
}

export let onPaused = null;
export const traceLog = [];
export let hangReported = false;
export const scripts = new Map();

/** 자바스크립트가 멈췄을 때: 디버거로 멈추게 해서 어디서 돌고 있는지 기록한다 */
export async function diagnoseHang() {
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
export function cdp(method, params = {}, timeout = 20000) {
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
export async function run(fn, ...args) {
  const expression = `(async () => (${fn.toString()})(...${JSON.stringify(args)}))()`;
  const r = await cdp("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? "앱 안에서 예외");
  return r.result?.result?.value;
}

export async function waitFor(fn, timeout, what, ...args) {
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

export async function screenshot(name) {
  try {
    const r = await cdp("Page.captureScreenshot", { format: "png" }, 8000);
    fs.mkdirSync(RESULTS, { recursive: true });
    fs.writeFileSync(path.join(RESULTS, `${name}.png`), Buffer.from(r.result.data, "base64"));
  } catch { /* 화면이 없으면 건너뜀 */ }
}

/** 네이티브 대화상자(되돌리기 확인 등)의 단추를 누른다. index: 단추 순서 (0부터) */
export function clickDialog(index) {
  const ps = path.join(HERE, "dialog.ps1");
  return execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps, "-ProcId", String(app.pid), "-Index", String(index)], { encoding: "utf8" });
}

export async function waitDialog(index, timeout = 8000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const out = clickDialog(index);
    if (out.includes("클릭")) return out;
    await sleep(300);
  }
  throw new Error("대화상자가 뜨지 않았습니다");
}

export async function stop() {
  try {
    ws?.close();
  } catch { /* 이미 닫힘 */ }
  if (app && app.exitCode === null) app.kill();
  await sleep(500);
}

// ── 앱 안에서 쓰는 조작 (문자열로 옮겨 실행) ───────────────────
export const ui = {
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

// ── 시나리오 등록과 실행 ───────────────────────────────────
const scenarios = [];
export const scenario = (name, fn) => scenarios.push({ name, fn });

export const openScm = () => run(() => {
  if (!document.querySelector("#view-scm")?.offsetParent) document.querySelector('.ab-item[data-view="scm"]').click();
  return true;
});

/** 메뉴 팝업에서 글자가 든 항목을 누른다 */
export const pickMenu = (text) => run((t) => {
  const it = [...document.querySelectorAll("#menu-popup .menu-item")].find((x) => x.textContent.includes(t));
  it?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  return !!it;
}, text);

/** 등록한 시나리오를 차례로 돌리고 결과를 알린 뒤 끝낸다. `cleanup`: 끝날 때 할 일 (모의 서버 닫기 등) */
export async function runAll(cleanup = () => {}) {
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
  cleanup();
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
}
