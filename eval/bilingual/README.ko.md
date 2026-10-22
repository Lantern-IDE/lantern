# 영·한 병렬 벤치마크 (eval/bilingual)

[English](README.md)

같은 정답 파일에 영어 질문과 한국어 질문을 짝지어, **질문 언어만 바꿨을 때** 맥락 검색이 얼마나 달라지는지 잽니다. [eval/bench](../bench)(저장소 3개, 60문항)를 키운 것입니다.

| | |
|---|---|
| 저장소 | 10개, 언어 8개 (아래 표) |
| 문항 | 320개 (저장소마다 최근 커밋 32개), 그중 2026년 커밋 212개 |
| 질문 | 영어: 커밋 메시지 제목. 한국어: LLM 초안을 별도 LLM이 다시 검토 (320개 중 4개 고침) |
| 정답 | 그 커밋이 고친 코드 파일 (테스트·빌드·예제·문서 제외). 1개 249문항, 2개 61문항, 3개 10문항 |
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
| `q_ko_llm`, `q_ko_llm_src` | `q_ko`를 LLM이 질문마다 한 번씩 불려 영어 코드 검색어로 다시 쓴 것 (아래), 'LLM이 다시 쓴 뒤 검색' 비교용. `q_ko_llm_src`는 다시 쓸 때 받은 한국어 |
| `q_ko_llm_2`, `q_ko_llm_3`, `q_ko_llm_norepo` | 같은 설정으로 두 번 더 부른 KO→LLM 재작성과, 저장소 이름 없이 부른 재작성(모델의 저장소 지식을 가르는 통제). 각각 `_src` 필드가 있음 |
| `q_ko_native`, `q_ko_native_by` | 선택: 사람이 처음부터 쓴 한국어 질문 ([native_sheet.py](native_sheet.py)) |
| `q_para` | `q`를 LLM이 영어로 바꿔 말한 것 (말 바꿈 통제, 아래) |
| `q_ko_loan`, `q_ko_loan_dict_terms` | `q_ko`를 LLM이 외래어를 많이 쓰게 바꾼 것 (스트레스 테스트, 아래)과 거기에 사전이 넓힌 영어 낱말 |
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
- **한국어 → LLM 재작성** (`q_ko_llm`, [rewrite.py](rewrite.py)): 코딩 에이전트가 모델 호출 한 번을 더 써서 할 수 있는 일입니다. 한국어 질문의 초안·검토를 맡은 것과 같은 Claude Opus 5.5(`claude-opus-5-5`)를 Claude Code 헤드리스 모드로 도구 없이 질문마다 한 번씩 부르고, 저장소 이름과 한국어 질문 하나만 [prompts.md](prompts.md) 3번 지시문(식별자는 그대로, 질문에 없는 정보는 더하지 않음, 검색어만 답함)과 함께 줬습니다
  - [leak.py](leak.py)로 점검합니다([leak-check.md](leak-check.md)). 개발 중에 보지 않은 261문항에서 44개(17%)는 한국어 질문에 없던 코드 식별자를, 24개(9%)는 정답 파일 이름을 더했습니다(`router`, `logger`처럼 평범한 낱말인 경우가 많음). 합쳐 61개로, 모델이 이 인기 저장소들을 알고 있다는 뜻으로 봅니다. 영어 커밋 제목을 글자 그대로 되살린 것은 9개(3%)뿐이고, 모델 학습 데이터 시점(2026년 6월) 전후 커밋에서 비율이 같아(5/145 대 4/116) 커밋 자체를 기억한 것은 아닙니다. 재작성은 커밋 제목보다 길어서(평균 10낱말 대 7낱말) 혼합 방식이 영어보다도 높습니다(처음 3개 70.9 대 68.2). 의심 문항 61개를 빼도 68.0(영어 65.0, 한국어 57.5)입니다. 저장소에 대한 모델의 지식과 한국어 질문과 같은 모델이라는 점의 덕을 보는 강한 참고점으로 보고, 상한으로 보지 않습니다
  - 비용([measurements/rewrite.json](measurements/rewrite.json), 320번 호출, 통계는 개발에 쓰지 않은 261문항 기준): API 시간 중앙값 3.4초(90% 5.0초), 출력 25토큰, 정가 기준 질문당 약 $0.01. 태그 `icpc2027-eval-v4`까지는 모든 문항을 한 번에 넘긴 일괄 재작성이었습니다(prompts.md)
  - **반복과 통제.** 재작성은 한 번씩 뽑은 것이라, 같은 설정으로 두 번 더 불렀고(`q_ko_llm_2`, `q_ko_llm_3`), 저장소 이름 없이 한국어 질문만 주는 통제도 만들었습니다(`q_ko_llm_norepo`, `rewrite.py --no-repo`). 세 번에서 혼합은 처음 3개 70.9 / 71.6 / 71.3, 모델 없는 Lantern은 60.5 / 63.6 / 61.7, Lantern + 임베딩은 70.5 / 71.3 / 68.6(영어 67.0, 저장소 이름 없이 65.5)이고, 실행마다 56~61개가 의심 문항입니다. 저장소 이름이 없으면 의심 문항이 11개로 줄고 혼합은 67.4(영어 68.2, 한국어 56.3)입니다: 언어 격차는 재작성만으로 거의 메워지고, 저장소 이름이 모델의 저장소 지식으로 3.4~4.2점을 더합니다. 이 지식은 커밋이 아니라 저장소에 대한 것입니다: 의심 문항은 모델 학습 시점 뒤 커밋에서도 비율이 같고(1회차 29/116 대 32/145), 시점 뒤 116문항에서도 혼합이 재작성으로 71.6~75.9(영어 71.6)입니다. 실행별 표는 보고서의 "LLM rewrite runs"와 [leak-check.md](leak-check.md)에 있습니다
