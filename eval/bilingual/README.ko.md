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

## 재기

```bash
cargo build --release -p lantern-context
bash eval/bilingual/run_all.sh <저장소를 받을 폴더> [aider가 깔린 파이썬]
```

한국어 질문 번역(NLLB) → Lantern → BM25·임베딩·RRF → Aider RepoMap → Lantern+임베딩 → 규모 순으로 재고, [결과-비교.md](결과-비교.md)(전체)와 `결과-비교-test.md`(평가용 세트)를 씁니다. 모델 호출이 없어 무료지만, CPU에서 처음 임베딩을 만드느라 몇 시간 걸립니다 (벡터는 캐시해서 다음부터 빠름).

| 스크립트 | 하는 일 |
|---|---|
| [../bench/run.mjs](../bench/run.mjs) `--set eval/bilingual` | 키워드 검색 2가지와 Lantern |
| [baselines.py](baselines.py) | BM25(파일·40줄 조각), 다국어 임베딩(multilingual-e5-small), BM25+임베딩 RRF |
| [repomap_baseline.py](repomap_baseline.py) | Aider RepoMap (aider-chat 0.86.2, 질문 낱말을 언급된 식별자로) |
| [embed_server.py](embed_server.py) | Lantern 의미 검색을 같은 임베딩 모델로 재는 로컬 `/embeddings` 서버 |
| [translate.py](translate.py) | 한국어 질문 → 영어 기계 번역 (번역한 뒤 검색 비교) |
| [scale.py](scale.py) | 인덱싱 시간·메모리·인덱스 크기·질문당 조립 시간 |
| [report.py](report.py) | Recall@8k, R@10f, Acc@1/3/5, MRR, 부트스트랩 95% 구간, McNemar, 논문용 LaTeX 표(`--tex`) |

주 결과는 [eval/bench](../bench)와 겹치지 않는 261문항입니다 (`report.py --subset unseen`, `결과-비교-unseen.md`). eval/bench는 엔진의 순위·사전을 고칠 때 확인에 썼기 때문입니다. 평가용·개발용 세트: 커밋 해시 앞 8자리 정수를 10으로 나눈 나머지가 0~2면 개발용(약 30%), 나머지는 평가용입니다. 엔진의 사전·가중치는 개발용으로만 맞추고, 보고는 평가용으로 합니다.

## 한계

- 커밋 메시지는 고친 뒤에 쓴 글이라, 실제 사용자 질문보다 식별자가 많이 들어 있습니다 (파일 이름이 나오는 커밋만 거름)
- 정답은 '고친 파일'이라, 읽어야 했지만 고치지 않은 파일은 정답에 없습니다
- 한국어 질문은 LLM이 옮기고 LLM이 검토한 것이라, 한국 개발자가 직접 쓴 질문이 아니고 LLM 말투가 일부 남을 수 있습니다
- 초안이 영어에서 나왔으므로, 같은 계열의 번역기로 다시 옮기면 원문에 가깝게 돌아가 '번역한 뒤 검색'이 실제보다 유리할 수 있습니다 (그래서 초안을 쓴 LLM과 다른 번역 모델을 씀)
- 인기 저장소라 오래된 커밋은 모델 학습 데이터에 들어 있을 수 있습니다. 2026년 커밋만 따로 잴 수 있게 커밋 날짜를 함께 봅니다

## 라이선스

문항 파일은 위 오픈소스 저장소의 커밋 제목과 파일 경로를 씁니다 (MIT, Apache-2.0, BSD, jq 라이선스. 각 저장소 참고). 한국어 질문·스크립트·주석은 Lantern과 함께 Apache-2.0입니다.
