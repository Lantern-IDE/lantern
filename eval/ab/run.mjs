// Lantern 맥락 엔진 A/B 평가.
//   A: 같은 모델 + 기본 도구(파일 목록, grep, 파일 읽기)
//   B: A + get_context 도구 (lantern CLI로 질문에 맞는 맥락을 조립)
// 질문마다 두 방식을 돌려 토큰, 도구 호출 수, 답을 기록하고, --judge를 주면 모델이 두 답을 (순서를 섞어) 비교 채점한다.
//
// 사용:
//   Claude:     $env:ANTHROPIC_API_KEY="..."; node run.mjs --project <프로젝트> --golden <정답 파일 JSON> --judge --yes
//   로컬 모델:  node run.mjs --project <프로젝트> --golden <정답 파일 JSON> --provider openai --base-url http://localhost:11434/v1 --model qwen2.5-coder:14b --yes
// 옵션: --model  --lantern ../../target/release/lantern.exe  --max-turns 12  --limit 3  --out <폴더>
//       --golden <정답 파일 JSON>: 질문도 여기서 읽고, 각 방식이 정답 파일을 실제로 봤는지 센다 (AI 채점 없이 객관적)
// Claude는 요금이 들어 --yes 없이 실행하면 예상 호출 수만 보여주고 멈춘다.

import Anthropic from "@anthropic-ai/sdk";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));

// ── 인자 ─────────────────────────────────────────────
const argv = process.argv.slice(2);
const flag = (name) => argv.includes(`--${name}`);
const opt = (name, fallback) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[i + 1] : fallback;
};

const PROJECT = path.resolve(opt("project", ""));
const QUESTIONS = opt("questions", "");
const MODEL = opt("model", "claude-opus-5");
const LANTERN_BIN = path.resolve(opt("lantern", path.join(HERE, "..", "..", "target", "release", process.platform === "win32" ? "lantern.exe" : "lantern")));
const MAX_TURNS = Number(opt("max-turns", "12"));
const LIMIT = Number(opt("limit", "0"));
const BUDGET = Number(opt("budget", "8000"));
const OUT = path.resolve(opt("out", path.join(HERE, "results")));
const JUDGE = flag("judge");
const PROVIDER = opt("provider", "anthropic");
const BASE_URL = opt("base-url", "http://localhost:11434/v1").replace(/\/$/, "");
const GOLDEN = opt("golden", "");

if (!opt("project") || !fs.existsSync(PROJECT)) {
  console.error("--project <평가할 프로젝트 폴더>가 필요합니다.");
  process.exit(2);
}
if (!fs.existsSync(LANTERN_BIN)) {
  console.error(`lantern CLI를 찾지 못했습니다: ${Lantern}\n먼저 cargo build --release -p lantern-context 를 실행하세요.`);
  process.exit(2);
}

// 정답 파일이 있으면 질문도 거기서 읽는다
const goldenSet = GOLDEN ? JSON.parse(fs.readFileSync(path.resolve(GOLDEN), "utf8")).questions : null;
let questions = goldenSet
  ? goldenSet.map((g) => g.q)
  : QUESTIONS
  ? fs.readFileSync(QUESTIONS, "utf8").split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith("#"))
  : (console.error("--golden <정답 파일 JSON> 또는 --questions <질문 파일>이 필요합니다. 형식은 eval/README.md"), process.exit(2));
if (LIMIT > 0) questions = questions.slice(0, LIMIT);
const goldenOf = (q) => goldenSet?.find((g) => g.q === q)?.golden ?? null;

const calls = questions.length * 2 * MAX_TURNS + (JUDGE ? questions.length : 0);
console.log(`질문 ${questions.length}개 · 모델 ${MODEL} · 방식 2개 · 최대 호출 약 ${calls}회${JUDGE ? " (채점 포함)" : ""}`);
if (!flag("yes")) {
  console.log(PROVIDER === "anthropic"
    ? "실제 API 요금이 듭니다. 실행하려면 --yes 를 붙이세요. 먼저 --limit 1 로 한 문제만 돌려 보길 권합니다."
    : "로컬 모델로 실행하려면 --yes 를 붙이세요.");
  process.exit(0);
}
if (PROVIDER === "anthropic" && !process.env.ANTHROPIC_API_KEY) {
  console.error("ANTHROPIC_API_KEY 환경변수가 없습니다. 로컬 모델은 --provider openai --base-url ... --model ... 로 실행하세요.");
  process.exit(2);
}

