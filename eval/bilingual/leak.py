"""LLM 재작성 질문(q_ko_llm)의 누출 점검.

    python eval/bilingual/leak.py [--dir <클론 폴더>] [--subset unseen|all]

- 새 식별자: 재작성에 있는데 한국어 질문에 없는 코드 식별자 모양의 낱말 (밑줄, camelCase, 점·괄호가 붙은 이름)
- 정답 파일 이름: 정답 파일 이름(확장자 뺌)이 한국어 질문에는 없고 재작성에만 나온 문항
- 원문과의 거리: 영어 원문(커밋 제목)과 낱말이 얼마나 겹치는지 (Jaccard), 기계 번역(q_ko_mt)과 비교
--dir를 주면 누출 의심 문항을 뺀 나머지에서 BM25+임베딩 RRF의 Acc@3도 다시 계산한다.
"""

import argparse
import glob
import json
import os
import re

from baselines import HERE, terms

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+|[A-Za-z0-9]*[a-z][A-Z][A-Za-z0-9]*|[A-Za-z0-9]+_[A-Za-z0-9_]+|[A-Za-z_][A-Za-z0-9_]*\(\)")


def idents(text):
    return {m.group(0).rstrip("()").lower() for m in IDENT.finditer(text or "")}


def stem(path):
    return os.path.splitext(os.path.basename(path))[0].lstrip("-").lower()


def jaccard(a, b):
    a, b = set(terms(a or "")), set(terms(b or ""))
    return len(a & b) / len(a | b) if a | b else 0.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default=None)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--subset", default="unseen", choices=["unseen", "all"])
    a = ap.parse_args()
    seen = set()
    if a.subset == "unseen":
        for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
            with open(f, encoding="utf-8") as fh:
                seen |= {q["commit"] for q in json.load(fh)["questions"]}
    rows = []
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        for q in s["questions"]:
            if q["commit"] in seen or not q.get("q_ko_llm"):
                continue
            ko, llm = q["q_ko"], q["q_ko_llm"]
            ko_l = ko.lower()
            new_ids = sorted(i for i in idents(llm) if i not in ko_l)
            gold_leak = sorted(stem(g) for g in q["golden"] if re.search(rf"\b{re.escape(stem(g))}\b", llm.lower()) and stem(g) not in ko_l)
            rows.append({"project": s["project"], "commit": q["commit"], "ko": ko, "llm": llm, "new_ids": new_ids, "gold_leak": gold_leak,
                         "j_llm": jaccard(llm, q["q"]), "j_mt": jaccard(q.get("q_ko_mt"), q["q"]), "same": llm.strip().lower().rstrip(".") == q["q"].strip().lower().rstrip(".")})
    n = len(rows)
    ids = [r for r in rows if r["new_ids"]]
    gold = [r for r in rows if r["gold_leak"]]
    print(f"문항 {n}개 ({a.subset})")
    print(f"- 한국어 질문에 없던 식별자가 재작성에 들어간 문항: {len(ids)} ({len(ids) / n * 100:.1f}%)")
    for r in ids:
        print(f"    {r['project']}: {r['ko']} → {r['llm']}  [{', '.join(r['new_ids'])}]")
    print(f"- 정답 파일 이름이 재작성에만 나온 문항: {len(gold)} ({len(gold) / n * 100:.1f}%)")
    for r in gold:
        print(f"    {r['project']}: {r['ko']} → {r['llm']}  [{', '.join(r['gold_leak'])}]")
    print(f"- 영어 원문과 낱말 겹침(Jaccard) 평균: LLM 재작성 {sum(r['j_llm'] for r in rows) / n:.3f}, 기계 번역 {sum(r['j_mt'] for r in rows) / n:.3f}")
    print(f"- 영어 원문과 (대소문자·마침표 빼고) 똑같은 재작성: {sum(r['same'] for r in rows)}")
    if a.dir:
        suspect = {r["commit"] for r in ids + gold}
        same = {r["commit"] for r in rows if r["same"]}

        def hybrid(lang):
            out = {}
            for f in glob.glob(os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)), "baselines", f"*-{lang}.json")):
                with open(f, encoding="utf-8") as fh:
                    b = json.load(fh)
                if b["lang"] != lang:
                    continue
                for row in b["rows"]:
                    out[row["commit"]] = any(g in row["methods"]["hybrid"]["files"][:3] for g in row["golden"])
            return out

        llm, en, ko = hybrid("ko-llm"), hybrid("en"), hybrid("ko")
        acc = lambda res, cs: f"{sum(res[c] for c in cs) / len(cs) * 100:.1f}"  # noqa: E731
        allc = [r["commit"] for r in rows if r["commit"] in llm]
        keep = [c for c in allc if c not in suspect]
        rest = [c for c in allc if c not in same]
        print(f"- BM25+임베딩 RRF Acc@3, 한국어→LLM: 전체 {acc(llm, allc)} ({len(allc)}문항), 누출 의심 뺀 나머지 {acc(llm, keep)} ({len(keep)}문항)")
        print(f"- 원문과 똑같은 재작성을 뺀 {len(rest)}문항에서 BM25+임베딩 RRF Acc@3: 영어 {acc(en, rest)}, 한국어 {acc(ko, rest)}, 한국어→LLM {acc(llm, rest)}")


if __name__ == "__main__":
    main()
