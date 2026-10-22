# 오류 분석: 영어로는 맞히고 한국어로는 놓친 문항

개발 중에 보지 않은 261문항에서, Lantern(모델 없음)이 영어로는 정답 파일을 앞 3개 안에 넣고 한국어로는 넣지 못한 26문항입니다. 반대로 한국어로만 맞힌 문항은 14개입니다 (순변화 −12문항 = 처음 3개 −4.6점).
문항 목록은 `python eval/bilingual/errors.py --dir <클론 폴더>`로 다시 뽑을 수 있고, 원인은 스크립트가 아니라 질문·사전 확장·순위를 하나씩 읽고 붙였습니다(Claude Code가 함) (평가한 엔진: 태그 `icpc2027-eval`부터 `icpc2027-eval-v6`까지 같음). [English](error-analysis.md).

| 원인 | 문항 |
|---|---|
| 핵심 낱말이 사전에 없음 | 16 (그중 영어를 한글로 적은 외래어 11) |
| 사전이 넓힌 일반어가 엉뚱한 파일을 끌어옴 | 3 |
| 정답이 4~6위로 밀림 (확장은 맞음) | 7 |

외래어(프록시, 파라미터, 헤더, 버전, 코루틴 등)는 영어 철자로 되돌리는 규칙으로 기계적으로 다룰 수 있어 다음 개선 거리입니다. 다만 이 문항들을 보고 고치면 이 문항들은 더 이상 '보지 않은 문항'이 아니므로, 고친 엔진은 새 문항이나 개발용 세트로 재야 합니다.

| 저장소 | 한국어 질문 | 사전이 넓힌 낱말 | 한국어 순위 | 놓친 낱말 (영어) | 원인 |
|---|---|---|---|---|---|
| axios | 요청 끝나면 progress 이벤트 리스너 제거하는 건 어디서 해? | request, event | - | 제거(remove), 끝나면(completion) | 사전에 없음 |
| axios | 조합한 signal이 바로 abort되게 해줘 | - | 9 | 조합(compose) | 사전에 없음 |
| axios | 형식이 잘못된 XHR 업로드 이벤트는 어디서 막아? | upload, event, block, prevent, guard | 4 | 형식이 잘못된(malformed) | 4~6위 |
| commons-lang | 더 이상 안 쓰는 시스템 프로퍼티 상수 deprecated 처리 | handle, process | 11 | 시스템(system), 프로퍼티(property), 상수(constant) | 사전에 없음 (외래어) |
| flask | 세션에 제네릭 베이스 클래스 쓰기 | session | 4 | 제네릭(generic), 베이스(base) | 사전에 없음 (외래어) |
| flask | trusted_hosts 설정하고 검사하는 건 어디서 해? | config, setting, check, validate, guard | 4 | - | 일반어 확장 (설정 → config가 config.py를 끌어옴) |
| flask | 파이썬 타입 어노테이션이랑 반환 타입 추가 | type, return, add, create | 9 | 어노테이션(annotation) | 사전에 없음 (외래어) |
| flask | 템플릿 context processor를 async로도 쓰려면 어디를 고쳐야 해? | template, fix | 4 | - | 4~6위 |
| gin | 구조체 포인터 역참조 | - | - | 구조체(struct), 포인터(pointer), 역참조(dereference) | 사전에 없음 (외래어) |
| gin | Go 최소 버전을 1.15로 바꾸려면 어디를 고쳐야 해? | change, update, replace, fix | 8 | 버전(version), 최소(min) | 사전에 없음 (외래어) |
| hono | key 있는 대량 업데이트에서 조회 작업 좀 줄여줘 | find, get, query, fetch, job, task, reduce, shrink | 30 | - | 일반어 확장 (조회·작업이 너무 넓게 퍼짐) |
| hono | 와일드카드 미들웨어를 매칭되는 라우트랑 연결하는 건 어디서 해? | middleware, connect, link, edge | - | 와일드카드(wildcard), 라우트(route) | 일반어 확장 (연결 → edge가 lambda-edge 어댑터를 끌어옴) |
| hono | 304 응답 헤더 거를 때 일부 헤더를 건너뛰는 문제 | response | - | 헤더(header), 거르다(filter), 건너뛰다(skip) | 사전에 없음 (외래어) |
| httpx | 에러 날 때 프록시 프로토콜 scheme 보여주려면 어디 고쳐야 해? | error, fix | 5 | 프록시(proxy), 프로토콜(protocol) | 사전에 없음 (외래어) |
| httpx | 쿼리 파라미터 공백은 '%20'으로 인코딩 | whitespace, space, encoding | 12 | 쿼리(query), 파라미터(parameter) | 사전에 없음 (외래어) |
| jq | 중첩 함수에 파라미터가 너무 많으면 assertion 실패하는 버그 | fail, failure, error | 12 | 파라미터(parameter), 중첩(nested), 함수(function) | 사전에 없음 (외래어) |
| jq | 에러 처리 일관되게 하려고 `range/3`를 C로 구현 | error, handle, process | 5 | - | 4~6위 |
| jq | 재귀 객체 병합 깊이 제한해서 스택 오버플로 막으려면 어디 봐야 해? | - | - | 스택(stack), 오버플로(overflow), 재귀(recursive), 병합(merge) | 사전에 없음 (외래어) |
| okhttp | ServiceMetadata 레코드를 스펙대로 정렬 | sort, order | 4 | - | 4~6위 |
| okhttp | retryOnConnectionFailure가 ECH 재시도에는 영향 안 주게 해줘 | - | 6 | - | 4~6위 (테스트 파일이 앞섬) |
| okhttp | @Nested JUnit 5 테스트에서 @StartStop 필드가 시작 안 되는 문제 | test, spec, bootstrap, init | 4 | - | 4~6위 |
| okhttp | deflate된 메시지 뒤에 남은 데이터는 어디서 버려? | message | 5 | - | 4~6위 |
| okhttp | 실험적 코루틴 API 그만 쓰게 바꿔줘 | change, update, replace | - | 코루틴(coroutine), 실험적(experimental) | 사전에 없음 (외래어) |
| vite | 재시작 도중에 요청된 재시작이 버려지던 문제 (#23392) | request | 11 | 재시작(restart) | 사전에 없음 |
| vite | hmr 기본값을 직접 수정해버리는 문제 (#23575) | update, edit | - | 기본값(default) | 사전에 없음 |
| vite | 초기화 끝나면 이전 environment 해제하는 건 어디서 해? | init, initialize, reset | - | 해제(release) | 사전에 없음 |