const client = PROVIDER === "anthropic" ? new Anthropic({ maxRetries: 4 }) : null;

// ── 모델 호출 (Anthropic 형식으로 통일) ─────────────────────
// 대화는 Anthropic Messages 형식으로 쌓고, OpenAI 호환(Ollama, LM Studio 등)은 보낼 때 바꾼다.
function toOpenAI(system, messages) {
  const out = [{ role: "system", content: system }];
  for (const m of messages) {
    if (typeof m.content === "string") {
      out.push({ role: m.role, content: m.content });
      continue;
    }
    if (m.role === "assistant") {
      const text = m.content.filter((b) => b.type === "text").map((b) => b.text).join("\n");
      const calls = m.content.filter((b) => b.type === "tool_use").map((b) => ({ id: b.id, type: "function", function: { name: b.name, arguments: JSON.stringify(b.input) } }));
      out.push({ role: "assistant", content: text || null, ...(calls.length ? { tool_calls: calls } : {}) });
    } else {
      for (const b of m.content) {
        if (b.type === "tool_result") out.push({ role: "tool", tool_call_id: b.tool_use_id, content: String(b.content) });
        else if (b.type === "text") out.push({ role: "user", content: b.text });
      }
    }
  }
  return out;
}

async function chat({ system, messages, tools, maxTokens }) {
  if (client) {
    return client.messages.create({
      model: MODEL,
      max_tokens: maxTokens,
      ...(tools ? { thinking: { type: "adaptive" }, cache_control: { type: "ephemeral" }, tools } : {}),
      ...(system ? { system } : {}),
      messages,
    });
  }
  const body = {
    model: MODEL,
    max_tokens: maxTokens,
    stream: false,
    messages: toOpenAI(system ?? "", messages),
    ...(tools ? { tools: tools.map((t) => ({ type: "function", function: { name: t.name, description: t.description, parameters: t.input_schema } })) } : {}),
  };
  const headers = { "content-type": "application/json", ...(process.env.OPENAI_API_KEY ? { authorization: `Bearer ${process.env.OPENAI_API_KEY}` } : {}) };
  const res = await fetch(`${BASE_URL}/chat/completions`, { method: "POST", headers, body: JSON.stringify(body) });
  if (!res.ok) throw new Error(`모델 API 오류 ${res.status}: ${(await res.text()).slice(0, 300)}`);
  const j = await res.json();
  const choice = j.choices?.[0] ?? {};
  const msg = choice.message ?? {};
  const content = [];
  if (msg.content) content.push({ type: "text", text: msg.content });
  for (const c of msg.tool_calls ?? []) {
    let input = {};
    try {
      input = JSON.parse(c.function?.arguments || "{}");
    } catch {
      input = { __invalid: c.function?.arguments };
    }
    content.push({ type: "tool_use", id: c.id ?? `call_${content.length}`, name: c.function?.name, input });
  }
  const finish = choice.finish_reason;
  return {
    content,
    stop_reason: (msg.tool_calls?.length ? "tool_use" : finish === "length" ? "max_tokens" : "end_turn"),
    usage: { input_tokens: j.usage?.prompt_tokens ?? 0, output_tokens: j.usage?.completion_tokens ?? 0 },
  };
}

// ── 도구 (읽기 전용, 프로젝트 밖으로 못 나감) ────────────────
const IGNORE = new Set([".git", "node_modules", "target", "dist", "build", ".next", ".lantern", "__pycache__", ".venv", "coverage"]);
const MAX_RESULT = 20_000;

function inside(rel) {
  const target = path.resolve(PROJECT, String(rel ?? "."));
  const r = path.relative(PROJECT, target);
  if (r === ".." || r.startsWith(`..${path.sep}`) || path.isAbsolute(r)) throw new Error("프로젝트 밖 경로입니다");
  return target;
}

function* walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (IGNORE.has(e.name)) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (e.isFile()) yield p;
  }
}

const rel = (p) => path.relative(PROJECT, p).split(path.sep).join("/");
const clip = (s) => (s.length > MAX_RESULT ? `${s.slice(0, MAX_RESULT)}\n…(${s.length - MAX_RESULT}자 생략)` : s);

