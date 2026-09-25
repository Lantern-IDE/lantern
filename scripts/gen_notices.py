"""THIRD_PARTY_NOTICES.md 생성기.

배포 파일에 실제로 들어가는 의존성만 모은다:
  - Rust: lantern-app에서 일반(normal) 의존성으로 닿는 크레이트 (Windows 대상)
  - npm: app/package.json의 dependencies에서 닿는 패키지 (devDependencies 제외)
사용: python scripts/gen_notices.py  (저장소 루트에서, 의존성을 바꾼 뒤 다시 실행)
"""
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"

# 코드 의존성이 아닌, 번들에 들어가는 자산
ASSETS = [
    ("@vscode/codicons (아이콘 글꼴)", "CC-BY-4.0", "https://github.com/microsoft/vscode-codicons",
     "아이콘은 Microsoft의 VS Code Codicons를 CC BY 4.0에 따라 사용합니다."),
    ("Pretendard (글꼴)", "OFL-1.1", "https://github.com/orioncactus/pretendard",
     "Pretendard는 SIL Open Font License 1.1에 따라 배포됩니다."),
]


def rust_packages():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
        cwd=ROOT, capture_output=True, text=True, encoding="utf-8", check=True,
    ).stdout
    meta = json.loads(out)
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace = set(meta["workspace_members"])
    start = [i for i in workspace if pkgs[i]["name"] == "lantern-app"]
    seen, stack = set(), list(start)
    while stack:
        cur = stack.pop()
        if cur in seen:
            continue
        seen.add(cur)
        for d in nodes[cur]["deps"]:
            if any(k["kind"] in (None, "normal") for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    result = []
    for i in seen - workspace:
        p = pkgs[i]
        result.append((p["name"], p["version"], p.get("license") or "(라이선스 파일 참고)", p.get("repository") or ""))
    return sorted(result)


def npm_packages():
    app = ROOT / "app"
    out = subprocess.run(
        "npm ls --omit=dev --all --json", cwd=app, capture_output=True, text=True, encoding="utf-8", shell=True,
    ).stdout
    tree = json.loads(out or "{}")
    found = {}

    def walk(deps):
        for name, info in (deps or {}).items():
            key = (name, info.get("version", ""))
            if key in found:
                continue
            pj = app / "node_modules" / name / "package.json"
            lic, repo = "(라이선스 파일 참고)", ""
            if pj.exists():
                data = json.loads(pj.read_text(encoding="utf-8"))
                lic = data.get("license") or lic
                if isinstance(lic, dict):
                    lic = lic.get("type", "")
                r = data.get("repository")
                repo = r.get("url", "") if isinstance(r, dict) else (r or "")
            found[key] = (name, info.get("version", ""), lic, repo)
            walk(info.get("dependencies"))

    walk(tree.get("dependencies"))
    return sorted(found.values())


def section(title, rows):
    by_license = defaultdict(list)
    for name, ver, lic, repo in rows:
        by_license[lic].append((name, ver, repo))
    lines = [f"## {title} ({len(rows)}개)\n"]
    for lic in sorted(by_license):
        lines.append(f"### {lic}\n")
        for name, ver, repo in by_license[lic]:
            repo = repo.replace("git+", "").removesuffix(".git")
            lines.append(f"- {name} {ver}" + (f" — {repo}" if repo else ""))
        lines.append("")
    return "\n".join(lines)


def main():
    rust = rust_packages()
    npm = npm_packages()
    parts = [
        "# 제3자 소프트웨어 고지 (Third-Party Notices)\n",
        "Lantern에는 아래 오픈소스 소프트웨어와 자산이 포함되어 있습니다. 각 구성 요소의 저작권은 해당 저작자에게 있으며,",
        "각 라이선스 전문은 해당 저장소 또는 패키지에 포함된 LICENSE 파일에서 확인할 수 있습니다.\n",
        "이 파일은 `python scripts/gen_notices.py`로 생성합니다. 직접 고치지 마세요.\n",
        "## 자산\n",
        *[f"- **{n}** — {l} — {u}\n  {note}" for n, l, u, note in ASSETS],
        "",
        section("Rust 크레이트", rust),
        section("JavaScript 패키지", npm),
    ]
    (ROOT / "THIRD_PARTY_NOTICES.md").write_text("\n".join(parts), encoding="utf-8", newline="\n")
    licenses = sorted({r[2] for r in rust + npm})
    print(f"Rust {len(rust)}개, npm {len(npm)}개")
    print("라이선스 종류:", ", ".join(licenses))
    risky = [r for r in rust + npm if any(k in r[2].upper() for k in ("GPL", "AGPL", "SSPL", "BUSL")) and "LGPL" not in r[2].upper()]
    for r in risky:
        print("검토 필요:", r[0], r[1], r[2])
    return 1 if risky and "--strict" in sys.argv else 0


if __name__ == "__main__":
    sys.exit(main())
