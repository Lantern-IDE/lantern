# Phase 0 · 맥락 엔진 설계

> 상위 문서: 기획서(비공개) 4장, 8장
> 상태: 구현 완료, 정성 평가 대기 · v0.2 · 2026-09-24

## 1. 목표와 범위

IDE를 만들기 전에 **"맥락을 잘 조립하면 AI 답변이 나아지고 토큰이 줄어드는가"**를 검증한다.
그래서 맥락 엔진을 IDE와 분리된 단독 도구로 먼저 만든다.

| 구분 | 포함 | 제외 (다음 단계) |
|---|---|---|
| 인덱싱 | Tree-sitter 심볼·참조 추출, 전문 검색, git 동시 변경 분석, 증분 갱신 | LSP 기반 정밀 참조, 파일 감시(watch) 데몬 |
| 검색 | BM25 전문 검색 (식별자 분해 포함) | 임베딩 의미 검색 (Phase 0b, `embeddings` 기능 플래그) |
| 조립 | 후보 수집 → 그래프 확장 → 순위 → 예산 내 압축 → 근거 표시 | 모델 기반 재순위 |
| 인터페이스 | CLI, MCP 서버 (stdio) | IDE 연동 (Phase 1) |
| 언어 | Rust, Python, TypeScript/TSX, JavaScript | 그 외 언어 |

## 2. 구성

```
crates/lantern-context/
├─ src/
│  ├─ lib.rs          # 공개 API
│  ├─ lang.rs         # 확장자 → 언어, tree-sitter 태그 설정
│  ├─ indexer.rs      # 파일 순회, 증분 판단, 파싱 결과 저장
│  ├─ store.rs        # SQLite 스키마와 질의
│  ├─ tokenize.rs     # 식별자 분해 (camelCase, snake_case)
│  ├─ git.rs          # 동시 변경(co-change) 분석
│  ├─ assemble.rs     # 맥락 조립기
│  ├─ mcp.rs          # MCP 서버 (JSON-RPC over stdio)
│  └─ main.rs         # CLI (`lantern`)
```

- 인덱스 위치: `<프로젝트>/.lantern/index.db` (git 제외 대상)
- 프로젝트 메모리: `<프로젝트>/.lantern/memory/*.md`
- 외부 서비스 호출 없음. 모든 처리는 로컬.

## 3. 인덱스 스키마 (SQLite)

```sql
files(id, path UNIQUE, lang, hash, mtime, size, indexed_at)
symbols(id, file_id, name, kind, start_line, end_line,
        start_byte, end_byte, signature, doc)
refs(id, file_id, name, kind, line, enclosing_symbol_id)
symbols_fts  -- FTS5(name, path, terms, body) : BM25 검색용
cochange(file_a, file_b, count)                 -- a < b
meta(key, value)                                -- 스키마 버전, 마지막 git HEAD
```

설계 결정:

- **참조 해석은 이름 기반**으로 한다. `refs.name`과 `symbols.name`을 조회 시점에 조인한다.
  같은 이름의 함수가 여러 개면 오탐이 생기지만, Phase 0에서는 속도와 단순함을 택한다.
  동명 심볼이 많을 때는 같은 파일, import한 파일 순으로 가중치를 준다. LSP 연동은 Phase 1.
- **`terms` 컬럼**에 식별자를 분해한 단어를 넣는다. `parseUserConfig`는 `parse user config`로도 검색된다.
  사용자가 자연어로 물어도 코드 식별자와 연결되게 하기 위해서다.
- **`body`**는 심볼 본문 앞부분(최대 2,000자)만 넣는다. 인덱스 크기를 억제하기 위해서다.

## 4. 인덱싱

1. `ignore` 크레이트로 `.gitignore`를 따르며 파일을 순회한다. `.lantern/ignore`로 추가 제외 가능.
2. 1MB를 넘는 파일, 바이너리, 지원하지 않는 확장자는 건너뛴다.
3. **증분 판단**: `mtime`과 `size`가 같으면 건너뛰고, 다르면 내용 해시(blake3)를 비교한다.
   해시까지 같으면 `mtime`만 갱신한다.
4. 바뀐 파일은 기존 행을 지우고 다시 넣는다. 파일 단위 트랜잭션.
5. 사라진 파일은 인덱스에서 삭제한다.
6. git HEAD가 바뀌었으면 최근 커밋 500개로 co-change를 다시 계산한다.
   파일을 30개 넘게 건드린 커밋은 대량 변경(포매팅, 이름 변경 등)으로 보고 제외한다.

심볼과 참조 추출은 각 문법 크레이트가 제공하는 `tags.scm` 질의를 `tree-sitter-tags`로 실행한다.
정의(`@definition.*`)는 `symbols`로, 참조(`@reference.*`)는 `refs`로 저장한다.
참조가 어느 심볼 안에 있는지는 줄 범위로 가장 안쪽 정의를 찾아 `enclosing_symbol_id`에 넣는다.

**신선도 보장**: MCP 서버는 질의를 받을 때마다 증분 인덱싱을 먼저 실행한다.
바뀐 파일이 없으면 `stat` 비용만 들기 때문에 별도 감시 데몬 없이도 인덱스가 최신 상태로 유지된다.

## 5. 맥락 조립 알고리즘

입력: 질문 텍스트, 선택적 포커스(파일, 줄), 토큰 예산(기본 8,000)

