// 맥락 적중률 평가 (모델 호출 없음, 무료, 결정적).
// 질문마다 "답하려면 꼭 봐야 하는 파일(정답 파일)"을 정해 두고, 같은 토큰 예산에서
//   A: 키워드 검색 (질문의 단어로 grep → 많이 맞은 파일부터 통째로 읽기) — AI 에이전트가 맥락 엔진 없이 흔히 하는 방식
//   B: Lantern 맥락 엔진 (lantern context)
// 이 정답 파일을 얼마나 가져오는지 잰다. 모델이 좋은 답을 하려면 먼저 이 파일을 봐야 한다.
//
// 사용: node eval/retrieval.mjs --project <프로젝트> --golden <정답 파일 JSON> [--budget 8000] [--lantern target/release/lantern.exe]
// 결과: eval/results/retrieval-<시각>.md / .json

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
};
const PROJECT = path.resolve(opt("project", ""));
const GOLDEN = opt("golden", "");
const BUDGET = Number(opt("budget", "8000"));
const LANTERN_BIN = path.resolve(opt("lantern", path.join(HERE, "..", "target", "release", process.platform === "win32" ? "lantern.exe" : "lantern")));
const OUT = path.resolve(opt("out", path.join(HERE, "results")));

if (!fs.existsSync(PROJECT) || !GOLDEN) {
  console.error("--project <평가할 프로젝트 폴더>와 --golden <정답 파일 JSON>이 필요합니다. 형식은 eval/README.md");
  process.exit(2);
}
const golden = JSON.parse(fs.readFileSync(path.resolve(GOLDEN), "utf8"));
const questions = golden.questions;

// ── 프로젝트 파일 (맥락 엔진과 같은 기준: 코드 파일, 무시 폴더 제외) ─────────
const CODE = /\.(ts|tsx|js|jsx|mjs|cjs|py|rs)$/;
const IGNORE = new Set([".git", "node_modules", "target", "dist", "build", ".next", ".lantern", "coverage", ".turbo", ".venv", "__pycache__"]);
function* walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (IGNORE.has(e.name)) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (e.isFile() && CODE.test(e.name)) yield p;
  }
}
const rel = (p) => path.relative(PROJECT, p).split(path.sep).join("/");
// 맥락 엔진과 같은 기준으로 빌드 결과물(한 줄이 매우 긴 압축 파일)은 뺀다 (indexer.rs is_unindexable)
const minified = (t) => t.length > 20_000 && t.length / t.split("\n").length > 500;
const files = [...walk(PROJECT)]
  .filter((p) => fs.statSync(p).size < 400_000)
  .map((p) => ({ path: rel(p), text: fs.readFileSync(p, "utf8") }))
  .filter((f) => !minified(f.text));
// 토큰은 맥락 엔진과 같은 거친 추정 (글자 4개 ≈ 1토큰)
const tokens = (s) => Math.ceil(s.length / 4);

// ── A: 키워드 검색 ────────────────────────────────────────
const STOP = new Set(["the", "and", "for", "how", "what", "where", "does", "어디서", "어떻게", "무엇을", "하나", "해", "곳", "로직", "하려면", "어디를", "고쳐야", "때", "시"]);
function keywords(q) {
  const latin = (q.match(/[A-Za-z][A-Za-z0-9_]{2,}/g) ?? []).map((w) => w.toLowerCase());
  const korean = (q.match(/[가-힣]{2,}/g) ?? []).map((w) => w.replace(/(은|는|이|가|을|를|의|에서|에|으로|로|와|과|하는|한|할|된|되는)$/, "")).filter((w) => w.length >= 2);
  return [...new Set([...latin, ...korean])].filter((w) => !STOP.has(w));
}

function grepRetrieve(q) {
  const terms = keywords(q);
  const scored = files
    .map((f) => {
      const lower = f.text.toLowerCase();
      const pathLower = f.path.toLowerCase();
      let distinct = 0;
      let hits = 0;
      for (const t of terms) {
        const n = lower.split(t).length - 1 + (pathLower.includes(t) ? 3 : 0);
        if (n > 0) {
          distinct++;
          hits += n;
        }
      }
      return { f, distinct, hits };
    })
    .filter((x) => x.distinct > 0)
    .sort((a, b) => b.distinct - a.distinct || b.hits - a.hits || a.f.path.localeCompare(b.f.path));
  // 예산 안에서 맞은 파일을 통째로 (예산보다 큰 파일은 앞부분만)
  const picked = [];
  let used = 0;
  for (const x of scored) {
    if (used >= BUDGET) break;
    const t = Math.min(tokens(x.f.text), BUDGET - used);
    picked.push(x.f.path);
    used += t;
  }
  return { files: picked, used, terms };
}

// ── B: Lantern ─────────────────────────────────────────────
function lanternRetrieve(q) {
  const env = { ...process.env, LANTERN_INDEX_DIR: process.env.LANTERN_INDEX_DIR || path.join(OUT, "index") };
  const out = execFileSync(LANTERN_BIN, ["-C", PROJECT, "context", q, "--budget", String(BUDGET), "--json"], { encoding: "utf8", env, maxBuffer: 64 * 1024 * 1024 });
  const r = JSON.parse(out);
  const picked = [];
  for (const it of r.items) if (!picked.includes(it.path)) picked.push(it.path);
  return { files: picked, used: r.used_tokens, ms: r.elapsed_ms };
}

