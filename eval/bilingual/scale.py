"""벤치마크 저장소마다 Lantern의 규모·속도를 잰다 (모델 호출 없음).

    python eval/bilingual/scale.py --dir <클론 폴더> --lantern <엔진 실행 파일> [--set eval/bilingual] [--tex <표 파일>]

- 처음 인덱싱(빈 인덱스에서 index --full)의 걸린 시간과 최대 메모리(RSS)
- 바뀐 것이 없을 때 다시 인덱싱한 시간
- 인덱스 크기, 파일·심볼·참조 수 (lantern stats --json)
- 질문당 맥락 조립 시간: 벤치마크 측정 결과(retrieval-*.json의 Lantern ms) 평균
각 저장소는 벤치마크의 기준 커밋으로 맞춘 뒤 잰다. 인덱스는 임시 폴더에 만든다. psutil이 필요하다.
"""

import argparse
import glob
import json
import os
import re
import shutil
import statistics
import subprocess
import tempfile
import time

import psutil

HERE = os.path.dirname(os.path.abspath(__file__))


def run_measured(cmd, env):
    """걸린 시간(초)과 최대 RSS(MB)"""
    started = time.perf_counter()
    p = psutil.Popen(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    peak = 0
    while p.poll() is None:
        try:
            peak = max(peak, p.memory_info().rss)
        except psutil.Error:
            pass
        time.sleep(0.02)
    elapsed = time.perf_counter() - started
    if p.returncode:
        raise SystemExit(f"{' '.join(cmd)} 실패: {p.stderr.read().decode(errors='replace')[-400:]}")
    return elapsed, peak / 2**20


def dir_mb(path):
    return sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(path) for f in fs) / 2**20


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--lantern", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--tex", default=None)
    a = ap.parse_args()
    results_dir = os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)))
    rows = []
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        name = s["project"]
        repo = os.path.abspath(os.path.join(a.dir, name))
        subprocess.run(["git", "-C", repo, "-c", "core.longpaths=true", "checkout", "-q", "--detach", s["ref"]], check=True)
        idx = tempfile.mkdtemp(prefix=f"lantern-scale-{name}-")
        env = {**os.environ, "LANTERN_INDEX_DIR": idx}
        full_s, full_mb = run_measured([a.lantern, "-C", repo, "index", "--full"], env)
        again_s, _ = run_measured([a.lantern, "-C", repo, "index"], env)
        out = subprocess.run([a.lantern, "-C", repo, "stats"], env=env, capture_output=True, text=True, encoding="utf-8").stdout
        nums = [int(x.replace(",", "")) for x in re.findall(r"[\d,]+", out.splitlines()[0])] if out else []
        size = dir_mb(idx)
        shutil.rmtree(idx, ignore_errors=True)
        ms = []
        pat = re.compile(rf"^retrieval-{re.escape(name)}-\d{{4}}-.*\.json$")
        cands = sorted(x for x in os.listdir(results_dir) if pat.match(x)) if os.path.isdir(results_dir) else []
        if cands:
            with open(os.path.join(results_dir, cands[-1]), encoding="utf-8") as fh:
                ms = [r["lantern"]["ms"] for r in json.load(fh)["rows"] if r["lantern"].get("ms") is not None]
        row = {
            "project": name,
            "files": nums[0] if nums else None,
            "symbols": nums[1] if len(nums) > 1 else None,
            "index_s": full_s,
            "index_mb": full_mb,
            "reindex_s": again_s,
            "size_mb": size,
            "assemble_ms": statistics.median(ms) if ms else None,
        }
        rows.append(row)
        print(f"{name:13} 파일 {row['files']:>5} 심볼 {row['symbols']:>6} | 인덱싱 {full_s:5.1f}초 최대 {full_mb:4.0f}MB | 다시 {again_s:4.2f}초 | 인덱스 {size:5.1f}MB | 조립 중앙값 {row['assemble_ms'] or 0:5.1f}ms", flush=True)
    if a.tex:
        lines = [
            "% scale.py로 만든 표. 손으로 고치지 말고 다시 만든다",
            "\\begin{table}[t]",
            "\\centering",
            "\\caption{Indexing cost and per-question context assembly time (median) on a desktop CPU (Intel Core i5-11400, 6 cores, 32 GB RAM), no model calls.}",
            "\\label{tab:scale}",
            "\\setlength{\\tabcolsep}{3pt}",
            "\\begin{tabular}{lrrrrr}",
            "\\toprule",
            "Repository & Files & Index (s) & Peak (MB) & Size (MB) & Query (ms) \\\\",
            "\\midrule",
        ]
        for r in rows:
            lines.append(f"{r['project']} & {r['files']:,} & {r['index_s']:.1f} & {r['index_mb']:.0f} & {r['size_mb']:.1f} & {r['assemble_ms']:.0f} \\\\")
        lines += ["\\bottomrule", "\\end{tabular}", "\\end{table}", ""]
        os.makedirs(os.path.dirname(os.path.abspath(a.tex)), exist_ok=True)
        with open(a.tex, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("\n".join(lines))


if __name__ == "__main__":
    main()