```
1. 시드 수집
   - 질문을 식별자 분해 → FTS5 BM25 검색 → 상위 30개 심볼
   - 질문에 심볼 이름이 그대로 등장하면 가산점
   - 포커스 위치를 감싸는 심볼은 최고점 시드
2. 그래프 확장 (1홉, 점수 × 0.5)
   - 호출 대상(callee): 시드 본문 안의 참조 → 해당 이름의 정의
   - 호출자(caller): 시드 이름을 참조하는 곳의 감싸는 심볼
3. 동시 변경 가산
   - 시드 파일과 자주 함께 바뀐 파일의 심볼에 소폭 가산
4. 순위 정렬, 중복 제거 (부모 심볼이 들어가면 자식은 제외)
5. 예산 내 압축
   - 프로젝트 메모리를 예산의 최대 15%까지 먼저 배정
   - 상위 항목은 본문 전체, 예산이 부족해지면 시그니처만
   - 토큰 추정: 문자 수 ÷ 4 (추후 토크나이저로 교체)
6. 출력
   - Markdown(모델용) 또는 JSON(도구용)
   - 항목마다 포함 근거 표시: "'token' 일치"(이름·시그니처·설명에서 일치한 질문 단어), "본문에서 일치", "X가 사용", "X를 사용", "함께 자주 변경"
```

점수 식 (초기값, 평가로 조정):

```
score = 1.0 × bm25_정규화 + 2.0 × 이름_정확_일치 + 3.0 × 포커스
      + 0.5 × 그래프_이웃_점수 + 0.3 × cochange_정규화
```

## 6. 인터페이스

### CLI

| 명령 | 설명 |
|---|---|
| `lantern index [경로]` | 증분 인덱싱. 처리 파일 수와 시간 출력 |
| `lantern search <질의>` | 심볼 검색 결과 |
| `lantern symbol <이름>` | 정의, 호출자, 호출 대상 |
| `lantern context <질문> [--file F --line N] [--budget T] [--json]` | 조립된 맥락 출력 |
| `lantern stats` | 인덱스 통계 |
| `lantern bench <질문 파일>` | 지연 시간, 토큰 수를 기준 방식과 비교 |
| `lantern mcp [경로]` | MCP 서버 실행 (stdio) |

### MCP 도구

| 도구 | 입력 | 출력 |
|---|---|---|
| `get_context` | `query`, `file?`, `line?`, `budget_tokens?` | 조립된 맥락 (Markdown) |
| `search_symbols` | `query`, `limit?` | 심볼 목록 (이름, 종류, 위치, 시그니처) |
| `get_symbol` | `name` | 정의 본문, 호출자, 호출 대상 |
| `find_references` | `name` | 참조 위치 목록 |

MCP는 외부 SDK 없이 JSON-RPC 2.0을 직접 구현한다 (`initialize`, `tools/list`, `tools/call`, `ping`).
의존성을 줄이고, 명세 변경에 직접 대응하기 위해서다.

Claude Code 연결 예:

```bash
claude mcp add lantern -- lantern mcp /path/to/project
```

## 7. 검증 방법 (Phase 0 완료 기준)

**정량 (자동, `lantern bench`)**

- 초기·증분 인덱싱 시간, 맥락 조립 p50/p95
- 토큰 수: Lantern 맥락 vs 기준 방식("상위 결과가 속한 파일을 통째로 넣기")

**정성 (수동)**

1. 실제 프로젝트 2개에서 질문 20개를 만든다 (구조 이해, 버그 위치, 수정 영향, 규칙 준수 각 5개).
2. 같은 모델로 A(MCP 없음) / B(Lantern MCP 연결)로 답을 받는다.
3. 질문마다 정답 여부, 필요한 파일을 찾았는지, 사용 토큰을 기록한다 (`eval/결과.md` 양식).
4. **B가 정답률에서 앞서고 토큰은 같거나 적으면 통과.** 아니면 IDE 개발 전에 조립 방식을 고친다.

## 8. 목표 수치 (기획서 6장과 동일 기준)

| 항목 | 목표 |
|---|---|
| 초기 인덱싱 (10만 LOC, 임베딩 제외) | 10초 미만 |
| 증분 인덱싱 (파일 1개) | 200ms 미만 (p95) |
| 맥락 조립 | 300ms 미만 (p95) |
| 토큰 절감 | 기준 방식 대비 30% 이상 |

실측 (jellySafe, TS 880개): 초기 인덱싱 2.5초, 조립 p95 10ms, 토큰 53% 절감 → [eval/결과.md](../eval/결과.md)

구현 중 바뀐 점:
- Rust 태그 질의는 문법 크레이트 것을 쓰지 않고 직접 정의했다. tree-sitter-tags는 이름 노드 하나에 태그 하나만 만드는데, 원본 질의가 `impl Foo`의 `Foo`를 참조로 먼저 잡아서 impl 블록을 정의로 얻을 수 없었다.
- SQLite `cache_size`를 64MB로 올렸다. 기본값(2MB)에서는 초기 인덱싱 트랜잭션이 디스크로 계속 넘쳐 15초 걸렸고, 올린 뒤 1.9초가 됐다.
- 환경변수 `LANTERN_INDEX_DIR`을 추가했다. 설정하면 프로젝트 폴더에 `.lantern/`를 만들지 않고 인덱스를 밖에 둔다.

## 9. 다음 단계 (Phase 0b)

- 임베딩 의미 검색: 로컬 ONNX 임베딩 모델 + sqlite-vec, `embeddings` 기능 플래그
- 파일 감시 데몬 (`notify`), LSP 참조로 이름 기반 해석 보강
- 정확한 토크나이저로 토큰 추정 교체
