# Prompts used to build the benchmark's question forms

Every LLM step was run once with Claude Opus 5.5 (`claude-opus-5-5`), on 2026-10-18 to 2026-10-21. Steps 1, 2, 4 and 5 ran as Claude Code subagents, each over all questions in one pass. Step 3, the rewrite that the KO→LLM baseline evaluates, is one separate call per question ([rewrite.py](rewrite.py)), so each rewrite sees only its own question. The texts below are verbatim, except that local file paths are replaced by `<input file>` and `<output file>`. The agents had no access to the repositories, the gold files, Lantern or its dictionary unless stated.

## 1. Korean drafts (`q_ko_draft`)

```text
You are helping build a benchmark of natural-language code search questions written in Korean. Your job is to write a Korean version of each English question, as a Korean software developer would naturally type it to a colleague or an AI coding assistant.

Input file (read it with the Read tool): <input file>
It is a JSON list of 320 items: {"id", "repo", "en"}. "repo" is the open-source project the question is about (flask, httpx, hono, vite, commons-lang, gin, axum, okhttp, axios, jq). "en" is the English question (a commit message title).

Write the output file with the Write tool: <output file>
Format: one JSON object mapping every id to its Korean text, e.g. {"axios-01": "...", "axios-02": "...", ...}. All 320 ids must be present. If an item truly cannot be rendered, use an empty string for it and also add it to a separate object under the key "_notes" with a short Korean reason (e.g. {"_notes": {"jq-07": "..."}}).

Rules (follow strictly):
1. Write it the way a Korean developer would actually ask or request it, in natural Korean. Avoid word-for-word translation and translationese.
2. Do NOT add any information that is not in the English text: no file names, function names, class names, causes, or guesses about where the code is. Do not look anything up. Do not open any repository, commit, website, or source code. Work only from the English text.
3. Keep code identifiers and proper nouns in their original form when a Korean developer would (e.g. MethodView, `verify=False`, GOAWAY, DnsOverHttps, CONNECT, strftime/1, JUnit). Ordinary words should be written the way Korean developers usually say them (both native Korean words and common transliterations like 리스너, 헤더, 세션 are fine — use whichever sounds most natural in context).
4. Vary the form across items, roughly evenly mixed: requests (e.g. "~하게 고쳐줘", "~해줘"), questions (e.g. "~는 어디서 처리해?", "~하려면 어디를 봐야 해?"), and short noun-phrase issue titles (e.g. "~ 문제", "~ 안 되는 버그"). Choose what fits each item; do not make them all the same pattern.
5. Keep the meaning of the original. Keep it about as long as the original; do not pad.
6. Be careful with negations and conditions (e.g. "don't", "without", "only when").

When done, reply with only: the number of items written, the number left empty, and 5 sample lines (id: en → ko). Do not paste the whole file in your reply.
```

## 2. Review of the drafts (`q_ko`, 4 of 320 changed)

```text
You are a careful native-level Korean reviewer for a benchmark of natural-language code-search questions. Another writer produced Korean versions of English questions; your job is to review each Korean draft and fix it where needed.

Input (read with the Read tool): <input file>
A JSON list of 320 items {"id", "repo", "en", "draft"}. "en" is the original English question (a commit message subject from an open-source project), "draft" is the Korean draft.

Output (write with the Write tool): <output file>
Format: one JSON object mapping every id to {"ko": "<final Korean>", "changed": true|false, "why": "<short Korean reason if changed, else empty>"}. All 320 ids must be present. If a draft is fine, copy it unchanged with changed=false.

Review criteria (fix only what violates them; do not rewrite for taste):
1. Meaning: the Korean must say the same thing as the English. Check negations ("don't", "without", "not"), conditions ("when", "only if", "after"), direction (from/to), and quantities carefully. Fix any mistranslation.
2. No added information: remove anything not in the English (file names, function names, causes, guesses about location, extra detail). Also do not drop essential content.
3. Natural: it should read like something a Korean developer would actually type to a colleague or an AI assistant. Fix clear translationese or awkward grammar. Mixed Korean with English code terms is normal and fine.
4. Code identifiers and proper nouns may stay in English (that is how Korean developers write). Do not translate identifiers into Korean.
5. Keep the existing sentence form (request / question / issue title) unless it causes a meaning error.

Rules: work only from the English and the draft. Do not open any repository, website, commit or source code, and do not use outside knowledge about any specific search tool. Be conservative: most drafts should stay unchanged.

When finished, reply with only: total items, number changed, and up to 8 examples of changes (id: draft → ko, why). Do not paste the whole file.
```

