"""사람이 처음부터 쓰는 한국어 질문 표 (벤치마크의 한국어 질문은 LLM이 쓰고 LLM이 검토했으므로, 그 영향을 따로 재려고).

    python eval/bilingual/native_sheet.py export <작성표.xlsx> --dir <클론 폴더> [--per-repo 6] [--seed 7]
    python eval/bilingual/native_sheet.py import <작성표.xlsx>    # 쓴 문항만 각 JSON의 q_ko_native(작성자 A), q_ko_native_b(작성자 B)로

- 개발 중에 한 번도 보지 않은 저장소(eval/bench에 없는 7개)에서 저장소마다 같은 수를 무작위로 고른다 (기본 7 × 6 = 42문항).
- 작성자에게 영어 커밋 제목을 보여 주지 않는다 (보여 주면 번역하게 된다). 대신 그 커밋이 바꾼 코드(diff)를 파일 경로를 뺀 채
  같은 이름의 .html 파일로 보여 주고, "이 변경을 코딩 에이전트에게 맡긴다면 어떻게 말하겠는가"를 묻는다.
- 한 문항을 두 사람이 따로 쓸 수 있다 (작성자에 따른 차이). 작성자 칸에는 이름 대신 W1, W2 같은 부호만 적는다.
- 분석 계획은 질문을 받기 전에 정해 둔다: native-plan.md. 측정은 --lang ko-native / ko-native-b.
openpyxl이 필요하다 (pip install openpyxl).
"""

import argparse
import glob
import html
import json
import os
import random
import subprocess
import sys

from openpyxl import Workbook, load_workbook
from openpyxl.styles import Alignment, Font, PatternFill

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = "Arial"
HEAD = ["번호", "저장소", "커밋 (확인용)", "바뀐 코드 보기", "질문 (작성자 A)", "작성자 A", "질문 (작성자 B)", "작성자 B", "메모"]
WIDTH = [16, 13, 14, 16, 60, 10, 60, 10, 24]
INPUT = PatternFill("solid", start_color="FFF6D5")
COL_HASH, COL_LINK, COL_A, COL_WA, COL_B, COL_WB, COL_NOTE = 3, 4, 5, 6, 7, 8, 9
DIFF_LINES = 140

GUIDE = [
    ["한국어 질문 직접 쓰는 법"],
    [],
    ["무엇", "오픈소스 저장소에서 실제로 있었던 코드 변경을 보고, 그 변경을 코딩 에이전트(Claude Code, Cursor 등)에게 맡긴다면 한국어로 어떻게 말할지 씁니다."],
    ["", "'바뀐 코드 보기'를 누르면 변경 내용이 브라우저에 열립니다 (- 는 지운 줄, + 는 새로 쓴 줄). 파일 이름은 일부러 가렸습니다."],
    ["", "평소 내가 실제로 칠 말로 씁니다. 짧아도 되고, 영어 낱말이나 외래어를 섞어도 됩니다."],
    [],
    ["지킬 것 1", "번역기·ChatGPT·Claude 등 도구를 쓰지 않습니다."],
    ["지킬 것 2", "변경하기 '전'의 입장에서 씁니다: 무엇이 문제이거나 무엇을 바꾸고 싶은지. 새로 생긴 줄(+)에만 있는 이름은 쓰지 않습니다."],
    ["지킬 것 3", "파일 이름이나 경로는 쓰지 않습니다. 원래 코드에 있던 함수·옵션 이름은 평소처럼 써도 됩니다."],
    ["지킬 것 4", "커밋 번호로 GitHub를 찾아보지 않습니다 (영어 설명을 보게 되므로)."],
    ["지킬 것 5", "무엇을 바꾼 건지 도저히 모르겠으면 비워 두고 메모에 적습니다."],
    [],
    ["작성자", "이름 대신 W1, W2처럼 부호만 적습니다. 한 문항을 두 사람이 쓰면 A·B 칸을 하나씩 씁니다 (서로의 답은 보지 않기)."],
    ["공개", "쓴 질문은 벤치마크에 넣어 Lantern 저장소에 Apache-2.0으로 공개하고, 논문에는 작성자 수만 밝힙니다. 이름 등 개인정보는 모으지 않습니다."],
    ["분석", "분석 방법은 미리 정해 두었습니다 (eval/bilingual/native-plan.md). 결과가 어떻게 나오든 그대로 보고합니다."],
    [],
    ["예시 (가상의 변경)"],
    ["바뀐 코드", "- if path.endswith('/'): return match(path)\n+ return match(path.rstrip('/'))"],
    ["질문 예", "라우트 매칭할 때 끝에 슬래시 붙어도 되게 해줘"],
    [],
    ["노란 칸만 채웁니다. 다 쓰면: python eval/bilingual/native_sheet.py import <이 파일>"],
]


