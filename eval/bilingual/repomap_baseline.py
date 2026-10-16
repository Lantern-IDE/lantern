"""Aider RepoMap 기준선. aider-chat이 깔린 파이썬으로 돌린다 (의존성이 많아 따로 가상환경 권장).

    python -m venv .venv-aider && .venv-aider/Scripts/pip install aider-chat==0.86.2
    .venv-aider/Scripts/python eval/bilingual/repomap_baseline.py --dir <클론 폴더> [--lang en|ko|ko-mt] [--budget 8000] [--only gin,jq]

Aider처럼 질문의 낱말을 '언급된 식별자'로, 질문에 나온 경로를 '언급된 파일'로 넘겨 PageRank를 기울인다.
Aider는 지도를 그릴 때 고른 태그를 파일 이름순으로 다시 정렬하므로, 순위(get_ranked_tags)의 앞에서부터
지도가 예산 안에 들어가는 만큼을 고르고 그 안의 파일을 순위 순서대로 둔다. 파일 고르는 기준은 baselines.py와 같다.
결과는 baselines.py와 같은 폴더에 <저장소>-<언어>-repomap.json으로 쓴다.
"""

import argparse
import glob
import json
import math
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from baselines import HERE, git, read_files  # noqa: E402

from aider.repomap import RepoMap  # noqa: E402
from aider.special import filter_important_files  # noqa: E402


class Model:
    def token_count(self, text):
        return math.ceil(len(text) / 4)


class IO:
    def tool_output(self, *a, **k):
        pass

    tool_warning = tool_error = tool_output

    def read_text(self, fname):
        try:
            with open(fname, encoding="utf-8", errors="replace") as fh:
                return fh.read()
        except OSError:
            return None


def rank_files(rm, root, files, query, budget):
    rels = [p for p, _ in files]
    abs_files = [os.path.join(root, *p.split("/")) for p in rels]
    idents = set(re.split(r"\W+", query)) - {""}
    mentioned = set()
    for w in query.split():
        w = w.rstrip(",.!;:?").strip("\"'`*_")
        if w in rels:
            mentioned.add(w.replace("/", os.sep))
    ranked = rm.get_ranked_tags([], abs_files, mentioned, idents)
    have = {t[0] for t in ranked}
    special = [(f,) for f in filter_important_files([p.replace("/", os.sep) for p in rels]) if f not in have]
    ranked = special + ranked
    lo, hi, best = 0, len(ranked), 0
    while lo <= hi:
        mid = (lo + hi) // 2
        if mid == 0 or rm.token_count(rm.to_tree(ranked[:mid], set())) <= budget:
            best, lo = mid, mid + 1
        else:
            hi = mid - 1
    order = []
    for tag in ranked[:best]:
        p = tag[0].replace(os.sep, "/")
        if p not in order:
            order.append(p)
    used = rm.token_count(rm.to_tree(ranked[:best], set())) if best else 0
    return order, used


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--lang", default="en", choices=["en", "ko", "ko-mt"])
    ap.add_argument("--budget", type=int, default=8000)
    ap.add_argument("--out", default=None)
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    out_dir = a.out or os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)), "baselines")
    os.makedirs(out_dir, exist_ok=True)
    field = {"en": "q", "ko": "q_ko", "ko-mt": "q_ko_mt"}[a.lang]
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        name = s["project"]
        if a.only and name not in a.only.split(","):
            continue
        qs = [q for q in s["questions"] if q.get(field)]
        if not qs:
            continue
        repo = os.path.abspath(os.path.join(a.dir, name))
        if git(repo, "status", "--porcelain", "--untracked-files=no").strip():
            sys.exit(f"{repo}에 커밋하지 않은 변경이 있습니다")
        original = git(repo, "rev-parse", "HEAD").strip()
        rm = RepoMap(map_tokens=a.budget, root=repo, main_model=Model(), io=IO(), map_mul_no_files=1)
        rows = []
        started = time.time()
        for q in qs:
            git(repo, "checkout", "-q", "--detach", q["parent"])
            files = read_files(repo)
            rm.tree_cache = {}
            order, used = rank_files(rm, repo, files, q[field], a.budget)
            rows.append({"commit": q["commit"], "q": q[field], "golden": q["golden"], "methods": {"repomap": {"files": order, "used": used}}})
            top3 = any(g in order[:3] for g in q["golden"])
            print(f"  {name} {len(rows):2}/{len(qs)} repomap:{'O' if top3 else '.'} files={len(order)}", flush=True)
        git(repo, "checkout", "-q", "--detach", original)
        path = os.path.join(out_dir, f"{name}-{a.lang}-repomap.json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump({"project": name, "lang": a.lang, "budget": a.budget, "methods": ["repomap"], "rows": rows}, fh, ensure_ascii=False, indent=1)
        print(f"{name} ({a.lang}): {len(rows)}문항 {time.time() - started:.0f}초 → {path}")


if __name__ == "__main__":
    main()