- **영어 바꿔 말하기** (`q_para`, `en-para`): 말을 바꾼 것만의 효과를 보는 통제입니다. 같은 모델이 영어 커밋 제목을 한국어 초안과 같은 기준(자연스럽게, 요청·질문·이슈 제목을 섞어, 식별자 그대로, 정보 추가 없음)으로 영어 그대로 다시 썼습니다. `en` → `en-para` 하락이 말 바꿈의 몫이고, `ko`까지 남은 하락이 언어의 몫입니다
- **외래어를 많이 쓴 한국어** (`q_ko_loan`, `ko-loan`): 사람이 쓴 질문이 아니라 스트레스 테스트입니다. 같은 모델이 한국어 질문을 외래어를 많이 쓰는 말투로 바꿨습니다(요청 → 리퀘스트, 기본값 → 디폴트, progress → 프로그레스). 식별자와 뜻은 그대로이고, 320개 중 51개는 바꿀 것이 없었습니다. 개발 중에 보지 않은 261문항에서 사전이 걸리는 문항이 147개(56%, 원래 191개)로 줄고 영어 낱말이 없는 문항이 56개(원래 27개)로 늡니다. 이때 BM25(조각)·혼합·Lantern이 처음 3개에서 8.0·7.3·6.1점 더 떨어지고, Lantern(49.8)은 여전히 혼합(49.0)과 비깁니다
- **한국어 + 사전, 절반 가중** (`ko-dictw`, 비교 대상만): 같은 사전 낱말을 Lantern처럼 따로, 절반 가중치로 셉니다 (질문 낱말의 BM25 점수 + 넓힌 낱말의 BM25 점수 × 0.5). 흔한 동사가 질문의 식별자를 누르지 않게 하려는 것입니다. 임베딩 쪽은 `ko-dict`와 같은 글을 받습니다. 민감도(`--dict-weight`, 개발 중에 보지 않은 261문항): BM25(조각) 처음 3개가 사전 없이 51.3, 가중치 0.25 / 0.5 / 0.75 / 1.0(1.0은 그냥 덧붙인 `ko-dict`)에서 55.2 / 57.1 / 57.1 / 55.9라, 정확한 가중치보다 사전이 있느냐가 중요합니다
- **개발자가 쓴 질문** (`q_ko_native`, `q_ko_native_b`): [native_sheet.py](native_sheet.py)가 개발 중에 한 번도 보지 않은 7개 저장소에서 저장소마다 6개(42개)를 골라, 작성표와 함께 커밋이 바꾼 코드를 파일 이름을 가리고 보여 주는 HTML을 만듭니다. 영어 커밋 제목은 보여 주지 않습니다(보여 주면 번역하게 됨). 개발자가 번역 도구 없이 "이 변경을 에이전트에게 맡긴다면 어떻게 말할지"를 쓰고, 두 번째 개발자가 같은 문항을 따로 쓸 수 있습니다(B). 작성자는 부호로만 적고, 쓴 질문을 공개하는 데 동의를 받습니다. 분석은 [native-plan.md](native-plan.md)에 미리 정해 두었고, `import`는 계획대로 정답 파일 이름이 들어간 질문을 뺍니다

