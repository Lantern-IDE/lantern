// 맥락 적중률 평가 (모델 호출 없음, 무료, 결정적).
// 질문마다 "답하려면 꼭 봐야 하는 파일(정답 파일)"을 정해 두고, 같은 토큰 예산에서
//   A: 키워드 검색 (질문의 단어로 grep → 많이 맞은 파일부터 통째로 읽기) — AI 에이전트가 맥락 엔진 없이 흔히 하는 방식
//   B: Lantern 맥락 엔진 (lantern context)
// 이 정답 파일을 얼마나 가져오는지 잰다. 모델이 좋은 답을 하려면 먼저 이 파일을 봐야 한다.
//
// 사용: node eval/retrieval.mjs --project <프로젝트> --golden <정답 파일 JSON> [--budget 8000] [--lantern target/release/lantern.exe] [--lang ko]
// 결과: eval/results/retrieval-<시각>.md / .json
//
// 질문에 `parent`(커밋)가 있으면 질문마다 프로젝트를 그 커밋으로 꺼내서 잰다 (eval/bench의 공개 벤치마크).
// 이때 프로젝트는 변경 없는 git 클론이어야 하고, 끝나면 원래 커밋으로 돌려 놓는다.
// --lang ko: 질문의 한국어판(`q_ko`)으로 잰다. 정답 파일은 같다.

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
const LANG = opt("lang", "en");
const golden = JSON.parse(fs.readFileSync(path.resolve(GOLDEN), "utf8"));
const questions = golden.questions
  .map((q) => (LANG === "ko" ? { ...q, q: q.q_ko } : q))
  .filter((q) => q.q);

// ── 커밋별 평가: 질문마다 부모 커밋으로 꺼낸다 ─────────────────
const git = (...a) => execFileSync("git", ["-C", PROJECT, ...a], { encoding: "utf8" }).trim();
const needsCheckout = questions.some((q) => q.parent);
let original = null;
if (needsCheckout) {
  if (git("status", "--porcelain", "--untracked-files=no")) {
    console.error(`${PROJECT}에 커밋하지 않은 변경이 있습니다. 커밋별 평가는 깨끗한 클론에서만 합니다.`);
    process.exit(2);
  }
  original = git("rev-parse", "HEAD");
}
function checkout(rev) {
  if (rev) execFileSync("git", ["-C", PROJECT, "-c", "core.longpaths=true", "checkout", "-q", "--detach", rev], { stdio: "ignore" });
}

// ── 프로젝트 파일 (맥락 엔진과 같은 기준: 코드 파일, 무시 폴더 제외) ─────────
const CODE = /\.(ts|tsx|js|jsx|mjs|cjs|py|rs|java|go|cs)$/;
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
const readFiles = () =>
  [...walk(PROJECT)]
    .filter((p) => fs.statSync(p).size < 400_000)
    .map((p) => ({ path: rel(p), text: fs.readFileSync(p, "utf8") }))
    .filter((f) => !minified(f.text));
let files = readFiles();
// 토큰은 맥락 엔진과 같은 거친 추정 (글자 4개 ≈ 1토큰)
const tokens = (s) => Math.ceil(s.length / 4);

// ── A: 키워드 검색 ────────────────────────────────────────
const STOP = new Set(["the", "and", "for", "how", "what", "where", "does", "어디서", "어떻게", "무엇을", "하나", "해", "곳", "로직", "하려면", "어디를", "고쳐야", "때", "시"]);
function keywords(q) {
  const latin = (q.match(/[A-Za-z][A-Za-z0-9_]{2,}/g) ?? []).map((w) => w.toLowerCase());
  const korean = (q.match(/[가-힣]{2,}/g) ?? []).map((w) => w.replace(/(은|는|이|가|을|를|의|에서|에|으로|로|와|과|하는|한|할|된|되는)$/, "")).filter((w) => w.length >= 2);
  return [...new Set([...latin, ...korean])].filter((w) => !STOP.has(w));
}

function grepScored(terms) {
  return files
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
}

