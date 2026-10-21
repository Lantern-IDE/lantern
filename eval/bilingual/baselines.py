"""비교 대상(기준선) 검색. Lantern과 같은 문항·같은 토큰 예산으로 파일 순서를 만들어 저장한다.

    python eval/bilingual/baselines.py --dir <클론 폴더> [--set eval/bilingual] [--lang en|ko|ko-dict|ko-mt|ko-llm]
        [--methods bm25-file,bm25-chunk,dense,hybrid] [--budget 8000] [--out <폴더>] [--only gin,jq]
        [--embed-model BAAI/bge-m3 --tag bgem3]   (다른 임베딩 모델: 방법 이름·결과 파일에 이름표가 붙음)

- bm25-file: 파일 단위 BM25, 순위대로 파일을 통째로 예산까지 담음
- bm25-chunk: 40줄 조각 단위 BM25, 순위대로 조각을 예산까지 담음
- dense: 같은 조각을 다국어 임베딩(multilingual-e5-small)으로 찾음 (sentence-transformers 필요)
- hybrid: bm25-chunk와 dense의 순위를 RRF(k=60)로 섞음
- 질문: en = 영어(q), ko = 한국어(q_ko), ko-dict = 한국어 + Lantern 사전이 넓힌 영어 낱말(q_ko_dict, expand.py),
  ko-mt = 한국어를 기계 번역한 영어(q_ko_mt), ko-llm = 한국어를 LLM이 영어 검색어로 다시 쓴 것(q_ko_llm)
- 임베딩 입력은 512토큰에서 자른다 (모델마다 같은 조건). e5 계열은 query:/passage: 접두어를 붙인다

파일은 엔진(indexer.rs)과 같은 기준으로 고른다: .gitignore에 걸리거나 숨긴 경로와 늘 건너뛰는 폴더(vendor 등) 제외,
코드 확장자, 1 MiB 이하, 바이너리·압축 파일 제외. eval/retrieval.mjs(키워드 검색)도 같고, check_files.py가 엔진 색인과
파일 목록이 똑같은지 확인한다. 내용도 엔진처럼 바이트 그대로 읽어(줄바꿈 CRLF 포함) 토큰 추정(글자 4개 ≈ 1토큰)이 엔진과 같다.
지표는 report.py가 Lantern 결과와 함께 계산한다.
"""

import argparse
import glob
import hashlib
import json
import math
import os
import pickle
import re
import subprocess
import sys
import time
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
# 엔진과 같은 기준 (lang.rs Lang::from_path, indexer.rs SKIP_DIRS·MAX_FILE_BYTES·is_unindexable)
CODE = re.compile(r"\.(ts|mts|cts|tsx|js|jsx|mjs|cjs|py|pyi|rs|java|go|cs|kt|kts|c|h|cpp|cc|cxx|hpp|hh|hxx|ipp)$", re.IGNORECASE)
SKIP_DIRS = {"node_modules", "target", "dist", "build", "out", ".next", ".nuxt", "vendor", "__pycache__", ".venv", "venv", ".git", ".lantern", "coverage"}
MAX_FILE_BYTES = 1024 * 1024
CHUNK_LINES = 40
RRF_K = 60
EMBED_MODEL = "intfloat/multilingual-e5-small"
# 잰 모델의 Hugging Face 판 (다시 잴 때 같은 가중치를 받게)
REVISION = {"intfloat/multilingual-e5-small": "614241f622f53c4eeff9890bdc4f31cfecc418b3", "facebook/nllb-200-distilled-600M": "f8d333a098d19b4fd9a8b18f94170487ad3f821d"}
STOP = set("the a an and or of to in on for with by from is are be as at it this that when if not no into via".split())


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, "-c", "core.longpaths=true", *args], check=True, capture_output=True, text=True, encoding="utf-8").stdout


