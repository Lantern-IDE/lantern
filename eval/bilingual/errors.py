"""Lantern이 영어로는 맞히고(정답이 앞 3개 안) 한국어로는 놓친 문항을 뽑는다 (오류 분석용).

    python eval/bilingual/errors.py --dir <클론 폴더> [--subset unseen|all]

문항마다 사전이 넓힌 낱말, 질문에 있던 영어 낱말, 한국어로 물었을 때 정답 파일의 순위(예산 밖이면 -)를 표로 쓴다.
원인 분류는 사람이 읽고 붙인다 (error-analysis.md).
"""

import argparse
import glob
import json
import os

from baselines import HERE, terms


def latest(rdir, proj, suffix):
    fs = sorted(glob.glob(os.path.join(rdir, f"retrieval-{proj}{suffix}-2*.json")))
    with open(fs[-1], encoding="utf-8") as fh:
        return {r["commit"]: r for r in json.load(fh)["rows"]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--subset", default="unseen", choices=["unseen", "all"])
    a = ap.parse_args()
    seen = set()
    if a.subset == "unseen":
        for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
            with open(f, encoding="utf-8") as fh:
                seen |= {q["commit"] for q in json.load(fh)["questions"]}
    rdir = os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)))
    lost = gained = 0
    rows = ["| 저장소 | 영어 질문 | 한국어 질문 | 사전이 넓힌 낱말 | 질문의 영어 낱말 | 한국어 순위 |", "|---|---|---|---|---|---|"]
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        en, ko = latest(rdir, s["project"], ""), latest(rdir, s["project"], "-ko")
        for q in s["questions"]:
            c = q["commit"]
            if c in seen or c not in en or c not in ko:
                continue
            hit = lambda r: any(g in r[c]["lantern"]["files"][:3] for g in q["golden"])  # noqa: E731
            gained += hit(ko) and not hit(en)
            if hit(en) and not hit(ko):
                lost += 1
                files = ko[c]["lantern"]["files"]
                rank = next((str(i + 1) for i, p in enumerate(files) if p in q["golden"]), "-")
                ascii_ = [t for t in terms(q["q_ko"]) if t.isascii()]
                rows.append(f"| {s['project']} | {q['q']} | {q['q_ko']} | {', '.join(q.get('q_ko_dict_terms') or []) or '-'} | {', '.join(ascii_) or '-'} | {rank} |")
    print(f"영어로만 맞힘 {lost}문항, 한국어로만 맞힘 {gained}문항\n")
    print("\n".join(rows))


if __name__ == "__main__":
    main()
