# Error analysis: questions Lantern answers in English but not in Korean

[한국어](error-analysis.ko.md)

On the 261 questions not used during development, Lantern without a model puts a gold file in the top 3 for 26 questions in English but not in Korean, and for 14 questions only in Korean (net −12 questions = −4.6 A@3 points).
The list comes from `python eval/bilingual/errors.py --dir <clones>` (also written by `run_all.sh` to `results/bilingual/error-candidates.md`). The causes were assigned by reading each question, its dictionary expansion and the ranking, not by a script; Claude Code did this (see the paper's acknowledgment). The engine is the same from tag `icpc2027-eval` to `icpc2027-eval-v6`.

| Cause | Questions |
|---|---|
| A key word is not in the dictionary | 16 (11 of them Hangul spellings of English terms) |
| A generic expansion pulls in unrelated files | 3 |
| The gold file ranks 4th to 6th (the expansion is right) | 7 |

Loanwords (프록시 proxy, 파라미터 parameter, 헤더 header, 버전 version, 코루틴 coroutine…) could be handled mechanically by mapping Hangul spellings back to English, which is the next improvement. Fixing them by looking at these questions would make these questions no longer unseen, so a fixed engine must be measured on new questions or the development set.

| Repository | Korean question (gloss) | Dictionary terms | Rank in Korean | Missed words | Cause |
|---|---|---|---|---|---|
| axios | 요청 끝나면 progress 이벤트 리스너 제거하는 건 어디서 해? (where are progress event listeners removed when a request ends?) | request, event | - | 제거 (remove), 끝나면 (on completion) | not in the dictionary |
| axios | 조합한 signal이 바로 abort되게 해줘 (make the composed signal abort immediately) | - | 9 | 조합 (compose) | not in the dictionary |
| axios | 형식이 잘못된 XHR 업로드 이벤트는 어디서 막아? (where are malformed XHR upload events blocked?) | upload, event, block, prevent, guard | 4 | 형식이 잘못된 (malformed) | rank 4–6 |
| commons-lang | 더 이상 안 쓰는 시스템 프로퍼티 상수 deprecated 처리 (deprecate unused system property constants) | handle, process | 11 | 시스템 (system), 프로퍼티 (property), 상수 (constant) | not in the dictionary (loanwords) |
| flask | 세션에 제네릭 베이스 클래스 쓰기 (use a generic base class for sessions) | session | 4 | 제네릭 (generic), 베이스 (base) | not in the dictionary (loanwords) |
| flask | trusted_hosts 설정하고 검사하는 건 어디서 해? (where is trusted_hosts set and checked?) | config, setting, check, validate, guard | 4 | - | generic expansion (설정 → config pulls in config.py) |
| flask | 파이썬 타입 어노테이션이랑 반환 타입 추가 (add Python type annotations and return types) | type, return, add, create | 9 | 어노테이션 (annotation) | not in the dictionary (loanword) |
| flask | 템플릿 context processor를 async로도 쓰려면 어디를 고쳐야 해? (what to change to allow async template context processors?) | template, fix | 4 | - | rank 4–6 |
| gin | 구조체 포인터 역참조 (dereference struct pointers) | - | - | 구조체 (struct), 포인터 (pointer), 역참조 (dereference) | not in the dictionary (loanwords) |
| gin | Go 최소 버전을 1.15로 바꾸려면 어디를 고쳐야 해? (what to change to make Go 1.15 the minimum version?) | change, update, replace, fix | 8 | 버전 (version), 최소 (min) | not in the dictionary (loanword) |
| hono | key 있는 대량 업데이트에서 조회 작업 좀 줄여줘 (reduce lookups in keyed bulk updates) | find, get, query, fetch, job, task, reduce, shrink | 30 | - | generic expansion (조회 and 작업 spread too wide) |
| hono | 와일드카드 미들웨어를 매칭되는 라우트랑 연결하는 건 어디서 해? (where is wildcard middleware connected to matching routes?) | middleware, connect, link, edge | - | 와일드카드 (wildcard), 라우트 (route) | generic expansion (연결 → edge pulls in the Lambda@Edge adapter) |
| hono | 304 응답 헤더 거를 때 일부 헤더를 건너뛰는 문제 (some headers are skipped when filtering 304 response headers) | response | - | 헤더 (header), 거르다 (filter), 건너뛰다 (skip) | not in the dictionary (loanword) |
| httpx | 에러 날 때 프록시 프로토콜 scheme 보여주려면 어디 고쳐야 해? (what to change to show the proxy protocol scheme on errors?) | error, fix | 5 | 프록시 (proxy), 프로토콜 (protocol) | not in the dictionary (loanwords) |
| httpx | 쿼리 파라미터 공백은 '%20'으로 인코딩 (encode spaces in query parameters as '%20') | whitespace, space, encoding | 12 | 쿼리 (query), 파라미터 (parameter) | not in the dictionary (loanwords) |
| jq | 중첩 함수에 파라미터가 너무 많으면 assertion 실패하는 버그 (assertion fails when nested functions have too many parameters) | fail, failure, error | 12 | 파라미터 (parameter), 중첩 (nested), 함수 (function) | not in the dictionary (loanword) |
| jq | 에러 처리 일관되게 하려고 `range/3`를 C로 구현 (implement `range/3` in C for consistent error handling) | error, handle, process | 5 | - | rank 4–6 |
| jq | 재귀 객체 병합 깊이 제한해서 스택 오버플로 막으려면 어디 봐야 해? (where to limit recursive object merge depth to prevent stack overflow?) | - | - | 스택 (stack), 오버플로 (overflow), 재귀 (recursive), 병합 (merge) | not in the dictionary (loanwords) |
| okhttp | ServiceMetadata 레코드를 스펙대로 정렬 (sort ServiceMetadata records per the spec) | sort, order | 4 | - | rank 4–6 |
| okhttp | retryOnConnectionFailure가 ECH 재시도에는 영향 안 주게 해줘 (keep retryOnConnectionFailure from affecting ECH retries) | - | 6 | - | rank 4–6 (a test file ranks higher) |
| okhttp | @Nested JUnit 5 테스트에서 @StartStop 필드가 시작 안 되는 문제 (@StartStop fields are not started in @Nested JUnit 5 tests) | test, spec, bootstrap, init | 4 | - | rank 4–6 |
| okhttp | deflate된 메시지 뒤에 남은 데이터는 어디서 버려? (where is data left after a deflated message discarded?) | message | 5 | - | rank 4–6 |
| okhttp | 실험적 코루틴 API 그만 쓰게 바꿔줘 (stop using experimental coroutine APIs) | change, update, replace | - | 코루틴 (coroutine), 실험적 (experimental) | not in the dictionary (loanword) |
| vite | 재시작 도중에 요청된 재시작이 버려지던 문제 (#23392) (a restart requested during a restart was dropped) | request | 11 | 재시작 (restart) | not in the dictionary |
| vite | hmr 기본값을 직접 수정해버리는 문제 (#23575) (the hmr default value is mutated) | update, edit | - | 기본값 (default) | not in the dictionary |
| vite | 초기화 끝나면 이전 environment 해제하는 건 어디서 해? (where is the previous environment released after initialization?) | init, initialize, reset | - | 해제 (release) | not in the dictionary |
