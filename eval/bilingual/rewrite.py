"""한국어 질문마다 LLM을 한 번씩 불러 영어 검색 질의로 다시 쓴다 (KO→LLM, q_ko_llm). 실제 모델 호출이라 계정 사용량을 쓴다.

    python eval/bilingual/rewrite.py [--set eval/bilingual] [--model claude-opus-5-5] [--force]

API 키 없이 이 PC의 Claude Code 헤드리스 모드(claude -p)로 부른다: 도구 없음, 설정·CLAUDE.md 없음(빈 폴더에서),
재작성 지시문(prompts.md 3번)만 시스템 프롬프트로, 입력은 저장소 이름과 한국어 질문 하나뿐이라 다른 문항을 보지 않는다.
호출마다 API 시간·실제 걸린 시간·토큰·비용을 measurements/rewrite.json에 남긴다. 입력 토큰에는 Claude Code가 붙이는
최소한의 내용이 들어가고, 비용은 Claude Code가 알려 주는 정가 기준(캐시 쓰기 포함)이다. 한 번씩만 뽑으므로 다시 돌리면 결과가 조금 다르다.
같은 한국어 질문을 같은 모델로 이미 다시 쓴 문항은 건너뛰므로(q_ko_llm_src) 중간에 멈춰도 이어서 돌릴 수 있다.
"""

import argparse
import datetime
import glob
import json
import os
import shutil
import subprocess
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "measurements", "rewrite.json")

SYSTEM = """You are the query-rewriting step of a coding agent. A developer asked a question in Korean about an open-source repository; before searching the code, you rewrite it as an English code-search query.

Rules:
1. Write a short English query (one line) that a coding agent would use to search this repository for the code to read or change: say what the code does or what should change, in the words developers use in English code and commit messages.
2. Keep every code identifier, option, flag, header, function and type name from the Korean exactly as written.
3. Do not add information that is not in the Korean question (no guessed file names, function names or causes). Work only from the Korean text and the repository name.
4. Reply with the query only; no explanations.
"""


def save(path, obj):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(json.dumps(obj, ensure_ascii=False, indent=2) + "\n")
    os.replace(tmp, path)


def call(claude, sysfile, cwd, model, proj, ko):
    for attempt in range(3):
        t = time.time()
        try:
            out = subprocess.run(
                [claude, "-p", "--model", model, "--system-prompt-file", sysfile, "--tools", "", "--setting-sources", "", "--no-session-persistence", "--output-format", "json"],
                input=f"Repository: {proj}\nQuestion: {ko}\n", capture_output=True, text=True, encoding="utf-8", cwd=cwd, timeout=180, check=True,
            )
            r = json.loads(out.stdout)
            text = " ".join((r.get("result") or "").split())
            if text and not r.get("is_error"):
                return r, text, round(time.time() - t, 2)
        except (subprocess.SubprocessError, json.JSONDecodeError) as e:
            print(f"  다시 시도 ({attempt + 1}): {type(e).__name__}")
        time.sleep(10)
    raise SystemExit(f"{proj}: 세 번 모두 실패: {ko}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--model", default="claude-opus-5-5")
    ap.add_argument("--force", action="store_true", help="이미 다시 쓴 문항도 다시 부름")
    a = ap.parse_args()
    claude = shutil.which("claude")
    work = tempfile.mkdtemp(prefix="lantern-rewrite-")
    sysfile = os.path.join(work, "system.txt")
    with open(sysfile, "w", encoding="utf-8") as fh:
        fh.write(SYSTEM)
    empty = os.path.join(work, "empty")
    os.makedirs(empty)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    rows = []
    if os.path.exists(OUT):
        with open(OUT, encoding="utf-8") as fh:
            rows = json.load(fh)["calls"]
    total = 0
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        proj = s["project"]
        todo = [q for q in s["questions"] if q.get("q_ko") and (a.force or q.get("q_ko_llm_src") != q["q_ko"] or s.get("llm_model") != a.model)]
        for q in todo:
            r, text, wall = call(claude, sysfile, empty, a.model, proj, q["q_ko"])
            u = r["usage"]
            rows = [x for x in rows if (x["project"], x["commit"]) != (proj, q["commit"])]
            rows.append({
                "project": proj, "commit": q["commit"], "ko": q["q_ko"], "rewrite": text, "api_ms": r["duration_api_ms"], "wall_s": wall,
                "in": u["input_tokens"] + u.get("cache_creation_input_tokens", 0) + u.get("cache_read_input_tokens", 0), "out": u["output_tokens"],
                "usd": r["total_cost_usd"], "at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
            })
            q["q_ko_llm"] = text
            q["q_ko_llm_src"] = q["q_ko"]
            s["llm_model"] = a.model
            s["llm_rewrite"] = (f"{a.model}, one Claude Code call per question (claude -p, no tools) given only the repository name and the Korean question; "
                                "prompt 3 in eval/bilingual/prompts.md, calls in eval/bilingual/measurements/rewrite.json")
            save(f, s)
            save(OUT, {"model": a.model, "system": SYSTEM, "calls": rows})
            total += 1
            print(f"{proj} {q['commit'][:8]}: {text}", flush=True)
        print(f"{proj}: {len(todo)}개 다시 씀 (건너뜀 {len(s['questions']) - len(todo)})", flush=True)
    print(f"모두 {total}번 호출")


if __name__ == "__main__":
    main()
