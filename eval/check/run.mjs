// lantern check를 공개 커밋으로 잰다. 모델 호출 없음.
//
// 커밋이 고친 코드 파일 중 하나만 부모에 얹고 check를 돌려, 나머지 정답 파일을 가리키는지와 경고 수를 본다.
// 같은 시점에서 시그니처 변화와 상관없이 파일의 심볼을 모두 보는 넓은 그물(영향 반경)도 재서 숫자의 맥락을 준다.
// 방법과 결과는 README.md. 클론은 건드리지 않고 임시 클론(--shared)에서만 일한다.
//
// 사용: cargo build --release -p lantern-context
//       node eval/check/run.mjs --dir <저장소 클론 폴더> [--max-seconds 180] [--repos flask,gin] [--lantern <실행 파일>]
//         [--keep-index] [--out <결과 폴더>]
//       node eval/check/run.mjs --summarize   재지 않고 results.json으로 results.md만 다시 쓴다
//
// 커밋마다 빈 인덱스에서 시작한다. 인덱스를 이어 쓰면 영향 반경의 호출자 상한이 인덱스 이력에 따라 흔들린다 (--keep-index로 비교).

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(HERE, "..", "..");
const argv = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
};
const SET = path.resolve(opt("set", path.join(ROOT, "eval", "bilingual")));
const OUT = path.resolve(opt("out", HERE));
const MAX_SECONDS = Number(opt("max-seconds", "180"));
const REPOS = opt("repos", "").split(",").filter(Boolean);
const EXE = process.platform === "win32" ? "lantern.exe" : "lantern";
const LANTERN = opt("lantern", path.join(process.env.CARGO_TARGET_DIR || path.join(ROOT, "target"), "release", EXE));
// graph.rs의 MAX_DEFS: 이보다 많이 정의된 이름은 영향 반경도 잇지 않는다
const MAX_DEFS = 3;
// check.rs의 MAX_FINDINGS: --all 없이 보이는 수
const MAX_SHOWN = 20;
const WHOLE_FILE = "1-100000000";
const FAMILY = { typescript: "js", tsx: "js", javascript: "js", c: "c", cpp: "c" };
const family = (lang) => FAMILY[lang] ?? lang;

// 재는 동안에는 저장소가 네트워크로 나가지 않는다. 없는 본문은 재기 전에 fetchMissing이 한 번만 받는다
const gitEnv = { ...process.env, GIT_NO_LAZY_FETCH: "1", GIT_TERMINAL_PROMPT: "0", GIT_LFS_SKIP_SMUDGE: "1" };

