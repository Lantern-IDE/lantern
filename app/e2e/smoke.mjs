// 실제 모델로 하는 짧은 확인 (릴리스 전에 사람이 돌린다). E2E와 같은 격리 폴더·시나리오 프로젝트를 쓰고,
// 모델만 진짜다. 키는 환경변수로만 받아 사용자 설정·자격 증명 저장소에 쓰지 않는다.
// 격리 설정에 비용 한도를 걸어(기본 $0.5) 넘으면 앱이 요청을 막는다.
//
// 사용:  cd app && npx tauri build --no-bundle
//        ANTHROPIC_API_KEY=... node e2e/smoke.mjs --yes                     (기본: claude-haiku-4-5)
//        OPENAI_API_KEY=... node e2e/smoke.mjs --provider openai --model gpt-5-mini --yes
//        node e2e/smoke.mjs --limit 1.0 --only 계획 --yes
// --yes가 없으면 무엇을 할지만 보이고 끝낸다 (돌리면 계정 사용량이 든다).

import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const argv = process.argv.slice(2);
const opt = (k, d) => (argv.includes(`--${k}`) ? argv[argv.indexOf(`--${k}`) + 1] : d);
const PROVIDER = opt("provider", "anthropic");
const PRESETS = {
  anthropic: { model: "claude-haiku-4-5-20251001", env: "ANTHROPIC_API_KEY", price: [1.0, 5.0] },
  openai: { model: "gpt-5-mini", env: "OPENAI_API_KEY", price: [0.25, 2.0] },
};
const preset = PRESETS[PROVIDER];
if (!preset) {
  console.error(`--provider는 ${Object.keys(PRESETS).join(" 또는 ")}`);
  process.exit(2);
}
const MODEL = opt("model", preset.model);
const LIMIT = Number(opt("limit", "0.5"));
const PRICE_IN = Number(opt("price-in", preset.price[0]));
const PRICE_OUT = Number(opt("price-out", preset.price[1]));

console.log(`실제 모델 확인: ${PROVIDER} · ${MODEL} · 비용 한도 $${LIMIT} (100만 토큰당 입력 $${PRICE_IN}·출력 $${PRICE_OUT}로 셈)`);
console.log("시나리오: 질문 · 계획 먼저→진행→관련 테스트 확인 · MCP 도구 · 웹 문서");
if (!process.env[preset.env]) {
  console.error(`환경변수 ${preset.env}가 없습니다. 키는 환경변수로만 받습니다.`);
  process.exit(2);
}
if (!argv.includes("--yes")) {
  console.log("돌리면 계정 사용량이 듭니다. 진행하려면 --yes를 붙이세요.");
  process.exit(0);
}

// 확인을 마친 뒤에 불러온다 (불러오면 임시 폴더를 만든다)
const { DIRS, file, HERE, run, runAll, scenario, sleep, TMP, ui } = await import("./harness.mjs");

// ── 가짜 문서 서버 (웹 문서 시나리오) ──────────────────────────
const docs = http.createServer((req, res) => {
  res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  res.end("<html><body><h1>Cookie signing</h1><p>Sign session cookies with HMAC-SHA256 and rotate the key every 90 days.</p></body></html>");
});
await new Promise((r) => docs.listen(0, "127.0.0.1", r));
const DOCS_URL = `http://127.0.0.1:${docs.address().port}/docs/cookie`;

// ── 격리 설정: 진짜 모델 + 비용 한도 + 가짜 MCP 서버 ─────────────
fs.writeFileSync(path.join(DIRS.home, "config.toml"), `[models.smoke]
provider = "${PROVIDER}"
model = "${MODEL}"
api_key_env = "${preset.env}"
max_tokens = 4000
price_input = ${PRICE_IN}
price_output = ${PRICE_OUT}

[routing]
default = "smoke"

[budget]
monthly_usd_limit = ${LIMIT}

[mcp.tracker]
command = "node"
args = [${JSON.stringify(path.join(HERE, "mcp-server.mjs"))}]
`);

/** 지금 작업이 끝날 때까지 (승인 카드는 모두 승인). 끝난 상태와 화면 글을 돌려준다 */
async function finish(timeout = 180000, maxApprovals = 10) {
  const end = Date.now() + timeout;
  let approved = 0;
  while (Date.now() < end) {
    const s = await run(() => {
      const log = document.querySelector(".task-log:not(.hidden)");
      const card = log?.querySelector(".approval:not(.resolved)");
      const row = document.querySelector("#task-list .task.active");
      const status = [...(row?.classList ?? [])].find((c) => c.startsWith("s-"))?.slice(2) ?? "";
      const plan = !!log?.querySelector(".plan-actions");
      return { card: card?.dataset.kind ?? null, status, plan, text: log?.innerText ?? "", err: !!log?.querySelector(".err-box") };
    });
    if (s.card && approved < maxApprovals) {
      await run(() => {
        document.querySelector(".task-log:not(.hidden) .approval:not(.resolved) .actions .btn-primary")?.click();
        return true;
      });
      approved++;
      await sleep(500);
      continue;
    }
    if (s.plan || s.err || ["done", "error", "stopped"].includes(s.status)) return { ...s, approved };
    await sleep(1000);
  }
  throw new Error("작업이 제한 시간 안에 끝나지 않음");
}

