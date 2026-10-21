"""Lantern과 비교 대상의 결과를 같은 지표로 묶는다.

    python eval/bilingual/report.py --dir <클론 폴더> [--set eval/bilingual] [--md <결과 파일>] [--subset all|unseen|test|dev|2026]
        [--tex <논문용 LaTeX 표 파일>]

읽는 것
- <dir>/results/retrieval-<저장소>[-ko]-<시각>.json  (eval/retrieval.mjs: 키워드, 키워드 조각, Lantern) 가장 최근 것
- <dir>/results/baselines/<저장소>-<언어>.json       (baselines.py)
- <dir>/results/lantern-<이름>/retrieval-….json      (Lantern 변형, 예: 의미 검색 켬 → 방법 이름 lantern-<이름>)

지표 (문항마다 계산해 평균)
- Recall@B: 정답 파일 중 예산 안에 담긴 비율
- Acc@k: 담긴 파일 중 앞 k개 안에 정답 파일이 하나라도 있는가
- MRR: 처음 나온 정답 파일 순위의 역수 (예산 안에 없으면 0)
- R@10f: 앞 10개 파일 안에 든 정답 파일 비율 (파일을 많이 담을수록 유리한 효과를 뺀 적중률)
세트 나누기: 커밋 해시 앞 8자리를 정수로 본 값의 10으로 나눈 나머지가 0~2면 dev(약 30%), 나머지는 test.
unseen: 예전 60문항 세트(eval/bench)와 같은 커밋을 뺀 문항. 그 세트는 엔진의 순위·사전을 고칠 때 확인에 썼으므로
개발 중에 보지 않은 문항만으로 보려면 이것을 쓴다 (주 결과).
신뢰구간: 문항 단위 paired bootstrap 10,000회 95%. 이진 지표(Acc@3) 차이는 McNemar 정확 검정.
"""

import argparse
import glob
import json
import math
import os
import random
import re

HERE = os.path.dirname(os.path.abspath(__file__))
LABEL = {
    "grep": "키워드 (파일)",
    "snippets": "키워드 (조각)",
    "bm25-file": "BM25 (파일)",
    "bm25-chunk": "BM25 (조각)",
    "dense": "임베딩 (조각)",
    "hybrid": "BM25+임베딩 RRF",
    "repomap": "Aider RepoMap",
    "lantern": "**Lantern**",
    "lantern-no-dict": "Lantern − 한국어 용어 확장",
    "lantern-no-name": "Lantern − 식별자 가산",
    "lantern-no-graph": "Lantern − 호출 그래프",
    "lantern-no-cochange": "Lantern − 함께 바뀐 파일",
    "lantern-sem": "**Lantern + 임베딩 (e5-small)**",
}
LANGS = {"en": "영어", "ko": "한국어", "ko-mt": "한국어→기계 번역"}
# 논문 표에 쓰는 영어 이름
TEX_LABEL = {
    "grep": "Keyword (file)",
    "snippets": "Keyword (snippet)",
    "bm25-file": "BM25 (file)",
    "bm25-chunk": "BM25 (chunk)",
    "dense": "Dense (mE5-small)",
    "hybrid": "BM25 + Dense (RRF)",
    "repomap": "Aider RepoMap",
    "lantern": "\\textbf{Lantern}",
    "lantern-no-dict": "\\quad $-$ Korean term expansion",
    "lantern-no-name": "\\quad $-$ identifier match",
    "lantern-no-graph": "\\quad $-$ call graph",
    "lantern-no-cochange": "\\quad $-$ co-change",
    "lantern-sem": "\\textbf{Lantern + Dense}",
}
SUBSET_TEX = {"all": "all questions", "unseen": "questions not used during development", "test": "test split", "dev": "dev split", "2026": "2026 commits"}
TEX_LANG = {"en": "English", "ko": "Korean", "ko-mt": "Korean$\\to$MT"}


