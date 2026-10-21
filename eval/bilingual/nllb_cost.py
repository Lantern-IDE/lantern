"""번역한 뒤 검색(KO→MT)의 비용: NLLB-200으로 한국어 질문을 하나씩 옮길 때 질문당 시간과 메모리 (모델 호출 없음, 로컬 CPU).

    python eval/bilingual/nllb_cost.py [--set eval/bilingual] [--threads 4] [--out eval/bilingual/measurements/nllb.json]

에이전트가 질문을 받을 때마다 옮기는 상황이라 묶지 않고 한 문항씩(배치 1) 옮긴다. 번역 설정은 translate.py와 같다(beam 4,
최대 128토큰). 개발 중에 보지 않은 문항(eval/bench에 없는 커밋)만 잰다. 메모리는 이 프로세스의 최대 작업 집합(Windows)
또는 최대 RSS다. 시간 측정이라 다시 재면 조금씩 다르다. transformers, sentencepiece, psutil이 필요하다.
"""

import argparse
import glob
import json
import os
import statistics
import time

import psutil

from baselines import REVISION

HERE = os.path.dirname(os.path.abspath(__file__))


def peak_mb():
    m = psutil.Process().memory_info()
    return (getattr(m, "peak_wset", 0) or m.rss) / 2**20


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--model", default="facebook/nllb-200-distilled-600M")
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--out", default=os.path.join(HERE, "measurements", "nllb.json"))
    a = ap.parse_args()
    import torch
    from transformers import AutoModelForSeq2SeqLM, AutoTokenizer

    torch.set_num_threads(a.threads)
    seen = set()
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            seen |= {q["commit"] for q in json.load(fh)["questions"]}
    qs = []
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        qs += [(s["project"], q) for q in s["questions"] if q.get("q_ko") and q["commit"] not in seen]
    t = time.perf_counter()
    tok = AutoTokenizer.from_pretrained(a.model, src_lang="kor_Hang", revision=REVISION.get(a.model))
    model = AutoModelForSeq2SeqLM.from_pretrained(a.model, revision=REVISION.get(a.model))
    model.eval()
    load_s = time.perf_counter() - t
    bos = tok.convert_tokens_to_ids("eng_Latn")
    rows = []
    with torch.inference_mode():
        for proj, q in qs:
            t = time.perf_counter()
            enc = tok([q["q_ko"]], return_tensors="pt")
            out = model.generate(**enc, forced_bos_token_id=bos, num_beams=4, max_new_tokens=128)
            text = tok.batch_decode(out, skip_special_tokens=True)[0].strip()
            rows.append({"project": proj, "commit": q["commit"], "ms": round((time.perf_counter() - t) * 1000, 1), "same_as_stored": text == q.get("q_ko_mt")})
    ms = sorted(r["ms"] for r in rows)
    summary = {
        "model": a.model, "threads": a.threads, "questions": len(rows), "load_s": round(load_s, 2),
        "median_ms": statistics.median(ms), "p90_ms": ms[int(0.9 * len(ms)) - 1], "peak_mb": round(peak_mb()),
        "same_as_stored": sum(r["same_as_stored"] for r in rows),
    }
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    with open(a.out, "w", encoding="utf-8", newline="\n") as fh:
        json.dump({**summary, "rows": rows}, fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    print(f"{len(rows)}문항 ({a.threads}스레드, 한 문항씩): 질문당 중앙값 {summary['median_ms'] / 1000:.2f}초 (90% {summary['p90_ms'] / 1000:.2f}초), "
          f"모델 로드 {load_s:.1f}초, 최대 메모리 {summary['peak_mb']}MB, 저장된 번역과 같음 {summary['same_as_stored']}/{len(rows)}")


if __name__ == "__main__":
    main()