async function newTask(text, agent) {
  await run(() => {
    document.querySelector("#btn-new-task").click();
    return true;
  });
  await run(ui.sendTask, text, agent, false);
}

const toolNames = () => run(() => [...document.querySelectorAll(".task-log:not(.hidden) details.tool .tname")].map((t) => t.textContent));

scenario("질문: 맥락을 받아 코드 위치로 답한다", async () => {
  await newTask("signCookie 함수는 어디서 호출돼? 파일 경로로 답해줘", "ask");
  const r = await finish();
  if (r.err || r.status === "error") throw new Error(`오류로 끝남: ${r.text.slice(-300)}`);
  if (!/issueSession|session\.ts/.test(r.text)) throw new Error(`답에 호출 위치가 없음: ${r.text.slice(-400)}`);
});

scenario("계획 먼저 → 진행 → 고친 뒤 관련 테스트 확인", async () => {
  // 관련 테스트: 서명 접미사가 '.signed'여야 통과
  fs.writeFileSync(path.join(DIRS.proj, ".lantern", "smoke-check.mjs"), `import fs from "node:fs";\nconst s = fs.readFileSync("src/auth/session.ts", "utf8");\nif (!s.includes("'.signed'")) { console.log("FAIL: signCookie must append '.signed'"); process.exit(1); }\nconsole.log("PASS");\n`);
  fs.writeFileSync(path.join(DIRS.proj, ".lantern", "config.toml"), `[agent]\ntest_command = "node .lantern/smoke-check.mjs {files}"\n`);
  await run(() => {
    const box = document.querySelector("#plan-first");
    if (!box.checked) box.click();
    return true;
  });
  await newTask("signCookie가 '.sig' 대신 '.signed'를 붙이도록 바꿔줘", "code");
  const plan = await finish();
  if (!plan.plan) throw new Error(`계획 단계에서 끝나지 않음 (상태 ${plan.status}): ${plan.text.slice(-300)}`);
  await run(() => {
    [...document.querySelectorAll(".task-log:not(.hidden) .plan-actions button")].find((b) => b.classList.contains("btn-primary")).click();
    return true;
  });
  const r = await finish();
  await run(() => {
    const box = document.querySelector("#plan-first");
    if (box.checked) box.click();
    return true;
  });
  if (r.err || r.status === "error") throw new Error(`오류로 끝남: ${r.text.slice(-300)}`);
  if (!file("src/auth/session.ts").includes("'.signed'")) throw new Error("파일이 바뀌지 않음");
  const tools = await toolNames();
  if (!tools.some((t) => t.includes("관련 테스트로 확인"))) throw new Error(`관련 테스트 확인이 돌지 않음: ${tools.join(", ")}`);
});

scenario("MCP 도구: 승인하고 결과로 답한다", async () => {
  await newTask("티켓 T-1 내용을 확인해서 한 줄로 요약해줘", "code");
  const r = await finish();
  const tools = await toolNames();
  if (!tools.some((t) => t.includes("tracker"))) throw new Error(`MCP 도구를 쓰지 않음: ${tools.join(", ")}`);
  if (!/쿠키|7일|cookie/i.test(r.text)) throw new Error(`답에 티켓 내용이 없음: ${r.text.slice(-300)}`);
});

scenario("웹 문서: 승인하고 읽어서 답한다", async () => {
  await newTask(`${DOCS_URL} 문서를 읽고 쿠키 서명 방법을 한 줄로 알려줘`, "ask");
  const r = await finish();
  if (!/HMAC/i.test(r.text)) throw new Error(`답에 문서 내용이 없음: ${r.text.slice(-300)}`);
});

const startedAt = Date.now();
await runAll(() => {
  docs.close();
  const cost = (() => {
    try {
      const f = fs.readdirSync(DIRS.data).find((x) => x.startsWith("usage"));
      return f ? fs.readFileSync(path.join(DIRS.data, f), "utf8").slice(0, 300) : "(사용량 기록 없음)";
    } catch {
      return "(사용량 기록 없음)";
    }
  })();
  console.log(`\n걸린 시간 ${((Date.now() - startedAt) / 1000).toFixed(0)}초 · 사용량 기록: ${cost}`);
  console.log(`(임시 폴더: ${TMP})`);
});