def unindexable(b):
    """바이너리(앞 8KB에 NUL)이거나 한 줄이 매우 긴 번들·압축 파일 (indexer.rs is_unindexable과 같음, 바이트 기준)"""
    return b"\0" in b[:8192] or (len(b) > 20_000 and len(b) // (b.count(b"\n") + 1) > 500)


def list_files(root):
    """엔진이 색인하는 파일 경로 (indexer.rs walk): .gitignore·.git/info/exclude·전역 제외에 걸리지 않고(git이 판단),
    숨긴 경로(.으로 시작)와 늘 건너뛰는 폴더 밑이 아닌 코드 파일"""
    listed = git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")
    # 추적 중이어도 무시 규칙에 걸리면 엔진은 건너뛴다
    ignored = set(git(root, "ls-files", "-z", "--cached", "--ignored", "--exclude-standard").split("\0"))
    out = []
    for rel in sorted(set(listed) - ignored):
        parts = rel.split("/")
        if not rel or not CODE.search(parts[-1]) or any(x.startswith(".") for x in parts) or any(x in SKIP_DIRS for x in parts[:-1]):
            continue
        p = os.path.join(root, *parts)
        if os.path.isfile(p) and not os.path.islink(p):
            out.append(rel)
    return out


def read_files(root):
    """엔진과 같은 파일을 (경로, 내용)으로: 1 MiB를 넘거나 바이너리·압축 파일이면 뺀다"""
    out = []
    for rel in list_files(root):
        p = os.path.join(root, *rel.split("/"))
        if os.path.getsize(p) > MAX_FILE_BYTES:
            continue
        with open(p, "rb") as fh:
            b = fh.read()
        if not unindexable(b):
            out.append((rel, b.decode("utf-8", errors="replace")))
    return out


def tokens(s):
    return math.ceil(len(s) / 4)


WORD = re.compile(r"[A-Za-z_][A-Za-z0-9_]*|[0-9]+|[가-힣]+")
CAMEL = re.compile(r"[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+")


def terms(text):
    """식별자는 통째로 + 낱말로 쪼갠 것 (parseUserConfig → parseuserconfig, parse, user, config)"""
    out = []
    for w in WORD.findall(text):
        lw = w.lower()
        parts = [p.lower() for piece in w.split("_") for p in CAMEL.findall(piece)]
        if len(parts) > 1:
            out.append(lw)
        out.extend(p for p in (parts or [lw]) if len(p) >= 2 and p not in STOP)
    return out


class BM25:
    def __init__(self, docs, k1=1.2, b=0.75):
        self.k1, self.b = k1, b
        self.tf = [Counter(d) for d in docs]
        self.len = [len(d) for d in docs]
        self.avg = sum(self.len) / max(1, len(docs))
        df = Counter()
        for t in self.tf:
            df.update(t.keys())
        n = len(docs)
        self.idf = {w: math.log(1 + (n - c + 0.5) / (c + 0.5)) for w, c in df.items()}

    def scores(self, query):
        q = [w for w in set(query) if w in self.idf]
        out = []
        for tf, ln in zip(self.tf, self.len):
            s = 0.0
            norm = self.k1 * (1 - self.b + self.b * ln / self.avg)
            for w in q:
                f = tf.get(w)
                if f:
                    s += self.idf[w] * f * (self.k1 + 1) / (f + norm)
            out.append(s)
        return out


DICT_WEIGHT = 0.5  # Lantern이 사전으로 넓힌 낱말에 주는 가중치와 같게


def lex_scores(bm, qt, extra, weight=DICT_WEIGHT):
    """질문 낱말 점수 + 사전이 넓힌 낱말 점수 × weight (extra가 없으면 BM25 그대로)"""
    base = bm.scores(qt)
    if not extra:
        return base
    return [x + weight * y for x, y in zip(base, bm.scores(extra))]


def chunks_of(files):
    out = []
    for path, text in files:
        lines = text.split("\n")
        for i in range(0, len(lines), CHUNK_LINES):
            body = "\n".join(lines[i : i + CHUNK_LINES])
            if body.strip():
                out.append((path, i + 1, body))
    return out


def pack_files(order, files_by_path, budget):
    """순위대로 파일을 통째로 담는다 (마지막 파일은 남은 예산만큼)"""
    picked, used = [], 0
    for p in order:
        if used >= budget:
            break
        picked.append(p)
        used += min(tokens(files_by_path[p]), budget - used)
    return picked, used


def pack_chunks(order, chunks, budget):
    """순위대로 조각을 담는다. 파일 순서는 그 파일의 조각이 처음 담긴 순서"""
    picked, used = [], 0
    for i in order:
        path, _, body = chunks[i]
        t = tokens(body)
        if used + t > budget:
            if budget - used < 50:
                break
            continue
        used += t
        if path not in picked:
            picked.append(path)
    return picked, used


def ranked(scores):
    return [i for i in sorted(range(len(scores)), key=lambda i: (-scores[i], i)) if scores[i] > 0]


class Embedder:
    def __init__(self, cache_path, model=EMBED_MODEL):
        from sentence_transformers import SentenceTransformer  # 무거워서 쓸 때만

        self.model = SentenceTransformer(model, device="cpu", revision=REVISION.get(model))
        self.model.max_seq_length = 512
        e5 = "e5" in model.lower()
        self.qp, self.dp = ("query: ", "passage: ") if e5 else ("", "")
        self.cache_path = cache_path
        self.cache = {}
        if os.path.exists(cache_path):
            with open(cache_path, "rb") as fh:
                self.cache = pickle.load(fh)
        self.dirty = False

    def docs(self, texts):
        import numpy as np

        keys = [hashlib.sha1(t.encode("utf-8", "replace")).hexdigest() for t in texts]
        missing = [i for i, k in enumerate(keys) if k not in self.cache]
        if missing:
            vecs = self.model.encode([self.dp + texts[i] for i in missing], batch_size=32, normalize_embeddings=True, show_progress_bar=len(missing) > 2000)
            for i, v in zip(missing, vecs):
                self.cache[keys[i]] = v.astype("float32")
            self.dirty = True
        return np.stack([self.cache[k] for k in keys])

    def query(self, q):
        return self.model.encode([self.qp + q], normalize_embeddings=True)[0]

    def save(self):
        if self.dirty:
            with open(self.cache_path, "wb") as fh:
                pickle.dump(self.cache, fh)
            self.dirty = False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--lang", default="en", choices=["en", "en-para", "ko", "ko-dict", "ko-dictw", "ko-mt", "ko-llm", "ko-loan", "ko-native", "ko-native-b"])
    ap.add_argument("--methods", default="bm25-file,bm25-chunk")
    ap.add_argument("--budget", type=int, default=8000)
    ap.add_argument("--out", default=None)
    ap.add_argument("--only", default="")
    ap.add_argument("--embed-model", default=EMBED_MODEL)
    ap.add_argument("--tag", default="", help="다른 임베딩 모델일 때 방법 이름·결과 파일에 붙일 이름 (예: bgem3)")
    ap.add_argument("--dict-weight", type=float, default=DICT_WEIGHT, help="ko-dictw에서 사전 낱말 가중치 (민감도 분석: 0.5가 아니면 언어 이름이 ko-dictw25처럼 바뀜)")
    a = ap.parse_args()
    lang = a.lang if a.lang != "ko-dictw" or a.dict_weight == DICT_WEIGHT else f"ko-dictw{round(a.dict_weight * 100)}"
    methods = a.methods.split(",")
    suffix = f"-{a.tag}" if a.tag else ""
    out_dir = a.out or os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)), "baselines")
    os.makedirs(out_dir, exist_ok=True)
    field = {"en": "q", "en-para": "q_para", "ko": "q_ko", "ko-dict": "q_ko_dict", "ko-dictw": "q_ko_dict", "ko-mt": "q_ko_mt", "ko-llm": "q_ko_llm", "ko-loan": "q_ko_loan", "ko-native": "q_ko_native", "ko-native-b": "q_ko_native_b"}[a.lang]
    need_dense = any(m in ("dense", "hybrid") for m in methods)

    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        name = s["project"]
        if a.only and name not in a.only.split(","):
            continue
        qs = [q for q in s["questions"] if q.get(field)]
        if not qs:
            print(f"{name}: {field} 없음, 건너뜀")
            continue
        repo = os.path.join(a.dir, name)
        if git(repo, "status", "--porcelain", "--untracked-files=no").strip():
            sys.exit(f"{repo}에 커밋하지 않은 변경이 있습니다")
        original = git(repo, "rev-parse", "HEAD").strip()
        emb = Embedder(os.path.join(out_dir, f"emb-{name}{suffix}.pkl"), a.embed_model) if need_dense else None
        rows = []
        started = time.time()
        for q in qs:
            git(repo, "checkout", "-q", "--detach", q["parent"])
            files = read_files(repo)
            by_path = dict(files)
            query = q[field]
            qt = terms(query)
            extra = []
            if a.lang == "ko-dictw":
                # Lantern처럼: 사전이 넓힌 낱말은 따로, 절반 가중치로 (흔한 동사가 질문의 식별자를 누르지 않게)
                qt = terms(q["q_ko"])
                extra = [t for t in terms(" ".join(q.get("q_ko_dict_terms") or [])) if t not in qt]
            row = {"commit": q["commit"], "q": query, "golden": q["golden"], "methods": {}}
            if "bm25-file" in methods:
                order = ranked(lex_scores(BM25([terms(p + " " + t) for p, t in files]), qt, extra, a.dict_weight))
                picked, used = pack_files([files[i][0] for i in order], by_path, a.budget)
                row["methods"]["bm25-file"] = {"files": picked, "used": used}
            chunks = chunks_of(files) if any(m != "bm25-file" for m in methods) else []
            lex = None
            if any(m in ("bm25-chunk", "hybrid") for m in methods):
                lex = ranked(lex_scores(BM25([terms(p + " " + b) for p, _, b in chunks]), qt, extra, a.dict_weight))
                if "bm25-chunk" in methods:
                    picked, used = pack_chunks(lex, chunks, a.budget)
                    row["methods"]["bm25-chunk"] = {"files": picked, "used": used}
            if need_dense:
                import numpy as np

                mat = emb.docs([f"{p}\n{b}" for p, _, b in chunks])
                sims = mat @ emb.query(query)
                dense = [int(i) for i in np.argsort(-sims, kind="stable")]
                if "dense" in methods:
                    picked, used = pack_chunks(dense, chunks, a.budget)
                    row["methods"]["dense" + suffix] = {"files": picked, "used": used}
                if "hybrid" in methods:
                    fused = defaultdict(float)
                    for r, i in enumerate(lex):
                        fused[i] += 1 / (RRF_K + r + 1)
                    for r, i in enumerate(dense[:1000]):
                        fused[i] += 1 / (RRF_K + r + 1)
                    order = sorted(fused, key=lambda i: (-fused[i], i))
                    picked, used = pack_chunks(order, chunks, a.budget)
                    row["methods"]["hybrid" + suffix] = {"files": picked, "used": used}
                emb.save()
            rows.append(row)
            hit = {m: any(g in v["files"][:3] for g in q["golden"]) for m, v in row["methods"].items()}
            print(f"  {name} {len(rows):2}/{len(qs)} " + " ".join(f"{m}:{'O' if h else '.'}" for m, h in hit.items()), flush=True)
        git(repo, "checkout", "-q", "--detach", original)
        path = os.path.join(out_dir, f"{name}-{lang}{suffix}.json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump({"project": name, "lang": lang, "budget": a.budget, "embed_model": a.embed_model if need_dense else None, "methods": [m + (suffix if m in ("dense", "hybrid") else "") for m in methods], "rows": rows}, fh, ensure_ascii=False, indent=1)
        print(f"{name} ({lang}): {len(rows)}문항 {time.time() - started:.0f}초 → {path}")


if __name__ == "__main__":
    main()
