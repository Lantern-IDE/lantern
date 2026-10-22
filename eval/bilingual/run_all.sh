#!/usr/bin/env bash
# 영·한 병렬 벤치마크를 모든 방법·모든 질문 형태로 잰다 (모델 API 호출 없음, 무료. CPU에서 처음에는 몇 시간).
#
#   bash eval/bilingual/run_all.sh <클론 폴더> [aider가 깔린 파이썬]
#   VSCODE=<vscode 클론> bash eval/bilingual/run_all.sh ...   (규모 표의 vscode 행. 클론을 커밋 76e9dff로 꺼내 둘 것)
#
# 미리: README의 'Reproducing' (Rust·Node·Python 버전, pip install -r eval/bilingual/requirements.txt, aider 가상환경)
# 순서: 파일 규칙 점검 → 사전 확장 (번역·LLM이 만든 형태는 저장된 것) → Lantern → 구성 요소별 기여 → BM25·임베딩·RRF
#       → RepoMap → Lantern+임베딩 → 규모·혼합 검색 비용·번역 비용 → 보고서·논문 표·누출 점검·오류 후보
# 결과: <클론 폴더>/results/bilingual/ (논문 표는 그 밑 tables/), 보고서는 eval/bilingual/results-*.md,
#       시간 측정 원자료는 eval/bilingual/measurements/
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

log "한국어 질문 번역 (저장된 NLLB 번역을 씀. q_ko_mt_src를 지우면 다시 옮김), 사전으로 넓힌 한국어 질문"
python eval/bilingual/translate.py
python eval/bilingual/expand.py --lantern "$LANTERN"
# LLM이 만든 형태(q_ko_draft·q_ko, q_ko_llm, q_para, q_ko_loan)는 JSON에 저장된 것을 쓴다 (지시문은 prompts.md).
# q_ko_llm을 다시 만들려면 rewrite.py (질문마다 모델을 한 번씩 부르므로 계정 사용량을 씀)
# 사람이 직접 쓴 한국어 질문(q_ko_native, native_sheet.py)이 있으면 그것도 잰다
KO=ko,ko-dict,ko-mt,ko-llm,ko-loan
NATIVE=$(python -c "import glob,json;print(any(q.get('q_ko_native') for f in glob.glob('eval/bilingual/*.json') for q in json.load(open(f,encoding='utf-8'))['questions']))")
[ "$NATIVE" = True ] && KO=$KO,ko-native
NATIVE_B=$(python -c "import glob,json;print(any(q.get('q_ko_native_b') for f in glob.glob('eval/bilingual/*.json') for q in json.load(open(f,encoding='utf-8'))['questions']))")
[ "$NATIVE_B" = True ] && KO=$KO,ko-native-b
# 같은 설정으로 다시 부른 재작성(q_ko_llm_2, q_ko_llm_3)과 저장소 이름을 뺀 통제(q_ko_llm_norepo, rewrite.py --no-repo)가 있으면 그것도 잰다
for extra in q_ko_llm_2:ko-llm2 q_ko_llm_3:ko-llm3 q_ko_llm_norepo:ko-llm-norepo; do
  HAS=$(python -c "import glob,json,sys;print(any(q.get(sys.argv[1]) for f in glob.glob('eval/bilingual/*.json') for q in json.load(open(f,encoding='utf-8'))['questions']))" "${extra%%:*}")
  [ "$HAS" = True ] && KO=$KO,${extra##*:}
done

log "파일 규칙 점검: 엔진·비교 대상·키워드 검색이 같은 파일을 보는지 (저장소마다 커밋 4곳)"
python eval/bilingual/check_files.py --dir "$DIR" --lantern "$LANTERN"

log "Lantern (en, en-para, $KO)"
# Lantern은 점수가 같으면 심볼 id로 순서를 정하고, id는 색인해 온 순서에 따라 달라진다(#139).
# 지난 측정이 남긴 인덱스에서 시작하면 결과가 조금 달라지므로 늘 빈 인덱스에서 시작한다
rm -rf "$RES"/index-*
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

log "규모 (VSCODE=<vscode 클론>을 주면 큰 저장소도), 혼합 검색 비용, 번역 비용 (시간 측정이라 다른 작업 없이)"
mkdir -p "$RES/tables"
python eval/bilingual/scale.py --dir "$DIR" --lantern "$LANTERN" ${VSCODE:+--extra vscode=$VSCODE} --json eval/bilingual/measurements/scale.json --tex "$RES/tables/scale.tex"
python eval/bilingual/hybrid_cost.py --dir "$DIR" --threads 4 --out eval/bilingual/measurements/hybrid_cost.json
python eval/bilingual/nllb_cost.py --threads 4

log "보고서, 논문 표 (results.tex, scale.tex), 누출 점검, 오류 후보"
python eval/bilingual/report.py --dir "$DIR" --md eval/bilingual/results-all.md > /dev/null
python eval/bilingual/report.py --dir "$DIR" --subset test --md eval/bilingual/results-test.md > /dev/null
python eval/bilingual/report.py --dir "$DIR" --subset unseen --md eval/bilingual/results-unseen.md --tex "$RES/tables/results.tex" > /dev/null   # 논문 주 결과와 표 I
python eval/bilingual/report.py --dir "$DIR" --subset fresh-repos --md eval/bilingual/results-fresh-repos.md > /dev/null   # 개발 중에 한 번도 보지 않은 7개 저장소
python eval/bilingual/report.py --dir "$DIR" --subset dev-repos --md eval/bilingual/results-dev-repos.md > /dev/null   # 개발에 쓴 3개 저장소의 보지 않은 문항 (과적합 점검)
python eval/bilingual/leak.py --dir "$DIR" --md eval/bilingual/leak-check.md > /dev/null
python eval/bilingual/errors.py --dir "$DIR" > "$RES/error-candidates.md"   # 분류는 사람이 읽고 붙인다 (error-analysis.md)
log "끝: eval/bilingual/results-*.md, leak-check.md, measurements/, $RES/tables/"
