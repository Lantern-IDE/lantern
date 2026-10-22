"""한국어 질문에 Lantern의 한국어 개발 용어 사전이 넓힌 영어 낱말을 붙여 q_ko_dict에 넣는다.
비교 대상(BM25·임베딩·혼합)에도 같은 사전을 적용해, 한국어 성능이 사전에서 오는지 Lantern의 순위에서 오는지 가른다.

    python eval/bilingual/expand.py --lantern <엔진 실행 파일> [--set eval/bilingual]

넓힌 낱말은 엔진이 실제로 찾는 낱말(lantern context --json의 query_terms) 가운데, 질문에 원래 있던
영어 식별자에서 나온 것을 뺀 영어 낱말이다. 빈 폴더를 대상으로 돌려 인덱싱 없이 얻는다.
q_ko_dict_terms에 넓힌 낱말을 남기고, 사전이 하나라도 걸린 질문 비율(사전 적중률)을 알려 준다.
외래어 스트레스 테스트(q_ko_loan)에도 같은 방법으로 q_ko_loan_dict_terms를 남긴다 (Lantern은 이 형태를 그대로 받으므로 적중률 보고용).
NLLB가 영어 제목을 옮긴 한국어(q_ko_nllb)에는 q_ko_nllb_dict_terms와 q_ko_nllb_dict를 남긴다 (비교 대상에 같은 사전을 주는 ko-nllb-dictw용).
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
    a.lantern = os.path.abspath(a.lantern)
    empty = tempfile.mkdtemp(prefix="lantern-expand-")
    env = {**os.environ, "LANTERN_INDEX_DIR": tempfile.mkdtemp(prefix="lantern-expand-idx-")}
    seen = set()  # 개발 중 확인에 쓴 예전 세트(eval/bench). 주 결과는 이 문항을 뺀 것
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            seen |= {q["commit"] for q in json.load(fh)["questions"]}

    def dict_terms(text):
        out = subprocess.run([a.lantern, "-C", empty, "context", "--json", "--", text], env=env, capture_output=True, text=True, encoding="utf-8", check=True).stdout
        asked = set(terms(text))
        return [t for t in json.loads(out)["query_terms"] if t.isascii() and t not in asked]

    count = {}  # (형태, 문항 묶음) → [질문 수, 사전이 걸린 질문 수]
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        for q in s["questions"]:
            for field in ("q_ko", "q_ko_loan", "q_ko_nllb"):
                if not q.get(field):
                    continue
                extra = dict_terms(q[field])
                q[f"{field}_dict_terms"] = extra
                if field != "q_ko_loan":
                    # 사전 낱말을 덧붙인 질문 (임베딩에 줄 때). q_ko_dict, q_ko_nllb_dict
                    q[f"{field}_dict"] = (q[field] + " " + " ".join(extra)).strip()
                for part in ("all", "unseen"):
                    if part == "unseen" and q["commit"] in seen:
                        continue
                    c = count.setdefault((field, part), [0, 0])
                    c[0] += 1
                    c[1] += bool(extra)
        with open(f, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(json.dumps(s, ensure_ascii=False, indent=2) + "\n")
    for (field, part), (total, hit) in sorted(count.items()):
        print(f"{field} ({part}): {total}개 중 사전이 걸린 질문 {hit}개 ({hit / max(1, total) * 100:.1f}%)")


if __name__ == "__main__":
    main()