def sets():
    for f in sorted(glob.glob(os.path.join(HERE, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            yield f, json.load(fh)


def dev_repos_and_seen():
    repos, seen = set(), set()
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            b = json.load(fh)
        repos.add(b["project"])
        seen |= {q["commit"] for q in b["questions"]}
    return repos, seen


def diff_without_paths(repo, commit, gold):
    """정답 파일의 변경만, 파일 이름이 드러나는 줄은 빼고"""
    out = subprocess.run(["git", "-C", repo, "-c", "core.longpaths=true", "show", "--format=", "--unified=3", commit, "--", *gold],
                         capture_output=True, text=True, encoding="utf-8", errors="replace", check=True).stdout
    keep = []
    for line in out.splitlines():
        if line.startswith(("diff --git", "index ", "--- ", "+++ ", "new file", "deleted file", "similarity", "rename ", "old mode", "new mode")):
            if line.startswith("diff --git") and keep:
                keep.append("")  # 파일이 바뀌는 자리는 빈 줄로만 표시
            continue
        keep.append(line)
    if len(keep) > DIFF_LINES:
        keep = keep[:DIFF_LINES] + [f"... (이하 {len(keep) - DIFF_LINES}줄 생략)"]
    return "\n".join(keep)


def export(path, clones, per_repo, seed):
    dev, seen = dev_repos_and_seen()
    rnd = random.Random(seed)
    picked = []
    for _, s in sets():
        if s["project"] in dev:
            continue
        cand = [(i, q) for i, q in enumerate(s["questions"], 1) if q["commit"] not in seen]
        rnd.shuffle(cand)
        picked += [(s, i, q) for i, q in sorted(cand[:per_repo], key=lambda x: x[0])]

    page = os.path.splitext(path)[0] + ".html"
    parts = ["<!doctype html><html lang='ko'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>",
             "<title>바뀐 코드 보기</title><style>body{font:15px/1.6 system-ui,sans-serif;max-width:980px;margin:0 auto;padding:24px 16px;background:#fbfaf8;color:#1d1f24}"
             "h2{font-size:17px;margin:36px 0 8px;padding-top:12px;border-top:1px solid #ddd}pre{background:#fff;border:1px solid #e3e0da;border-radius:6px;padding:12px;overflow-x:auto;font:13px/1.45 ui-monospace,Consolas,monospace}"
             ".a{color:#0a6b2e;background:#e9f7ee;display:block}.d{color:#9a1c1c;background:#fbecec;display:block}.h{color:#6a5acd;display:block}"
             "@media (prefers-color-scheme:dark){body{background:#14161a;color:#e6e7ea}pre{background:#1b1e23;border-color:#2a2e35}.a{background:#12301d;color:#7ee2a0}.d{background:#3a1717;color:#f5a3a3}}</style></head><body>",
             "<h1>바뀐 코드 보기</h1><p>문항마다 그 커밋이 바꾼 코드입니다 (파일 이름은 가렸고, - 는 지운 줄, + 는 새로 쓴 줄). 이 변경을 코딩 에이전트에게 맡긴다면 어떻게 말할지 작성표에 적어 주세요.</p>"]
    wb = Workbook()
    guide = wb.active
    guide.title = "작성 안내"
    for r in GUIDE:
        guide.append(r)
    for row in guide.iter_rows():
        for c in row:
            c.font = Font(name=FONT, size=11)
            c.alignment = Alignment(wrap_text=True, vertical="top")
    guide["A1"].font = Font(name=FONT, size=14, bold=True)
    guide["A17"].font = Font(name=FONT, size=11, bold=True)
    for col, w in zip("AB", [16, 100]):
        guide.column_dimensions[col].width = w

    ws = wb.create_sheet("질문")
    ws.append(HEAD)
    for c in ws[1]:
        c.font = Font(name=FONT, size=11, bold=True)
    for s, i, q in picked:
        key = f'{s["project"]}-{i:02d}'
        diff = diff_without_paths(os.path.join(clones, s["project"]), q["commit"], q["golden"])
        lines = []
        for line in diff.splitlines():
            cls = "a" if line.startswith("+") else "d" if line.startswith("-") else "h" if line.startswith("@@") else ""
            lines.append(f"<span class='{cls}'>{html.escape(line)}</span>" if cls else html.escape(line))
        parts.append(f"<h2 id='{key}'>{key} ({html.escape(s['project'])})</h2><pre>{chr(10).join(lines)}</pre>")
        ws.append([key, s["project"], q["commit"][:10], "열기", q.get("q_ko_native") or "", q.get("q_ko_native_by") or "",
                   q.get("q_ko_native_b") or "", q.get("q_ko_native_b_by") or "", ""])
        r = ws.max_row
        ws.cell(r, COL_LINK).hyperlink = f"{os.path.basename(page)}#{key}"
        for col in range(1, len(HEAD) + 1):
            ws.cell(r, col).font = Font(name=FONT, size=11)
            ws.cell(r, col).alignment = Alignment(wrap_text=True, vertical="top")
        ws.cell(r, COL_LINK).font = Font(name=FONT, size=11, color="0563C1", underline="single")
        for col in (COL_A, COL_WA, COL_B, COL_WB, COL_NOTE):
            ws.cell(r, col).fill = INPUT
    parts.append("</body></html>")
    for i, w in enumerate(WIDTH, 1):
        ws.column_dimensions[chr(64 + i)].width = w
    ws.freeze_panes = "E2"
    wb.active = 1
    wb.save(path)
    with open(page, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(parts))
    print(f"{path}: 질문 {len(picked)}개 (저장소 {len({s['project'] for s, _, _ in picked})}개), 바뀐 코드: {page}")


def import_(path):
    ws = load_workbook(path)["질문"]
    filled = {}
    for row in ws.iter_rows(min_row=2, values_only=True):
        key, _, short, _, a, wa, b, wb_ = row[:8]
        if key:
            filled[key] = (str(short or ""), str(a or "").strip(), str(wa or "").strip(), str(b or "").strip(), str(wb_ or "").strip())
    counts, writers, excluded = [0, 0], set(), []
    for f, s in sets():
        for i, q in enumerate(s["questions"], 1):
            key = f'{s["project"]}-{i:02d}'
            if key not in filled:
                continue
            short, a, wa, b, wb_ = filled[key]
            if not q["commit"].startswith(short):
                sys.exit(f"{key}: 작성표의 커밋이 JSON과 다릅니다. 작성표를 다시 만드세요 (export)")
            # 미리 정한 제외 규칙 (native-plan.md): 정답 파일 이름이나 경로가 들어간 질문
            # index, utils처럼 흔한 낱말인 파일 이름은 확장자까지 있어야 이름으로 본다
            generic = {"index", "utils", "util", "main", "mod", "lib", "__init__", "types", "helpers", "common", "core"}
            names = {os.path.basename(g).lower() for g in q["golden"]} | {g.lower() for g in q["golden"]}
            names |= {stem for stem in (os.path.splitext(os.path.basename(g))[0].lower().lstrip("-") for g in q["golden"]) if stem not in generic}
            for text, who, field, n in ((a, wa, "q_ko_native", 0), (b, wb_, "q_ko_native_b", 1)):
                if text and any(nm in text.lower() for nm in names):
                    excluded.append(f"{key} ({'AB'[n]})")
                    text = ""
                if text:
                    q[field], q[field + "_by"] = text, who or "?"
                    counts[n] += 1
                    writers.add(who or "?")
                else:
                    q.pop(field, None)
                    q.pop(field + "_by", None)
        with open(f, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(json.dumps(s, ensure_ascii=False, indent=2) + "\n")
    print(f"작성자 A {counts[0]}개, 작성자 B {counts[1]}개 (작성표 {len(filled)}문항), 작성자 {len(writers)}명")
    if excluded:
        print(f"정답 파일 이름이 들어가 뺀 질문 {len(excluded)}개: {', '.join(excluded)}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(usage=__doc__)
    ap.add_argument("cmd", choices=["export", "import"])
    ap.add_argument("path")
    ap.add_argument("--dir", help="export: 저장소 클론 폴더 (바뀐 코드를 뽑는 데 씀)")
    ap.add_argument("--per-repo", type=int, default=6)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    if a.cmd == "export":
        if not a.dir:
            sys.exit("export에는 --dir <클론 폴더>가 필요합니다")
        export(a.path, a.dir, a.per_repo, a.seed)
    else:
        import_(a.path)
