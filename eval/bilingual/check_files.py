"""비교 대상(baselines.py, repomap_baseline.py)과 키워드 검색(eval/retrieval.mjs)이 Lantern 엔진과 똑같은 파일을 보는지 확인한다.

    python eval/bilingual/check_files.py --dir <클론 폴더> --lantern <엔진 실행 파일> [--set eval/bilingual] [--per-repo 3]

저장소마다 벤치마크 기준 커밋과 문항 부모 커밋 몇 개(처음·가운데·끝)로 꺼내, 엔진으로 색인한 뒤(임시 LANTERN_INDEX_DIR)
색인에 든 파일 목록을 baselines.read_files와 retrieval.mjs --list-files의 목록과 비교한다. 하나라도 다르면 경로를 보여 주고 1로 끝난다.
"""

import argparse
import glob
import json
import os
import sqlite3
import subprocess
import sys
import tempfile

from baselines import HERE, git, read_files

ROOT = os.path.dirname(os.path.dirname(HERE))


def engine_files(lantern, repo, index_dir):
    env = {**os.environ, "LANTERN_INDEX_DIR": index_dir}
    subprocess.run([lantern, "-C", repo, "index"], env=env, check=True, capture_output=True)
    (db,) = glob.glob(os.path.join(index_dir, "*.db"))
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        return {r[0] for r in con.execute("SELECT path FROM files")}
    finally:
        con.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--lantern", required=True)
    ap.add_argument("--set", default=HERE)
    ap.add_argument("--per-repo", type=int, default=3)
    a = ap.parse_args()
    a.lantern = os.path.abspath(a.lantern)
    bad = 0
    checked = 0
    with tempfile.TemporaryDirectory(prefix="lantern-files-") as tmp:
        for f in sorted(glob.glob(os.path.join(a.set, "*.json"))):
            with open(f, encoding="utf-8") as fh:
                s = json.load(fh)
            repo = os.path.join(a.dir, s["project"])
            if git(repo, "status", "--porcelain", "--untracked-files=no").strip():
                sys.exit(f"{repo}에 커밋하지 않은 변경이 있습니다")
            original = git(repo, "rev-parse", "HEAD").strip()
            qs = s["questions"]
            picks = [qs[round(i * (len(qs) - 1) / max(1, a.per_repo - 1))]["parent"] for i in range(a.per_repo)] if a.per_repo else []
            index_dir = os.path.join(tmp, s["project"])
            os.makedirs(index_dir)
            try:
                for rev in [s["ref"], *picks]:
                    git(repo, "checkout", "-q", "--detach", rev)
                    e = engine_files(a.lantern, repo, index_dir)
                    b = {p for p, _ in read_files(repo)}
                    out = subprocess.run(["node", os.path.join(ROOT, "eval", "retrieval.mjs"), "--project", repo, "--list-files"],
                                         check=True, capture_output=True, text=True, encoding="utf-8").stdout
                    k = set(json.loads(out))
                    checked += 1
                    same = e == b == k
                    print(f"{s['project']} {rev[:8]}: 엔진 {len(e)} · 비교 대상 {len(b)} · 키워드 {len(k)} {'같음' if same else '다름'}", flush=True)
                    if not same:
                        bad += 1
                        for label, x in (("비교 대상", b), ("키워드", k)):
                            for p in sorted(x - e)[:10]:
                                print(f"    {label}에만: {p}")
                            for p in sorted(e - x)[:10]:
                                print(f"    엔진에만 ({label}에 없음): {p}")
            finally:
                git(repo, "checkout", "-q", "--detach", original)
    print(f"{checked}곳 중 {checked - bad}곳 같음")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
