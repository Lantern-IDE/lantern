// 공개 벤치마크 한 번에 돌리기: 저장소를 받아(없으면) 고정한 커밋으로 맞추고, 영어·한국어 질문으로 잰 뒤
// eval/bench/결과.md에 합계를 쓴다. 모델 호출 없음, 무료.
//
// 사용: cargo build --release -p lantern-context
//       node eval/bench/run.mjs --dir <저장소를 받을 폴더> [--budget 8000]

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
const DIR = path.resolve(opt("dir", path.join(HERE, "..", "results", "bench")));
const BUDGET = opt("budget", "8000");
const RESULTS = path.join(DIR, "results");
fs.mkdirSync(RESULTS, { recursive: true });

const sets = fs.readdirSync(HERE).filter((f) => f.endsWith(".json")).sort();
const table = [];
for (const f of sets) {
  const set = JSON.parse(fs.readFileSync(path.join(HERE, f), "utf8"));
  const repo = path.join(DIR, set.project);
  if (!fs.existsSync(path.join(repo, ".git"))) {
    console.log(`${set.url} 받는 중…`);
    execFileSync("git", ["-c", "core.longpaths=true", "clone", "-q", "--filter=blob:none", "--no-checkout", set.url, repo], { stdio: "inherit" });
    execFileSync("git", ["-C", repo, "config", "core.longpaths", "true"]);
  }
  execFileSync("git", ["-C", repo, "checkout", "-q", "-f", "--detach", set.ref], { stdio: "inherit" });
  for (const lang of ["en", "ko"]) {
    console.log(`\n== ${set.project} (${lang})`);
    execFileSync("node", [path.join(HERE, "..", "retrieval.mjs"), "--project", repo, "--golden", path.join(HERE, f), "--budget", BUDGET, "--out", RESULTS, "--lang", lang], { stdio: "inherit" });
    // retrieval-<프로젝트>[-ko]-<시각>.json (영어 쪽이 한국어 파일을 집지 않게 시각 앞까지 정확히 맞춘다)
    const name = new RegExp(`^retrieval-${set.project.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}${lang === "ko" ? "-ko" : ""}-\\d{4}-.*\\.json$`);
    const latest = fs.readdirSync(RESULTS).filter((x) => name.test(x)).sort().pop();
    const r = JSON.parse(fs.readFileSync(path.join(RESULTS, latest), "utf8"));
    table.push({ project: set.project, url: set.url, ref: set.ref, lang, n: r.rows.length, s: r.summary });
  }
}

const pct = (x) => `${Math.round(x * 100)}%`;
const avg = (rows, f) => rows.reduce((n, r) => n + f(r), 0) / rows.length;
const line = (label, rows) =>
  `| ${label} | ${rows.reduce((n, r) => n + r.n, 0)} | ${pct(avg(rows, (r) => r.s.grep.recall))} | ${pct(avg(rows, (r) => r.s.snippets.recall))} | **${pct(avg(rows, (r) => r.s.lantern.recall))}** | ${pct(avg(rows, (r) => r.s.grep.top3))} | ${pct(avg(rows, (r) => r.s.snippets.top3))} | **${pct(avg(rows, (r) => r.s.lantern.top3))}** | ${pct(avg(rows, (r) => r.s.chance))} |`;
const md = [
  "# 공개 벤치마크 결과",
  "",
  `\`node eval/bench/run.mjs\`가 쓴 파일입니다. 예산 ${BUDGET} 토큰 · 모델 호출 없음 · ${new Date().toISOString().slice(0, 10)}`,
  "",
  "질문은 오픈소스 저장소의 실제 커밋 메시지, 정답 파일은 그 커밋이 고친 코드 파일이고, 각 질문은 그 커밋의 부모 시점에서 잽니다 (만드는 법: `eval/bench/make.mjs`). 한국어 질문은 같은 커밋 메시지를 사람이 옮긴 것이고 정답 파일은 같습니다.",
  "",
  "- 적중률: 정답 파일 중 가져온 비율의 평균",
  "- 앞 3개: 가져온 파일 중 처음 3개 안에 정답 파일이 있는 질문 비율 (많이 가져올수록 유리한 효과를 뺀 지표)",
  "- 무작위: Lantern이 가져온 것과 같은 수의 파일을 무작위로 골랐을 때 정답을 하나라도 가져올 확률",
  "",
  "| 저장소 (언어) | 질문 | 키워드 적중률 | 키워드(조각) 적중률 | Lantern 적중률 | 키워드 앞 3개 | 키워드(조각) 앞 3개 | Lantern 앞 3개 | 무작위 |",
  "|---|---|---|---|---|---|---|---|---|",
  ...table.map((r) => line(`[${r.project}](${r.url}/tree/${r.ref.slice(0, 10)}) ${r.lang === "ko" ? "한국어" : "영어"}`, [r])),
  line("**합계 (영어)**", table.filter((r) => r.lang === "en")),
  line("**합계 (한국어)**", table.filter((r) => r.lang === "ko")),
  "",
  "키워드: 질문의 단어로 grep → 많이 맞은 파일부터 통째로 읽기. 키워드(조각): 같은 grep에서 맞은 줄 앞뒤 12줄만 가져와 예산을 여러 파일에 나누기.",
  "",
].join("\n");
fs.writeFileSync(path.join(HERE, "결과.md"), md);
console.log(`\n${path.join(HERE, "결과.md")}`);
console.log(md);