function git(cwd, args, input) {
  const r = spawnSync("git", ["-C", cwd, ...args], { env: gitEnv, input, maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${String(r.stderr).trim().slice(0, 400)}`);
  return r.stdout;
}

// 클론이 그대로인지 비교할 지문: 설정·HEAD·인덱스의 크기와 시간, 팩 목록
function fingerprint(repo) {
  const g = path.join(repo, ".git");
  const stat = (p) => {
    const s = fs.statSync(path.join(g, p));
    return `${p}:${s.size}:${s.mtimeMs}`;
  };
  const packs = fs.readdirSync(path.join(g, "objects", "pack")).sort().join(",");
  return [stat("config"), stat("HEAD"), stat("index"), stat("objects"), packs].join("|");
}

// 블롭 없는 클론에는 커밋 쪽 본문이 없을 수 있다. 임시 클론에만 받으니 원본 클론은 그대로다
function fetchMissing(clone, url, questions) {
  const specs = questions.flatMap((q) => q.golden.map((g) => `${q.commit}:${g}`));
  const oids = [...new Set(String(git(clone, ["rev-parse", ...specs])).trim().split(/\r?\n/))];
  const check = String(git(clone, ["cat-file", "--batch-check"], `${oids.join("\n")}\n`)).trim().split(/\r?\n/);
  const missing = oids.filter((_, i) => check[i].endsWith(" missing"));
  if (missing.length) {
    git(clone, ["-c", "fetch.negotiationAlgorithm=noop", "fetch", "-q", "--no-tags", "--no-write-fetch-head", "--recurse-submodules=no", url, ...missing]);
  }
  return { needed: oids.length, fetched: missing.length };
}

// 그물이 집은 파일을 정답과 견준 것. 목록 전체는 크니 수와 정답만 남긴다
function compare(flagged, file, otherGold, commitOther) {
  const set = new Set(flagged);
  set.delete(file);
  return { n: set.size, gold: otherGold.filter((g) => set.has(g)), commit_other: commitOther.filter((c) => set.has(c)).length };
}

function runImpact(env, clone, file) {
  const t = performance.now();
  const r = spawnSync(LANTERN, ["-C", clone, "graph", "impact", file, "--lines", WHOLE_FILE], { env, encoding: "utf8", maxBuffer: 1 << 28 });
  const ms = performance.now() - t;
  if (r.status !== 0) throw new Error(`graph impact ${file}: ${r.stderr}`);
  // 이력을 못 읽으면 동시 변경이 조용히 비므로 멈춘다
  if (/건너뜀/.test(r.stderr)) throw new Error(`graph impact ${file}: ${r.stderr}`);
  const v = JSON.parse(r.stdout);
  const callers = v.graph.nodes.filter((n) => n.kind === "symbol" && n.depth >= 1);
  return { ms, cochange: v.cochanged.map(([p]) => p), callers1: callers.filter((n) => n.depth === 1).map((n) => n.path), callers2: callers.filter((n) => n.depth === 2).map((n) => n.path) };
}

// 파일의 모든 심볼(클래스 같은 바깥 심볼 포함)에서 상한 없이: 같은 언어 계열의 참조, 함께 바뀐 파일 전부
function everySymbol(db, file) {
  const refs = db
    .prepare(
      `SELECT DISTINCT fr.path AS path, fr.lang AS lang, fs.lang AS own FROM symbols s
       JOIN files fs ON fs.id = s.file_id
       JOIN refs r ON r.name = s.name
       JOIN files fr ON fr.id = r.file_id
       WHERE fs.path = ? AND (SELECT COUNT(*) FROM symbols d WHERE d.name = s.name) <= ${MAX_DEFS}`,
    )
    .all(file)
    .filter((x) => family(x.lang) === family(x.own))
    .map((x) => x.path);
  const cochange = db.prepare("SELECT file_b AS path FROM cochange WHERE file_a = ? UNION SELECT file_a FROM cochange WHERE file_b = ?").all(file, file).map((x) => x.path);
  return { refs, cochange };
}

function openIndex(indexDir) {
  const dbs = fs.readdirSync(indexDir).filter((f) => f.endsWith(".db"));
  if (dbs.length !== 1) throw new Error(`인덱스가 ${dbs.length}개: ${dbs.join(", ")}`);
  return new DatabaseSync(path.join(indexDir, dbs[0]), { readOnly: true });
}

function runCheck(env, clone, parent) {
  const t = performance.now();
  const r = spawnSync(LANTERN, ["-C", clone, "check", "--base", parent, "--json", "--all"], { env, encoding: "utf8", maxBuffer: 1 << 28 });
  const ms = performance.now() - t;
  // check는 --strict 없이는 오류에도 0으로 끝나므로 빈 출력을 오류로 본다
  if (r.status !== 0 || !r.stdout.trim()) throw new Error(`check: ${r.stderr}`);
  return { ms, report: JSON.parse(r.stdout) };
}

// 커밋 하나. 파일마다 한 행
function measureCommit(env, clone, indexDir, q, rows) {
  git(clone, ["checkout", "-q", "-f", "--detach", q.parent]);
  if (!argv.includes("--keep-index")) fs.rmSync(indexDir, { recursive: true, force: true });
  const commitFiles = String(git(clone, ["diff-tree", "-r", "--no-renames", "--name-only", "-z", q.parent, q.commit])).split("\0").filter(Boolean);
  const commitOther = commitFiles.filter((f) => !q.golden.includes(f));

  // 넓은 그물은 부모 상태에서 잰다. 첫 호출이 인덱스와 동시 변경을 만든다
  const naive = new Map();
  for (const file of q.golden) naive.set(file, { impact: runImpact(env, clone, file) });
  const db = openIndex(indexDir);
  for (const file of q.golden) naive.get(file).every = everySymbol(db, file);
  db.close();

  for (const file of q.golden) {
    const otherGold = q.golden.filter((g) => g !== file);
    const abs = path.join(clone, file);
    fs.writeFileSync(abs, git(clone, ["cat-file", "blob", `${q.commit}:${file}`]));
    const { ms, report } = runCheck(env, clone, q.parent);
    git(clone, ["checkout", "-q", q.parent, "--", file]);

    const findings = report.findings.map((f) => ({ kind: f.kind, symbol: f.symbol, path: f.path, line: f.line, refs_in_file: f.refs_in_file, in_test: f.in_test }));
    const paths = (kind) => findings.filter((f) => !kind || f.kind === kind).map((f) => f.path);
    const { impact, every } = naive.get(file);
    rows.push({
      project: q.project,
      commit: q.commit,
      parent: q.parent,
      file,
      other_gold: otherGold,
      commit_other: commitOther.length,
      changed_files: report.changed_files,
      changed_code_files: report.changed_code_files,
      changes: { removed: report.removed, signature: report.signature, body: report.body },
      findings,
      check_ms: Math.round(ms),
      impact_ms: Math.round(impact.ms),
      builds_index: file === q.golden[0],
      nets: {
        check: compare(paths(), file, otherGold, commitOther),
        // --all 없이 보이는 앞 20개만
        check_shown: compare(findings.slice(0, MAX_SHOWN).map((f) => f.path), file, otherGold, commitOther),
        dangling: compare(paths("dangling"), file, otherGold, commitOther),
        stale_caller: compare(paths("stale_caller"), file, otherGold, commitOther),
        impact: compare([...impact.cochange, ...impact.callers1, ...impact.callers2], file, otherGold, commitOther),
        impact_callers1: compare(impact.callers1, file, otherGold, commitOther),
        impact_cochange: compare(impact.cochange, file, otherGold, commitOther),
        every_refs: compare(every.refs, file, otherGold, commitOther),
        every_cochange: compare(every.cochange, file, otherGold, commitOther),
        every: compare([...every.refs, ...every.cochange], file, otherGold, commitOther),
      },
    });
  }
}

function measureRepo(set, rows, tmp) {
  const repo = path.join(opt("dir", ""), set.project);
  if (!fs.existsSync(path.join(repo, ".git"))) throw new Error(`클론이 없습니다: ${repo} (node eval/bench/run.mjs --set eval/bilingual --dir ...로 받으세요)`);
  const questions = set.questions.filter((q) => q.golden.length >= 2).map((q) => ({ ...q, project: set.project }));
  const before = fingerprint(repo);
  const clone = path.join(tmp, set.project);
  const indexDir = path.join(tmp, "index", set.project);
  const env = { ...gitEnv, LANTERN_INDEX_DIR: indexDir, LANTERN_DATA_DIR: path.join(tmp, "data") };

  const started = performance.now();
  git(tmp, ["clone", "-q", "--shared", "--no-checkout", "--no-tags", "-c", "core.autocrlf=false", "-c", "core.longpaths=true", "-c", "core.fsmonitor=false", "-c", `core.hooksPath=${path.join(tmp, "no-hooks")}`, "-c", "gc.auto=0", "-c", "maintenance.auto=false", repo, clone]);
  const blobs = fetchMissing(clone, set.url, questions);
  const setupMs = performance.now() - started;

  let done = 0;
  for (const q of questions) {
    if ((performance.now() - started) / 1000 > MAX_SECONDS) break;
    measureCommit(env, clone, indexDir, q, rows);
    done++;
    console.log(`  ${set.project} ${done}/${questions.length} ${q.commit.slice(0, 10)}`);
  }
  const wallMs = performance.now() - started;
  // 임시 클론의 대체 객체 연결을 끊지 않고 지워도 원본에는 영향이 없다
  fs.rmSync(clone, { recursive: true, force: true, maxRetries: 5 });
  if (fingerprint(repo) !== before) throw new Error(`클론이 바뀌었습니다: ${repo}`);
  return { project: set.project, commits: questions.length, done, capped: done < questions.length, setup_ms: Math.round(setupMs), wall_ms: Math.round(wallMs), blobs_needed: blobs.needed, blobs_fetched: blobs.fetched };
}

// ── 표 ──────────────────────────────────────────────────

const sum = (xs) => xs.reduce((a, b) => a + b, 0);
function quantile(xs, p) {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.ceil(s.length * p) - 1)];
}
const median = (xs) => {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};
const pct = (a, b) => (b ? `${((100 * a) / b).toFixed(1)}%` : "n/a");
const num = (x) => (Number.isInteger(x) ? String(x) : x.toFixed(2));

function perCommit(rows, value) {
  const by = new Map();
  for (const r of rows) by.set(r.commit, (by.get(r.commit) ?? 0) + value(r));
  return [...by.values()];
}

function netRow(label, rows, name) {
  const net = (r) => r.nets[name];
  const commits = new Set(rows.map((r) => r.commit));
  const hitRows = rows.filter((r) => net(r).gold.length > 0);
  const hitCommits = new Set(hitRows.map((r) => r.commit));
  const pairs = sum(rows.map((r) => r.other_gold.length));
  const found = sum(rows.map((r) => net(r).gold.length));
  const flagged = sum(rows.map((r) => net(r).n));
  const commitOther = sum(rows.map((r) => net(r).commit_other));
  const flags = rows.map((r) => net(r).n);
  const byCommit = perCommit(rows, (r) => net(r).n);
  return `| ${label} | ${hitCommits.size} / ${commits.size} (${pct(hitCommits.size, commits.size)}) | ${hitRows.length} / ${rows.length} (${pct(hitRows.length, rows.length)}) | ${found} / ${pairs} (${pct(found, pairs)}) | ${num(sum(flags) / flags.length)} / ${num(median(flags))} | ${num(sum(byCommit) / byCommit.length)} / ${num(median(byCommit))} | ${flagged - found} / ${flagged} (${pct(flagged - found, flagged)}) | ${commitOther} |`;
}

function summarize(data) {
  const { rows, repos, meta } = data;
  const commits = new Set(rows.map((r) => r.commit)).size;
  const warnings = (r) => r.findings.length;
  const byCommit = perCommit(rows, warnings);
  const count = (r, kind) => r.findings.filter((f) => f.kind === kind).length;
  const totalKind = (kind) => sum(rows.map((r) => count(r, kind)));
  const kindByCommit = (kind) => perCommit(rows, (r) => count(r, kind));
  const dangling = totalKind("dangling");
  const stale = totalKind("stale_caller");
  const danglingByCommit = kindByCommit("dangling");
  const staleByCommit = kindByCommit("stale_caller");
  const capped = rows.filter((r) => r.findings.length > MAX_SHOWN);
  const lostToCap = sum(rows.map((r) => r.nets.check.gold.length - r.nets.check_shown.gold.length));
  const changing = rows.filter((r) => r.changes.removed + r.changes.signature > 0);
  const changingCommits = new Set(changing.map((r) => r.commit));
  const hitCommits = new Set(rows.filter((r) => r.nets.check.gold.length > 0).map((r) => r.commit));
  const warned = rows.filter((r) => r.findings.length > 0);
  const warnedCommits = new Set(warned.map((r) => r.commit)).size;
  const multiChanged = rows.filter((r) => r.changed_files !== 1);
  const notCode = rows.filter((r) => r.changed_code_files !== 1);
  const inTest = sum(rows.map((r) => r.findings.filter((f) => f.in_test).length));
  const times = rows.map((r) => r.check_ms);
  const buildTimes = rows.filter((r) => r.builds_index).map((r) => r.impact_ms);
  const impactTimes = rows.filter((r) => !r.builds_index).map((r) => r.impact_ms);

  const md = [
    "# lantern check on public commits: results",
    "",
    `Written by \`node eval/check/run.mjs\`. Method and limits: [README.md](README.md). Lantern ${meta.lantern}, ${meta.date}, ${meta.platform}; cap ${meta.max_seconds} s per repository.`,
    "",
    `${commits} commits with 2 or more gold files, ${rows.length} applications (one gold file applied to the parent), ${sum(rows.map((r) => r.other_gold.length))} (applied file, other gold file) pairs.`,
    "",
    "## Does a warning point to the other gold files?",
    "",
    "| Method | Commits with a hit | Applications with a hit | Pairs found | Files flagged per application (mean / median) | Files flagged per commit (mean / median) | Flagged files outside gold (upper bound on noise) | Of those, other files the commit changed |",
    "|---|---|---|---|---|---|---|---|",
    netRow("`lantern check`", rows, "check"),
    netRow("check, Dangling only", rows, "dangling"),
    netRow("check, Stale caller only", rows, "stale_caller"),
    netRow("Impact radius: co-change + callers, 2 hops (`graph impact`)", rows, "impact"),
    netRow("Impact radius, co-change only", rows, "impact_cochange"),
    netRow("Impact radius, direct callers only", rows, "impact_callers1"),
    netRow("Every symbol, every reference, every co-change pair", rows, "every"),
    netRow("Every symbol, references only", rows, "every_refs"),
    netRow("Every co-change pair only", rows, "every_cochange"),
    "",
    "## Warnings from `lantern check`",
    "",
    "| | Count |",
    "|---|---|",
    `| Warnings, all applications | ${sum(rows.map(warnings))} |`,
    `| Dangling | ${dangling} |`,
    `| Stale caller | ${stale} |`,
    `| In test files | ${inTest} |`,
    `| Warnings per commit, mean / median / max | ${num(sum(byCommit) / byCommit.length)} / ${num(median(byCommit))} / ${Math.max(...byCommit)} |`,
    `| Dangling per commit, mean / median / max | ${num(dangling / commits)} / ${num(median(danglingByCommit))} / ${Math.max(...danglingByCommit)} |`,
    `| Stale caller per commit, mean / median / max | ${num(stale / commits)} / ${num(median(staleByCommit))} / ${Math.max(...staleByCommit)} |`,
    `| Commits with at least one warning | ${warnedCommits} / ${commits} |`,
    `| Applications with at least one warning | ${warned.length} / ${rows.length} |`,
    `| Applications with a removed symbol or a changed signature | ${changing.length} / ${rows.length} |`,
    `| Commits with a removed symbol or a changed signature in some gold file | ${changingCommits.size} / ${commits} |`,
    `| Commits with a hit, among those | ${[...hitCommits].filter((c) => changingCommits.has(c)).length} / ${changingCommits.size} |`,
    `| Applications with more warnings than the ${MAX_SHOWN} shown without \`--all\` | ${capped.length} |`,
    `| Gold hits only past those ${MAX_SHOWN} | ${lostToCap} |`,
    `| Applications where check saw anything but the one applied file | ${multiChanged.length} |`,
    `| Applied files outside the languages Lantern parses | ${notCode.length} |`,
    "",
    "## By repository",
    "",
    "| Repository | Commits measured | Applications | With removed or changed signature | Commits hit: check | Commits hit: impact | Commits hit: every | Dangling | Stale caller | Files per application: check / impact / every |",
    "|---|---|---|---|---|---|---|---|---|---|",
    ...repos.map((repo) => {
      const rs = rows.filter((r) => r.project === repo.project);
      const hit = (name) => new Set(rs.filter((r) => r.nets[name].gold.length > 0).map((r) => r.commit)).size;
      const mean = (name) => num(sum(rs.map((r) => r.nets[name].n)) / (rs.length || 1));
      const kind = (k) => sum(rs.map((r) => count(r, k)));
      return `| ${repo.project} | ${repo.done} of ${repo.commits} | ${rs.length} | ${rs.filter((r) => r.changes.removed + r.changes.signature > 0).length} | ${hit("check")} | ${hit("impact")} | ${hit("every")} | ${kind("dangling")} | ${kind("stale_caller")} | ${mean("check")} / ${mean("impact")} / ${mean("every")} |`;
    }),
    "",
    "## Run time",
    "",
    `\`lantern check\`, one process per application, one file changed, index already at the parent: median ${median(times)} ms, 90th percentile ${quantile(times, 0.9)} ms, max ${Math.max(...times)} ms; ${times.filter((t) => t <= 3000).length} of ${times.length} within 3 s. \`graph impact\` on a ready index: median ${median(impactTimes)} ms, max ${Math.max(...impactTimes)} ms. The first \`graph impact\` of each commit also builds the index from empty: median ${median(buildTimes)} ms, max ${Math.max(...buildTimes)} ms.`,
    "",
    "| Repository | Wall time (s) | Of which setup (s) | Cap reached | Gold blobs fetched into the temp clone | `check` median / max (ms) |",
    "|---|---|---|---|---|---|",
    ...repos.map((repo) => {
      const ts = rows.filter((r) => r.project === repo.project).map((r) => r.check_ms);
      return `| ${repo.project} | ${(repo.wall_ms / 1000).toFixed(1)} | ${(repo.setup_ms / 1000).toFixed(1)} | ${repo.capped ? "yes" : "no"} | ${repo.blobs_fetched} of ${repo.blobs_needed} | ${median(ts)} / ${ts.length ? Math.max(...ts) : 0} |`;
    }),
    `| **Total** | ${(sum(repos.map((r) => r.wall_ms)) / 1000).toFixed(1)} | ${(sum(repos.map((r) => r.setup_ms)) / 1000).toFixed(1)} | ${repos.filter((r) => r.capped).length} repositories | ${sum(repos.map((r) => r.blobs_fetched))} of ${sum(repos.map((r) => r.blobs_needed))} | |`,
    "",
  ].join("\n");
  fs.writeFileSync(path.join(OUT, "results.md"), md);
  console.log(md);
}

// ── 실행 ────────────────────────────────────────────────

if (argv.includes("--summarize")) {
  summarize(JSON.parse(fs.readFileSync(path.join(OUT, "results.json"), "utf8")));
} else {
  if (!opt("dir", "")) throw new Error("--dir <저장소 클론 폴더>가 필요합니다");
  if (!fs.existsSync(LANTERN)) throw new Error(`실행 파일이 없습니다: ${LANTERN} (cargo build --release -p lantern-context)`);
  fs.mkdirSync(OUT, { recursive: true });
  const sets = fs
    .readdirSync(SET)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => JSON.parse(fs.readFileSync(path.join(SET, f), "utf8")))
    .filter((s) => !REPOS.length || REPOS.includes(s.project));
  const rows = [];
  const repos = [];
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "lantern-check-"));
  try {
    for (const set of sets) {
      console.log(`== ${set.project}`);
      repos.push(measureRepo(set, rows, tmp));
    }
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 5 });
  }
  const meta = {
    date: new Date().toLocaleDateString("sv"),
    lantern: String(git(ROOT, ["rev-parse", "--short", "HEAD"])).trim(),
    platform: `${process.platform}, node ${process.version}`,
    max_seconds: MAX_SECONDS,
    keep_index: argv.includes("--keep-index"),
  };
  const data = { meta, repos, rows };
  fs.writeFileSync(path.join(OUT, "results.json"), `${JSON.stringify(data, null, 1)}\n`);
  summarize(data);
}
