"""영·한 병렬 벤치마크의 한국어 질문 작성표.

    python eval/bilingual/ko_sheet.py export <작성표.xlsx>   # 질문 JSON → 작성표 (이미 쓴 한국어 질문은 그대로 채움)
    python eval/bilingual/ko_sheet.py import <작성표.xlsx>   # 작성표의 한국어 질문 → 각 JSON의 q_ko

작성표에는 정답 파일을 넣지 않는다. 파일 이름을 보면 한국어 질문에 힌트가 섞이기 때문이다.
openpyxl이 필요하다 (pip install openpyxl).
"""

import glob
import json
import os
import sys

from openpyxl import Workbook, load_workbook
from openpyxl.styles import Alignment, Font, PatternFill

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = "Arial"
HEAD = ["번호", "저장소", "영어 질문 (커밋 메시지)", "한국어 질문 (여기에 쓰기)", "메모", "커밋 (뜻이 모호할 때만)"]
WIDTH = [12, 13, 60, 60, 24, 18]
INPUT = PatternFill("solid", start_color="FFF6D5")


def sets():
    for f in sorted(glob.glob(os.path.join(HERE, "*.json"))):
        with open(f, encoding="utf-8") as fh:
            yield f, json.load(fh)


def export(path):
    wb = Workbook()
    guide = wb.active
    guide.title = "작성 안내"
    rows = [
        ["한국어 질문 쓰는 법"],
        [],
        ["1", "한국 개발자가 동료나 AI에게 실제로 묻듯이 자연스럽게 씁니다. 단어마다 옮기는 직역은 피합니다."],
        ["2", "영어 질문에 없는 정보는 더하지 않습니다: 파일 이름, 함수 이름, 원인 추측 등. 코드는 보지 않고 씁니다."],
        ["3", "코드 이름(MethodView, verify=False)과 고유명사는 영어 그대로 둬도 됩니다. 일반 낱말은 평소 쓰는 말로 씁니다 (요청 본문 / 리퀘스트 바디 둘 다 괜찮음)."],
        ["4", "말투는 자유입니다: '~하게 고쳐줘', '~는 어디서 처리해?', '~ 문제' 등. 여러 모양이 섞이는 게 좋습니다."],
        ["5", "뜻이 모호하면 커밋 링크를 열어 메시지 본문까지는 읽어도 됩니다. 바뀐 코드(diff)는 보지 않습니다."],
        ["6", "예전 벤치마크(eval/bench)의 한국어 질문은 보지 말고 새로 씁니다."],
        ["7", "도저히 옮기기 어려우면 비워 두고 메모 칸에 이유를 적습니다."],
        [],
        ["예시 (벤치마크에 없는 가상의 질문)"],
        HEAD,
        ["(예) demo-01", "demo", "handle trailing slash in route matching", "라우트 매칭할 때 끝에 붙은 슬래시도 처리해줘", "", ""],
        ["(예) demo-02", "demo", "retry idle connections after the peer resets them", "상대가 끊은 유휴 커넥션은 다시 연결해서 재시도하게", "", ""],
        ["(예) demo-03", "demo", "Fix crash when the config file is empty", "설정 파일이 비어 있으면 죽는 문제", "", ""],
        [],
        ["노란 칸(한국어 질문, 메모)만 고칩니다. 다 쓰면 python eval/bilingual/ko_sheet.py import <이 파일>로 벤치마크에 넣습니다."],
    ]
    for r in rows:
        guide.append(r)
    for row in guide.iter_rows():
        for c in row:
            c.font = Font(name=FONT, size=11)
            c.alignment = Alignment(wrap_text=True, vertical="top")
    guide["A1"].font = Font(name=FONT, size=14, bold=True)
    guide["A11"].font = Font(name=FONT, size=11, bold=True)
    for c in guide[12]:
        c.font = Font(name=FONT, size=11, bold=True)
    guide.column_dimensions["A"].width = 14
    guide.column_dimensions["B"].width = 100
    for col, w in zip("CDEF", WIDTH[2:]):
        guide.column_dimensions[col].width = w

    ws = wb.create_sheet("질문")
    ws.append(HEAD)
    for c in ws[1]:
        c.font = Font(name=FONT, size=11, bold=True)
    n = 0
    for _, s in sets():
        for i, q in enumerate(s["questions"], 1):
            url = f'{s["url"]}/commit/{q["commit"]}'
            ws.append([f'{s["project"]}-{i:02d}', s["project"], q["q"], q.get("q_ko") or "", q.get("q_ko_note") or "", "열기"])
            r = ws.max_row
            ws.cell(r, 6).hyperlink = url
            ws.cell(r, 6).font = Font(name=FONT, size=11, color="0563C1", underline="single")
            for col in (1, 2, 3, 4, 5):
                ws.cell(r, col).font = Font(name=FONT, size=11)
            for col in (4, 5):
                ws.cell(r, col).fill = INPUT
            for col in range(1, 7):
                ws.cell(r, col).alignment = Alignment(wrap_text=True, vertical="top")
            n += 1
    for i, w in enumerate(WIDTH, 1):
        ws.column_dimensions[chr(64 + i)].width = w
    ws.freeze_panes = "C2"
    ws.auto_filter.ref = f"A1:F{ws.max_row}"
    wb.active = 1
    wb.save(path)
    print(f"{path}: 질문 {n}개")


def import_(path):
    ws = load_workbook(path)["질문"]
    filled = {}
    for row in ws.iter_rows(min_row=2, values_only=True):
        key, _, en, ko, note = row[:5]
        if key:
            filled[key] = (en, (ko or "").strip(), (note or "").strip())
    done = 0
    total = 0
    for f, s in sets():
        for i, q in enumerate(s["questions"], 1):
            total += 1
            key = f'{s["project"]}-{i:02d}'
            if key not in filled:
                continue
            en, ko, note = filled[key]
            if en != q["q"]:
                sys.exit(f"{key}: 작성표의 영어 질문이 JSON과 다릅니다. 작성표를 다시 만드세요 (export)")
            q["q_ko"] = ko or None
            if note:
                q["q_ko_note"] = note
            else:
                q.pop("q_ko_note", None)
            done += bool(ko)
        with open(f, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(json.dumps(s, ensure_ascii=False, indent=2) + "\n")
    print(f"한국어 질문 {done}/{total}개")


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("export", "import"):
        sys.exit(__doc__)
    (export if sys.argv[1] == "export" else import_)(sys.argv[2])