const TOOL_IMPL = {
  list_files({ dir }) {
    const files = [...walk(inside(dir))].map(rel);
    return clip(files.slice(0, 2000).join("\n") + (files.length > 2000 ? `\n…외 ${files.length - 2000}개` : ""));
  },
  grep({ pattern, dir }) {
    let re;
    try {
      re = new RegExp(pattern, "i");
    } catch {
      re = new RegExp(pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "i");
    }
    const hits = [];
    for (const f of walk(inside(dir))) {
      if (fs.statSync(f).size > 1_000_000) continue;
      const lines = fs.readFileSync(f, "utf8").split(/\r?\n/);
      lines.forEach((l, i) => {
        if (hits.length < 150 && re.test(l)) hits.push(`${rel(f)}:${i + 1}: ${l.trim().slice(0, 200)}`);
      });
      if (hits.length >= 150) break;
    }
    return hits.length ? clip(hits.join("\n")) : "일치 없음";
  },
  read_file({ path: p, start_line, end_line }) {
    const lines = fs.readFileSync(inside(p), "utf8").split(/\r?\n/);
    const s = Math.max(1, Number(start_line) || 1);
    const e = Math.min(lines.length, Number(end_line) || lines.length);
    return clip(lines.slice(s - 1, e).map((l, i) => `${s + i}\t${l}`).join("\n"));
  },
  get_context({ query }) {
    // 평가 대상 프로젝트에 .lantern/를 만들지 않도록 색인은 결과 폴더에 둔다
    const env = { ...process.env, LANTERN_INDEX_DIR: process.env.LANTERN_INDEX_DIR || path.join(OUT, "index") };
    return clip(execFileSync(LANTERN_BIN, ["-C", PROJECT, "context", String(query), "--budget", String(BUDGET)], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024, env }));
  },
};

const str = { type: "string" };
const BASE_TOOLS = [
  { name: "list_files", description: "List files under a directory of the project (relative path, default project root).", input_schema: { type: "object", properties: { dir: str } } },
  { name: "grep", description: "Case-insensitive regex search over project files. Returns up to 150 'path:line: text' hits.", input_schema: { type: "object", properties: { pattern: str, dir: str }, required: ["pattern"] } },
  { name: "read_file", description: "Read a project file with line numbers. Optionally a 1-based line range.", input_schema: { type: "object", properties: { path: str, start_line: { type: "integer" }, end_line: { type: "integer" } }, required: ["path"] } },
];
const CONTEXT_TOOL = {
  name: "get_context",
  description: "Ask the project's context engine for the code most relevant to a question: matching symbol definitions, their callers/callees, and files that change together, within a token budget. Usually the best first step.",
  input_schema: { type: "object", properties: { query: str }, required: ["query"] },
};

function validate(tool, input) {
  const schema = [...BASE_TOOLS, CONTEXT_TOOL].find((t) => t.name === tool)?.input_schema;
  if (!schema || typeof input !== "object" || input === null) return false;
  return (schema.required ?? []).every((k) => typeof input[k] === "string" && input[k].length > 0);
}

const SYSTEM = [
  "You are answering a question about the codebase in the current project. Use the tools to find the relevant code before answering.",
  "Answer in Korean. Cite the specific files and line numbers or function names your answer relies on.",
  "Be concise: at most about 15 lines. Do not guess; if you could not find something, say so.",
].join("\n");