## 3. Rewrite into an English search query (`q_ko_llm`, KO→LLM)

One call per question through Claude Code's headless mode (`claude -p --model claude-opus-5-5 --tools "" --setting-sources ""`, run in an empty folder, so the model has no tools, settings or project files). The system prompt is the text below; the user message is `Repository: <name>` and `Question: <Korean question>` on two lines. Each call's output, API time, tokens and list price are in [measurements/rewrite.json](measurements/rewrite.json) (320 calls on 2026-10-21, $3.72 in total).

```text
You are the query-rewriting step of a coding agent. A developer asked a question in Korean about an open-source repository; before searching the code, you rewrite it as an English code-search query.

Rules:
1. Write a short English query (one line) that a coding agent would use to search this repository for the code to read or change: say what the code does or what should change, in the words developers use in English code and commit messages.
2. Keep every code identifier, option, flag, header, function and type name from the Korean exactly as written.
3. Do not add information that is not in the Korean question (no guessed file names, function names or causes). Work only from the Korean text and the repository name.
4. Reply with the query only; no explanations.
```

Up to tag `icpc2027-eval-v4`, `q_ko_llm` came from a single Claude Code subagent that rewrote all 320 questions in one pass with the batched version below. Those rewrites were terser and closer to the commit subjects (18% reproduced the subject verbatim, against 3% for the per-question calls), and they no longer match how the paper describes and times the baseline, so v5 replaced them. They remain in the git history.

```text
You are the query-rewriting step of a coding agent. A developer asked a question in Korean about an open-source repository; before searching the code, you rewrite it as an English code-search query.

Input (read with the Read tool): <input file>
A JSON list of 320 items {"id", "repo", "ko"}. "repo" is the project (flask, httpx, hono, vite, commons-lang, gin, axum, okhttp, axios, jq) and "ko" is the developer's Korean question or request.

Output (write with the Write tool): <output file>
Format: one JSON object mapping every id to the English query string, e.g. {"axios-01": "...", ...}. All 320 ids must be present.

Rules:
1. Write a short English query (one line) that a coding agent would use to search this repository for the code to read or change: say what the code does or what should change, in the words developers use in English code and commit messages.
2. Keep every code identifier, option, flag, header, function and type name from the Korean exactly as written.
3. Do not add information that is not in the Korean question (no guessed file names, function names or causes), and do not look anything up: do not open any repository, website, commit or source code. Work only from the Korean text and the repository name.
4. One query per item; no explanations.

When done, reply with only the number of items written and 5 sample lines (id: ko → query).
```

## 4. English rewording control (`q_para`, EN-para)

Same guidelines as prompt 1, but staying in English.

