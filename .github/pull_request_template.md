## 요약

<!-- 무엇을, 왜 바꾸는지 한두 문장. 문제의 근거·재현 조건을 먼저 씁니다. -->

## 관련 이슈

<!-- Closes #번호 / Refs #번호 -->

## 변경 내용

-

## 범위에서 뺀 것

<!-- 하지 않은 것과 그 이유. 없으면 지웁니다. -->

## 검증

```
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && npm run typecheck && npm test
python scripts/i18n_check.py
cd app && npx tauri build --no-bundle && npm run e2e   # Windows, 화면 흐름을 바꿨다면
```

- [ ] 위 명령이 모두 통과한다
- [ ] 화면 문구를 추가했다면 영어 번역(`app/src/locales/en.json`)도 넣었다
- [ ] 새 의존성을 추가했다면 `python scripts/gen_notices.py`로 제3자 고지를 갱신했다
- [ ] 커밋 메시지가 `TYPE: 요약` 규칙을 따른다

## 리뷰 포인트

<!-- 판단이 갈릴 수 있는 부분, 확인이 필요한 부분 -->