```bash
python eval/bilingual/native_sheet.py export 작성표.xlsx --dir <클론 폴더>   # 작성표.html도 만듦
python eval/bilingual/native_sheet.py import 작성표.xlsx
```

## 재기

준비물(잰 판): Rust 1.97, Node.js 24, Python 3.12와 `pip install -r eval/bilingual/requirements.txt`(모델 판은 `baselines.py`에 고정), git, bash(Windows는 Git Bash). Aider RepoMap은 선택이고 따로 가상환경이 필요합니다(aider-chat 0.86.2는 Python 3.10~3.12에서만 깔림). 처음에는 네트워크가 필요합니다. `eval/bench/run.mjs`가 저장소 10개를 받고(blob 없이 받아 꺼낼 때마다 받음, 모두 약 250 MB), 임베딩 모델(약 470 MB)과 NLLB(약 2.5 GB, 번역 비용 측정에만)를 한 번 받습니다.

```bash
cargo build --release -p lantern-context
python -m venv .venv-aider && .venv-aider/bin/pip install aider-chat==0.86.2      # 선택 (Windows: .venv-aider\Scripts\pip)
git clone https://github.com/microsoft/vscode <vscode> && git -C <vscode> checkout 76e9dff43b237f3fcbb8a289ed2f8bdf28105baf   # 선택, 규모 표의 vscode 행
VSCODE=<vscode> bash eval/bilingual/run_all.sh <저장소를 받을 폴더> [.venv-aider/bin/python]
```

모든 방법이 같은 파일을 보는지 확인하고(`check_files.py`), 한국어 질문을 사전으로 넓힌 뒤, 질문 형태마다 Lantern·구성 요소별 기여·BM25·임베딩·RRF·Aider RepoMap·Lantern+임베딩을 재고, 규모와 비용을 잽니다. 보고서(`results-unseen.md` 주 결과, `results-fresh-repos.md`, `results-dev-repos.md`, `results-all.md`, `results-test.md`), 누출 점검(`leak-check.md`), 시간 측정 원자료(`measurements/`), 논문 표 두 개(`<클론 폴더>/results/bilingual/tables/`)를 씁니다. 모델 API 호출이 없어 무료입니다. LLM이 만든 형태(`q_ko_draft`, `q_ko`, `q_ko_llm`, `q_para`, `q_ko_loan`)와 NLLB 번역(`q_ko_mt`)은 JSON에 저장된 것을 씁니다. `rewrite.py`는 실제 모델 호출로 `q_ko_llm`을 다시 만들고(정가 기준 약 $4), `q_ko_mt_src`를 지우면 `translate.py`가 다시 옮깁니다. CPU에서 처음 임베딩을 만드느라 몇 시간 걸립니다(벡터는 조각 글로 캐시).

