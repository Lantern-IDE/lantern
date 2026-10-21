"""LLM 재작성(ko-llm)을 질문마다 한 번씩 부를 때의 지연 시간·토큰·비용 (실제 모델 호출, 계정 사용량을 씀).

    python eval/bilingual/rewrite_latency.py [--per-repo 3] [--model claude-opus-5-5] [--out latency.json]

API 키 없이 이 PC의 Claude Code 헤드리스 모드(claude -p)로 부른다: 도구 없음, 설정·CLAUDE.md 없음(빈 폴더에서),
재작성 지시문만 시스템 프롬프트로. 개발 중에 보지 않은 문항에서 저장소마다 --per-repo개를 무작위로 고른다.
입력 토큰에는 Claude Code가 붙이는 최소한의 내용이 들어가고, 비용은 Claude Code가 알려 주는 정가 기준(캐시 쓰기 포함)이다.
"""

import argparse
import glob
import json
import os
import random
import shutil
import statistics
import subprocess
import tempfile
import time

from baselines import HERE

SYSTEM = """You are the query-rewriting step of a coding agent. A developer asked a question in Korean about an open-source repository; before searching the code, you rewrite it as an English code-search query.

Rules:
1. Write a short English query (one line) that a coding agent would use to search this repository for the code to read or change: say what the code does or what should change, in the words developers use in English code and commit messages.
2. Keep every code identifier, option, flag, header, function and type name from the Korean exactly as written.
3. Do not add information that is not in the Korean question (no guessed file names, function names or causes). Work only from the Korean text and the repository name.
4. Reply with the query only; no explanations.
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--per-repo", type=int, default=3)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--model", default="claude-opus-5-5")
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    seen = set()
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            seen |= {q["commit"] for q in json.load(fh)["questions"]}
    rnd = random.Random(a.seed)
    picked = []
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        cand = [q for q in s["questions"] if q["commit"] not in seen and q.get("q_ko")]
        rnd.shuffle(cand)
        picked += [(s["project"], q) for q in cand[: a.per_repo]]
    claude = shutil.which("claude")
    work = tempfile.mkdtemp(prefix="lantern-rewrite-")
    sysfile = os.path.join(work, "system.txt")
    with open(sysfile, "w", encoding="utf-8") as fh:
        fh.write(SYSTEM)
    empty = os.path.join(work, "empty")
    os.makedirs(empty)
    rows = []
    for proj, q in picked:
        t = time.time()
        out = subprocess.run(
            [claude, "-p", "--model", a.model, "--system-prompt-file", sysfile, "--tools", "", "--setting-sources", "", "--no-session-persistence", "--output-format", "json"],
            input=f"Repository: {proj}\nQuestion: {q['q_ko']}\n", capture_output=True, text=True, encoding="utf-8", cwd=empty, timeout=180, check=True,
        )
        r = json.loads(out.stdout)
        u = r["usage"]
        rows.append({
            "project": proj, "ko": q["q_ko"], "rewrite": (r.get("result") or "").strip(), "api_ms": r["duration_api_ms"], "wall_s": round(time.time() - t, 2),
            "in": u["input_tokens"] + u.get("cache_creation_input_tokens", 0) + u.get("cache_read_input_tokens", 0), "out": u["output_tokens"], "usd": r["total_cost_usd"],
        })
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            json.dump(rows, fh, ensure_ascii=False, indent=1)
    med = lambda k: statistics.median(x[k] for x in rows)  # noqa: E731
    p90 = sorted(x["api_ms"] for x in rows)[int(0.9 * len(rows)) - 1]
    print(f"{len(rows)}문항: API 시간 중앙값 {med('api_ms'):.0f}ms (90% {p90}ms), 실제 걸린 시간 중앙값 {med('wall_s')}초, "
          f"입력 {med('in'):.0f}·출력 {med('out'):.0f}토큰, 비용 중앙값 ${med('usd'):.4f}")


if __name__ == "__main__":
    main()
