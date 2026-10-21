"""Lantern과 비교 대상의 결과를 같은 지표로 묶는다.

    python eval/bilingual/report.py --dir <클론 폴더> [--set eval/bilingual] [--md <결과 파일>] [--subset all|unseen|test|dev|2026]
        [--tex <논문용 LaTeX 표 파일>]

읽는 것
- <dir>/results/retrieval-<저장소>[-<언어>]-<시각>.json  (eval/retrieval.mjs: 키워드, 키워드 조각, Lantern) 가장 최근 것
- <dir>/results/baselines/<저장소>-<언어>[-<이름표>].json  (baselines.py, repomap_baseline.py)
- <dir>/results/lantern-<이름>/retrieval-….json      (Lantern 변형, 예: 의미 검색 켬 → 방법 이름 lantern-<이름>)

지표 (문항마다 계산해 평균)
- Recall@B: 정답 파일 중 예산 안에 담긴 비율
- Acc@k: 담긴 파일 중 앞 k개 안에 정답 파일이 하나라도 있는가
- MRR: 처음 나온 정답 파일 순위의 역수 (예산 안에 없으면 0)
- R@10f: 앞 10개 파일 안에 든 정답 파일 비율 (파일을 많이 담을수록 유리한 효과를 뺀 적중률)
세트 나누기: 커밋 해시 앞 8자리를 정수로 본 값의 10으로 나눈 나머지가 0~2면 dev(약 30%), 나머지는 test.
unseen: 예전 60문항 세트(eval/bench)와 같은 커밋을 뺀 문항. 그 세트는 엔진의 순위·사전을 고칠 때 확인에 썼으므로
개발 중에 보지 않은 문항만으로 보려면 이것을 쓴다 (주 결과).
질문 언어: en 영어, ko 한국어, ko-dict 한국어 + Lantern 사전이 넓힌 영어 낱말(비교 대상에도 같은 사전을 줌),
ko-mt 한국어 → 기계 번역, ko-llm 한국어 → LLM이 영어 검색어로 다시 씀.
신뢰구간: 문항 단위 paired bootstrap 10,000회 95%. 핵심 비교는 저장소 단위(클러스터) bootstrap도 함께.
이진 지표(Acc@3) 차이는 McNemar 정확 검정, 한 언어 안의 여러 비교는 Holm 보정 p도 함께.
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
    "dense": "임베딩 (조각, e5-small)",
    "hybrid": "BM25+임베딩 RRF",
    "dense-bgem3": "임베딩 (조각, bge-m3)",
    "hybrid-bgem3": "BM25+bge-m3 RRF",
    "repomap": "Aider RepoMap",
    "lantern": "**Lantern**",
    "lantern-no-dict": "Lantern − 한국어 용어 확장",
    "lantern-no-name": "Lantern − 식별자 가산",
    "lantern-no-graph": "Lantern − 호출 그래프",
    "lantern-no-cochange": "Lantern − 함께 바뀐 파일",
    "lantern-no-copies": "Lantern − 보관용 사본 강등",
    "lantern-sem": "**Lantern + 임베딩 (e5-small)**",
}
LANGS = {"en": "영어", "en-para": "영어 바꿔 말하기(LLM)", "ko": "한국어", "ko-dict": "한국어+사전", "ko-dictw": "한국어+사전(절반 가중)", "ko-mt": "한국어→기계 번역", "ko-llm": "한국어→LLM 재작성", "ko-loan": "외래어를 많이 쓴 한국어(LLM)", "ko-native": "사람이 쓴 한국어", "ko-native-b": "사람이 쓴 한국어(B)"}
# 논문 표에 쓰는 영어 이름
TEX_LABEL = {
    "bm25-file": "BM25 (file)",
    "bm25-chunk": "BM25 (chunk)",
    "dense": "Dense (mE5-small)",
    "hybrid": "BM25 + Dense (RRF)",
    "dense-bgem3": "Dense (BGE-M3)",
    "hybrid-bgem3": "BM25 + BGE-M3 (RRF)",
    "repomap": "Aider RepoMap",
    "lantern": "\\textbf{Lantern}",
    "lantern-no-dict": "\\quad $-$ Korean term expansion",
    "lantern-no-name": "\\quad $-$ identifier match",
    "lantern-no-graph": "\\quad $-$ call graph",
    "lantern-no-cochange": "\\quad $-$ co-change",
    "lantern-sem": "\\textbf{Lantern + Dense}",
}
SUBSET_TEX = {"all": "all questions", "unseen": "questions not used during development", "fresh-repos": "repositories not used during development", "dev-repos": "unseen questions from the development repositories", "test": "test split", "dev": "dev split", "2026": "2026 commits"}
# 논문 표의 사전 열은 Lantern과 같은 결합(절반 가중, ko-dictw). 그냥 덧붙인 ko-dict는 보고서(md)에만
TEX_LANG = {"en": "EN", "ko": "KO", "ko-dictw": "KO+dict", "ko-mt": "KO$\\to$MT", "ko-llm": "KO$\\to$LLM"}


def tex_table(res, keep, order, subset):
    """언어마다 Acc@3·MRR 두 칸. 각 열의 가장 큰 값은 굵게. TEX_LABEL에 없는 방법(보조 변형)은 논문 표에서 뺀다"""
    langs = [l for l in TEX_LANG if any(k[0] == l for k in res)]
    order = [m for m in order if m in TEX_LABEL]
    vals = {}
    for lang in langs:
        for m in order:
            rows = [v for c, v in res.get((lang, m), {}).items() if keep(c)]
            if rows:
                vals[(lang, m)] = (mean([x["acc3"] for x in rows]) * 100, mean([x["mrr"] for x in rows]))
    best = {(l, i): max(v[i] for (l2, _), v in vals.items() if l2 == l) for l in langs for i in range(2)}
    n = max(len([c for c in res.get(("en", "lantern"), {}) if keep(c)]), 1)
    env = "table*" if len(langs) > 1 else "table"  # 언어가 여럿이면 두 단에 걸침
    lines = [
        f"% report.py --subset {subset} 로 만든 표. 손으로 고치지 말고 다시 만든다",
        f"\\begin{{{env}}}[t]",
        "\\centering",
        f"\\caption{{File-level retrieval within an 8k-token budget, Acc@3 (\\%) and MRR ({SUBSET_TEX[subset]}, $n={n}$). "
        "Columns: the question each method gets, in English, in Korean, in Korean plus Lantern's dictionary terms "
        "(KO+dict: half weight for BM25, appended for dense; compare with Lantern's KO), translated by NLLB or rewritten by an LLM. "
        "Indented rows remove one Lantern component; --: not run. Keyword search: full report.}",
        "\\label{tab:results}",
        "\\setlength{\\tabcolsep}{3pt}",
        "\\begin{tabular}{l" + "rr" * len(langs) + "}",
        "\\toprule",
        " & " + " & ".join(f"\\multicolumn{{2}}{{c}}{{{TEX_LANG[l]}}}" for l in langs) + " \\\\",
        " ".join(f"\\cmidrule(lr){{{2 + 2 * i}-{3 + 2 * i}}}" for i in range(len(langs))),
        "Method & " + " & ".join("A@3 & MRR" for _ in langs) + " \\\\",
        "\\midrule",
    ]
    for m in order:
        cells = []
        for l in langs:
            v = vals.get((l, m))
            if not v:
                cells += ["--"] * 2
                continue
            for i, x in enumerate(v):
                t = f"{x:.1f}" if i == 0 else f".{round(x * 1000):03d}"
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
            for lang in LANGS:
                suffix = "" if lang == "en" else f"-{lang}"
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
                        if lang == "ko-dict" and m == "lantern":
                            continue  # Lantern은 같은 사전을 스스로 적용하므로 넓힌 질문을 또 주는 것은 비교가 아님 (키워드만 씀)
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


def cluster_bootstrap_diff(a, b, groups, n=10_000, seed=7):
    """저장소 단위 bootstrap: 저장소를 복원 추출하고 뽑힌 저장소의 문항을 모두 쓴다 (같은 저장소 문항끼리의 상관을 반영)"""
    rnd = random.Random(seed)
    by = {}
    for x, y, g in zip(a, b, groups):
        by.setdefault(g, []).append(x - y)
    keys = sorted(by)
    sums = [(sum(by[g]), len(by[g])) for g in keys]
    k = len(keys)
    stats = []
    for _ in range(n):
        picked = [sums[rnd.randrange(k)] for _ in range(k)]
        stats.append(sum(s for s, _ in picked) / sum(c for _, c in picked))
    stats.sort()
    return stats[int(0.025 * n)], stats[int(0.975 * n) - 1]


def holm(ps):
    """Holm-Bonferroni 보정 p값 (입력 순서대로)"""
    order = sorted(range(len(ps)), key=lambda i: ps[i])
    out = [0.0] * len(ps)
    run = 0.0
    for rank, i in enumerate(order):
        run = max(run, min(1.0, (len(ps) - rank) * ps[i]))
        out[i] = run
    return out


# 핵심 비교 (A, B): 같은 문항에서 A − B. 비평에서 물은 것: 이득이 사전에서 오는가(비교 대상에도 사전을 줌),
# LLM 재작성·더 큰 임베딩과 비교하면 어떤가, 저장소 단위로 묶어도 결론이 같은가
KEY = [
    ("en", "lantern", "en", "hybrid"),
    ("ko", "lantern", "ko", "hybrid"),
    ("ko", "lantern", "ko-dict", "hybrid"),
    ("ko", "lantern", "ko-dict", "bm25-chunk"),
    ("ko", "lantern", "ko-dictw", "hybrid"),
    ("ko", "lantern", "ko-dictw", "bm25-chunk"),
    ("ko", "lantern", "ko-mt", "hybrid"),
    ("ko", "lantern", "ko-llm", "hybrid"),
    ("ko-llm", "lantern", "ko-llm", "hybrid"),
    ("ko", "lantern-sem", "ko", "hybrid"),
    ("ko", "lantern-sem", "ko-dict", "hybrid"),
    ("ko", "lantern-sem", "ko-mt", "hybrid"),
    ("ko", "lantern-sem", "ko-llm", "hybrid"),
    ("ko", "lantern-sem", "ko-llm", "lantern-sem"),
    ("ko", "lantern", "ko", "hybrid-bgem3"),
    ("ko", "lantern", "ko-dict", "hybrid-bgem3"),
    ("ko", "lantern-sem", "ko", "hybrid-bgem3"),
    ("ko", "lantern", "ko", "lantern-no-dict"),
    ("ko-dict", "hybrid", "ko", "hybrid"),
    ("ko-dict", "bm25-chunk", "ko", "bm25-chunk"),
    ("ko-dictw", "hybrid", "ko", "hybrid"),
    ("ko-dictw", "bm25-chunk", "ko", "bm25-chunk"),
    ("ko-dictw", "bm25-chunk", "ko-dict", "bm25-chunk"),
    ("ko-llm", "lantern-sem", "en", "lantern-sem"),
    # 영어를 영어로 바꿔 말하기만 한 통제: 하락이 말 바꿈 때문인지 언어 때문인지
    ("en-para", "hybrid", "en", "hybrid"),
    ("en-para", "lantern", "en", "lantern"),
    ("en-para", "bm25-chunk", "en", "bm25-chunk"),
    ("en-para", "lantern-sem", "en", "lantern-sem"),
    ("ko", "hybrid", "en-para", "hybrid"),
    ("ko", "lantern", "en-para", "lantern"),
    # 외래어를 많이 쓴 한국어 (LLM이 만든 스트레스 테스트, 사람이 쓴 질문이 아님)
    ("ko-loan", "lantern", "ko-loan", "hybrid"),
    ("ko-loan", "lantern", "ko", "lantern"),
    ("ko-loan", "hybrid", "ko", "hybrid"),
    ("ko-loan", "bm25-chunk", "ko", "bm25-chunk"),
    ("ko-loan", "lantern-sem", "ko-loan", "hybrid"),
    ("ko-loan", "lantern-sem", "ko", "lantern-sem"),
    ("ko-llm", "hybrid", "ko-mt", "hybrid"),
]


def pct(x):
    return f"{x * 100:.1f}"


def name(lang, m):
    return f"{LABEL[m]} [{LANGS[lang]}]"


def key_comparisons(questions, res, keep):
    """핵심 비교: 문항 단위와 저장소 단위 95% 구간을 함께. 없는 결과는 건너뛴다"""
    out = [
        "## 핵심 비교 (A − B, 같은 문항)",
        "",
        "문항 단위 paired bootstrap과 저장소 단위(클러스터) bootstrap의 95% 구간. 저장소가 10개뿐이라 저장소 단위 구간이 더 넓다.",
        "",
        "| A | B | 문항 | Acc@3 차이 | 문항 단위 구간 | 저장소 단위 구간 | McNemar p | MRR 차이 | 문항 단위 구간 | 저장소 단위 구간 |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    pairs = [(la, ma, lb, mb) for la, ma, lb, mb in KEY if (la, ma) in res and (lb, mb) in res]
    # 영어 → 한국어 하락의 차이: (Lantern 하락) − (혼합 하락)
    drops = [("lantern", "hybrid"), ("lantern-sem", "hybrid"), ("lantern", "hybrid-bgem3")]
    for la, ma, lb, mb in pairs:
        A, B = res[(la, ma)], res[(lb, mb)]
        cs = [c for c in A if c in B and keep(c)]
        if not cs:
            continue
        g = [questions[c]["project"] for c in cs]
        row = [name(la, ma), name(lb, mb), str(len(cs))]
        for metric in ("acc3", "mrr"):
            a, b = [float(A[c][metric]) for c in cs], [float(B[c][metric]) for c in cs]
            d, lo, hi = bootstrap_diff(a, b)
            clo, chi = cluster_bootstrap_diff(a, b, g)
            f = pct if metric == "acc3" else (lambda x: f"{x:+.3f}")
            row += [f(d), f"{f(lo)}~{f(hi)}", f"{f(clo)}~{f(chi)}"]
            if metric == "acc3":
                row.append(f"{mcnemar([A[c]['acc3'] for c in cs], [B[c]['acc3'] for c in cs]):.3g}")
        out.append("| " + " | ".join(row) + " |")
    for ma, mb in drops:
        keys = [("en", ma), ("ko", ma), ("en", mb), ("ko", mb)]
        if not all(k in res for k in keys):
            continue
        cs = [c for c in res[keys[0]] if all(c in res[k] for k in keys) and keep(c)]
        g = [questions[c]["project"] for c in cs]
        row = [f"{LABEL[ma]} 하락 (한국어 − 영어)", f"{LABEL[mb]} 하락", str(len(cs))]
        for metric in ("acc3", "mrr"):
            v = lambda k, c: float(res[k][c][metric])  # noqa: E731
            a = [v(keys[1], c) - v(keys[0], c) for c in cs]
            b = [v(keys[3], c) - v(keys[2], c) for c in cs]
            d, lo, hi = bootstrap_diff(a, b)
            clo, chi = cluster_bootstrap_diff(a, b, g)
            f = pct if metric == "acc3" else (lambda x: f"{x:+.3f}")
            row += [f(d), f"{f(lo)}~{f(hi)}", f"{f(clo)}~{f(chi)}"]
            if metric == "acc3":
                row.append("")
        out.append("| " + " | ".join(row) + " |")
    return out + [""]


def per_repo(questions, res, keep, order):
    """저장소마다 Acc@3. 영어·한국어 각각, Lantern이 혼합 검색 이상인 저장소 수"""
    repos = sorted({q["project"] for q in questions.values()})
    shown = [m for m in ("bm25-chunk", "hybrid", "hybrid-bgem3", "lantern", "lantern-sem") if m in order]
    out = ["## 저장소별 Acc@3", ""]
    for lang in ("en", "ko"):
        if not any((lang, m) in res for m in shown):
            continue
        out += [f"### {LANGS[lang]}", "", "| 저장소 | 문항 | " + " | ".join(LABEL[m] for m in shown) + " |", "|---|---|" + "---|" * len(shown)]
        ge = 0
        for r in repos:
            cs = [c for c, q in questions.items() if q["project"] == r and keep(c)]
            cells = []
            for m in shown:
                v = [res[(lang, m)][c]["acc3"] for c in cs if c in res.get((lang, m), {})]
                cells.append(pct(mean(v)) if v else "--")
            lan = [res[(lang, "lantern")][c]["acc3"] for c in cs if c in res.get((lang, "lantern"), {})]
            hyb = [res[(lang, "hybrid")][c]["acc3"] for c in cs if c in res.get((lang, "hybrid"), {})]
            ge += bool(lan and hyb and mean(lan) >= mean(hyb))
            out.append(f"| {r} | {len(cs)} | " + " | ".join(cells) + " |")
        out += ["", f"Lantern Acc@3 ≥ BM25+임베딩 RRF: 저장소 {len(repos)}개 중 {ge}개", ""]
    return out


def subgroups(questions, res, keep, order):
    """한국어 문항을 사전이 걸렸는지, 질문에 영어 식별자가 있는지로 나눠 Acc@3"""
    from baselines import terms

    groups = {
        "사전이 걸린 문항": lambda q: bool(q.get("q_ko_dict_terms")),
        "사전이 안 걸린 문항": lambda q: q.get("q_ko_dict_terms") == [],
        "한국어 질문에 영어 낱말이 있는 문항": lambda q: any(t.isascii() for t in terms(q.get("q_ko", ""))),
        "한국어 질문에 영어 낱말이 없는 문항": lambda q: not any(t.isascii() for t in terms(q.get("q_ko", ""))),
    }
    cols = [(l, m) for l, m in [("en", "lantern"), ("en", "hybrid"), ("ko", "bm25-chunk"), ("ko", "hybrid"), ("ko-dict", "hybrid"), ("ko-dictw", "hybrid"), ("ko", "lantern"),
                                ("ko", "lantern-no-dict"), ("ko", "lantern-sem"), ("ko-llm", "hybrid")] if (l, m) in res]
    out = ["## 한국어 문항 나눠 보기 (Acc@3)", "", "| 문항 | 수 | " + " | ".join(name(l, m) for l, m in cols) + " |", "|---|---|" + "---|" * len(cols)]
    for label, pred in groups.items():
        cs = [c for c, q in questions.items() if keep(c) and q.get("q_ko") and pred(q)]
        cells = []
        for k in cols:
            v = [res[k][c]["acc3"] for c in cs if c in res[k]]
            cells.append(pct(mean(v)) if v else "--")
        out.append(f"| {label} | {len(cs)} | " + " | ".join(cells) + " |")
    return out + [""]


def sensitivity(res, keep):
    """BM25에 사전 낱말을 얼마의 가중치로 더할지 (0 = 사전 없음, 1.0 = 그냥 덧붙임)"""
    steps = [("0", "ko"), ("0.25", "ko-dictw25"), ("0.5", "ko-dictw"), ("0.75", "ko-dictw75"), ("1.0", "ko-dict")]
    if not any((lang, "bm25-chunk") in res for _, lang in steps[1:4] if lang != "ko-dictw"):
        return []
    out = ["## 사전 가중치 민감도 (BM25, 한국어+사전)", "", "| 가중치 | BM25 (파일) Acc@3 | BM25 (파일) MRR | BM25 (조각) Acc@3 | BM25 (조각) MRR |", "|---|---|---|---|---|"]
    for w, lang in steps:
        cells = []
        for m in ("bm25-file", "bm25-chunk"):
            rows = [v for c, v in res.get((lang, m), {}).items() if keep(c)]
            cells += [pct(mean([x["acc3"] for x in rows])), f"{mean([x['mrr'] for x in rows]):.3f}"] if rows else ["--", "--"]
        out.append(f"| {w} | " + " | ".join(cells) + " |")
    return out + ["", "Lantern이 쓰는 가중치는 0.5. 1.0은 사전 낱말을 질문에 그냥 덧붙인 것과 같다.", ""]


def native(questions, res, keep, order):
    """사람이 직접 쓴 한국어 질문(작성자 A: q_ko_native, B: q_ko_native_b)이 있는 문항에서 LLM이 쓴 한국어 질문과 나란히.
    비교는 미리 정해 둔 것만 (native-plan.md): 사람 질문에서 Lantern − 혼합, 사람 − LLM 한국어, 영어 → 사람 한국어 하락의 차이"""
    out = []
    for lang_h, field, who in (("ko-native", "q_ko_native", "A"), ("ko-native-b", "q_ko_native_b", "B")):
        cs = [c for c, q in questions.items() if keep(c) and q.get(field)]
        if not cs:
            continue
        out += [f"## 사람이 쓴 한국어 질문, 작성자 {who} ({len(cs)}문항, 같은 문항의 LLM 한국어 질문과 비교)", "",
                "| 방법 | 영어 Acc@3 | LLM 한국어 Acc@3 | 사람 한국어 Acc@3 | 영어 MRR | LLM 한국어 MRR | 사람 한국어 MRR |", "|---|---|---|---|---|---|---|"]
        for m in order:
            if (lang_h, m) not in res:
                continue
            cells = []
            for metric in ("acc3", "mrr"):
                for lang in ("en", "ko", lang_h):
                    v = [res[(lang, m)][c][metric] for c in cs if c in res.get((lang, m), {})]
                    cells.append(("--" if not v else pct(mean(v)) if metric == "acc3" else f"{mean(v):.3f}"))
            out.append(f"| {LABEL[m]} | " + " | ".join(cells) + " |")
        out += ["", "| 비교 (같은 문항, 95% 구간) | Acc@3 차이 | 구간 | MRR 차이 | 구간 |", "|---|---|---|---|---|"]
        pairs = [(f"Lantern − 혼합 [사람 {who}]", (lang_h, "lantern"), (lang_h, "hybrid"), None),
                 (f"Lantern [사람 {who} − LLM 한국어]", (lang_h, "lantern"), ("ko", "lantern"), None),
                 (f"혼합 [사람 {who} − LLM 한국어]", (lang_h, "hybrid"), ("ko", "hybrid"), None),
                 (f"하락의 차이: Lantern − 혼합 [사람 {who} − 영어]", (lang_h, "lantern"), ("en", "lantern"), ((lang_h, "hybrid"), ("en", "hybrid")))]
        for label, ka, kb, drop in pairs:
            keys = [ka, kb] + (list(drop) if drop else [])
            if not all(k in res for k in keys):
                continue
            cc = [c for c in cs if all(c in res[k] for k in keys)]
            row = [label]
            for metric in ("acc3", "mrr"):
                a = [float(res[ka][c][metric]) - float(res[kb][c][metric]) for c in cc]
                b = [float(res[drop[0]][c][metric]) - float(res[drop[1]][c][metric]) for c in cc] if drop else [0.0] * len(cc)
                d, lo, hi = bootstrap_diff(a, b)
                f = pct if metric == "acc3" else (lambda x: f"{x:+.3f}")
                row += [f(d), f"{f(lo)}~{f(hi)}"]
            out.append("| " + " | ".join(row) + " |")
        out.append("")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--subset", default="all", choices=["all", "unseen", "fresh-repos", "dev-repos", "test", "dev", "2026"])
    ap.add_argument("--md", default=None)
    ap.add_argument("--tex", default=None)
    a = ap.parse_args()
    questions, res = load(a.dir, a.set)
    seen = set()
    dev_repos = set()  # 개발 중 확인에 쓴 예전 세트(eval/bench)의 저장소
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            b = json.load(fh)
        seen |= {q["commit"] for q in b["questions"]}
        dev_repos.add(b["project"])

    def keep(c):
        q = questions[c]
        if a.subset == "unseen":
            return c not in seen
        if a.subset == "fresh-repos":
            # 개발 중에 한 번도 보지 않은 저장소의 문항만 (저장소 수준의 누출까지 뺀 결과)
            return q["project"] not in dev_repos
        if a.subset == "dev-repos":
            # 개발 중 확인에 쓴 세 저장소에서, 그때 쓰지 않은 문항 (과적합이 있었다면 여기서 Lantern이 더 유리해야 함)
            return q["project"] in dev_repos and c not in seen
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
            out += ["", "Lantern − 방법 (같은 문항, 95% 구간)", "", "| 방법 | Recall@8k 차이 | Acc@3 차이 | Acc@3 McNemar p | Holm 보정 p |", "|---|---|---|---|---|"]
            cmp = []
            for m in order:
                if m == "lantern" or (lang, m) not in res:
                    continue
                cs = [c for c in lan if c in res[(lang, m)] and keep(c)]
                if not cs:
                    continue
                rd = bootstrap_diff([lan[c]["recall"] for c in cs], [res[(lang, m)][c]["recall"] for c in cs])
                ad = bootstrap_diff([float(lan[c]["acc3"]) for c in cs], [float(res[(lang, m)][c]["acc3"]) for c in cs])
                p = mcnemar([lan[c]["acc3"] for c in cs], [res[(lang, m)][c]["acc3"] for c in cs])
                cmp.append((m, rd, ad, p))
            for (m, rd, ad, p), ph in zip(cmp, holm([x[3] for x in cmp])):
                out.append(f"| {LABEL[m]} | {pct(rd[0])} ({pct(rd[1])}~{pct(rd[2])}) | {pct(ad[0])} ({pct(ad[1])}~{pct(ad[2])}) | {p:.3g} | {ph:.3g} |")
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
    out += key_comparisons(questions, res, keep)
    out += per_repo(questions, res, keep, order)
    out += subgroups(questions, res, keep, order)
    out += native(questions, res, keep, order)
    out += sensitivity(res, keep)
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
