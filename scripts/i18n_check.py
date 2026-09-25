"""UI 문자열 중 영어 번역(app/src/locales/en.json)이 없는 것을 찾는다. 사용: python scripts/i18n_check.py"""
import sys
import re, json, glob, html
ROOT = str(__import__("pathlib").Path(__file__).resolve().parent.parent / "app") + "/"
HANGUL = re.compile(r"[\uac00-\ud7a3]")

def tokens(src):
    """yield (kind, text) for string literals; template parts joined with ${} placeholders"""
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            i = src.find("\n", i); i = n if i < 0 else i; continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2); i = n if j < 0 else j + 2; continue
        if c in "\"'":
            j = i + 1; buf = []
            while j < n and src[j] != c:
                if src[j] == "\\": buf.append(src[j:j+2]); j += 2; continue
                if src[j] == "\n": break
                buf.append(src[j]); j += 1
            yield ("s", "".join(buf)); i = j + 1; continue
        if c == "`":
            j = i + 1; parts = []; cur = []
            while j < n and src[j] != "`":
                if src[j] == "\\": cur.append(src[j:j+2]); j += 2; continue
                if src.startswith("${", j):
                    depth = 1; k = j + 2
                    while k < n and depth:
                        if src[k] == "{": depth += 1
                        elif src[k] == "}": depth -= 1
                        k += 1
                    cur.append("\x00"); j = k; continue
                cur.append(src[j]); j += 1
            yield ("t", "".join(cur)); i = j + 1; continue
        i += 1

exact, templates = set(), set()
for f in glob.glob(ROOT + "src/*.ts"):
    src = open(f, encoding="utf-8").read()
    for kind, s in tokens(src):
        if not HANGUL.search(s): continue
        s = s.replace("\n", "\n").replace('\\"', '"').replace("\'", "'")
        if kind == "t" and "\x00" in s:
            # split by newline so each DOM text line can match; keep whole too
            templates.add(s.replace("\x00", "{}").strip())
        else:
            for part in s.split("\n"):
                if HANGUL.search(part): exact.add(part.strip())
# index.html: text between tags and attribute values
doc = open(ROOT + "index.html", encoding="utf-8").read()
doc = re.sub(r"<!--.*?-->", "", doc, flags=re.S)
for m in re.finditer(r">([^<>]+)<", doc):
    t = html.unescape(m.group(1)).strip()
    if HANGUL.search(t): exact.add(t)
for m in re.finditer(r'(?:title|placeholder|aria-label|data-label)="([^"]+)"', doc):
    t = html.unescape(m.group(1)).strip()
    if HANGUL.search(t): exact.add(t)
en = json.load(open(ROOT + "src/locales/en.json", encoding="utf-8"))
missing = [s for s in sorted(exact) if s not in en["exact"]] + [s for s in sorted(templates) if s not in en["templates"]]
for s in missing:
    print("번역 없음:", s)
print(f"문자열 {len(exact)}개, 템플릿 {len(templates)}개, 번역 없음 {len(missing)}개")
sys.exit(1 if missing else 0)