// ── 채점 ─────────────────────────────────────────────────
function score(picked, q) {
  const set = new Set(picked);
  const hit = q.golden.filter((g) => set.has(g));
  // 같은 역할의 다른 구현(also_ok)을 가져왔으면 부분 점수 대신 따로 센다
  const alt = (q.also_ok ?? []).filter((g) => set.has(g));
  const firstRank = picked.findIndex((p) => q.golden.includes(p));
  return { recall: hit.length / q.golden.length, all: hit.length === q.golden.length, any: hit.length > 0, hit, alt, rank: firstRank < 0 ? null : firstRank + 1 };
}

// 인덱스를 먼저 만든다 (첫 질문 시간에 인덱싱이 섞이지 않게)
console.log(`프로젝트 파일 ${files.length}개, 질문 ${questions.length}개, 예산 ${BUDGET} 토큰`);
lanternRetrieve("warmup");

const rows = questions.map((q, i) => {
  const a = grepRetrieve(q.q);
  const b = lanternRetrieve(q.q);
  const sa = score(a.files, q);
  const sb = score(b.files, q);
  console.log(`${String(i + 1).padStart(2)}. 키워드 ${Math.round(sa.recall * 100)}% · Lantern ${Math.round(sb.recall * 100)}% — ${q.q}`);
  return { q: q.q, golden: q.golden, grep: { ...a, ...sa }, lantern: { ...b, ...sb } };
});

const mean = (k, f) => rows.reduce((n, r) => n + f(r[k]), 0) / rows.length;
const pct = (x) => `${Math.round(x * 100)}%`;
const summary = {
  grep: { recall: mean("grep", (x) => x.recall), all: mean("grep", (x) => (x.all ? 1 : 0)), any: mean("grep", (x) => (x.any ? 1 : 0)), tokens: mean("grep", (x) => x.used), files: mean("grep", (x) => x.files.length) },
  lantern: { recall: mean("lantern", (x) => x.recall), all: mean("lantern", (x) => (x.all ? 1 : 0)), any: mean("lantern", (x) => (x.any ? 1 : 0)), tokens: mean("lantern", (x) => x.used), files: mean("lantern", (x) => x.files.length) },
};
const wins = rows.filter((r) => r.lantern.recall > r.grep.recall).length;
const losses = rows.filter((r) => r.lantern.recall < r.grep.recall).length;

fs.mkdirSync(OUT, { recursive: true });
const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-");
fs.writeFileSync(path.join(OUT, `retrieval-${stamp}.json`), JSON.stringify({ project: path.basename(PROJECT), budget: BUDGET, summary, rows }, null, 2));
const md = [
  `# 맥락 적중률 (${stamp})`,
  "",
  `프로젝트 \`${path.basename(PROJECT)}\` · 코드 파일 ${files.length}개 · 질문 ${rows.length}개 · 예산 ${BUDGET} 토큰 · 모델 호출 없음`,
  "",
  "정답 파일: 질문에 답하려면 꼭 봐야 하는 파일 (`eval/golden-*.json`). 같은 예산에서 각 방식이 정답 파일을 몇 개 가져오는지 잰다.",
  "",
  "| 지표 | 키워드 검색 | Lantern |",
  "|---|---|---|",
  `| 정답 파일 적중률 (평균) | ${pct(summary.grep.recall)} | ${pct(summary.lantern.recall)} |`,
  `| 정답 파일을 모두 가져온 질문 | ${pct(summary.grep.all)} | ${pct(summary.lantern.all)} |`,
  `| 정답 파일을 하나라도 가져온 질문 | ${pct(summary.grep.any)} | ${pct(summary.lantern.any)} |`,
  `| 평균 사용 토큰 | ${Math.round(summary.grep.tokens)} | ${Math.round(summary.lantern.tokens)} |`,
  `| 평균 가져온 파일 수 | ${summary.grep.files.toFixed(1)} | ${summary.lantern.files.toFixed(1)} |`,
  `| Lantern 승 / 무 / 패 | | ${wins} / ${rows.length - wins - losses} / ${losses} |`,
  "",
  "## 질문별",
  "",
  "| # | 질문 | 정답 파일 | 키워드 | Lantern | Lantern 첫 정답 순위 |",
  "|---|---|---|---|---|---|",
  ...rows.map((r, i) => `| ${i + 1} | ${r.q} | ${r.golden.map((g) => `\`${g.split("/").pop()}\``).join(", ")} | ${pct(r.grep.recall)} | ${pct(r.lantern.recall)} | ${r.lantern.rank ?? "-"} |`),
  "",
  "## 놓친 정답 파일",
  "",
  ...rows.flatMap((r, i) => {
    const miss = r.golden.filter((g) => !r.lantern.hit.includes(g));
    return miss.length ? [`- ${i + 1}. ${r.q}: ${miss.map((m) => `\`${m}\``).join(", ")}${r.lantern.alt.length ? ` (다른 구현은 가져옴: ${r.lantern.alt.map((m) => `\`${m}\``).join(", ")})` : ""}`] : [];
  }),
].join("\n");
const mdPath = path.join(OUT, `retrieval-${stamp}.md`);
fs.writeFileSync(mdPath, md);
console.log(`\n키워드 ${pct(summary.grep.recall)} vs Lantern ${pct(summary.lantern.recall)} (정답 파일 적중률)\n보고서: ${mdPath}`);
