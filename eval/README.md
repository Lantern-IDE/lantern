# 평가 (eval)

맥락 엔진이 "답하려면 꼭 봐야 하는 파일"을 얼마나 가져오는지 잽니다. 결과 해석은 [결과.md](결과.md).

## 공개 벤치마크 (`eval/bench`, 누구나 재현)

```bash
cargo build --release -p lantern-context
node eval/bench/run.mjs --dir <저장소를 받을 폴더>
```

오픈소스 저장소 3개(flask, hono, commons-lang)를 받아 고정한 커밋으로 맞추고, 영어·한국어 질문 60개씩으로 잰 뒤 [bench/결과.md](bench/결과.md)를 새로 씁니다.

- **질문과 정답은 실제 커밋에서** (`bench/make.mjs`): 질문 = 커밋 메시지 제목, 정답 파일 = 그 커밋이 고친 코드 파일(테스트 제외), 평가 시점 = 그 커밋의 부모. 사람이 정답을 고르지 않아 엔진을 아는 사람의 편향이 들어가지 않습니다
- **거르는 커밋**: 병합, 버전·문서·빌드·린트 손질, 새로 만든·지운·옮긴 파일이 있는 커밋, 코드 파일 4개 이상, 메시지에 정답 파일 이름이 나오는 커밋, 같은 정답 파일 묶음은 2개까지
- **한국어 질문**(`q_ko`): 같은 커밋 메시지를 사람이 옮긴 것. 식별자는 영어 그대로 둡니다
- **비교 방식**: 키워드 검색(파일 통째로), 키워드 검색(맞은 줄 앞뒤 12줄씩), Lantern. 그리고 Lantern과 같은 수의 파일을 무작위로 골랐을 때의 기대 적중률
- **한계**: 커밋 메시지는 짧고 영어라 실제 사용자 질문과 다릅니다. 정답 파일은 '고친 파일'이라, 읽어야 했지만 고치지 않은 파일은 정답에 없습니다

## 맥락 적중률 (모델 호출 없음, 무료)

```bash
cargo build --release -p lantern-context
node eval/retrieval.mjs --project <평가할 프로젝트> --golden <정답 파일 JSON> [--budget 8000]
```

같은 토큰 예산에서 키워드 검색(grep 후 파일 통째로 읽기)과 Lantern이 정답 파일을 몇 개 가져오는지 비교하고, `eval/results/`에 보고서를 남깁니다. 인덱스는 `eval/results/index`에 만들어서 평가 대상 폴더에는 아무것도 쓰지 않습니다.

## 정답 파일 JSON 형식

```json
{
  "project": "my-app",
  "questions": [
    {
      "q": "로그인하면 세션 쿠키는 어디서 발급해?",
      "golden": ["src/auth/session.ts", "src/auth/login.ts"],
      "also_ok": ["legacy/auth/session.ts"]
    }
  ]
}
```

- `golden`: 이 질문에 제대로 답하려면 꼭 봐야 하는 파일 (프로젝트 루트 기준 경로, `/` 구분)
- `also_ok` (선택): 같은 역할의 다른 구현처럼 가져와도 괜찮은 파일. 적중률에는 넣지 않고 따로 셉니다
- 식별자가 들어간 질문과, 동작만 묘사한 질문("사진 형식을 속이면 어떻게 막혀?")을 섞어야 키워드 검색과 차이가 드러납니다

결과.md의 수치는 비공개 프로젝트로 잰 것이라 정답 파일과 보고서는 이 저장소에 없습니다.

## 실제 모델 A/B (`eval/ab`)

같은 질문을 맥락 엔진 있이/없이 모델에 보내 답과 비용을 비교합니다. 로컬 모델이면 무료, Claude는 요금이 듭니다.

```bash
cd eval/ab && npm install
node run.mjs --project <프로젝트> --golden <정답 파일 JSON> --provider openai --base-url http://localhost:11434/v1 --model <모델> --yes
```
