# 영·한 병렬 벤치마크 (eval/bilingual)

같은 정답 파일에 영어 질문과 한국어 질문을 짝지어, **질문 언어만 바꿨을 때** 맥락 검색이 얼마나 달라지는지 잽니다. [eval/bench](../bench)(저장소 3개, 60문항)를 키운 것입니다.

| | |
|---|---|
| 저장소 | 10개, 언어 8개 (아래 표) |
| 문항 | 320개 (저장소마다 최근 커밋 32개), 그중 2026년 커밋 212개 |
| 질문 | 영어: 커밋 메시지 제목. 한국어: 같은 메시지를 한 사람(이 저장소 작성자)이 옮김 |
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

## 만드는 법

문항은 [make.mjs](../bench/make.mjs)가 커밋 이력에서 자동으로 뽑습니다. 사람이 문항이나 정답을 고르지 않습니다.

```bash
node eval/bench/make.mjs --repo <클론> --name <이름> --url <저장소 주소> --count 32 --ref <기준 커밋> --out eval/bilingual
```

거르는 커밋: 병합, 버전·문서·빌드·린트·의존성 손질, 새로 만들거나 지우거나 옮긴 파일이 있는 커밋, 코드 파일 4개 이상, 메시지에 정답 파일 이름이 나오는 커밋, 같은 정답 파일 묶음은 2개까지. 정답에서 빼는 파일: 테스트(`test/`, `jvmTest/`, `*_test.go`, `…Test.kt`, 테스트 지원 모듈 등), Gradle 빌드 스크립트와 빌드 로직, `examples/`·`docs/`·`playground/`·`scripts/`·`benchmarks/`.

## 한국어 질문

[ko_sheet.py](ko_sheet.py)로 작성표를 만들어 채운 뒤 다시 넣습니다.

```bash
python eval/bilingual/ko_sheet.py export 작성표.xlsx   # 질문 → 작성표
python eval/bilingual/ko_sheet.py import 작성표.xlsx   # 작성표 → 각 JSON의 q_ko
```

- 작성표에는 정답 파일을 넣지 않습니다. 파일 이름을 보면 질문에 힌트가 섞이기 때문입니다
- 쓰는 기준: 한국 개발자가 실제로 묻듯이 자연스럽게, 영어 질문에 없는 정보는 더하지 않음, 코드 이름은 영어 그대로 둬도 됨, 바뀐 코드(diff)는 보지 않음 (작성표의 '작성 안내' 시트)
- 옮기기 어려운 문항은 비워 두고 `q_ko_note`에 이유를 남깁니다. 한국어 결과는 한국어 질문이 있는 문항만으로 잽니다

## 재기

```bash
cargo build --release -p lantern-context
node eval/bench/run.mjs --dir <저장소를 받을 폴더> --set eval/bilingual
```

결과는 [결과.md](결과.md)에 씁니다.

## 한계

- 커밋 메시지는 고친 뒤에 쓴 글이라, 실제 사용자 질문보다 식별자가 많이 들어 있습니다 (파일 이름이 나오는 커밋만 거름)
- 정답은 '고친 파일'이라, 읽어야 했지만 고치지 않은 파일은 정답에 없습니다
- 한국어 질문은 한 사람이 옮겼고, 원래 한국어로 쓴 질문이 아닙니다
- 인기 저장소라 오래된 커밋은 모델 학습 데이터에 들어 있을 수 있습니다. 2026년 커밋만 따로 잴 수 있게 커밋 날짜를 함께 봅니다
