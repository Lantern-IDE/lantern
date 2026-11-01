#!/usr/bin/env bash
# CLI 바이너리 실행 확인: 판 번호, 그리고 임시 git 저장소에서 index와 context.
# 릴리스 워크플로(cli-release.yml)가 올리기 전에 돌리고, 손으로도 돌릴 수 있다.
# 사용: bash scripts/cli_smoke.sh <바이너리> <기대하는 버전>
set -euo pipefail

bin=${1:?바이너리 경로}
want=${2:?기대하는 버전}
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"

got=$("$bin" --version)
if [ "$got" != "lantern $want" ]; then
  echo "판이 다릅니다: '$got' (기대: 'lantern $want')" >&2
  exit 1
fi
echo "--version: $got"

repo=$(mktemp -d)
trap 'rm -rf "$repo"' EXIT
# Windows의 Git Bash에서 /tmp 경로를 실행 파일 인자로 넘기지 않으려고 폴더 안에서 돌린다
cd "$repo"

# 질문에 없는 문자열(본문)이 나와야 질문을 되풀이한 출력과 구별된다
cat > smoke_target.py <<'PY'
def lantern_smoke_marker():
    return "smoke-body-31415"
PY
git init -q
git add smoke_target.py
git -c user.name=smoke -c user.email=smoke@example.invalid commit -q -m smoke

"$bin" -C . index </dev/null
out=$("$bin" -C . context "what does lantern_smoke_marker return" </dev/null)
if ! grep -q "smoke-body-31415" <<<"$out"; then
  echo "context 출력에 인덱싱한 코드가 없습니다:" >&2
  echo "$out" >&2
  exit 1
fi
echo "index + context: ok"