function grepRetrieve(q) {
  const terms = keywords(q);
  const scored = grepScored(terms);
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

/** 키워드 검색 (조각): 파일을 통째로 읽지 않고 맞은 줄 앞뒤만 가져와 같은 예산을 여러 파일에 나눈다 (rg -C 방식) */
const WINDOW = 12;
function grepSnippets(q) {
  const terms = keywords(q);
  const picked = [];
  let used = 0;
  for (const x of grepScored(terms)) {
    if (used >= BUDGET) break;
    const lines = x.f.text.split("\n");
    const take = new Set();
    lines.forEach((l, i) => {
      const lower = l.toLowerCase();
      if (terms.some((t) => lower.includes(t))) for (let j = Math.max(0, i - WINDOW); j <= Math.min(lines.length - 1, i + WINDOW); j++) take.add(j);
    });
    if (!take.size) take.add(0); // 경로만 맞음: 첫 줄부터
    const text = [...take].sort((a, b) => a - b).map((i) => lines[i]).join("\n");
    // 파일 하나가 예산을 다 먹지 않게, 한 파일은 예산의 1/4까지
    used += Math.min(tokens(text), BUDGET / 4, BUDGET - used);
    picked.push(x.f.path);
  }
  return { files: picked, used, terms };
}

/** 파일 n개를 무작위로 골랐을 때 정답 파일을 하나라도 가져올 확률 */
function chanceAny(n, total, g) {
  if (n <= 0) return 0;
  let miss = 1;
  for (let i = 0; i < n; i++) miss *= Math.max(0, total - g - i) / (total - i);
  return 1 - miss;
}

// ── B: Lantern ─────────────────────────────────────────────
function lanternRetrieve(q) {
  const env = { ...process.env, LANTERN_INDEX_DIR: process.env.LANTERN_INDEX_DIR || path.join(OUT, `index-${path.basename(PROJECT)}`) };
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
  return { recall: hit.length / q.golden.length, all: hit.length === q.golden.length, any: hit.length > 0, top3: firstRank >= 0 && firstRank < 3, hit, alt, rank: firstRank < 0 ? null : firstRank + 1 };
}

// 인덱스를 먼저 만든다 (첫 질문 시간에 인덱싱이 섞이지 않게)
console.log(`프로젝트 파일 ${files.length}개, 질문 ${questions.length}개, 예산 ${BUDGET} 토큰`);
lanternRetrieve("warmup");

const rows = questions.map((q, i) => {
  if (q.parent) {
    checkout(q.parent);
    files = readFiles();
  }
  const a = grepRetrieve(q.q);
  const s = grepSnippets(q.q);
  const b = lanternRetrieve(q.q);
  const sa = score(a.files, q);
  const ss = score(s.files, q);
  const sb = score(b.files, q);
  const chance = chanceAny(b.files.length, files.length, q.golden.length);
  console.log(`${String(i + 1).padStart(2)}. 키워드 ${Math.round(sa.recall * 100)}% · 키워드(조각) ${Math.round(ss.recall * 100)}% · Lantern ${Math.round(sb.recall * 100)}% — ${q.q}`);
  return { q: q.q, golden: q.golden, commit: q.commit, grep: { ...a, ...sa }, snippets: { ...s, ...ss }, lantern: { ...b, ...sb }, chance };
});
if (original) checkout(original);

const mean = (k, f) => rows.reduce((n, r) => n + f(r[k]), 0) / rows.length;
const pct = (x) => `${Math.round(x * 100)}%`;
const sum = (k) => ({
  recall: mean(k, (x) => x.recall),
  all: mean(k, (x) => (x.all ? 1 : 0)),
  any: mean(k, (x) => (x.any ? 1 : 0)),
  top3: mean(k, (x) => (x.top3 ? 1 : 0)),
  tokens: mean(k, (x) => x.used),
  files: mean(k, (x) => x.files.length),
});
const summary = { grep: sum("grep"), snippets: sum("snippets"), lantern: sum("lantern"), chance: rows.reduce((n, r) => n + r.chance, 0) / rows.length };
const wins = rows.filter((r) => r.lantern.recall > Math.max(r.grep.recall, r.snippets.recall)).length;
const losses = rows.filter((r) => r.lantern.recall < Math.max(r.grep.recall, r.snippets.recall)).length;

fs.mkdirSync(OUT, { recursive: true });
const stamp = `${path.basename(PROJECT)}${LANG === "ko" ? "-ko" : ""}-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-")}`;
fs.writeFileSync(path.join(OUT, `retrieval-${stamp}.json`), JSON.stringify({ project: path.basename(PROJECT), lang: LANG, budget: BUDGET, summary, rows }, null, 2));
const md = [
  `# 맥락 적중률 (${stamp})`,
  "",
  `프로젝트 \`${path.basename(PROJECT)}\` · 코드 파일 ${files.length}개 · 질문 ${rows.length}개${LANG === "ko" ? " (한국어)" : ""} · 예산 ${BUDGET} 토큰 · 모델 호출 없음`,
  "",
  "정답 파일: 질문에 답하려면 꼭 봐야 하는 파일. 같은 예산에서 각 방식이 정답 파일을 몇 개 가져오는지 잰다.",
  "- 키워드 검색: 질문의 단어로 grep → 많이 맞은 파일부터 통째로 읽기",
  "- 키워드 검색 (조각): 같은 grep에서 맞은 줄 앞뒤 12줄만 가져와 예산을 여러 파일에 나누기",
  "- 앞 3개 파일: 가져온 파일 중 처음 3개 안에 정답 파일이 있는가 (많이 가져올수록 유리한 효과를 뺀 지표)",
  "- 무작위 기준: Lantern이 가져온 것과 같은 수의 파일을 무작위로 골랐을 때 정답을 하나라도 가져올 확률",
  "",
  "| 지표 | 키워드 검색 | 키워드 (조각) | Lantern |",
  "|---|---|---|---|",
  `| 정답 파일 적중률 (평균) | ${pct(summary.grep.recall)} | ${pct(summary.snippets.recall)} | ${pct(summary.lantern.recall)} |`,
  `| 정답 파일을 모두 가져온 질문 | ${pct(summary.grep.all)} | ${pct(summary.snippets.all)} | ${pct(summary.lantern.all)} |`,
  `| 정답 파일을 하나라도 가져온 질문 | ${pct(summary.grep.any)} | ${pct(summary.snippets.any)} | ${pct(summary.lantern.any)} (무작위 ${pct(summary.chance)}) |`,
  `| 앞 3개 파일에 정답 | ${pct(summary.grep.top3)} | ${pct(summary.snippets.top3)} | ${pct(summary.lantern.top3)} |`,
  `| 평균 사용 토큰 | ${Math.round(summary.grep.tokens)} | ${Math.round(summary.snippets.tokens)} | ${Math.round(summary.lantern.tokens)} |`,
  `| 평균 가져온 파일 수 | ${summary.grep.files.toFixed(1)} | ${summary.snippets.files.toFixed(1)} | ${summary.lantern.files.toFixed(1)} |`,
  `| Lantern 승 / 무 / 패 (두 키워드 방식 중 나은 쪽과) | | | ${wins} / ${rows.length - wins - losses} / ${losses} |`,
  "",
  "## 질문별",
  "",
  "| # | 질문 | 정답 파일 | 키워드 | 키워드 (조각) | Lantern | Lantern 첫 정답 순위 |",
  "|---|---|---|---|---|---|---|",
  ...rows.map((r, i) => `| ${i + 1} | ${r.q} | ${r.golden.map((g) => `\`${g.split("/").pop()}\``).join(", ")} | ${pct(r.grep.recall)} | ${pct(r.snippets.recall)} | ${pct(r.lantern.recall)} | ${r.lantern.rank ?? "-"} |`),
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
console.log(`\n정답 파일 적중률: 키워드 ${pct(summary.grep.recall)} · 키워드(조각) ${pct(summary.snippets.recall)} · Lantern ${pct(summary.lantern.recall)} (무작위 ${pct(summary.chance)})`);
console.log(`앞 3개 파일에 정답: 키워드 ${pct(summary.grep.top3)} · 키워드(조각) ${pct(summary.snippets.top3)} · Lantern ${pct(summary.lantern.top3)}\n보고서: ${mdPath}`);