// ── 한 방식 실행 (수동 루프: 호출마다 토큰을 모은다) ───────────
async function runArm(question, withContext) {
  const tools = withContext ? [...BASE_TOOLS, CONTEXT_TOOL] : BASE_TOOLS;
  const messages = [{ role: "user", content: question }];
  const stat = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, turns: 0, toolCalls: {}, filesRead: new Set(), seen: new Set(), answer: "", stop: "" };
  const started = Date.now();
  while (stat.turns < MAX_TURNS) {
    stat.turns++;
    const res = await chat({ system: SYSTEM, messages, tools, maxTokens: 8000 });
    stat.input += res.usage.input_tokens;
    stat.output += res.usage.output_tokens;
    stat.cacheRead += res.usage.cache_read_input_tokens ?? 0;
    stat.cacheWrite += res.usage.cache_creation_input_tokens ?? 0;
    stat.stop = res.stop_reason;
    if (res.stop_reason === "pause_turn") {
      messages.push({ role: "assistant", content: res.content });
      continue;
    }
    const uses = res.content.filter((b) => b.type === "tool_use");
    if (res.stop_reason !== "tool_use" || uses.length === 0) {
      stat.answer = res.content.filter((b) => b.type === "text").map((b) => b.text).join("\n").trim();
      break;
    }
    messages.push({ role: "assistant", content: res.content });
    const results = uses.map((u) => {
      stat.toolCalls[u.name] = (stat.toolCalls[u.name] ?? 0) + 1;
      if (!validate(u.name, u.input)) {
        return { type: "tool_result", tool_use_id: u.id, is_error: true, content: `잘못된 입력: ${JSON.stringify(u.input)}` };
      }
      try {
        const content = TOOL_IMPL[u.name](u.input);
        // 모델이 실제로 본 파일: 읽은 파일 + 맥락 엔진이 보낸 파일 (정답 파일 채점용)
        if (u.name === "read_file") {
          stat.filesRead.add(String(u.input.path).replace(/^\.\//, ""));
          stat.seen.add(String(u.input.path).replace(/^\.\//, ""));
        }
        if (u.name === "get_context") for (const m of content.matchAll(/^## (.+)$/gm)) stat.seen.add(m[1].trim());
        return { type: "tool_result", tool_use_id: u.id, content };
      } catch (e) {
        return { type: "tool_result", tool_use_id: u.id, is_error: true, content: String(e.message ?? e) };
      }
    });
    messages.push({ role: "user", content: results });
  }
  if (!stat.answer) stat.answer = `(최대 ${MAX_TURNS}턴 안에 답하지 못함)`;
  const golden = goldenOf(question);
  const seen = [...stat.seen];
  const recall = golden ? golden.filter((g) => seen.includes(g)).length / golden.length : null;
  return { ...stat, filesRead: [...stat.filesRead], seen, recall, ms: Date.now() - started };
}

// ── 채점: 두 답을 무작위 순서로 보여주고 JSON으로 받는다 ─────────
async function judge(question, a, b) {
  const flip = Math.random() < 0.5;
  const [x, y] = flip ? [b, a] : [a, b];
  const prompt = [
    `질문: ${question}`,
    "",
    "아래 두 답을 코드베이스를 조사한 개발자의 답으로서 평가하세요. 두 답은 같은 코드베이스를 조사했습니다.",
    "기준: correctness(사실에 맞고 질문에 답했는가), grounding(구체적인 파일·함수·줄을 근거로 들었는가), 각 1~5점.",
    "근거 없이 추측한 내용은 감점하세요. 길이는 점수에 반영하지 마세요.",
    "",
    `<answer id="X">\n${x.answer}\n</answer>`,
    `<answer id="Y">\n${y.answer}\n</answer>`,
    "",
    '다음 JSON만 출력하세요: {"X":{"correctness":n,"grounding":n},"Y":{"correctness":n,"grounding":n},"winner":"X"|"Y"|"tie","reason":"한 문장"}',
  ].join("\n");
  const res = await chat({ messages: [{ role: "user", content: prompt }], maxTokens: 2000 });
  const text = res.content.filter((c) => c.type === "text").map((c) => c.text).join("");
  const m = text.match(/\{[\s\S]*\}/);
  if (!m) return { error: text.slice(0, 200) };
  try {
    const j = JSON.parse(m[0]);
    const map = (id) => (id === "tie" ? "tie" : (id === "X") !== flip ? "A" : "B");
    return { A: flip ? j.Y : j.X, B: flip ? j.X : j.Y, winner: map(j.winner), reason: j.reason };
  } catch {
    return { error: m[0].slice(0, 200) };
  }
}

// ── 실행과 보고서 ─────────────────────────────────────
const rows = [];
for (const [i, q] of questions.entries()) {
  console.log(`\n[${i + 1}/${questions.length}] ${q}`);
  const A = await runArm(q, false);
  console.log(`  A: 입력 ${A.input} · 출력 ${A.output} · ${A.turns}턴 · ${JSON.stringify(A.toolCalls)}${A.recall !== null ? ` · 정답 파일 ${Math.round(A.recall * 100)}%` : ""}`);
  const B = await runArm(q, true);
  console.log(`  B: 입력 ${B.input} · 출력 ${B.output} · ${B.turns}턴 · ${JSON.stringify(B.toolCalls)}${B.recall !== null ? ` · 정답 파일 ${Math.round(B.recall * 100)}%` : ""}`);
  const verdict = JUDGE ? await judge(q, A, B) : null;
  if (verdict) console.log(`  채점: ${verdict.winner ?? "오류"} — ${verdict.reason ?? verdict.error}`);
  rows.push({ question: q, A, B, verdict });
}

const sum = (arm, k) => rows.reduce((n, r) => n + r[arm][k], 0);
const totalIn = (arm) => sum(arm, "input") + sum(arm, "cacheRead") + sum(arm, "cacheWrite");
const wins = (w) => rows.filter((r) => r.verdict?.winner === w).length;
const avg = (arm, k) => {
  const v = rows.map((r) => r.verdict?.[arm]?.[k]).filter((n) => typeof n === "number");
  return v.length ? (v.reduce((a, b) => a + b, 0) / v.length).toFixed(2) : "-";
};
const pct = (a, b) => (a ? `${Math.round((1 - b / a) * 100)}%` : "-");

fs.mkdirSync(OUT, { recursive: true });
const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-");
fs.writeFileSync(path.join(OUT, `ab-${stamp}.json`), JSON.stringify({ model: MODEL, project: PROJECT, budget: BUDGET, rows }, null, 2));

const md = [
  `# A/B 평가 결과 (${stamp})`,
  "",
  `${PROVIDER === "anthropic" ? "Anthropic" : `OpenAI 호환 (${BASE_URL})`} · 모델 ${MODEL} · 프로젝트 \`${path.basename(PROJECT)}\` · 질문 ${rows.length}개 · 맥락 예산 ${BUDGET} 토큰 · 최대 ${MAX_TURNS}턴`,
  "",
  "A = 기본 도구만 (파일 목록, grep, 파일 읽기) · B = A + Lantern get_context",
  "",
  "## 요약",
  "",
  "| 항목 | A | B | B의 절감 |",
  "|---|---|---|---|",
  `| 입력 토큰 합 (캐시 포함) | ${totalIn("A")} | ${totalIn("B")} | ${pct(totalIn("A"), totalIn("B"))} |`,
  `| 출력 토큰 합 | ${sum("A", "output")} | ${sum("B", "output")} | ${pct(sum("A", "output"), sum("B", "output"))} |`,
  `| 모델 호출(턴) 합 | ${sum("A", "turns")} | ${sum("B", "turns")} | ${pct(sum("A", "turns"), sum("B", "turns"))} |`,
  ...(goldenSet
    ? [`| 정답 파일을 실제로 본 비율 (평균) | ${Math.round((rows.reduce((n, r) => n + (r.A.recall ?? 0), 0) / rows.length) * 100)}% | ${Math.round((rows.reduce((n, r) => n + (r.B.recall ?? 0), 0) / rows.length) * 100)}% | |`]
    : []),
  `| 걸린 시간 합 (초) | ${(sum("A", "ms") / 1000).toFixed(0)} | ${(sum("B", "ms") / 1000).toFixed(0)} | ${pct(sum("A", "ms"), sum("B", "ms"))} |`,
  ...(JUDGE
    ? [
        `| 정확도 평균 (1~5) | ${avg("A", "correctness")} | ${avg("B", "correctness")} | |`,
        `| 근거 제시 평균 (1~5) | ${avg("A", "grounding")} | ${avg("B", "grounding")} | |`,
        `| 승 / 무 / 패 (B 기준) | | ${wins("B")} / ${wins("tie")} / ${wins("A")} | |`,
      ]
    : []),
  "",
  "**통과 기준 (기획서):** B의 정답률이 A와 같거나 높고, 입력 토큰은 같거나 적다.",
  "채점은 같은 모델이 두 답을 순서를 섞어 비교한 것이라 참고용입니다. 최종 판단 전에 아래 답을 직접 읽어 보세요.",
  "",
  "## 질문별",
  "",
  "| # | 질문 | A 입력/턴 | B 입력/턴 | 승자 | 이유 |",
  "|---|---|---|---|---|---|",
  ...rows.map((r, i) => `| ${i + 1} | ${r.question} | ${r.A.input + r.A.cacheRead + r.A.cacheWrite} / ${r.A.turns} | ${r.B.input + r.B.cacheRead + r.B.cacheWrite} / ${r.B.turns} | ${r.verdict?.winner ?? "-"} | ${(r.verdict?.reason ?? "").replace(/\|/g, "\\|")} |`),
  "",
  "## 답 원문",
  "",
  ...rows.flatMap((r, i) => [
    `### ${i + 1}. ${r.question}`,
    "",
    `**A** (${JSON.stringify(r.A.toolCalls)})`,
    "",
    r.A.answer,
    "",
    `**B** (${JSON.stringify(r.B.toolCalls)})`,
    "",
    r.B.answer,
    "",
  ]),
].join("\n");
const mdPath = path.join(OUT, `ab-${stamp}.md`);
fs.writeFileSync(mdPath, md);
console.log(`\n보고서: ${mdPath}`);
