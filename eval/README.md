# 평가 (eval)

맥락 엔진이 "답하려면 꼭 봐야 하는 파일"을 얼마나 가져오는지 잽니다. 결과 해석은 [결과.md](결과.md).

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
