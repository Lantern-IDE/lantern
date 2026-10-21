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
    ap.add_argument("--extra", action="append", default=[], help="벤치마크 밖의 큰 저장소 이름=경로 (예: vscode=C:/clones/vscode). 지금 커밋 그대로 잰다")
    ap.add_argument("--extra-questions", default=os.path.join(HERE, "scale_questions.txt"), help="큰 저장소의 조립 시간을 잴 질문 (한 줄에 하나)")
    ap.add_argument("--repeat", type=int, default=3, help="처음 인덱싱을 몇 번 재서 중앙값을 쓸지")
    a = ap.parse_args()
    results_dir = os.path.join(a.dir, "results", os.path.basename(os.path.normpath(a.set)))
    rows = []
    targets = []
    for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            s = json.load(fh)
        targets.append((s["project"], os.path.abspath(os.path.join(a.dir, s["project"])), s["ref"]))
    targets += [(e.split("=", 1)[0], os.path.abspath(e.split("=", 1)[1]), None) for e in a.extra]
    for name, repo, ref in targets:
        if ref:
            subprocess.run(["git", "-C", repo, "-c", "core.longpaths=true", "checkout", "-q", "--detach", ref], check=True)
        idx = tempfile.mkdtemp(prefix=f"lantern-scale-{name}-")
        env = {**os.environ, "LANTERN_INDEX_DIR": idx}
        # 처음 인덱싱은 파일 캐시 상태에 따라 크게 달라서(막 받은 vscode 92초 → 다시 48초) 빈 인덱스에서 여러 번 재고 중앙값을 쓴다
        runs = []
        for k in range(a.repeat):
            if k:
                shutil.rmtree(idx, ignore_errors=True)
                os.makedirs(idx, exist_ok=True)
            runs.append(run_measured([a.lantern, "-C", repo, "index", "--full"], env))
        full_s = statistics.median(s for s, _ in runs)
        full_mb = max(mb for _, mb in runs)
        again_s, _ = run_measured([a.lantern, "-C", repo, "index"], env)
        out = subprocess.run([a.lantern, "-C", repo, "stats"], env=env, capture_output=True, text=True, encoding="utf-8").stdout
        nums = [int(x.replace(",", "")) for x in re.findall(r"[\d,]+", out.splitlines()[0])] if out else []
        if not nums or nums[0] == 0:
            raise SystemExit(f"{name}: 인덱싱한 파일이 없습니다 ({repo}). 클론이 비었는지 확인하세요")
        size = dir_mb(idx)
        ms = []
        if ref:
            pat = re.compile(rf"^retrieval-{re.escape(name)}-\d{{4}}-.*\.json$")
            cands = sorted(x for x in os.listdir(results_dir) if pat.match(x)) if os.path.isdir(results_dir) else []
            if cands:
                with open(os.path.join(results_dir, cands[-1]), encoding="utf-8") as fh:
                    ms = [r["lantern"]["ms"] for r in json.load(fh)["rows"] if r["lantern"].get("ms") is not None]
        else:
            # 벤치마크 밖 저장소: 정해 둔 질문으로 맥락을 조립해 엔진이 알려 주는 조립 시간을 모은다
            with open(a.extra_questions, encoding="utf-8") as fh:
                for q in [x.strip() for x in fh if x.strip() and not x.startswith("#")]:
                    out = subprocess.run([a.lantern, "-C", repo, "context", "--json", "--", q], env=env, capture_output=True, text=True, encoding="utf-8", check=True).stdout
                    ms.append(json.loads(out)["elapsed_ms"])
        shutil.rmtree(idx, ignore_errors=True)
        row = {
            "project": name,
            "files": nums[0] if nums else None,
            "symbols": nums[1] if len(nums) > 1 else None,
            "index_s": full_s,
            "index_mb": full_mb,
            "reindex_s": again_s,
            "size_mb": size,
            "assemble_ms": statistics.median(ms) if ms else None,
            "extra": ref is None,
            # 벤치마크 밖 저장소는 잰 커밋을 남긴다 (다시 잴 때 같은 커밋으로)
            "commit": subprocess.run(["git", "-C", repo, "rev-parse", "--short=7", "HEAD"], capture_output=True, text=True).stdout.strip() if ref is None else ref[:7],
        }
        rows.append(row)
        print(f"{name:13} 파일 {row['files']:>5} 심볼 {row['symbols']:>6} | 인덱싱 {full_s:5.1f}초 최대 {full_mb:4.0f}MB | 다시 {again_s:4.2f}초 | 인덱스 {size:5.1f}MB | 조립 중앙값 {row['assemble_ms'] or 0:5.1f}ms", flush=True)
    if a.tex:
        lines = [
            "% scale.py로 만든 표. 손으로 고치지 말고 다시 만든다",
            "\\begin{table}[t]",
            "\\centering",
            "\\caption{Lantern's cost on a desktop CPU (Intel Core i5-11400, 6 cores, 32 GB RAM), no model calls. Files: code files indexed; "
            f"Index: full indexing from an empty index (median of {a.repeat} runs, warm file cache, up to 8 parsing threads); Peak: peak memory while indexing; "
            "Size: index on disk, which grows with symbols and references rather than files; "
            "Query: median context assembly time per question"
            + (f" (benchmark questions; for {', '.join(r['project'] + ' at commit ' + r['commit'] for r in rows if r.get('extra'))}, the questions in \\texttt{{eval/bilingual/scale\\_questions.txt}})" if any(r.get("extra") for r in rows) else "")
            + ".}",
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
