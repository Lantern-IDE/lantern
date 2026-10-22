"""Lantern과 비교 대상의 결과를 같은 지표로 묶는다. 보고서(md)와 논문 표(LaTeX)는 논문이 인용하므로 영어로 낸다.

    python eval/bilingual/report.py --dir <클론 폴더> [--set eval/bilingual] [--md <결과 파일>]
        [--subset all|unseen|fresh-repos|dev-repos|test|dev|2026] [--tex <논문용 LaTeX 표 파일>]

읽는 것
- <dir>/results/retrieval-<저장소>[-<언어>]-<시각>.json  (eval/retrieval.mjs: 키워드, 키워드 조각, Lantern) 가장 최근 것
- <dir>/results/baselines/<저장소>-<언어>[-<이름표>].json  (baselines.py, repomap_baseline.py)
- <dir>/results/lantern-<이름>/retrieval-….json      (Lantern 변형, 예: 의미 검색 켬 → 방법 이름 lantern-<이름>)

지표 (문항마다 계산해 평균)
- Recall@B: 정답 파일 중 예산 안에 담긴 비율
- A@k: 담긴 파일 중 앞 k개 안에 정답 파일이 하나라도 있는가
- MRR: 처음 나온 정답 파일 순위의 역수 (예산 안에 없으면 0)
- R@10f: 앞 10개 파일 안에 든 정답 파일 비율 (파일을 많이 담을수록 유리한 효과를 뺀 적중률)
세트 나누기: 커밋 해시 앞 8자리를 정수로 본 값의 10으로 나눈 나머지가 0~2면 dev(약 30%), 나머지는 test (참고용).
unseen: 예전 60문항 세트(eval/bench)와 같은 커밋을 뺀 문항. 그 세트는 엔진의 순위·사전을 고칠 때 확인에 썼으므로
개발 중에 보지 않은 문항만으로 보려면 이것을 쓴다 (주 결과).
질문 언어: en 영어, ko 한국어, ko-dict 한국어 + Lantern 사전이 넓힌 영어 낱말(비교 대상에도 같은 사전을 줌),
ko-mt 한국어 → 기계 번역, ko-llm 한국어 → LLM이 영어 검색어로 다시 씀.
신뢰구간: 문항 단위 paired bootstrap 10,000회 95%. 핵심 비교는 저장소 단위(클러스터) bootstrap도 함께.
이진 지표(A@3) 차이는 McNemar 정확 검정, 한 언어 안의 여러 비교는 Holm 보정 p도 함께.
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
    "grep": "Keyword (whole files)",
    "snippets": "Keyword (snippets)",
    "bm25-file": "BM25 (file)",
    "bm25-chunk": "BM25 (chunk)",
    "dense": "Dense (chunk, mE5-small)",
    "hybrid": "BM25 + Dense (RRF)",
    "dense-bgem3": "Dense (chunk, BGE-M3)",
    "hybrid-bgem3": "BM25 + BGE-M3 (RRF)",
    "repomap": "Aider RepoMap",
    "lantern": "**Lantern**",
    "lantern-no-dict": "Lantern − Korean term expansion",
    "lantern-no-name": "Lantern − identifier match",
    "lantern-no-graph": "Lantern − call graph",
    "lantern-no-cochange": "Lantern − co-change",
    "lantern-no-copies": "Lantern − archived-copy demotion",
    "lantern-sem": "**Lantern + Dense (mE5-small)**",
}
# 질문 형태의 짧은 이름 (표 안)과 절 제목
LANGS = {"en": "EN", "en-para": "EN-para", "ko": "KO", "ko-dict": "KO+dict appended", "ko-dictw": "KO+dict", "ko-mt": "KO→MT", "ko-llm": "KO→LLM",
         "ko-llm2": "KO→LLM run 2", "ko-llm3": "KO→LLM run 3", "ko-llm-norepo": "KO→LLM no repo",
         "ko-loan": "KO-loan", "ko-native": "KO-native A", "ko-native-b": "KO-native B"}
LANG_TITLE = {
    "en": "English questions (EN: commit subjects)",
    "en-para": "English questions reworded in English by an LLM (EN-para: control for rewording)",
    "ko": "Korean questions (KO)",
    "ko-dict": "Korean + Lantern's dictionary terms, appended to the question (KO+dict appended)",
    "ko-dictw": "Korean + Lantern's dictionary terms (KO+dict as in Table I: half weight for BM25 as in Lantern, appended for dense)",
    "ko-mt": "Korean translated into English by NLLB-200 (KO→MT)",
    "ko-llm": "Korean rewritten into an English search query by an LLM, one call per question (KO→LLM)",
    "ko-llm2": "KO→LLM, second run with the same setting (KO→LLM run 2)",
    "ko-llm3": "KO→LLM, third run with the same setting (KO→LLM run 3)",
    "ko-llm-norepo": "KO→LLM without the repository name (KO→LLM no repo: control for the model's knowledge of the repository)",
    "ko-loan": "Korean rewritten with many Hangul loanwords by an LLM (KO-loan: stress test, not native questions)",
    "ko-native": "Korean written by developer A (KO-native A)",
    "ko-native-b": "Korean written by developer B (KO-native B)",
}
# 논문 표에 쓰는 이름
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
    """언어마다 A@3·MRR 두 칸. 각 열의 가장 큰 값은 굵게. TEX_LABEL에 없는 방법(보조 변형)은 논문 표에서 뺀다"""
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
        # IEEEtran 표 캡션은 작은 대문자라 대소문자를 가리는 이름은 \textup으로 감싼다
        f"\\caption{{File-level retrieval within an 8k-token budget: top-3 accuracy (A@3, \\%) and MRR ({SUBSET_TEX[subset]}, $n={n}$). "
        "Columns: the question each method gets, in English, in Korean, in Korean plus Lantern's dictionary terms "
        "(\\textup{KO+dict}: half weight for BM25, appended for dense; compare with Lantern's KO), translated by NLLB or rewritten by an LLM. "
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
        for path in sorted(glob.glob(os.path.join(rdir, "baselines", f"{proj}-*.json"))):
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
    ("en", "lantern-sem", "en", "hybrid"),
    ("ko", "lantern", "ko", "hybrid"),
    ("ko", "lantern", "ko-dict", "hybrid"),
    ("ko", "lantern", "ko-dict", "bm25-chunk"),
    ("ko", "lantern", "ko-dictw", "hybrid"),
    ("ko", "lantern", "ko-dictw", "bm25-chunk"),
    ("ko", "lantern", "ko-mt", "hybrid"),
    ("ko", "lantern", "ko-llm", "hybrid"),
    ("ko-llm", "lantern", "ko-llm", "hybrid"),
    ("ko-llm", "lantern", "ko", "lantern"),
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
    # 재작성에 저장소 이름을 주지 않은 통제: 저장소 지식의 몫
    ("ko-llm", "hybrid", "ko-llm-norepo", "hybrid"),
    ("ko-llm", "lantern-sem", "ko-llm-norepo", "lantern-sem"),
    ("ko-llm-norepo", "hybrid", "en", "hybrid"),
    ("ko-llm-norepo", "hybrid", "ko", "hybrid"),
    ("ko", "lantern", "ko-llm-norepo", "hybrid"),
    ("ko", "lantern-sem", "ko-llm-norepo", "hybrid"),
]
REWRITE_RUNS = ["ko-llm", "ko-llm2", "ko-llm3"]
CUTOFF = "2026-07-01"  # 재작성 모델(Claude Opus 5.5)의 학습 데이터 시점 다음 날
# 영어 → 한국어 하락의 차이: (A의 하락) − (B의 하락)
DROPS = [("lantern", "hybrid"), ("lantern-no-dict", "hybrid"), ("lantern-sem", "hybrid"), ("lantern", "hybrid-bgem3")]


def pct(x):
    return f"{x * 100:.1f}"


def name(lang, m):
    return f"{LABEL[m]} [{LANGS[lang]}]"


def ci(f, lo, hi):
    return f"[{f(lo)}, {f(hi)}]"


def key_comparisons(questions, res, keep):
    """핵심 비교: 문항 단위와 저장소 단위 95% 구간을 함께. 없는 결과는 건너뛴다"""
    out = [
        "## Key comparisons (A − B on the same questions)",
        "",
        "95% intervals from a paired bootstrap over questions and from a cluster bootstrap over repositories. "
        "With only 7–10 repositories, the repository intervals are approximate and wider.",
        "",
        "| A | B | n | A@3 diff. | over questions | over repositories | McNemar p | MRR diff. | over questions | over repositories |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    pairs = [(la, ma, lb, mb) for la, ma, lb, mb in KEY if (la, ma) in res and (lb, mb) in res]
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
            row += [f(d), ci(f, lo, hi), ci(f, clo, chi)]
            if metric == "acc3":
                row.append(f"{mcnemar([A[c]['acc3'] for c in cs], [B[c]['acc3'] for c in cs]):.3g}")
        out.append("| " + " | ".join(row) + " |")
    for ma, mb in DROPS:
        keys = [("en", ma), ("ko", ma), ("en", mb), ("ko", mb)]
        if not all(k in res for k in keys):
            continue
        cs = [c for c in res[keys[0]] if all(c in res[k] for k in keys) and keep(c)]
        g = [questions[c]["project"] for c in cs]
        row = [f"drop of {LABEL[ma]} (KO − EN)", f"drop of {LABEL[mb]}", str(len(cs))]
        for metric in ("acc3", "mrr"):
            v = lambda k, c: float(res[k][c][metric])  # noqa: E731
            a = [v(keys[1], c) - v(keys[0], c) for c in cs]
            b = [v(keys[3], c) - v(keys[2], c) for c in cs]
            d, lo, hi = bootstrap_diff(a, b)
            clo, chi = cluster_bootstrap_diff(a, b, g)
            f = pct if metric == "acc3" else (lambda x: f"{x:+.3f}")
            row += [f(d), ci(f, lo, hi), ci(f, clo, chi)]
            if metric == "acc3":
                row.append("")
        out.append("| " + " | ".join(row) + " |")
    return out + [""]


def per_repo(questions, res, keep, order):
    """저장소마다 A@3. 영어·한국어 각각, Lantern이 혼합 검색 이상인 저장소 수"""
    repos = sorted({q["project"] for q in questions.values()})
    shown = [m for m in ("bm25-chunk", "hybrid", "hybrid-bgem3", "lantern", "lantern-sem") if m in order]
    out = ["## A@3 per repository", ""]
    for lang in ("en", "ko"):
        if not any((lang, m) in res for m in shown):
            continue
        out += [f"### {LANG_TITLE[lang]}", "", "| Repository | n | " + " | ".join(LABEL[m] for m in shown) + " |", "|---|---|" + "---|" * len(shown)]
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
        out += ["", f"Lantern A@3 ≥ {LABEL['hybrid']}: {ge} of {len(repos)} repositories", ""]
    return out


def subgroups(questions, res, keep, order):
    """한국어 문항을 사전이 걸렸는지, 질문에 영어 낱말(라틴 문자 낱말이나 숫자)이 있는지로 나눠 A@3"""
    from baselines import terms

    groups = {
        "dictionary fires": lambda q: bool(q.get("q_ko_dict_terms")),
        "dictionary does not fire": lambda q: q.get("q_ko_dict_terms") == [],
        "Korean question has an English word or number": lambda q: any(t.isascii() for t in terms(q.get("q_ko", ""))),
        "Korean question has no English word or number": lambda q: not any(t.isascii() for t in terms(q.get("q_ko", ""))),
    }
    cols = [(l, m) for l, m in [("en", "lantern"), ("en", "hybrid"), ("ko", "bm25-chunk"), ("ko", "hybrid"), ("ko-dict", "hybrid"), ("ko-dictw", "hybrid"), ("ko", "lantern"),
                                ("ko", "lantern-no-dict"), ("ko", "lantern-sem"), ("ko-llm", "hybrid")] if (l, m) in res]
    out = ["## Korean questions by subgroup (A@3)", "", "| Questions | n | " + " | ".join(name(l, m) for l, m in cols) + " |", "|---|---|" + "---|" * len(cols)]
    for label, pred in groups.items():
        cs = [c for c, q in questions.items() if keep(c) and q.get("q_ko") and pred(q)]
        cells = []
        for k in cols:
            v = [res[k][c]["acc3"] for c in cs if c in res[k]]
            cells.append(pct(mean(v)) if v else "--")
        out.append(f"| {label} | {len(cs)} | " + " | ".join(cells) + " |")
    return out + ["", "English word or number: a token of Latin letters or digits (the tokenizer of baselines.py).", ""]


def rewrite_runs(questions, res, keep):
    """LLM 재작성의 실행별 결과(같은 설정 세 번)와 저장소 이름을 뺀 통제, 그리고 재작성 모델의 학습 시점 전후로 나눠 본 결과"""
    runs = [l for l in REWRITE_RUNS if any(k[0] == l for k in res)]
    if len(runs) < 2 and not any(k[0] == "ko-llm-norepo" for k in res):
        return []
    methods = [m for m in ("bm25-chunk", "dense", "hybrid", "lantern", "lantern-sem") if any((l, m) in res for l in runs)]

    def cell(lang, m, metric, cs=None):
        rows = [v[metric] for c, v in res.get((lang, m), {}).items() if keep(c) and (cs is None or c in cs)]
        if not rows:
            return None
        return mean([float(x) for x in rows]) * (100 if metric != "mrr" else 1)

    f = lambda x, metric: "--" if x is None else (f"{x:.1f}" if metric != "mrr" else f"{x:.3f}")  # noqa: E731
    out = ["## LLM rewrite runs and the no-repository control", "",
           "KO→LLM is sampled once per question per run. Runs 2 and 3 repeat run 1 with the same prompt and inputs; "
           "the no-repository control gives the model only the Korean question.", ""]
    for metric, label in (("acc3", "A@3"), ("mrr", "MRR")):
        out += [f"| Method ({label}) | EN | " + " | ".join(f"run {i + 1}" for i in range(len(runs))) + " | mean | range | no repo |",
                "|---|---|" + "---|" * len(runs) + "---|---|---|"]
        for m in methods:
            vals = [cell(l, m, metric) for l in runs]
            ok = [v for v in vals if v is not None]
            rng = f"{f(min(ok), metric)}–{f(max(ok), metric)}" if ok else "--"
            avg = f(mean(ok), metric) if ok else "--"
            out.append(f"| {LABEL[m]} | {f(cell('en', m, metric), metric)} | " + " | ".join(f(v, metric) for v in vals)
                       + f" | {avg} | {rng} | {f(cell('ko-llm-norepo', m, metric), metric)} |")
        out.append("")
    # 학습 시점 전후: 시점 뒤 커밋은 모델이 볼 수 없었다
    before = {c for c, q in questions.items() if keep(c) and (q.get("date") or "") < CUTOFF}
    after = {c for c, q in questions.items() if keep(c) and (q.get("date") or "") >= CUTOFF}
    forms = [l for l in ("en", "ko", *runs, "ko-llm-norepo") if any(k[0] == l for k in res)]
    out += [f"A@3 by commit date: before or after the rewriting model's training cutoff ({CUTOFF})", "",
            "| Method | commits | n | " + " | ".join(LANGS[l] for l in forms) + " |", "|---|---|---|" + "---|" * len(forms)]
    for m in ("hybrid", "lantern", "lantern-sem"):
        for label, cs in (("before", before), ("after", after)):
            out.append(f"| {LABEL[m]} | {label} | {len(cs)} | " + " | ".join(f(cell(l, m, 'acc3', cs), 'acc3') for l in forms) + " |")
    return out + [""]


def sensitivity(res, keep):
    """BM25에 사전 낱말을 얼마의 가중치로 더할지 (0 = 사전 없음, 1.0 = 그냥 덧붙임)"""
    steps = [("0", "ko"), ("0.25", "ko-dictw25"), ("0.5", "ko-dictw"), ("0.75", "ko-dictw75"), ("1.0", "ko-dict")]
    if not any((lang, "bm25-chunk") in res for _, lang in steps[1:4] if lang != "ko-dictw"):
        return []
    out = ["## Dictionary weight sensitivity (BM25, Korean + dictionary terms)", "", "| Weight | BM25 (file) A@3 | BM25 (file) MRR | BM25 (chunk) A@3 | BM25 (chunk) MRR |", "|---|---|---|---|---|"]
    for w, lang in steps:
        cells = []
        for m in ("bm25-file", "bm25-chunk"):
            rows = [v for c, v in res.get((lang, m), {}).items() if keep(c)]
            cells += [pct(mean([x["acc3"] for x in rows])), f"{mean([x['mrr'] for x in rows]):.3f}"] if rows else ["--", "--"]
        out.append(f"| {w} | " + " | ".join(cells) + " |")
    return out + ["", "Lantern uses weight 0.5 (KO+dict). Weight 1.0 is the same as appending the terms to the question (KO+dict appended).", ""]


def native(questions, res, keep, order):
    """사람이 직접 쓴 한국어 질문(작성자 A: q_ko_native, B: q_ko_native_b)이 있는 문항에서 LLM이 쓴 한국어 질문과 나란히.
    비교는 미리 정해 둔 것만 (native-plan.md): 사람 질문에서 Lantern − 혼합, 사람 − LLM 한국어, 영어 → 사람 한국어 하락의 차이"""
    out = []
    for lang_h, field, who in (("ko-native", "q_ko_native", "A"), ("ko-native-b", "q_ko_native_b", "B")):
        cs = [c for c, q in questions.items() if keep(c) and q.get(field)]
        if not cs:
            continue
        out += [f"## Korean questions written by developer {who} ({len(cs)} questions, next to the LLM-written Korean questions)", "",
                "| Method | EN A@3 | LLM KO A@3 | native KO A@3 | EN MRR | LLM KO MRR | native KO MRR |", "|---|---|---|---|---|---|---|"]
        for m in order:
            if (lang_h, m) not in res:
                continue
            cells = []
            for metric in ("acc3", "mrr"):
                for lang in ("en", "ko", lang_h):
                    v = [res[(lang, m)][c][metric] for c in cs if c in res.get((lang, m), {})]
                    cells.append(("--" if not v else pct(mean(v)) if metric == "acc3" else f"{mean(v):.3f}"))
            out.append(f"| {LABEL[m]} | " + " | ".join(cells) + " |")
        out += ["", "| Pre-specified comparison (same questions, 95% interval) | A@3 diff. | interval | MRR diff. | interval |", "|---|---|---|---|---|"]
        pairs = [(f"Lantern − hybrid [native {who}]", (lang_h, "lantern"), (lang_h, "hybrid"), None),
                 (f"Lantern [native {who} − LLM KO]", (lang_h, "lantern"), ("ko", "lantern"), None),
                 (f"hybrid [native {who} − LLM KO]", (lang_h, "hybrid"), ("ko", "hybrid"), None),
                 (f"difference of drops: Lantern − hybrid [native {who} − EN]", (lang_h, "lantern"), ("en", "lantern"), ((lang_h, "hybrid"), ("en", "hybrid")))]
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
                row += [f(d), ci(f, lo, hi)]
            out.append("| " + " | ".join(row) + " |")
        out.append("")
    return out


INTRO = [
    "Generated by `eval/bilingual/report.py`; do not edit by hand.",
    "",
    "Each method gets the same files (the engine's file rules, checked by `check_files.py`) and an 8,000-token budget per question. "
    "Metrics are computed per question and averaged: Recall@8k = share of the gold files inside the budget; "
    "R@10f = share of the gold files among the first 10 files; A@k = a gold file among the first k files; "
    "MRR = reciprocal rank of the first gold file (0 if none fits); Files = files in the context. "
    "Differences are A − B on the same questions, in percentage points (MRR: absolute).",
    "",
    "Methods: Keyword = grep for the question's words, best-matching files first, whole files or ±12-line snippets; "
    "BM25 over whole files or 40-line chunks (k1 = 1.2, b = 0.75); Dense = multilingual-e5-small over the same chunks; "
    "BM25 + Dense = reciprocal rank fusion (k = 60) of chunk BM25 and the top 1,000 dense chunks; "
    "Lantern − X = Lantern with one component switched off; Lantern + Dense = Lantern with the same embedding model.",
    "",
    "Question forms: EN = the commit subject; EN-para = the subject reworded in English by an LLM (control for rewording); "
    "KO = the Korean question (written by an LLM from the subject); KO+dict = KO plus Lantern's dictionary terms (half weight for BM25, as in Lantern; "
    "appended for dense); KO+dict appended = the terms appended to the question for every method; KO→MT = KO translated by NLLB-200; "
    "KO→LLM = KO rewritten into an English search query by an LLM, one call per question (runs 2 and 3 repeat it; no repo = without the "
    "repository name); KO-loan = KO rewritten with many Hangul loanwords "
    "by an LLM (stress test, not native questions); KO-native A/B = Korean written by developers (only if collected).",
    "",
]


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

    n = sum(1 for c in questions if keep(c))
    out = [f"# English–Korean benchmark: all methods ({a.subset}: {SUBSET_TEX[a.subset]}, {n} questions)", "", *INTRO]
    order = [m for m in LABEL if any(k[1] == m for k in res)]
    for lang in [l for l in LANGS if any(k[0] == l for k in res)]:
        out += [f"## {LANG_TITLE[lang]}", "", "| Method | n | Recall@8k | R@10f | A@1 | A@3 | A@5 | MRR | Files |", "|---|---|---|---|---|---|---|---|---|"]
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
            out += ["", f"Lantern − method ({LANGS[lang]}, same questions, 95% interval; Holm correction over the rows of this table)", "",
                    "| Method | Recall@8k diff. | A@3 diff. | A@3 McNemar p | Holm p |", "|---|---|---|---|---|"]
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
                out.append(f"| {LABEL[m]} | {pct(rd[0])} {ci(pct, rd[1], rd[2])} | {pct(ad[0])} {ci(pct, ad[1], ad[2])} | {p:.3g} | {ph:.3g} |")
        out.append("")
    # 영어 → 한국어 하락 (같은 문항)
    if any(k[0] == "ko" for k in res):
        out += ["## Drop from English to Korean (KO − EN, same questions, 95% interval)", "", "| Method | n | Recall@8k | A@3 |", "|---|---|---|---|"]
        for m in order:
            en, ko = res.get(("en", m), {}), res.get(("ko", m), {})
            cs = [c for c in en if c in ko and keep(c)]
            if not cs:
                continue
            rd = bootstrap_diff([ko[c]["recall"] for c in cs], [en[c]["recall"] for c in cs])
            ad = bootstrap_diff([float(ko[c]["acc3"]) for c in cs], [float(en[c]["acc3"]) for c in cs])
            out.append(f"| {LABEL[m]} | {len(cs)} | {pct(rd[0])} {ci(pct, rd[1], rd[2])} | {pct(ad[0])} {ci(pct, ad[1], ad[2])} |")
        out.append("")
    out += key_comparisons(questions, res, keep)
    out += per_repo(questions, res, keep, order)
    out += subgroups(questions, res, keep, order)
    out += native(questions, res, keep, order)
    out += rewrite_runs(questions, res, keep)
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
