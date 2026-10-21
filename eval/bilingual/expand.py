"""한국어 질문에 Lantern의 한국어 개발 용어 사전이 넓힌 영어 낱말을 붙여 q_ko_dict에 넣는다.
비교 대상(BM25·임베딩·혼합)에도 같은 사전을 적용해, 한국어 성능이 사전에서 오는지 Lantern의 순위에서 오는지 가른다.

    python eval/bilingual/expand.py --lantern <엔진 실행 파일> [--set eval/bilingual]

넓힌 낱말은 엔진이 실제로 찾는 낱말(lantern context --json의 query_terms) 가운데, 질문에 원래 있던
영어 식별자에서 나온 것을 뺀 영어 낱말이다. 빈 폴더를 대상으로 돌려 인덱싱 없이 얻는다.
q_ko_dict_terms에 넓힌 낱말을 남기고, 사전이 하나라도 걸린 질문 비율(사전 적중률)을 알려 준다.
"""

import argparse
import glob
import json
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from baselines import HERE, terms  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lantern", required=True)
    ap.add_argument("--set", default=HERE)
    a = ap.parse_args()
    empty = tempfile.mkdtemp(prefix="lantern-expand-")
    env = {**os.environ, "LANTERN_INDEX_DIR": tempfile.mkdtemp(prefix="lantern-expand-idx-")}
    total = hit = 0
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        for q in s["questions"]:
            if not q.get("q_ko"):
                continue
            out = subprocess.run([a.lantern, "-C", empty, "context", "--json", "--", q["q_ko"]], env=env, capture_output=True, text=True, encoding="utf-8", check=True).stdout
            asked = set(terms(q["q_ko"]))
            extra = [t for t in json.loads(out)["query_terms"] if t.isascii() and t not in asked]
            q["q_ko_dict_terms"] = extra
            q["q_ko_dict"] = (q["q_ko"] + " " + " ".join(extra)).strip()
            total += 1
            hit += bool(extra)
        with open(f, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(json.dumps(s, ensure_ascii=False, indent=2) + "\n")
    print(f"한국어 질문 {total}개 중 사전이 걸린 질문 {hit}개 ({hit / max(1, total) * 100:.1f}%)")


if __name__ == "__main__":
    main()
