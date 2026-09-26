// 공개 벤치마크 만들기: 오픈소스 저장소의 실제 커밋에서 질문과 정답 파일을 뽑는다.
//   질문 = 커밋 메시지 제목, 정답 파일 = 그 커밋이 고친 코드 파일, 평가 시점 = 그 커밋의 부모.
// 사람이 정답을 고르지 않으므로 엔진을 아는 사람의 편향이 들어가지 않고, 누구나 같은 커밋으로 재현할 수 있다.
//
// 사용: node eval/bench/make.mjs --repo <클론 폴더> --name flask --url https://github.com/pallets/flask [--count 20]
// 결과: eval/bench/<name>.json (고정한 커밋과 질문 목록)

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
const REPO = path.resolve(opt("repo", ""));
const NAME = opt("name", path.basename(REPO));
const URL = opt("url", "");
const COUNT = Number(opt("count", "20"));
const REF = opt("ref", "HEAD");
if (!fs.existsSync(path.join(REPO, ".git"))) {
  console.error("--repo <git 클론 폴더>가 필요합니다");
  process.exit(2);
}
const git = (...a) => execFileSync("git", ["-C", REPO, ...a], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });

// 맥락 엔진이 읽는 언어와 같은 기준
const CODE = /\.(ts|tsx|js|jsx|mjs|cjs|py|rs|java|go|cs)$/;
const isTest = (p) => {
  const l = p.toLowerCase();
  const file = p.split("/").pop() ?? "";
  const stem = file.replace(/\.[^.]+$/, "");
  return /(^|\/)(tests?|__tests__|spec)\//.test(l) || /\.(test|spec)\./.test(l) || /_test\./.test(l) || file.startsWith("test_") || /(Test|Tests|IT)$/.test(stem);
};
// 질문으로 쓰기 어려운 커밋 (버전·문서·빌드·되돌리기 등)
const SKIP = /^(merge|bump|release|revert|chore|docs?|ci|build|style|test|tests|typo|format|lint|deps|update dependencies|prepare|version)\b/i;
// 도구·문서 손질 (코드 동작을 묻는 질문이 되지 않는다)
const SKIP_ANY = /\b(ruff|noqa|lint|linter|eslint|prettier|mypy|pyright|flake8|checkstyle|spotbugs|pmd|javadoc|docs?|docstrings?|typos?|changelog|readme|copyright|license|whitespace|formatting|comments?|unused|ternary)\b/i;
/** 같은 정답 파일 묶음은 이 수까지만 (한 파일에 몰린 이력이 결과를 좌우하지 않게) */
const MAX_SAME_GOLDEN = 2;

/** "fix(router): handle trailing slash (#123)" → "handle trailing slash" */
function clean(subject) {
  return subject
    .replace(/^\[[^\]]*\]\s*/, "")
    .replace(/^[a-z]+(\([^)]*\))?!?:\s*/i, "")
    .replace(/\s*\(#\d+\)\s*$/, "")
    .replace(/\s*#\d+\s*$/, "")
    .trim();
}

const head = git("rev-parse", REF).trim();
const log = git("log", "--no-merges", "--format=@@%H %P%n%s", "--name-status", "-n", "4000", head);
const questions = [];
for (const block of log.split("@@").slice(1)) {
  if (questions.length >= COUNT) break;
  const [first, subject, ...rest] = block.split("\n");
  const [sha, ...parents] = first.trim().split(" ");
  if (parents.length !== 1 || !subject || SKIP.test(subject.trim()) || SKIP_ANY.test(subject)) continue;
  const changes = rest.filter(Boolean).map((l) => l.split("\t"));
  if (changes.length > 6) continue; // 큰 리팩터링은 질문 하나로 가리키기 어렵다
  const code = changes.filter(([, p]) => p && CODE.test(p) && !isTest(p));
  // 새로 만든·지운·옮긴 파일은 부모 시점에 없거나 이름이 달라 찾을 수 없다
  if (!code.length || code.length > 3 || code.some(([s]) => s !== "M")) continue;
  const golden = code.map(([, p]) => p);
  const q = clean(subject);
  if (q.length < 20 || q.split(/\s+/).length < 4) continue;
  // 메시지에 파일 이름이 나오면 검색이 아니라 받아쓰기가 된다
  const leaks = golden.some((g) => {
    const stem = g.split("/").pop().replace(/\.[^.]+$/, "").toLowerCase();
    return stem.length >= 3 && q.toLowerCase().includes(stem);
  });
  if (leaks) continue;
  const key = [...golden].sort().join("|");
  if (questions.some((x) => x.q === q) || questions.filter((x) => [...x.golden].sort().join("|") === key).length >= MAX_SAME_GOLDEN) continue;
  questions.push({ q, q_ko: null, golden, commit: sha, parent: parents[0] });
}

const file = path.join(HERE, `${NAME}.json`);
// 다시 만들어도 사람이 옮긴 한국어 질문은 같은 커밋이면 그대로 둔다
if (fs.existsSync(file)) {
  const prev = new Map(JSON.parse(fs.readFileSync(file, "utf8")).questions.map((x) => [x.commit, x.q_ko]));
  for (const x of questions) x.q_ko = prev.get(x.commit) ?? null;
}
const out = { project: NAME, url: URL, ref: head, made_by: "eval/bench/make.mjs", questions };
fs.writeFileSync(file, JSON.stringify(out, null, 2) + "\n");
console.log(`${file}: 질문 ${questions.length}개 (기준 커밋 ${head.slice(0, 10)})`);
for (const [i, x] of questions.entries()) console.log(`${String(i + 1).padStart(2)}. ${x.q}  →  ${x.golden.join(", ")}`);