def tex_table(res, keep, order, subset):
    """언어마다 Recall@8k·Acc@3·MRR 세 칸. 각 열의 가장 큰 값은 굵게"""
    langs = [l for l in TEX_LANG if any(k[0] == l for k in res)]
    vals = {}
    for lang in langs:
        for m in order:
            rows = [v for c, v in res.get((lang, m), {}).items() if keep(c)]
            if rows:
                vals[(lang, m)] = (mean([x["recall"] for x in rows]) * 100, mean([x["acc3"] for x in rows]) * 100, mean([x["mrr"] for x in rows]))
    best = {(l, i): max(v[i] for (l2, _), v in vals.items() if l2 == l) for l in langs for i in range(3)}
    n = max(len([c for c in res.get(("en", "lantern"), {}) if keep(c)]), 1)
    env = "table*" if len(langs) > 1 else "table"  # 언어가 여럿이면 두 단에 걸침
    lines = [
        f"% report.py --subset {subset} 로 만든 표. 손으로 고치지 말고 다시 만든다",
        f"\\begin{{{env}}}[t]",
        "\\centering",
        f"\\caption{{File-level context retrieval within an 8k-token budget ({SUBSET_TEX[subset]}, $n={n}$). Indented rows remove one component from Lantern.}}",
        "\\label{tab:results}",
        "\\setlength{\\tabcolsep}{3pt}",
        "\\begin{tabular}{l" + "rrr" * len(langs) + "}",
        "\\toprule",
        " & " + " & ".join(f"\\multicolumn{{3}}{{c}}{{{TEX_LANG[l]}}}" for l in langs) + " \\\\",
        " ".join(f"\\cmidrule(lr){{{2 + 3 * i}-{4 + 3 * i}}}" for i in range(len(langs))),
        "Method & " + " & ".join("R@8k & A@3 & MRR" for _ in langs) + " \\\\",
        "\\midrule",
    ]
    for m in order:
        cells = []
        for l in langs:
            v = vals.get((l, m))
            if not v:
                cells += ["--"] * 3
                continue
            for i, x in enumerate(v):
                t = f"{x:.1f}" if i < 2 else f".{round(x * 1000):03d}"
                cells.append(f"\\textbf{{{t}}}" if abs(x - best[(l, i)]) < 1e-9 else t)
        lines.append(f"{TEX_LABEL[m]} & " + " & ".join(cells) + " \\\\")
    lines += ["\\bottomrule", "\\end{tabular}", f"\\end{{{env}}}", ""]
    return "\n".join(lines)


def split_of(commit):
    return "dev" if int(commit[:8], 16) % 10 < 3 else "test"


def score(files, golden):
    hit = [g for g in golden if g in files]
    first = next((i for i, p in enumerate(files) if p in golden), None)
    return {
        "recall": len(hit) / len(golden),
        "r10": sum(1 for g in golden if g in files[:10]) / len(golden),
        "acc1": first is not None and first < 1,
        "acc3": first is not None and first < 3,
        "acc5": first is not None and first < 5,
        "mrr": 0.0 if first is None else 1 / (first + 1),
        "files": len(files),
    }


