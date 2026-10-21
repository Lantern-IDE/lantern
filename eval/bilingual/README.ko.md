# 영·한 병렬 벤치마크 (eval/bilingual)

[English](README.md)

같은 정답 파일에 영어 질문과 한국어 질문을 짝지어, **질문 언어만 바꿨을 때** 맥락 검색이 얼마나 달라지는지 잽니다. [eval/bench](../bench)(저장소 3개, 60문항)를 키운 것입니다.

| | |
|---|---|
| 저장소 | 10개, 언어 8개 (아래 표) |
| 문항 | 320개 (저장소마다 최근 커밋 32개), 그중 2026년 커밋 212개 |
| 질문 | 영어: 커밋 메시지 제목. 한국어: LLM 초안을 별도 LLM이 다시 검토 (320개 중 4개 고침) |
| 정답 | 그 커밋이 고친 코드 파일 (테스트·빌드·예제·문서 제외). 1개 250문항, 2개 60문항, 3개 10문항 |
| 평가 시점 | 그 커밋의 부모 (고치기 전 코드에서 찾음) |

| 저장소 | 언어 | 기준 커밋 |
|---|---|---|
| [flask](https://github.com/pallets/flask) | Python | `d73fa1cdcb` |
| [httpx](https://github.com/encode/httpx) | Python | `b5addb64f0` |
| [hono](https://github.com/honojs/hono) | TypeScript | `ee0622e144` |
| [vite](https://github.com/vitejs/vite) | TypeScript | `74ae7099f9` |
| [commons-lang](https://github.com/apache/commons-lang) | Java | `29624cdb50` |
| [gin](https://github.com/gin-gonic/gin) | Go | `0f09c3a9b4` |
| [axum](https://github.com/tokio-rs/axum) | Rust | `fc78d5bcdb` |
| [okhttp](https://github.com/square/okhttp) | Kotlin | `5a4b597d11` |
| [axios](https://github.com/axios/axios) | JavaScript | `326843a390` |
| [jq](https://github.com/jqlang/jq) | C | `6b53f80311` |

## 데이터 형식

저장소마다 JSON 하나: `project`, `url`, `ref`(기준 커밋), `questions`. 문항마다

| 필드 | 뜻 |
|---|---|
| `q` | 영어 질문 (커밋 메시지 제목, 앞의 `fix:` 같은 접두어와 PR 번호는 뺌) |
| `q_ko_draft` | 이 엔진을 모르는 LLM 에이전트가 쓴 한국어 초안 |
| `q_ko` | 검토한 한국어 질문 (검토 전에는 `null`). 누가 검토했는지는 `q_ko_reviewed` (`"llm"`, 사람이 검토표로 했으면 `true`) |
| `q_ko_mt` | `q_ko`를 NLLB-200 600M으로 다시 영어로 옮긴 것 ('번역한 뒤 검색' 비교용) |
| `q_ko_dict`, `q_ko_dict_terms` | `q_ko`에 Lantern의 한국어 사전이 넓힌 영어 낱말(`q_ko_dict_terms`, 사전이 안 걸리면 빈 목록)을 붙인 것. 비교 대상에도 같은 사전을 주려고 |
| `q_ko_llm` | `q_ko`를 LLM이 영어 코드 검색어로 다시 쓴 것 (아래), 'LLM이 다시 쓴 뒤 검색' 비교용 |
| `q_ko_native`, `q_ko_native_by` | 선택: 사람이 처음부터 쓴 한국어 질문 ([native_sheet.py](native_sheet.py)) |
| `q_para` | `q`를 LLM이 영어로 바꿔 말한 것 (말 바꿈 통제, 아래) |
| `q_ko_loan` | `q_ko`를 LLM이 외래어를 많이 쓰게 바꾼 것 (스트레스 테스트, 아래) |
| `golden` | 정답 파일 (저장소 기준 경로) |
| `commit`, `parent`, `date` | 커밋, 그 부모(찾는 시점), 커밋 날짜 |

## 만드는 법

문항은 [make.mjs](../bench/make.mjs)가 커밋 이력에서 자동으로 뽑습니다. 사람이 문항이나 정답을 고르지 않습니다.

```bash
node eval/bench/make.mjs --repo <클론> --name <이름> --url <저장소 주소> --count 32 --ref <기준 커밋> --out eval/bilingual
```

거르는 커밋: 병합, 버전·문서·빌드·린트·의존성 손질, 새로 만들거나 지우거나 옮긴 파일이 있는 커밋, 코드 파일 4개 이상, 메시지에 정답 파일 이름이 나오는 커밋, 같은 정답 파일 묶음은 2개까지. 정답에서 빼는 파일: 테스트(`test/`, `jvmTest/`, `*_test.go`, `…Test.kt`, 테스트 지원 모듈 등), Gradle 빌드 스크립트와 빌드 로직, `examples/`·`docs/`·`playground/`·`scripts/`·`benchmarks/`.

## 한국어 질문

두 단계로 만듭니다.

1. **초안** (`q_ko_draft`): 이 엔진(과 그 한국어 사전)을 모르는 LLM 에이전트에게 영어 질문과 작성 기준만 주고 쓰게 했습니다. 저장소·커밋·코드는 보지 않았습니다. 엔진을 아는 쪽이 쓰면 사전에 있는 낱말로 기울 수 있어서입니다
2. **검토** (`q_ko`): 역시 엔진을 모르는 두 번째 LLM 에이전트가 같은 기준(뜻, 부정·조건, 없는 정보 추가 여부, 자연스러움)으로 모든 초안을 보고 320개 중 4개를 고쳤습니다. 아직 사람이 검토하지는 않았습니다. 아래 검토표로 사람이 검토할 수 있고, 검토한 문항만 `q_ko`가 되어 한국어 결과는 검토한 문항만으로 잽니다

```bash
python eval/bilingual/ko_sheet.py export 검토표.xlsx   # 질문·초안 → 검토표
python eval/bilingual/ko_sheet.py import 검토표.xlsx   # 검토표 → 각 JSON의 q_ko (고친 비율도 알려 줌)
```

- 작성·검토 기준: 한국 개발자가 실제로 묻듯이 자연스럽게, 영어 질문에 없는 정보는 더하지 않음, 코드 이름은 영어 그대로 둬도 됨, 바뀐 코드(diff)는 보지 않음 (검토표의 '검토 안내' 시트)
- 검토표에는 정답 파일을 넣지 않습니다. 파일 이름을 보면 질문에 힌트가 섞이기 때문입니다
- 질문으로 만들기 어려운 문항은 비워 두고 `q_ko_note`에 이유를 남깁니다

**번역한 뒤 검색** 비교용으로 한국어 질문을 로컬 번역 모델(NLLB-200 600M, CC-BY-NC 4.0)로 다시 영어로 옮겨 `q_ko_mt`에 둡니다 (`python eval/bilingual/translate.py`).

## 한국어 질문의 다른 형태

모든 방법에 같은 입력을 줘서, 단계마다의 효과를 가릅니다.

- **한국어 + 사전** (`q_ko_dict`, [expand.py](expand.py)): Lantern의 사전이 넓힌 영어 낱말(`lantern context --json`의 `query_terms` 중 질문에 원래 있던 식별자에서 나오지 않은 영어 낱말)을 한국어 질문 뒤에 붙입니다. BM25와 임베딩 비교 대상도 Lantern과 같은 사전을 쓰게 됩니다. 사전이 걸리는 문항은 320개 중 236개(개발 중에 보지 않은 261개 중 191개)입니다. Lantern은 같은 사전을 스스로 적용하므로 이 형태로는 재지 않습니다
- **한국어 → LLM 재작성** (`q_ko_llm`): 코딩 에이전트가 모델 호출 한 번을 더 써서 할 수 있는 일입니다. LLM 에이전트(Claude, Claude Code 하위 에이전트)에게 한국어 질문과 저장소 이름만 주고, 영어 README에 적은 지시문으로 320개를 한 번에 쓰게 했습니다 (식별자는 그대로, 질문에 없는 정보는 더하지 않음, 저장소·코드·웹은 보지 않음)
  - 모델은 Claude Opus 5.5(`claude-opus-5-5`)로, 한국어 질문의 초안·검토를 맡은 모델과 같습니다. [leak.py](leak.py)로 점검하면 개발 중에 보지 않은 261문항에서 한국어 질문에 없던 식별자나 정답 파일 이름을 더한 재작성은 없지만, 47개(18%)는 영어 커밋 제목과 글자 그대로 같고 원문과 낱말이 겹치는 정도도 기계 번역보다 큽니다(Jaccard 0.71 대 0.42). 그 47개를 빼도 혼합 방식은 처음 3개 66.4(영어 68.7, 한국어 56.5)입니다. 원문 그대로 되돌아오는 비율은 모델 학습 데이터 시점(2026년 6월) 뒤 커밋에서도 비슷해서(15.5% 대 이전 20.0%, Fisher p=0.42) 커밋 이력을 기억해서라기보다 번역이 투명해서로 봅니다. 같은 모델이라 유리할 수 있는 강한 참고점으로 보고, 상한으로 보지 않습니다. 비용([rewrite_latency.py](rewrite_latency.py), 질문마다 Claude Code로 한 번 호출, 30문항): API 시간 중앙값 2.6초(90% 4.8초), 출력 약 29토큰, 정가 기준 질문당 약 $0.01
- **영어 바꿔 말하기** (`q_para`, `en-para`): 말을 바꾼 것만의 효과를 보는 통제입니다. 같은 모델이 영어 커밋 제목을 한국어 초안과 같은 기준(자연스럽게, 요청·질문·이슈 제목을 섞어, 식별자 그대로, 정보 추가 없음)으로 영어 그대로 다시 썼습니다. `en` → `en-para` 하락이 말 바꿈의 몫이고, `ko`까지 남은 하락이 언어의 몫입니다
- **외래어를 많이 쓴 한국어** (`q_ko_loan`, `ko-loan`): 사람이 쓴 질문이 아니라 스트레스 테스트입니다. 같은 모델이 한국어 질문을 외래어를 많이 쓰는 말투로 바꿨습니다(요청 → 리퀘스트, 기본값 → 디폴트, progress → 프로그레스). 식별자와 뜻은 그대로이고, 320개 중 51개는 바꿀 것이 없었습니다. 개발 중에 보지 않은 261문항에서 사전이 걸리는 문항이 147개(56%, 원래 191개)로 줄고 영어 낱말이 없는 문항이 56개(원래 27개)로 늡니다. 이때 BM25(조각)·혼합·Lantern이 처음 3개에서 8.0·7.3·6.1점 더 떨어지고, Lantern(49.8)은 여전히 혼합(48.7)과 비깁니다
- **한국어 + 사전, 절반 가중** (`ko-dictw`, 비교 대상만): 같은 사전 낱말을 Lantern처럼 따로, 절반 가중치로 셉니다 (질문 낱말의 BM25 점수 + 넓힌 낱말의 BM25 점수 × 0.5). 흔한 동사가 질문의 식별자를 누르지 않게 하려는 것입니다. 임베딩 쪽은 `ko-dict`와 같은 글을 받습니다. 민감도(`--dict-weight`, 개발 중에 보지 않은 261문항): BM25(조각) 처음 3개가 사전 없이 51.3, 가중치 0.25 / 0.5 / 0.75 / 1.0(1.0은 그냥 덧붙인 `ko-dict`)에서 54.0 / 55.9 / 55.6 / 54.4라, 정확한 가중치보다 사전이 있느냐가 중요합니다
- **개발자가 쓴 질문** (`q_ko_native`, `q_ko_native_b`): [native_sheet.py](native_sheet.py)가 개발 중에 한 번도 보지 않은 7개 저장소에서 저장소마다 6개(42개)를 골라, 작성표와 함께 커밋이 바꾼 코드를 파일 이름을 가리고 보여 주는 HTML을 만듭니다. 영어 커밋 제목은 보여 주지 않습니다(보여 주면 번역하게 됨). 개발자가 번역 도구 없이 "이 변경을 에이전트에게 맡긴다면 어떻게 말할지"를 쓰고, 두 번째 개발자가 같은 문항을 따로 쓸 수 있습니다(B). 작성자는 부호로만 적고, 쓴 질문을 공개하는 데 동의를 받습니다. 분석은 [native-plan.md](native-plan.md)에 미리 정해 두었고, `import`는 계획대로 정답 파일 이름이 들어간 질문을 뺍니다

```bash
python eval/bilingual/native_sheet.py export 작성표.xlsx --dir <클론 폴더>   # 작성표.html도 만듦
python eval/bilingual/native_sheet.py import 작성표.xlsx
```

## 재기

```bash
cargo build --release -p lantern-context
bash eval/bilingual/run_all.sh <저장소를 받을 폴더> [aider가 깔린 파이썬]
```

한국어 질문 번역(NLLB)·사전 확장 → Lantern → BM25·임베딩·RRF → Aider RepoMap → Lantern+임베딩 → 규모 순으로 질문 형태마다 재고, [results-all.md](results-all.md)(전체), `results-test.md`(평가용 세트), `results-unseen.md`(주 결과)를 씁니다. 모델 API 호출이 없어 무료지만 (`q_ko_llm`은 저장해 둔 것을 씀), CPU에서 처음 임베딩을 만드느라 몇 시간 걸립니다 (벡터는 캐시해서 다음부터 빠름).

| 스크립트 | 하는 일 |
|---|---|
| [../bench/run.mjs](../bench/run.mjs) `--set eval/bilingual` | 키워드 검색 2가지와 Lantern |
| [baselines.py](baselines.py) | BM25(파일·40줄 조각), 다국어 임베딩(multilingual-e5-small. 다른 모델은 `--embed-model BAAI/bge-m3 --tag bgem3`처럼 따로, 4코어로 19시간쯤 걸려 `run_all.sh`에서는 뺌), BM25+임베딩 RRF. 낱말: 식별자는 Lantern처럼 통째로 + 낱말로 쪼갬, 한글은 붙어 있는 덩어리 그대로 (한국어 주석·문자열에만 맞음) |
| [expand.py](expand.py) | 한국어 질문 + Lantern 사전이 넓힌 영어 낱말(`q_ko_dict`), 사전 적중률 |
| [prompts.md](prompts.md) | LLM 단계마다 쓴 지시문 원문: 한국어 초안, 검토, LLM 재작성, 영어 바꿔 말하기 통제, 외래어 스트레스 테스트 |
| [ko_rating.py](ko_rating.py) | 저자가 LLM 한국어 질문 30개(무작위)를 고치지 않고 자연스러움·뜻만 평가하는 표. 기준은 [ko-rating-plan.md](ko-rating-plan.md)에 미리 정함 |
| [native_sheet.py](native_sheet.py) | 사람이 처음부터 쓰는 한국어 질문 표 (`q_ko_native`) |
| [leak.py](leak.py) | LLM 재작성 누출 점검 (새 식별자, 정답 파일 이름, 영어 원문과의 겹침, 모델 학습 시점 전후의 원문 복원 비율) |
| [hybrid_cost.py](hybrid_cost.py) | 같은 CPU에서 BM25+임베딩 혼합의 비용: 모든 조각을 처음부터 임베딩하는 시간, 질문당 시간 |
| [scale_questions.txt](scale_questions.txt) | 벤치마크 밖 큰 저장소의 조립 시간만 잴 질문 (`scale.py --extra vscode=<클론>`) |
| [rewrite_latency.py](rewrite_latency.py) | LLM 재작성의 지연 시간·토큰·비용 (질문마다 실제 모델 호출, 계정 사용량을 씀) |
| [errors.py](errors.py) | Lantern이 영어로만 맞힌 문항. 원인은 [error-analysis.md](error-analysis.md)에서 사람이 나눔 (26개 중 16개가 사전에 없는 낱말, 그중 11개가 외래어) |
| [repomap_baseline.py](repomap_baseline.py) | Aider RepoMap (aider-chat 0.86.2, 질문 낱말을 언급된 식별자로) |
| [embed_server.py](embed_server.py) | Lantern 의미 검색을 같은 임베딩 모델로 재는 로컬 `/embeddings` 서버 |
| [translate.py](translate.py) | 한국어 질문 → 영어 기계 번역 (번역한 뒤 검색 비교) |
| [scale.py](scale.py) | 인덱싱 시간·메모리·인덱스 크기·질문당 조립 시간 |
| [report.py](report.py) | Recall@8k, R@10f, Acc@1/3/5, MRR, 부트스트랩 95% 구간 (문항 단위, 핵심 비교는 저장소 단위도), McNemar와 Holm 보정, 저장소별·문항 묶음별 표, 논문용 LaTeX 표(`--tex`) |

주 결과는 [eval/bench](../bench)와 겹치지 않는 261문항입니다 (`report.py --subset unseen`, `results-unseen.md`). eval/bench는 엔진의 순위·사전을 고칠 때 확인에 썼기 때문입니다. 평가용·개발용 세트: 커밋 해시 앞 8자리 정수를 10으로 나눈 나머지가 0~2면 개발용(약 30%), 나머지는 평가용입니다. 엔진의 사전·가중치는 개발용으로만 맞추고, 보고는 평가용으로 합니다.

## 개발 중에 한 번도 보지 않은 저장소

개발 때 쓴 59문항은 flask, hono, commons-lang에서만 나왔습니다. 나머지 7개 저장소(224문항, `report.py --subset fresh-repos`, [results-fresh-repos.md](results-fresh-repos.md))에서도 결론이 같습니다: 영어에서 Lantern이 혼합보다 낮고(처음 3개 −8.9), 한국어에서 비기고(+1.3), 영어보다 덜 떨어지는 차이는 저장소 단위로도 유의하며(+10.3, [1.8, 19.2]), 사전을 빼면 4.9 깎이고, 사전을 절반 가중으로 준 BM25(조각)는 Lantern과 같습니다. 저장소가 7~10개뿐이라 저장소 단위 구간은 근사입니다. 개발에 쓴 3개 저장소의 보지 않은 37문항(`--subset dev-repos`, [results-dev-repos.md](results-dev-repos.md))에서는 Lantern이 오히려 유리하지 않습니다: 혼합 대비 영어 +2.7, 한국어 −8.1, 한국어 하락이 혼합보다 4문항 큼. 그 저장소들에 과적합됐다면 반대여야 합니다.

## Lantern 비용 측정에 대해

`scale.py`는 빈 인덱스에서 처음 인덱싱을 세 번 재고 중앙값을 씁니다(파일 캐시가 데워진 상태. 막 받은 vscode는 첫 번에 92초, 데워진 뒤 세 번의 중앙값은 42초). Lantern은 파일 해석에 스레드를 8개까지 씁니다. vscode는 커밋 `76e9dff`에서 [scale_questions.txt](scale_questions.txt)의 질문 10개로 잽니다 (예전 초안의 136ms는 즉석 질문 두 개로 잰 값). 모든 측정에서 엔진은 같습니다(엔진의 마지막 변경 d108680). 다시 빌드한 실행 파일로 gin·flask를 재면 파일 목록이 똑같습니다.

## 혼합 검색의 비용

[hybrid_cost.py](hybrid_cost.py)로 Lantern 규모 표와 같은 CPU(Intel Core i5-11400)에서 쟀습니다. multilingual-e5-small 4스레드, 저장소 기준 커밋의 모든 조각을 처음부터 임베딩, 질문당 시간은 BM25 + 질문 임베딩 + 유사도 + RRF + 담기(한국어 질문 10개 중앙값). Lantern은 같은 저장소를 2~5초에 인덱싱하고 5~12ms에 맥락을 조립합니다.

| 저장소 | 조각 | BM25 인덱스 (초) | 임베딩 (초) | 질문당 (ms) |
|---|---|---|---|---|
| axios | 1,393 | 1.7 | 137 | 16 |
| axum | 1,357 | 1.2 | 139 | 17 |
| commons-lang | 5,003 | 4.2 | 588 | 29 |
| flask | 502 | 0.4 | 54 | 17 |
| gin | 661 | 0.7 | 78 | 20 |
| hono | 2,380 | 2.1 | 262 | 18 |
| httpx | 473 | 0.5 | 54 | 19 |
| jq | 1,251 | 0.4 | 147 | 18 |
| okhttp | 4,025 | 3.4 | 433 | 25 |
| vite | 3,877 | 2.5 | 334 | 33 |

## 한계

- 커밋 메시지는 고친 뒤에 쓴 글이라, 실제 사용자 질문보다 식별자가 많이 들어 있습니다 (파일 이름이 나오는 커밋만 거름)
- 정답은 '고친 파일'이라, 읽어야 했지만 고치지 않은 파일은 정답에 없습니다
- 한국어 질문은 LLM이 옮기고 LLM이 검토한 것이라, 한국 개발자가 직접 쓴 질문이 아니고 LLM 말투가 일부 남을 수 있습니다
- 초안이 영어에서 나왔으므로, 같은 계열의 번역기로 다시 옮기면 원문에 가깝게 돌아가 '번역한 뒤 검색'이 실제보다 유리할 수 있습니다 (그래서 초안을 쓴 LLM과 다른 번역 모델을 씀). 초안을 쓴 LLM과 같은 계열인 'LLM 재작성'은 더 유리할 수 있습니다
- Lantern의 한국어 사전은 AI 코딩 도구로 썼고 한국어 질문은 LLM이 썼으므로, 같은 계열의 모델이면 사전을 보지 않고도 낱말이 겹칠 수 있습니다. 사람이 쓴 질문(`q_ko_native`)으로 이것을 재려고 합니다
- 임베딩은 작은 multilingual-e5-small만 썼습니다 (Lantern+임베딩도 같은 모델이라 그 비교는 맞춰져 있음). BGE-M3 같은 더 큰 다국어 모델은 재지 않았습니다
- 저장소가 10개뿐이라 저장소 단위 부트스트랩 구간이 넓고, 저장소마다 결과가 다릅니다 (보고서의 저장소별 표)
- 인기 저장소라 오래된 커밋은 모델 학습 데이터에 들어 있을 수 있습니다. 2026년 커밋만 따로 잴 수 있게 커밋 날짜를 함께 봅니다

## 라이선스

문항 파일은 위 오픈소스 저장소의 커밋 제목과 파일 경로를 씁니다 (MIT, Apache-2.0, BSD, jq 라이선스. 각 저장소 참고). 한국어 질문·스크립트·주석은 Lantern과 함께 Apache-2.0입니다.
