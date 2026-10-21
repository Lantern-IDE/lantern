#!/usr/bin/env bash
# 영·한 병렬 벤치마크를 모든 방법·모든 질문 언어로 잰다 (모델 호출 없음, 무료. CPU에서 처음에는 몇 시간).
#
#   bash eval/bilingual/run_all.sh <클론 폴더> [aider가 깔린 파이썬]
#
# 미리: cargo build --release -p lantern-context
#       pip install sentence-transformers transformers sentencepiece psutil openpyxl
#       python -m venv .venv-aider && .venv-aider/Scripts/pip install aider-chat==0.86.2   (RepoMap, 선택)
# 순서: 한국어 질문 번역(NLLB)·사전 확장 → Lantern → 구성 요소별 기여 → BM25·임베딩·RRF → RepoMap → Lantern+임베딩 → 규모 → 보고서
# 결과: <클론 폴더>/results/bilingual/, 보고서는 eval/bilingual/results-all.md
set -euo pipefail
DIR=${1:?클론 폴더}
AIDER_PY=${2:-}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
LANTERN="$ROOT/target/release/lantern"
[ -x "$LANTERN" ] || LANTERN="$LANTERN.exe"
RES="$DIR/results/bilingual"
export PYTHONIOENCODING=utf-8
cd "$ROOT"
log() { echo "[$(date +%H:%M:%S)] $*"; }

log "한국어 질문 번역 (NLLB-200), 사전으로 넓힌 한국어 질문"
python eval/bilingual/translate.py
python eval/bilingual/expand.py --lantern "$LANTERN"
# q_ko_llm(LLM이 영어 검색어로 다시 쓴 것)은 한 번 만들어 JSON에 두었다 (README의 '질문 언어'). 다시 만들지 않는다.
# 사람이 직접 쓴 한국어 질문(q_ko_native, native_sheet.py)이 있으면 그것도 잰다
# q_ko_loan(외래어를 많이 쓰게 LLM이 바꾼 한국어, 스트레스 테스트), q_para(영어를 영어로 바꿔 말한 통제)도 저장해 둔 것을 씀
KO=ko,ko-dict,ko-mt,ko-llm,ko-loan
NATIVE=$(python -c "import glob,json;print(any(q.get('q_ko_native') for f in glob.glob('eval/bilingual/*.json') for q in json.load(open(f,encoding='utf-8'))['questions']))")
[ "$NATIVE" = True ] && KO=$KO,ko-native
NATIVE_B=$(python -c "import glob,json;print(any(q.get('q_ko_native_b') for f in glob.glob('eval/bilingual/*.json') for q in json.load(open(f,encoding='utf-8'))['questions']))")
[ "$NATIVE_B" = True ] && KO=$KO,ko-native-b

log "Lantern (en, en-para, $KO)"
node eval/bench/run.mjs --dir "$DIR" --set eval/bilingual --langs en,en-para,$KO --lantern "$LANTERN"
cp eval/bilingual/결과.md "$RES/결과-lantern.md"

log "Lantern 구성 요소별 기여 (단계를 하나씩 끔)"
for step in dict name graph cochange copies; do
  MARK=$(mktemp)
  LANTERN_ABLATE=$step node eval/bench/run.mjs --dir "$DIR" --set eval/bilingual --langs en,ko --lantern "$LANTERN" > /dev/null
  mkdir -p "$RES/lantern-no-$step"
  find "$RES" -maxdepth 1 -name 'retrieval-*' -newer "$MARK" -exec mv {} "$RES/lantern-no-$step/" \;
done
cp "$RES/결과-lantern.md" eval/bilingual/결과.md

log "BM25·임베딩·RRF (multilingual-e5-small)"
# ko-dictw: 사전이 넓힌 낱말을 Lantern처럼 따로, 절반 가중치로 BM25에 결합
for lang in en en-para ${KO//,/ } ko-dictw; do
  python eval/bilingual/baselines.py --dir "$DIR" --lang $lang --methods bm25-file,bm25-chunk,dense,hybrid
done
# 사전 가중치 민감도 (0.5가 기본, 1.0은 그냥 덧붙인 ko-dict와 같음)
for w in 0.25 0.75; do
  python eval/bilingual/baselines.py --dir "$DIR" --lang ko-dictw --dict-weight $w --methods bm25-file,bm25-chunk
done
# 더 큰 임베딩 모델은 따로 (예: --embed-model BAAI/bge-m3 --tag bgem3). CPU에서 조각 약 5만 8천 개를 새로 만들어야 해서
# 4코어로 19시간쯤 걸려 기본 순서에서는 뺐다. 벡터는 문항마다 저장되므로 멈췄다가 이어서 돌릴 수 있다

if [ -n "$AIDER_PY" ]; then
  log "Aider RepoMap"
  for lang in en ko ko-mt; do
    "$AIDER_PY" eval/bilingual/repomap_baseline.py --dir "$DIR" --lang $lang
  done
fi

log "Lantern + 임베딩 (같은 multilingual-e5-small)"
python eval/bilingual/embed_server.py --port 8711 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true' EXIT
for i in $(seq 1 90); do curl -s -m 2 -X POST http://127.0.0.1:8711/v1/embeddings -d '{"input":["x"]}' >/dev/null && break; sleep 2; done
MARK=$(mktemp)
LANTERN_EMBED_URL=http://127.0.0.1:8711/v1 LANTERN_EMBED_MODEL=intfloat/multilingual-e5-small \
LANTERN_EMBED_QUERY="query: {query}" LANTERN_EMBED_DOC="passage: {path}
{text}" \
  node eval/bench/run.mjs --dir "$DIR" --set eval/bilingual --langs en,en-para,${KO/,ko-dict/} --lantern "$LANTERN"
mkdir -p "$RES/lantern-sem"
find "$RES" -maxdepth 1 -name 'retrieval-*' -newer "$MARK" -exec mv {} "$RES/lantern-sem/" \;
kill $SERVER 2>/dev/null || true
# 의미 검색 실행이 덮어쓴 결과.md를 Lantern 기본 결과로 되돌림
cp eval/bilingual/결과.md "$RES/결과-lantern-sem.md"
cp "$RES/결과-lantern.md" eval/bilingual/결과.md

log "규모 (VSCODE=<vscode 클론>을 주면 큰 저장소도), 혼합 검색 비용"
python eval/bilingual/scale.py --dir "$DIR" --lantern "$LANTERN" ${VSCODE:+--extra vscode=$VSCODE}   # 논문의 vscode는 커밋 76e9dff
python eval/bilingual/hybrid_cost.py --dir "$DIR" --threads 4 --out "$RES/../hybrid_cost.json"

log "보고서"
python eval/bilingual/report.py --dir "$DIR" --md eval/bilingual/results-all.md > /dev/null
python eval/bilingual/report.py --dir "$DIR" --subset test --md eval/bilingual/results-test.md > /dev/null
python eval/bilingual/report.py --dir "$DIR" --subset unseen --md eval/bilingual/results-unseen.md > /dev/null   # 논문 주 결과 (개발 중에 보지 않은 문항)
python eval/bilingual/report.py --dir "$DIR" --subset fresh-repos --md eval/bilingual/results-fresh-repos.md > /dev/null   # 개발 중에 한 번도 보지 않은 7개 저장소
log "끝: eval/bilingual/results-all.md, results-test.md, results-unseen.md, results-fresh-repos.md"