측정은 Windows 11(Intel Core i5-11400)에서 했고, Windows용 Git은 기본으로 파일을 CRLF 줄바꿈으로 꺼냅니다(`core.autocrlf=true`). 모든 방법이 같은 바이트를 읽으므로 토큰 추정에 CR도 들어갑니다. 다른 운영체제에서 LF로 꺼내면 추정이 조금 작아져, 담기는 맥락과 일부 결과의 끝자리가 달라질 수 있습니다. Lantern은 점수가 같으면 심볼 id로 순서를 정하는데, 이 id가 파일을 색인한 순서에 따라 달라지므로([#139](https://github.com/Lantern-IDE/lantern/issues/139)) `run_all.sh`는 빈 인덱스에서 시작합니다.

| 스크립트 | 하는 일 |
|---|---|
| [run_all.sh](run_all.sh) | 아래를 순서대로 모두 |
| [../bench/run.mjs](../bench/run.mjs) `--set eval/bilingual` | 키워드 검색 2가지와 Lantern |
| [check_files.py](check_files.py) | 비교 대상·RepoMap·키워드 검색이 Lantern이 색인하는 파일과 똑같은 파일을 보는지 확인 (`.gitignore`, 숨긴 경로, `vendor/` 등 건너뛰는 폴더, 1 MiB, 바이너리·압축 파일). 저장소마다 기준 커밋과 문항 커밋 3곳에서 |
| [baselines.py](baselines.py) | BM25(파일·40줄 조각), 다국어 임베딩(multilingual-e5-small. 다른 모델은 `--embed-model BAAI/bge-m3 --tag bgem3`처럼 따로, 4코어로 19시간쯤 걸려 `run_all.sh`에서는 뺌), BM25+임베딩 RRF. 낱말: 식별자는 Lantern처럼 통째로 + 낱말로 쪼갬, 한글은 붙어 있는 덩어리 그대로 (한국어 주석·문자열에만 맞음) |
| [expand.py](expand.py) | 한국어 질문 + Lantern 사전이 넓힌 영어 낱말(`q_ko_dict`), 사전 적중률 (외래어 형태도) |
| [rewrite.py](rewrite.py) | KO→LLM 재작성: Claude Code로 질문마다 실제 모델을 한 번씩 부름 (계정 사용량을 씀). 호출마다 결과·시간·토큰·비용을 [measurements/rewrite.json](measurements/rewrite.json)에 남김 |
| [prompts.md](prompts.md) | LLM 단계마다 쓴 지시문 원문: 한국어 초안, 검토, LLM 재작성, 영어 바꿔 말하기 통제, 외래어 스트레스 테스트 |
| [ko_rating.py](ko_rating.py) | 저자가 LLM 한국어 질문 30개(무작위)를 고치지 않고 자연스러움·뜻만 평가하는 표. 기준은 [ko-rating-plan.md](ko-rating-plan.md)에 미리 정함 |
| [native_sheet.py](native_sheet.py) | 사람이 처음부터 쓰는 한국어 질문 표 (`q_ko_native`) |
| [leak.py](leak.py) | LLM 재작성 누출 점검, [leak-check.md](leak-check.md)에 씀: 새 식별자, 정답 파일 이름, 영어 원문과의 겹침, 모델 학습 시점 전후의 원문 복원 비율(Fisher 정확 검정), 의심 문항을 뺀 처음 3개 |
| [hybrid_cost.py](hybrid_cost.py) | 같은 CPU에서 BM25+임베딩 혼합의 비용: 모든 조각을 처음부터 임베딩하는 시간, 질문당 시간 ([measurements/hybrid_cost.json](measurements/hybrid_cost.json)) |
| [nllb_cost.py](nllb_cost.py) | 번역한 뒤 검색의 비용: NLLB-200으로 한 문항씩 옮길 때 질문당 시간과 최대 메모리 ([measurements/nllb.json](measurements/nllb.json)) |
| [scale_questions.txt](scale_questions.txt) | 벤치마크 밖 큰 저장소의 조립 시간만 잴 질문 (`scale.py --extra vscode=<클론>`) |
| [errors.py](errors.py) | Lantern이 영어로만 맞힌 문항. 원인은 [error-analysis.ko.md](error-analysis.ko.md)에서 하나씩 읽고 나눔 (26개 중 16개가 사전에 없는 낱말, 그중 11개가 외래어) |
| [repomap_baseline.py](repomap_baseline.py) | Aider RepoMap (aider-chat 0.86.2, 질문 낱말을 언급된 식별자로) |
| [embed_server.py](embed_server.py) | Lantern 의미 검색을 같은 임베딩 모델로 재는 로컬 `/embeddings` 서버 |
| [translate.py](translate.py) | 한국어 질문 → 영어 기계 번역 (번역한 뒤 검색 비교) |
| [scale.py](scale.py) | 인덱싱 시간·메모리·인덱스 크기·질문당 조립 시간. 반복마다 잰 값은 [measurements/scale.json](measurements/scale.json)에, `--from`으로 다시 재지 않고 표만 만듦 |
| [report.py](report.py) | Recall@8k, R@10f, A@1/3/5, MRR, 부트스트랩 95% 구간 (문항 단위, 핵심 비교는 저장소 단위도), McNemar와 Holm 보정, 저장소별·문항 묶음별 표, 논문용 LaTeX 표(`--tex`). 보고서는 영어로 씀 |

모든 방법은 같은 8,000토큰 예산과 같은 파일(엔진의 파일 규칙, `check_files.py`로 확인)을 받고, 토큰은 글자 4개를 1토큰으로 셉니다. 주 결과는 [eval/bench](../bench)와 겹치지 않는 261문항입니다 (`report.py --subset unseen`, `results-unseen.md`). eval/bench는 엔진의 순위·사전을 고칠 때 확인에 썼기 때문입니다. 엔진과 사전·가중치는 이 벤치마크를 만든 뒤 바꾸지 않았습니다: 사전의 마지막 변경은 ee4c6dd(2026-10-14), 벤치마크는 2026-10-16에 만들었고, 그 뒤 엔진 커밋은 `-`로 시작하는 질문을 받게 한 CLI 수정(5206286)과 평가용 끄기 스위치(d108680, 켜지 않으면 동작이 같음)뿐입니다. 커밋 해시로 나눈 개발용·평가용 세트(해시 앞 8자리 정수를 10으로 나눈 나머지가 0~2면 개발용, 약 30%)는 참고용으로 `results-test.md`에 보고하고, 앞으로 맞출 때 쓰려고 남겨 둡니다.

### 논문 수치가 나오는 곳

| 논문 | 명령 | 결과 |
|---|---|---|
| 표 I, 결과·강건성 문단 | `report.py --subset unseen --tex …` | `results-unseen.md`, `tables/results.tex` |
| 개발에 안 쓴 저장소, 개발에 쓴 저장소 | `report.py --subset fresh-repos`, `--subset dev-repos` | `results-fresh-repos.md`, `results-dev-repos.md` |
| 표 II와 Lantern 비용 | `scale.py --json … --tex …` | `measurements/scale.json`, `tables/scale.tex` |
| 혼합 검색·NLLB·LLM 재작성 비용 | `hybrid_cost.py`, `nllb_cost.py`, `rewrite.py` | `measurements/hybrid_cost.json`, `measurements/nllb.json`, `measurements/rewrite.json` |
| 사전 적중률 (한국어, 외래어 형태) | `expand.py` | 화면 출력 |
| 재작성 누출 점검, NLLB가 망가뜨린 식별자 | `leak.py` | `leak-check.md` |
| 재작성 반복, 저장소 이름 뺀 통제, 모델 학습 시점 전후 | `report.py --subset unseen`, `leak.py` | `results-unseen.md`의 "LLM rewrite runs", `leak-check.md` |
| 오류 분석 | `errors.py` | `error-analysis.md` (읽고 분류) |

### 논문용 동결

ICPC 2027 논문의 데이터와 코드는 태그 `icpc2027-eval-v6`에서 동결합니다. 개발자가 쓴 질문처럼 나중에 더하는 것은 따로 보고하고 이 태그와 비교합니다.

## 개발 중에 한 번도 보지 않은 저장소

개발 때 쓴 59문항은 flask, hono, commons-lang에서만 나왔습니다. 나머지 7개 저장소(224문항, `report.py --subset fresh-repos`, [results-fresh-repos.md](results-fresh-repos.md))에서도 결론이 같습니다: 영어에서 Lantern이 혼합보다 낮고(처음 3개 −9.4), 한국어에서 비기고(+0.9), 영어보다 덜 떨어지는 차이는 저장소 단위로도 유의하며(+10.3, [1.8, 19.2]), 사전을 빼면 4.9 깎이고, 사전을 절반 가중으로 준 BM25(조각)도 Lantern만큼 합니다(Lantern −1.3, 유의하지 않음). 저장소가 7~10개뿐이라 저장소 단위 구간은 근사입니다. 개발에 쓴 3개 저장소의 보지 않은 37문항(`--subset dev-repos`, [results-dev-repos.md](results-dev-repos.md))에서는 Lantern이 오히려 유리하지 않습니다: 혼합 대비 영어 +2.7, 한국어 −8.1, 한국어 하락이 혼합보다 4문항 큼. 그 저장소들에 과적합된 흔적은 없습니다.

## Lantern 비용 측정에 대해

`scale.py`는 빈 인덱스에서 처음 인덱싱을 세 번 재고 중앙값을 씁니다. 첫 번 뒤로는 파일 캐시가 데워집니다(vscode는 [measurements/scale.json](measurements/scale.json)에 70.9, 39.5, 39.4초. 예전에 막 받은 vscode의 첫 번은 92초). Lantern은 파일 해석에 스레드를 8개까지 씁니다. vscode는 커밋 `76e9dff`에서 [scale_questions.txt](scale_questions.txt)의 질문 10개로 잽니다 (예전 초안의 136ms는 즉석 질문 두 개로 잰 값). 모든 측정에서 엔진은 같습니다(엔진의 마지막 변경 d108680). 다시 빌드한 실행 파일로 gin·flask를 재면 파일 목록이 똑같습니다.

## 혼합 검색의 비용

[hybrid_cost.py](hybrid_cost.py)로 Lantern 규모 표와 같은 CPU(Intel Core i5-11400)에서 쟀습니다. multilingual-e5-small 4스레드, 저장소 기준 커밋의 모든 조각을 처음부터 임베딩, 질문당 시간은 BM25 + 질문 임베딩 + 유사도 + RRF + 담기(한국어 질문 10개 중앙값, [measurements/hybrid_cost.json](measurements/hybrid_cost.json)). Lantern은 같은 저장소를 2~5초에 인덱싱하고 5~14ms에 맥락을 조립합니다. 번역한 뒤 검색에는 NLLB-200이 더 듭니다: 4스레드로 한 문항씩 옮기면 질문당 중앙값 2.0초(90% 2.8초), 모델 로드 3.6초, 최대 메모리 약 3 GB ([nllb_cost.py](nllb_cost.py), [measurements/nllb.json](measurements/nllb.json)).

| 저장소 | 조각 | BM25 인덱스 (초) | 임베딩 (초) | 질문당 (ms) |
|---|---|---|---|---|
| axios | 1,375 | 0.3 | 117 | 16 |
| axum | 1,357 | 0.3 | 119 | 18 |
| commons-lang | 5,484 | 1.5 | 552 | 27 |
| flask | 502 | 0.2 | 45 | 17 |
| gin | 661 | 0.2 | 62 | 16 |
| hono | 2,380 | 0.5 | 225 | 19 |
| httpx | 473 | 0.1 | 46 | 14 |
| jq | 733 | 0.2 | 69 | 16 |
| okhttp | 4,025 | 0.9 | 360 | 21 |
| vite | 3,847 | 0.9 | 270 | 23 |

## 한계

- 커밋 메시지는 고친 뒤에 쓴 글이라, 실제 사용자 질문보다 식별자가 많이 들어 있습니다 (파일 이름이 나오는 커밋만 거름)
- 정답은 '고친 파일'이라, 읽어야 했지만 고치지 않은 파일은 정답에 없습니다
- 한국어 질문은 LLM이 옮기고 LLM이 검토한 것이라, 한국 개발자가 직접 쓴 질문이 아니고 LLM 말투가 일부 남을 수 있습니다
- 초안이 영어에서 나왔으므로, 같은 계열의 번역기로 다시 옮기면 원문에 가깝게 돌아가 '번역한 뒤 검색'이 실제보다 유리할 수 있습니다 (그래서 초안을 쓴 LLM과 다른 번역 모델을 씀). 초안을 쓴 LLM과 같은 계열인 'LLM 재작성'은 더 유리할 수 있습니다
- Lantern의 한국어 사전은 한국어 질문을 쓴 것과 같은 모델(Claude Opus 5.5, Claude Code)로 썼으므로, 질문을 쓴 쪽이 사전을 보지 않았어도 낱말이 겹칠 수 있습니다. 사람이 쓴 질문(`q_ko_native`)으로 이것을 재려고 합니다
- LLM 재작성은 이 인기 저장소들에 대한 모델의 지식을 더합니다(재작성의 17%가 질문에 없던 식별자를 더함). 비공개이거나 덜 알려진 코드에서는 덜 도울 수 있습니다
- 임베딩은 작은 multilingual-e5-small만 썼습니다 (Lantern+임베딩도 같은 모델이라 그 비교는 맞춰져 있음). BGE-M3 같은 더 큰 다국어 모델은 재지 않았습니다
- 저장소가 10개뿐이라 저장소 단위 부트스트랩 구간이 넓고, 저장소마다 결과가 다릅니다 (보고서의 저장소별 표)
- 인기 저장소라 오래된 커밋은 모델 학습 데이터에 들어 있을 수 있습니다. 2026년 커밋만 따로 잴 수 있게 커밋 날짜를 함께 봅니다

## 라이선스

문항 파일은 위 오픈소스 저장소의 커밋 제목과 파일 경로를 씁니다 (MIT, Apache-2.0, BSD, jq 라이선스. 각 저장소 참고). 한국어 질문·스크립트·주석은 Lantern과 함께 Apache-2.0입니다.
