"""BM25+임베딩 혼합 검색(baselines.py의 hybrid)의 비용: 인덱싱(조각 읽기·BM25·임베딩)과 질문당 시간.

    python eval/bilingual/hybrid_cost.py --dir <클론 폴더> [--threads 4] [--queries 10] [--out <결과 JSON>]

저장소마다 기준 커밋(ref)에서 모든 조각을 캐시 없이 처음부터 임베딩한다 (multilingual-e5-small, CPU).
질문당 시간은 BM25 점수 + 질문 임베딩 + 유사도 + RRF + 예산 담기. 질문 글은 그 저장소의 한국어 질문.
Lantern의 scale.py(인덱싱·질문당 조립 시간)와 같은 CPU에서 비교한다. 임베딩 모델을 처음 받는 시간은 넣지 않는다.
"""

import argparse
import glob
import json
import os
import statistics
import time
from collections import defaultdict

from baselines import BM25, EMBED_MODEL, HERE, RRF_K, chunks_of, git, pack_chunks, ranked, read_files, terms


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--queries", type=int, default=10)
    ap.add_argument("--budget", type=int, default=8000)
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    import numpy as np
    import psutil
    import torch
    from sentence_transformers import SentenceTransformer

    torch.set_num_threads(a.threads)
    model = SentenceTransformer(EMBED_MODEL, device="cpu")
    model.max_seq_length = 512
    proc = psutil.Process()
    rows = []
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        repo = os.path.join(a.dir, s["project"])
        original = git(repo, "rev-parse", "HEAD").strip()
        git(repo, "checkout", "-q", "--detach", s["ref"])
        base_rss = proc.memory_info().rss
        t = time.time()
        files = read_files(repo)
        chunks = chunks_of(files)
        bm = BM25([terms(p + " " + b) for p, _, b in chunks])
        t_lex = time.time() - t
        t = time.time()
        mat = model.encode([f"passage: {p}\n{b}" for p, _, b in chunks], batch_size=32, normalize_embeddings=True)
        t_emb = time.time() - t
        peak = (proc.memory_info().rss - base_rss) / 2**20
        times = []
        for q in [q for q in s["questions"] if q.get("q_ko")][: a.queries]:
            t = time.time()
            lex = ranked(bm.scores(terms(q["q_ko"])))
            sims = mat @ model.encode([f"query: {q['q_ko']}"], normalize_embeddings=True)[0]
            dense = [int(i) for i in np.argsort(-sims, kind="stable")]
            fused = defaultdict(float)
            for r, i in enumerate(lex):
                fused[i] += 1 / (RRF_K + r + 1)
            for r, i in enumerate(dense[:1000]):
                fused[i] += 1 / (RRF_K + r + 1)
            pack_chunks(sorted(fused, key=lambda i: (-fused[i], i)), chunks, a.budget)
            times.append((time.time() - t) * 1000)
        git(repo, "checkout", "-q", "--detach", original)
        row = {"project": s["project"], "chunks": len(chunks), "lex_s": t_lex, "embed_s": t_emb, "mem_mb": peak, "vec_mb": mat.nbytes / 2**20, "query_ms": statistics.median(times)}
        rows.append(row)
        print(f"{row['project']:13} 조각 {row['chunks']:6} | BM25 {t_lex:5.1f}초 | 임베딩 {t_emb:6.1f}초 | 메모리 +{peak:5.0f}MB | 벡터 {row['vec_mb']:5.1f}MB | 질문당 {row['query_ms']:6.0f}ms", flush=True)
    print(f"임베딩 {a.threads}스레드, 질문당 시간은 질문 {a.queries}개 중앙값")
    if a.out:
        with open(a.out, "w", encoding="utf-8", newline="\n") as fh:
            json.dump(rows, fh, indent=1)


if __name__ == "__main__":
    main()
