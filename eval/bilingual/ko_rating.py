"""저자(한국어 모국어 화자)가 LLM이 쓴 한국어 질문을 평가만 하는 표. 질문은 고치지 않는다 (결과를 이미 봤으므로).

    python eval/bilingual/ko_rating.py export <평가표.xlsx> [--n 30] [--seed 11]
    python eval/bilingual/ko_rating.py import <평가표.xlsx>     # 평가를 ratings/ko-ratings.json에 쓰고 요약을 출력

기준은 평가 전에 정해 둔다: ko-rating-plan.md. 개발 중에 보지 않은 문항에서 무작위로 고른다.
openpyxl이 필요하다 (pip install openpyxl).
"""

import argparse
import glob
import json
import os
import random
import sys
from collections import Counter

from openpyxl import Workbook, load_workbook
from openpyxl.styles import Alignment, Font, PatternFill
from openpyxl.worksheet.datavalidation import DataValidation

HERE = os.path.dirname(os.path.abspath(__file__))
# 하위 폴더에: eval/bilingual/*.json은 모든 측정 스크립트가 문항 세트로 읽는다
OUT = os.path.join(HERE, "ratings", "ko-ratings.json")
FONT = "Arial"
HEAD = ["번호", "저장소", "영어 커밋 제목", "LLM이 쓴 한국어 질문", "자연스러움 (1~3)", "뜻", "메모"]
WIDTH = [16, 13, 55, 55, 14, 12, 30]
INPUT = PatternFill("solid", start_color="FFF6D5")
GUIDE = [
    ["LLM 한국어 질문 평가 (질문은 고치지 않습니다)"],
    [],
    ["자연스러움", "3 = 한국 개발자가 실제로 이렇게 묻겠다 / 2 = 뜻은 통하지만 번역투나 어색한 데가 있다 / 1 = 이렇게는 묻지 않겠다"],
    ["뜻", "같음 = 영어 제목과 뜻이 같다 / 일부 = 조건·부정·대상 일부가 빠지거나 달라졌다 / 다름 = 뜻이 틀렸다"],
    ["메모", "어색한 곳이나 틀린 곳을 짧게 (선택)"],
    [],
    ["지킬 것", "질문을 고치지 않습니다. 정답 파일이나 검색 결과는 보지 않습니다. 한 번 적은 평가는 바꾸지 않습니다."],
    ["기준", "평가 기준은 eval/bilingual/ko-rating-plan.md에 미리 적어 두었습니다. 평가자는 저자 한 사람이라는 한계를 논문에 밝힙니다."],
    [],
    ["다 하면: python eval/bilingual/ko_rating.py import <이 파일>"],
]


def sets():
    for f in sorted(glob.glob(os.path.join(HERE, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            yield f, json.load(fh)


def export(path, n, seed):
    seen = set()
    for f in glob.glob(os.path.join(HERE, "..", "bench", "*.json")):
        with open(f, encoding="utf-8") as fh:
            seen |= {q["commit"] for q in json.load(fh)["questions"]}
    pool = [(s["project"], i, q) for _, s in sets() for i, q in enumerate(s["questions"], 1) if q["commit"] not in seen and q.get("q_ko")]
    picked = random.Random(seed).sample(pool, n)
    wb = Workbook()
    guide = wb.active
    guide.title = "평가 기준"
    for r in GUIDE:
        guide.append(r)
    for row in guide.iter_rows():
        for c in row:
            c.font = Font(name=FONT, size=11)
            c.alignment = Alignment(wrap_text=True, vertical="top")
    guide["A1"].font = Font(name=FONT, size=14, bold=True)
    for col, w in zip("AB", [14, 100]):
        guide.column_dimensions[col].width = w
    ws = wb.create_sheet("평가")
    ws.append(HEAD)
    for c in ws[1]:
        c.font = Font(name=FONT, size=11, bold=True)
    nat = DataValidation(type="list", formula1='"1,2,3"', allow_blank=True)
    mean = DataValidation(type="list", formula1='"같음,일부,다름"', allow_blank=True)
    ws.add_data_validation(nat)
    ws.add_data_validation(mean)
    for proj, i, q in picked:
        ws.append([f"{proj}-{i:02d}", proj, q["q"], q["q_ko"], "", "", ""])
        r = ws.max_row
        for col in range(1, len(HEAD) + 1):
            ws.cell(r, col).font = Font(name=FONT, size=11)
            ws.cell(r, col).alignment = Alignment(wrap_text=True, vertical="top")
        for col in (5, 6, 7):
            ws.cell(r, col).fill = INPUT
        nat.add(ws.cell(r, 5))
        mean.add(ws.cell(r, 6))
    for k, w in enumerate(WIDTH, 1):
        ws.column_dimensions[chr(64 + k)].width = w
    ws.freeze_panes = "E2"
    wb.active = 1
    wb.save(path)
    print(f"{path}: {n}문항")


def import_(path):
    ws = load_workbook(path)["평가"]
    ko = {f'{s["project"]}-{i:02d}': q["q_ko"] for _, s in sets() for i, q in enumerate(s["questions"], 1)}
    rows = []
    for key, proj, en, q_ko, nat, mean, note in ws.iter_rows(min_row=2, values_only=True):
        if not key:
            continue
        if ko.get(key) != q_ko:
            sys.exit(f"{key}: 평가표의 한국어 질문이 JSON과 다릅니다 (질문은 고치지 않습니다)")
        if nat is None and not mean:
            continue
        rows.append({"id": key, "naturalness": int(nat) if nat is not None else None, "meaning": mean or None, "note": note or ""})
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as fh:
        json.dump({"rater": "author (native Korean speaker)", "plan": "ko-rating-plan.md", "ratings": rows}, fh, ensure_ascii=False, indent=1)
    nat = Counter(r["naturalness"] for r in rows if r["naturalness"])
    mean = Counter(r["meaning"] for r in rows if r["meaning"])
    print(f"평가 {len(rows)}문항 → {OUT}")
    print(f"자연스러움: 3 {nat[3]} · 2 {nat[2]} · 1 {nat[1]}")
    print(f"뜻: 같음 {mean['같음']} · 일부 {mean['일부']} · 다름 {mean['다름']}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(usage=__doc__)
    ap.add_argument("cmd", choices=["export", "import"])
    ap.add_argument("path")
    ap.add_argument("--n", type=int, default=30)
    ap.add_argument("--seed", type=int, default=11)
    a = ap.parse_args()
    (export(a.path, a.n, a.seed) if a.cmd == "export" else import_(a.path))