def load(dir_, set_dir):
    """{(lang, method): {commit: score}} 와 문항 정보"""
    questions = {}
    for f in sorted(glob.glob(os.path.join(set_dir, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        for q in s["questions"]:
            questions[q["commit"]] = {**q, "project": s["project"]}
    res = {}
    rdir = os.path.join(dir_, "results", os.path.basename(os.path.normpath(set_dir)))
    variants = [(rdir, ("grep", "snippets", "lantern"), None)]
    if os.path.isdir(rdir):
        variants += [(os.path.join(rdir, d), ("lantern",), d) for d in sorted(os.listdir(rdir)) if d.startswith("lantern-")]
    # 정렬해서 돈다: 집합 순서(PYTHONHASHSEED)에 따라 문항 순서가 바뀌면 부트스트랩 구간도 바뀐다
    for proj in sorted({q["project"] for q in questions.values()}):
        for vdir, kinds, rename in variants:
            for lang, suffix in (("en", ""), ("ko", "-ko"), ("ko-mt", "-ko-mt")):
                pat = re.compile(rf"^retrieval-{re.escape(proj)}{suffix}-\d{{4}}-.*\.json$")
                cands = sorted(x for x in os.listdir(vdir) if pat.match(x)) if os.path.isdir(vdir) else []
                if not cands:
                    continue
                with open(os.path.join(vdir, cands[-1]), encoding="utf-8") as fh:
                    r = json.load(fh)
                for row in r["rows"]:
                    if row["commit"] not in questions:
                        continue
                    for m in kinds:
                        res.setdefault((lang, rename or m), {})[row["commit"]] = score(row[m]["files"], row["golden"])
        for path in glob.glob(os.path.join(rdir, "baselines", f"{proj}-*.json")):
            with open(path, encoding="utf-8") as fh:
                b = json.load(fh)
            for row in b["rows"]:
                if row["commit"] not in questions:
                    continue
                for m, v in row["methods"].items():
                    res.setdefault((b["lang"], m), {})[row["commit"]] = score(v["files"], row["golden"])
    return questions, res


def mean(xs):
    return sum(xs) / len(xs) if xs else float("nan")


def bootstrap_diff(a, b, n=10_000, seed=7):
    """짝지은 두 값 목록의 평균 차이(a-b)와 95% 구간"""
    rnd = random.Random(seed)
    d = [x - y for x, y in zip(a, b)]
    k = len(d)
    stats = sorted(sum(d[rnd.randrange(k)] for _ in range(k)) / k for _ in range(n))
    return mean(d), stats[int(0.025 * n)], stats[int(0.975 * n) - 1]


def mcnemar(a, b):
    """이진 결과 짝의 McNemar 정확 검정 p값 (양측)"""
    b01 = sum(1 for x, y in zip(a, b) if x and not y)
    b10 = sum(1 for x, y in zip(a, b) if y and not x)
    n = b01 + b10
    if n == 0:
        return 1.0
    k = min(b01, b10)
    p = sum(math.comb(n, i) for i in range(k + 1)) / 2**n
    return min(1.0, 2 * p)


def pct(x):
    return f"{x * 100:.1f}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--subset", default="all", choices=["all", "unseen", "test", "dev", "2026"])
    ap.add_argument("--md", default=None)
    ap.add_argument("--tex", default=None)
    a = ap.parse_args()
    questions, res = load(a.dir, a.set)
    seen = set()
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            seen |= {q["commit"] for q in json.load(fh)["questions"]}

    def keep(c):
        q = questions[c]
        if a.subset == "unseen":
            return c not in seen
        if a.subset in ("test", "dev"):
            return split_of(c) == a.subset
        if a.subset == "2026":
            return (q.get("date") or "").startswith("2026")
        return True

    out = [f"# 영·한 병렬 벤치마크 비교 ({a.subset})", ""]
    order = [m for m in LABEL if any(k[1] == m for k in res)]
    for lang in [l for l in LANGS if any(k[0] == l for k in res)]:
        out += [f"## {LANGS[lang]}", "", "| 방법 | 문항 | Recall@8k | R@10f | Acc@1 | Acc@3 | Acc@5 | MRR | 파일 수 |", "|---|---|---|---|---|---|---|---|---|"]
        for m in order:
            r = res.get((lang, m))
            if not r:
                continue
            rows = [v for c, v in r.items() if keep(c)]
            out.append(
                f"| {LABEL[m]} | {len(rows)} | {pct(mean([x['recall'] for x in rows]))} | {pct(mean([x['r10'] for x in rows]))} | {pct(mean([x['acc1'] for x in rows]))} | "
                f"{pct(mean([x['acc3'] for x in rows]))} | {pct(mean([x['acc5'] for x in rows]))} | {mean([x['mrr'] for x in rows]):.3f} | {mean([x['files'] for x in rows]):.1f} |"
            )
        # Lantern과 각 방법의 차이 (같은 문항끼리)
        lan = res.get((lang, "lantern"), {})
        if lan:
            out += ["", "Lantern − 방법 (같은 문항, 95% 구간)", "", "| 방법 | Recall@8k 차이 | Acc@3 차이 | Acc@3 McNemar p |", "|---|---|---|---|"]
            for m in order:
                if m == "lantern" or (lang, m) not in res:
                    continue
                cs = [c for c in lan if c in res[(lang, m)] and keep(c)]
                if not cs:
                    continue
                rd = bootstrap_diff([lan[c]["recall"] for c in cs], [res[(lang, m)][c]["recall"] for c in cs])
                ad = bootstrap_diff([float(lan[c]["acc3"]) for c in cs], [float(res[(lang, m)][c]["acc3"]) for c in cs])
                p = mcnemar([lan[c]["acc3"] for c in cs], [res[(lang, m)][c]["acc3"] for c in cs])
                out.append(f"| {LABEL[m]} | {pct(rd[0])} ({pct(rd[1])}~{pct(rd[2])}) | {pct(ad[0])} ({pct(ad[1])}~{pct(ad[2])}) | {p:.3g} |")
        out.append("")
    # 영어 → 한국어 하락 (같은 문항)
    if any(k[0] == "ko" for k in res):
        out += ["## 영어 → 한국어 하락 (같은 문항, 95% 구간)", "", "| 방법 | 문항 | Recall@8k | Acc@3 |", "|---|---|---|---|"]
        for m in order:
            en, ko = res.get(("en", m), {}), res.get(("ko", m), {})
            cs = [c for c in en if c in ko and keep(c)]
            if not cs:
                continue
            rd = bootstrap_diff([ko[c]["recall"] for c in cs], [en[c]["recall"] for c in cs])
            ad = bootstrap_diff([float(ko[c]["acc3"]) for c in cs], [float(en[c]["acc3"]) for c in cs])
            out.append(f"| {LABEL[m]} | {len(cs)} | {pct(rd[0])} ({pct(rd[1])}~{pct(rd[2])}) | {pct(ad[0])} ({pct(ad[1])}~{pct(ad[2])}) |")
        out.append("")
    text = "\n".join(out)
    print(text)
    if a.tex:
        os.makedirs(os.path.dirname(os.path.abspath(a.tex)), exist_ok=True)
        with open(a.tex, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(tex_table(res, keep, order, a.subset))
    if a.md:
        with open(a.md, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text + "\n")


if __name__ == "__main__":
    main()