```text
You are helping build a control condition for a benchmark of natural-language code search questions. Your job is to write a reworded English version of each English question, as a software developer would naturally type it to a colleague or an AI coding assistant.

Input file (read it with the Read tool): <input file>
It is a JSON list of 320 items: {"id", "repo", "en"}. "repo" is the open-source project the question is about (flask, httpx, hono, vite, commons-lang, gin, axum, okhttp, axios, jq). "en" is the English question (a commit message title).

Write the output file with the Write tool: <output file>
Format: one JSON object mapping every id to its reworded English text, e.g. {"axios-01": "...", "axios-02": "...", ...}. All 320 ids must be present.

Rules (follow strictly):
1. Write it the way a developer would actually ask or request it, in natural English, in your own words. Do not copy the original wording: reword it the way a different person describing the same change would, while keeping it natural.
2. Do NOT add any information that is not in the English text: no file names, function names, class names, causes, or guesses about where the code is. Do not look anything up. Do not open any repository, commit, website, or source code. Work only from the English text.
3. Keep code identifiers and proper nouns exactly in their original form (e.g. MethodView, `verify=False`, GOAWAY, DnsOverHttps, CONNECT, strftime/1, JUnit). Ordinary words may be replaced by synonyms or rephrased.
4. Vary the form across items, roughly evenly mixed: requests (e.g. "make ... do ...", "please change ..."), questions (e.g. "where is ... handled?", "what do I need to change to ...?"), and short issue titles (e.g. "... not working", "... bug when ..."). Choose what fits each item; do not make them all the same pattern.
5. Keep the meaning of the original. Keep it about as long as the original; do not pad.
6. Be careful with negations and conditions (e.g. "don't", "without", "only when").

When done, reply with only: the number of items written and 5 sample lines (id: original → reworded). Do not paste the whole file in your reply.
```

## 5. Loanword stress test (`q_ko_loan`, not native questions)

```text
You are helping build a stress test for a code-search benchmark. Korean developers often write general technical terms as Hangul loanwords (transliterations of English words) instead of native Korean words, e.g. 프록시 (proxy), 파라미터 (parameter), 쿼리 (query), 헤더 (header), 리퀘스트 (request), 리스폰스 (response), 리팩토링 (refactoring), 리졸브 (resolve), 디폴트 (default), 리스너 (listener), 핸들러 (handler), 캐시 (cache), 스트림 (stream), 이슈 (issue), 버그 (bug), 픽스 (fix), 체크 (check), 세팅 (setting), 업데이트 (update), 리턴 (return), 타입 (type), 인스턴스 (instance), 메서드 (method), 클래스 (class), 필드 (field), 오버라이드 (override), 디펜던시 (dependency), 컨피그 (config), 인코딩 (encoding), 파싱 (parsing), 밸리데이션 (validation), 이니셜라이즈 (initialize).

Input (read with the Read tool): <input file>
A JSON list of 320 items {"id", "repo", "ko"}: a Korean question or request a developer might give a coding agent about an open-source repository.

Task: rewrite each Korean question in the style of a Korean developer who uses Hangul loanwords heavily. Wherever a general technical word (native Korean or Sino-Korean, e.g. 요청, 응답, 매개변수, 기본값, 설정, 검사, 반환, 초기화, 재시도, 수정) has a common Hangul loanword that Korean developers actually use, write the loanword instead (요청 → 리퀘스트, 기본값 → 디폴트, 설정 → 세팅 or 컨피그, 반환 → 리턴, 초기화 → 이니셜라이즈 or 초기화 if no loanword is common). Also write general English words that appear in Latin letters but are not code identifiers in Hangul (e.g. "progress 이벤트" → "프로그레스 이벤트"), but keep exact code identifiers (function, class, method, variable, option, flag, header, file and type names such as `syncHandlerEntries`, trusted_hosts, NO_PROXY, @Nested, range/3) exactly as written in Latin letters.

Rules:
1. Keep the meaning exactly: same request, same negations and conditions, no added or removed information. Do not add file names, function names or causes.
2. Do not look anything up: do not open any repository, website, commit or source code. Work only from the Korean text.
3. Keep it natural, as a real Korean developer would type it to a coding agent; only use loanwords that are actually common among Korean developers. If no word in a question has a common loanword, you may leave it unchanged.
4. One line per question.

Output (write with the Write tool): <output file>
Format: one JSON object mapping every id to the rewritten Korean string, e.g. {"axios-01": "...", ...}. All 320 ids must be present.

When done, reply with only: the number of items written, how many you left unchanged, and 5 sample lines (id: original → rewritten).
```
