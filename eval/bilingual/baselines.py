"""비교 대상(기준선) 검색. Lantern과 같은 문항·같은 토큰 예산으로 파일 순서를 만들어 저장한다.

    python eval/bilingual/baselines.py --dir <클론 폴더> [--set eval/bilingual] [--lang en|ko|ko-mt]
        [--methods bm25-file,bm25-chunk,dense,hybrid] [--budget 8000] [--out <폴더>] [--only gin,jq]

- bm25-file: 파일 단위 BM25, 순위대로 파일을 통째로 예산까지 담음
- bm25-chunk: 40줄 조각 단위 BM25, 순위대로 조각을 예산까지 담음
- dense: 같은 조각을 다국어 임베딩(multilingual-e5-small)으로 찾음 (sentence-transformers 필요)
- hybrid: bm25-chunk와 dense의 순위를 RRF(k=60)로 섞음
- 질문: en = 영어(q), ko = 한국어(q_ko), ko-mt = 한국어를 기계 번역한 영어(q_ko_mt)

파일을 고르는 기준(코드 확장자, 무시 폴더, 400KB 미만, 압축 파일 제외)과 토큰 추정(글자 4개 ≈ 1토큰)은
eval/retrieval.mjs와 같다. 지표는 report.py가 Lantern 결과와 함께 계산한다.
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
CODE = re.compile(r"\.(ts|mts|cts|tsx|js|jsx|mjs|cjs|py|pyi|rs|java|go|cs|kt|kts|c|h|cpp|cc|cxx|hpp|hh|hxx|ipp)$")
IGNORE = {".git", "node_modules", "target", "dist", "build", ".next", ".lantern", "coverage", ".turbo", ".venv", "__pycache__"}
CHUNK_LINES = 40
RRF_K = 60
EMBED_MODEL = "intfloat/multilingual-e5-small"
STOP = set("the a an and or of to in on for with by from is are be as at it this that when if not no into via".split())


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, "-c", "core.longpaths=true", *args], check=True, capture_output=True, text=True).stdout


def read_files(root):
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in IGNORE]
        for name in filenames:
            if not CODE.search(name):
                continue
            p = os.path.join(dirpath, name)
            if os.path.getsize(p) >= 400_000:
                continue
            with open(p, encoding="utf-8", errors="replace") as fh:
                text = fh.read()
            if len(text) > 20_000 and len(text) / max(1, text.count("\n") + 1) > 500:
                continue  # 빌드 결과물 (한 줄이 매우 긴 압축 파일)
            out.append((os.path.relpath(p, root).replace(os.sep, "/"), text))
    out.sort()
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
    def __init__(self, cache_path):
        from sentence_transformers import SentenceTransformer  # 무거워서 쓸 때만

        self.model = SentenceTransformer(EMBED_MODEL, device="cpu")
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
            vecs = self.model.encode(["passage: " + texts[i] for i in missing], batch_size=64, normalize_embeddings=True, show_progress_bar=len(missing) > 2000)
            for i, v in zip(missing, vecs):
                self.cache[keys[i]] = v.astype("float32")
            self.dirty = True
        return np.stack([self.cache[k] for k in keys])

    def query(self, q):
        return self.model.encode(["query: " + q], normalize_embeddings=True)[0]

    def save(self):
        if self.dirty:
            with open(self.cache_path, "wb") as fh:
                pickle.dump(self.cache, fh)
            self.dirty = False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--lang", default="en", choices=["en", "ko", "ko-mt"])
    ap.add_argument("--methods", default="bm25-file,bm25-chunk")
    ap.add_argument("--budget", type=int, default=8000)
    ap.add_argument("--out", default=None)
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    methods = a.methods.split(",")
    out_dir = a.out or os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)), "baselines")
    os.makedirs(out_dir, exist_ok=True)
    field = {"en": "q", "ko": "q_ko", "ko-mt": "q_ko_mt"}[a.lang]
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
        emb = Embedder(os.path.join(out_dir, f"emb-{name}.pkl")) if need_dense else None
        rows = []
        started = time.time()
        for q in qs:
            git(repo, "checkout", "-q", "--detach", q["parent"])
            files = read_files(repo)
            by_path = dict(files)
            query = q[field]
            qt = terms(query)
            row = {"commit": q["commit"], "q": query, "golden": q["golden"], "methods": {}}
            if "bm25-file" in methods:
                order = ranked(BM25([terms(p + " " + t) for p, t in files]).scores(qt))
                picked, used = pack_files([files[i][0] for i in order], by_path, a.budget)
                row["methods"]["bm25-file"] = {"files": picked, "used": used}
            chunks = chunks_of(files) if any(m != "bm25-file" for m in methods) else []
            lex = None
            if any(m in ("bm25-chunk", "hybrid") for m in methods):
                lex = ranked(BM25([terms(p + " " + b) for p, _, b in chunks]).scores(qt))
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
                    row["methods"]["dense"] = {"files": picked, "used": used}
                if "hybrid" in methods:
                    fused = defaultdict(float)
                    for r, i in enumerate(lex):
                        fused[i] += 1 / (RRF_K + r + 1)
                    for r, i in enumerate(dense[:1000]):
                        fused[i] += 1 / (RRF_K + r + 1)
                    order = sorted(fused, key=lambda i: (-fused[i], i))
                    picked, used = pack_chunks(order, chunks, a.budget)
                    row["methods"]["hybrid"] = {"files": picked, "used": used}
                emb.save()
            rows.append(row)
            hit = {m: any(g in v["files"][:3] for g in q["golden"]) for m, v in row["methods"].items()}
            print(f"  {name} {len(rows):2}/{len(qs)} " + " ".join(f"{m}:{'O' if h else '.'}" for m, h in hit.items()), flush=True)
        git(repo, "checkout", "-q", "--detach", original)
        path = os.path.join(out_dir, f"{name}-{a.lang}.json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump({"project": name, "lang": a.lang, "budget": a.budget, "methods": methods, "rows": rows}, fh, ensure_ascii=False, indent=1)
        print(f"{name} ({a.lang}): {len(rows)}문항 {time.time() - started:.0f}초 → {path}")


if __name__ == "__main__":
    main()
