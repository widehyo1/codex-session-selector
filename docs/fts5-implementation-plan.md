# FTS5 검색 강화 구현 계획

## 1. 문서 상태와 목표 버전

- 상태: 구현 준비 완료
- 대상 기능: 3번 FTS5 검색 강화
- 목표 package version: `0.3.0` 유지
- 목표 index schema version: `2`
- 선행 조건:
  - TUI exec visibility toggle 완료
  - canonical incremental index schema v1 완료
- 공개 시점: 이 계획의 구현, 검증과 handoff 완료 이후 별도 승인 시점

이 문서는 구현자가 추가 제품·schema 결정을 하지 않고 그대로 red/green
workflow를 실행할 수 있는 기준이다. 이 문서를 작성하는 단계에서는 production
code, schema 또는 package version을 변경하지 않는다.

## 2. 목표와 사용자에게 보이는 최종 동작

1. selector의 `/` 검색은 schema v2 DB의 FTS5 index를 사용한다.
2. 검색 scope는 다음 순서로 순환한다.

   ```text
   all → message → cwd → branch → repo → date → exec → all
   ```

3. `all`은 first message, cwd, repository URL, branch, timestamp/date,
   exec command와 exec output을 검색한다.
4. `exec`은 command와 output만 검색한다.
5. exec 내용은 canonical index에 항상 저장되고 항상 검색된다. selector의
   `exec: shown|hidden` 상태는 다음 replay의 표시 상태만 뜻하며 검색 결과를
   바꾸지 않는다.
6. 공백으로 나눈 일반 검색어는 AND로 결합하고 각 검색어는 token prefix로
   해석한다. `"quoted phrase"`는 정확한 token phrase이고, 공백으로 분리한
   `|`는 OR group을 만든다.
7. 결과는 BM25 relevance가 높은 순서로 표시한다. 같은 rank에서는 최신
   timestamp, 큰 `session_key` 순으로 정렬한다.
8. 검색어가 비어 있으면 현재처럼 timestamp 내림차순의 전체 view를 표시한다.
9. 기본 subsession/empty-message filter와 CLI option의 의미는 유지한다.
10. schema v1 DB는 다음 refresh/index 때 canonical row를 다시 parse하지 않고
    schema v2와 FTS 문서로 자동 migration한다.
11. `--no-refresh`로 schema v0/v1 DB를 직접 여는 selector는 DB를 몰래
    변경하거나 서로 다른 검색 의미로 fallback하지 않는다. 다음 오류로
    종료한다.

    ```text
    search index schema 2 is required; refresh the index or run `select-codex-session index`
    ```

12. 잘못된 interactive query는 TUI를 종료하지 않는다. 직전 성공 결과를
    유지하고 footer status에 `search query error: ...`를 표시한다.
13. SQLite/FTS 오류도 직전 결과와 selector 상태를 유지하고
    `search failed: ...; refresh or rebuild the index`를 표시한다.

## 3. 구현 범위와 명시적 비범위

### 3.1 구현 범위

- Contentless-Delete FTS5 virtual table
- schema v1 → v2 in-place migration
- canonical/FTS 동일 transaction 동기화
- FTS dirty-state 감지 trigger
- FTS internal integrity와 row identity 검증
- 불일치 시 FTS-only rebuild
- message/metadata/exec command/output 검색
- scope별 MATCH query
- restricted query compiler와 FTS escaping
- BM25 column weight와 deterministic tie-break
- selector의 persistent read-only search connection
- 한국어, path, repository URL, command와 output fixture
- schema/index/search/selector/CLI 회귀 test
- index 시간, DB 크기와 query latency benchmark
- README와 구현 handoff 갱신

### 3.2 명시적 비범위

- replay JSONL 검색
- result snippet/highlight UI
- typo tolerance, stemming, fuzzy search 또는 semantic/vector search
- raw FTS5 query 문법 노출
- `NOT`, `NEAR`, 괄호, 사용자 지정 column filter
- 별도 exact-term operator
- 검색 결과 개수 제한, pagination 또는 background query thread
- query history나 검색 설정의 disk persistence
- custom tokenizer, ICU tokenizer 또는 한국어 형태소 분석기
- exec 종류/name/call_id/session_id 검색
- index DB 암호화
- canonical `sessions`/`exec_events` column 변경
- 외부 process가 FTS shadow table을 직접 변경한 경우의 무손실 복구 보장
- 새 CLI search subcommand
- 새 dependency, Cargo feature 또는 Rust minimum version 변경
- package version 변경, publish, tag 또는 GitHub release

`filter_sessions`와 `searchable_text` 공개 함수는 API 호환을 위해 유지한다.
이 함수의 기존 in-memory substring 의미도 바꾸지 않지만 selector production
경로에서는 더 이상 사용하지 않는다.

## 4. 현재 동작과 호환성 계약

### 4.1 현재 baseline

- package: `0.3.0`
- Rust minimum: `1.97`
- `rusqlite`: `0.40.1`, `bundled` feature
- lockfile의 `libsqlite3-sys`: `0.38.1`
- bundled SQLite: `3.53.2`
- canonical schema: `PRAGMA user_version = 1`
- stable identities:
  - `sessions.session_key`
  - `exec_events.exec_key`
  - `IndexDelta.touched_session_keys`
- index transaction:
  - `BEGIN IMMEDIATE`
  - canonical mutation
  - foreign key/quick check/invariant 확인
  - `user_version` 기록
  - commit
- selector:
  - 전체 `SessionRow`를 memory에 읽음
  - query마다 lowercase all-term substring filter
  - scope: `all`, `message`, `cwd`, `branch`, `repo`, `date`
  - filter 갱신 시 첫 row 선택
  - 빈 결과면 selection `None`
- exec visibility:
  - normal mode의 `e`만 toggle
  - search mode의 `e`는 query 문자
  - 다음 replay 표시 상태만 제어
- legacy DB load:
  - `load_sessions_with_view`는 old seven-column `sessions`도 읽을 수 있음
- FTS table, FTS trigger와 search API는 없음

### 4.2 유지할 동작

- query가 비면 timestamp 내림차순
- query 수정 또는 scope 전환 성공 시 첫 결과 선택
- 빈 결과의 selection `None`
- search `Tab`, `Enter`, `Esc`, `Backspace`와 문자 입력 key 의미
- replay 복귀 뒤 query, scope, focus, selection과 exec visibility 유지
- `--include-subsessions`, `--include-empty-messages` read-time filter
- `--include-exec`와 TUI `e`의 replay visibility 의미
- canonical index가 exec event를 option과 무관하게 항상 저장하는 계약
- legacy/unknown/future schema의 overwrite 방지
- root가 바뀌거나 `--rebuild`이면 canonical full rebuild
- index 실패 시 기존 DB 전체 rollback
- `load_sessions` 공개 API와 기존 legacy read compatibility

### 4.3 의도적으로 달라지는 동작

| 현재 | 구현 후 |
| --- | --- |
| selector가 모든 row를 memory substring scan | schema v2 FTS5 MATCH query |
| 모든 term이 임의 문자열 중간에서도 match | 일반 term은 token prefix match |
| 입력 순서인 최신순만 사용 | BM25, timestamp, session_key 순 |
| exec 검색 scope 없음 | `exec` scope와 `all`의 exec 검색 |
| schema v1 | schema v2 |
| canonical table 4개, trigger 없음 | FTS/state/shadow table과 dirty trigger 추가 |
| v0/v1 `--no-refresh` selector 허용 | actionable schema error |

예를 들어 `README` token은 `read`와 `readm`으로 찾을 수 있지만 중간 문자열
`ead`로는 찾지 못한다. `Fix README parser`는 `fix read`로 찾을 수 있고,
`"readme parser"`는 두 token이 연속할 때만 찾는다.

## 5. 확정한 기술·구조 결정

### 5.1 SQLite와 FTS5 runtime

Contentless-Delete는 SQLite `3.43.0`부터 지원된다. 현재 bundled SQLite
`3.53.2`는 이 조건을 만족한다. `libsqlite3-sys`의 bundled build도
`SQLITE_ENABLE_FTS5`를 설정한다.

구현은 connection을 연 직후 다음을 확인한다.

```sql
SELECT sqlite_version();
SELECT sqlite_compileoption_used('ENABLE_FTS5');
```

- version이 `3.43.0`보다 낮으면 즉시 오류
- FTS5 compile option이 `0`이면 즉시 오류
- exact error:

  ```text
  SQLite 3.43.0 or newer with FTS5 is required; found <version>
  ```

관련 SQLite 공식 문서:

- [FTS5와 Contentless-Delete](https://www.sqlite.org/fts5.html#contentless_delete_tables)
- [SQLite 3.43.0 release](https://www.sqlite.org/releaselog/3_43_0.html)
- [FTS5 tokenizer와 query syntax](https://www.sqlite.org/fts5.html#full_text_query_syntax)
- [FTS5 BM25와 rank](https://www.sqlite.org/fts5.html#the_bm25_function)

`rusqlite` feature와 dependency는 변경하지 않는다. system `sqlite3` CLI
version은 production binary가 사용하는 bundled SQLite version이 아니므로
runtime 판정 근거로 쓰지 않는다.

### 5.2 FTS document identity와 column

FTS 문서는 session당 정확히 하나다.

```text
sessions_fts.rowid == sessions.session_key
```

indexed column 순서와 source는 고정한다.

| 순서 | FTS column | canonical source | BM25 weight |
| ---: | --- | --- | ---: |
| 0 | `first_message` | `sessions.first_message` | 10.0 |
| 1 | `cwd` | `sessions.cwd` 또는 `""` | 4.0 |
| 2 | `repository_url` | `sessions.repository_url` 또는 `""` | 4.0 |
| 3 | `branch` | `sessions.branch` 또는 `""` | 5.0 |
| 4 | `timestamp` | `sessions.timestamp` 또는 `""` | 2.0 |
| 5 | `date` | timestamp의 첫 10 Unicode scalar 또는 `""` | 2.0 |
| 6 | `exec_command` | event_index 순 command의 newline join | 1.5 |
| 7 | `exec_output` | event_index 순 output의 newline join | 0.25 |

exec command와 output 사이에는 각각 newline 하나만 넣는다. 빈 value도
순서를 바꾸지 않지만 최종 문자열의 불필요한 trailing newline은 넣지 않는다.
`kind`, `name`, `call_id`, `session_id`는 검색하지 않는다.

하나의 session FTS row를 선택한 이유:

- selector result identity가 session이므로 별도 rank merge가 필요 없음
- 한 session의 metadata와 exec term을 AND로 결합할 수 있음
- `touched_session_keys`만으로 document 전체를 다시 만들 수 있음
- session delete는 stable rowid 하나의 DELETE임

별도 session/exec FTS table은 서로 다른 BM25 score를 합치는 임의 규칙이
필요하므로 사용하지 않는다. external-content table은 exec의 1:N aggregate가
단일 canonical content table과 직접 대응하지 않으므로 사용하지 않는다.
contentful FTS table은 canonical text를 한 번 더 저장하므로 사용하지 않는다.

### 5.3 tokenizer

다음을 사용한다.

```text
unicode61 remove_diacritics 2
prefix='2 3'
detail=full
columnsize=1
```

- `unicode61`: Unicode letter/number token, case-insensitive 검색
- `remove_diacritics 2`: Latin diacritic 정규화
- 2/3-token prefix index: 짧은 한국어·path component·command prefix 가속
- `detail=full`: phrase와 column scope 지원
- default `columnsize=1`: BM25 계산 지원

`trigram`은 임의 substring을 보존하지만 3 Unicode character 미만의 FTS
query가 match하지 않아 짧은 한국어 검색이 불안정하고 index 크기가 커지므로
사용하지 않는다. custom 한국어 tokenizer와 형태소 분석은 비범위다.

한국어는 공백 없는 전체 어절이 하나의 token이다. 따라서 `검색기능`은
`검색` prefix로 찾지만 중간 token인 `기능`으로는 찾지 못한다. 이 동작을
fixture와 README에 명시한다.

### 5.4 query 문법

사용자 입력을 FTS5 raw query로 전달하지 않는다. Rust parser가 다음 restricted
문법만 받는다.

```text
query       := or_group (WS "|" WS or_group)*
or_group    := atom (WS atom)*
atom        := bare | quoted
bare        := 한 개 이상의 non-whitespace character, 단독 "|" 제외
quoted      := '"' (escaped_quote | escaped_backslash | character)* ['"']
```

규칙:

1. 같은 group의 atom은 `AND`.
2. 공백 양쪽에 있는 `|`만 `OR`. `foo|bar`는 한 bare atom이다.
3. bare atom은 token phrase prefix다.
4. quoted atom은 exact token phrase다.
5. quote가 닫히지 않은 입력은 interactive typing 상태로 보고 문자열 끝에서
   닫힌 것으로 처리한다.
6. quoted text에서 `\"`와 `\\`만 escape다. 다른 `\x`는 `\`와 `x` literal이다.
7. atom에 Unicode alphanumeric character가 하나도 없으면
   `query contains no searchable token` 오류다.
8. leading/trailing/consecutive OR는 `OR requires an expression on both sides`
   오류다.
9. FTS5 `NOT`, `NEAR`, `:`, `{}`, `()`, `+`, `-`, `*`는 operator로 노출하지
   않고 atom text로 quote한다.
10. 빈 query 또는 whitespace-only query는 MATCH를 실행하지 않는다.

compile 예:

```text
fix read
→ ("fix"* AND "read"*)

"readme parser" | cargo test
→ ("readme parser") OR ("cargo"* AND "test"*)
```

FTS string escape는 `"`를 `""`로 바꾼 뒤 SQL bind parameter에 넣는다.
query text를 SQL string interpolation하지 않는다.

scope는 compiler 결과 전체에 다음 column filter를 적용한다.

```text
all      → filter 없음
message  → {first_message} : (...)
cwd      → {cwd} : (...)
branch   → {branch} : (...)
repo     → {repository_url} : (...)
date     → {date} : (...)
exec     → {exec_command exec_output} : (...)
```

### 5.5 ranking과 정렬

non-empty query SQL은 `rank`의 BM25 mapping을 고정한다.

```sql
rank MATCH
  'bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)'
```

SQLite FTS5의 BM25는 더 좋은 match가 더 작은 숫자다. 정렬은 다음과 같다.

```sql
ORDER BY
    sessions_fts.rank ASC,
    sessions.timestamp DESC,
    sessions.session_key DESC
```

NULL timestamp는 SQLite DESC 정렬에 따라 non-NULL 뒤에 온다. 결과 제한은
두지 않는다. query가 비면 rank를 계산하지 않고 다음만 사용한다.

```sql
ORDER BY sessions.timestamp DESC, sessions.session_key DESC
```

### 5.6 exec visibility와 검색의 관계

FTS document는 `--include-exec`와 TUI `ExecVisibility`에 무관하게 command와
output을 항상 포함한다. `all`과 `exec` scope도 visibility와 무관하다.

이 결정은 현재 canonical DB가 exec를 항상 저장한다는 계약과
`exec: shown|hidden`이 replay view만 뜻한다는 계약을 유지한다. normal mode의
`e`로 visibility를 바꿔도 query를 다시 실행하거나 결과/selection을 바꾸지
않는다. search mode의 `e`는 계속 query 문자다.

## 6. 목표 schema

### 6.1 schema version

```rust
pub(crate) const SCHEMA_VERSION: i64 = 2;
pub(crate) const FTS_MIN_SQLITE: (u32, u32, u32) = (3, 43, 0);
```

canonical v1의 네 table과 index는 그대로 유지한다. 다음 object만 추가한다.

### 6.2 exact DDL

```sql
CREATE TABLE fts_sync_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    dirty INTEGER NOT NULL CHECK (dirty IN (0, 1))
) STRICT;

INSERT INTO fts_sync_state(singleton, dirty) VALUES (1, 1);

CREATE VIRTUAL TABLE sessions_fts USING fts5(
    first_message,
    cwd,
    repository_url,
    branch,
    timestamp,
    date,
    exec_command,
    exec_output,
    content='',
    contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3',
    detail=full,
    columnsize=1
);

CREATE TRIGGER sessions_fts_dirty_ai
AFTER INSERT ON sessions
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER sessions_fts_dirty_au
AFTER UPDATE ON sessions
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER sessions_fts_dirty_ad
AFTER DELETE ON sessions
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER exec_events_fts_dirty_ai
AFTER INSERT ON exec_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER exec_events_fts_dirty_au
AFTER UPDATE ON exec_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER exec_events_fts_dirty_ad
AFTER DELETE ON exec_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;
```

`fts_sync_state.dirty`는 canonical text가 FTS와 다를 수 있음을 뜻한다.
trigger는 FTS를 직접 변경하지 않는다. production indexer가 canonical
mutation 뒤 aggregate document를 한 번 만든 후 FTS를 동기화하고 clean으로
전환한다.

예상 FTS shadow table:

```text
sessions_fts_data
sessions_fts_idx
sessions_fts_docsize
sessions_fts_config
```

`sessions_fts_content`는 contentless이므로 없어야 한다.

### 6.3 schema validation

schema v2 validation은 다음을 exact하게 검사한다.

- 기존 canonical table/column/STRICT/UNIQUE/foreign key
- `fts_sync_state` column, STRICT, singleton row
- `sessions_fts`가 virtual table
- 8개 user column 순서
- `sqlite_schema.sql`의 FTS option:
  - `content=''`
  - `contentless_delete=1`
  - tokenizer
  - prefix
  - detail
  - columnsize
- 예상 shadow table set
- 여섯 trigger의 name, target table, timing/event와 body
- unexpected table/view/trigger 없음

whitespace와 SQLite가 보존하지 않는 trailing semicolon 차이만 normalize한다.
keyword case나 option 값이 다른 object는 schema v2로 인정하지 않는다.

### 6.4 v1 migration

`SchemaState`는 다음을 구분한다.

```rust
pub(crate) enum SchemaState {
    Empty,
    Legacy,
    CanonicalV1 { sessions_root: PathBuf },
    Current { sessions_root: PathBuf },
    Future { version: i64 },
    Unknown { version: i64, reason: String },
}
```

v1 schema가 exact canonical v1일 때만 `CanonicalV1`이다. migration은 같은
root이고 `--rebuild`가 아닐 때 다음 한 `BEGIN IMMEDIATE` transaction에서
수행한다.

```text
v1 fingerprint load와 source scan
→ fts_sync_state/FTS/trigger 생성(dirty=1)
→ canonical incremental mutation
→ 모든 canonical session에서 FTS population
→ FTS invariant 확인
→ dirty=0
→ canonical invariant 확인
→ PRAGMA user_version=2
→ commit
```

unchanged source는 다시 parse하지 않는다. migration 중 하나라도 실패하면
v1 DB 전체를 보존한다. root가 다르거나 `--rebuild`이면 기존 정책대로
canonical+FTS full rebuild를 수행한다.

## 7. 목표 파일 구조와 책임

```text
src/
  application.rs
  selector/mod.rs
  indexer.rs
  indexer/
    schema.rs
    store.rs
    fts.rs          # 새 파일: DDL lifecycle, document build/sync/health
    search.rs       # 새 파일: query parser/compiler, read-only search API
  test_support.rs
tests/
  cli.rs
  fts_benchmark.rs  # ignored release benchmark
scripts/
  benchmark-fts.sh
docs/
  fts5-implementation-plan.md
    fts5-handoff.md   # 실행 handoff; 구현 완료 시 결과를 갱신
README.md
```

- `schema.rs`
  - schema version/state/detection
  - canonical/FTS object exact validation
  - create/drop/ensure schema orchestration
- `store.rs`
  - canonical row mutation과 existing view loader 유지
  - FTS 구현 세부사항을 소유하지 않음
- `fts.rs`
  - runtime capability check
  - `SearchDocument` 생성
  - FTS create/drop/populate/delta sync
  - dirty/internal/rowid health와 repair
- `search.rs`
  - 사용자 query parse/escape/compile
  - read-only connection
  - view filter와 ranked row query
- `indexer.rs`
  - v1 migration과 v2 sync mode 선택
  - canonical mutation과 FTS mutation transaction 순서
- `selector/mod.rs`
  - `SearchIndex` 소유
  - scope `Exec`
  - refresh 성공/실패 state transition
- `application.rs`
  - schema v2 search backend open
  - selector 생성과 actionable schema error
- `test_support.rs`
  - FTS용 session/exec fixture helper
- `fts_benchmark.rs`, `benchmark-fts.sh`
  - deterministic corpus와 측정 output

## 8. 타입과 함수 시그니처

### 8.1 `indexer/fts.rs`

```rust
pub(crate) const FTS_DDL: &str;
pub(crate) const FTS_TRIGGER_DDL: &str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchDocument {
    pub session_key: SessionKey,
    pub first_message: String,
    pub cwd: String,
    pub repository_url: String,
    pub branch: String,
    pub timestamp: String,
    pub date: String,
    pub exec_command: String,
    pub exec_output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FtsSyncMode {
    Delta,
    Populate,
    Rebuild,
}

pub(crate) fn verify_runtime(conn: &Connection) -> Result<()>;
pub(crate) fn create_schema(tx: &Transaction<'_>) -> Result<()>;
pub(crate) fn drop_schema(tx: &Transaction<'_>) -> Result<()>;
pub(crate) fn preflight(tx: &Transaction<'_>) -> Result<FtsSyncMode>;
pub(crate) fn load_document(
    tx: &Transaction<'_>,
    session_key: SessionKey,
) -> Result<Option<SearchDocument>>;
pub(crate) fn populate_all(tx: &Transaction<'_>) -> Result<()>;
pub(crate) fn rebuild(tx: &Transaction<'_>) -> Result<()>;
pub(crate) fn apply_delta(
    tx: &Transaction<'_>,
    delta: &IndexDelta,
) -> Result<()>;
pub(crate) fn verify_invariants(
    tx: &Transaction<'_>,
    check_internal_index: bool,
) -> Result<()>;
pub(crate) fn mark_clean(tx: &Transaction<'_>) -> Result<()>;
```

### 8.2 `indexer/search.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchScope {
    All,
    FirstMessage,
    Cwd,
    Branch,
    Repository,
    Date,
    Exec,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchHit {
    pub row: SessionRow,
    pub rank: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueryAtom {
    Prefix(String),
    Phrase(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueryGroup {
    pub atoms: Vec<QueryAtom>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchQuery {
    pub groups: Vec<QueryGroup>,
}

pub(crate) struct SearchIndex {
    conn: Connection,
    view: SessionView,
}

pub(crate) fn parse_query(input: &str) -> Result<SearchQuery, QueryError>;
pub(crate) fn compile_match(
    query: &SearchQuery,
    scope: SearchScope,
) -> Result<String, QueryError>;

impl SearchIndex {
    pub(crate) fn open(path: &Path, view: SessionView) -> Result<Self>;
    pub(crate) fn search(
        &self,
        input: &str,
        scope: SearchScope,
    ) -> Result<Vec<SearchHit>>;
}
```

`QueryError`는 `thiserror` dependency를 추가하지 않고 `Display`,
`std::error::Error`를 직접 구현한다. `SearchHit.rank`는 non-empty query
test용이다. empty query에서는 `0.0`을 넣는다.

`SearchScope`는 selector module에서 `search.rs`로 이동해 compiler와 UI가
같은 enum을 사용한다.

### 8.3 selector

```rust
pub(crate) struct SelectorApp {
    search_index: SearchIndex,
    filtered: Vec<SessionRow>,
    list_state: ListState,
    query: String,
    search_scope: SearchScope,
    // 기존 나머지 field 유지
}

impl SelectorApp {
    pub(crate) fn new(
        search_index: SearchIndex,
        exec_visibility: ExecVisibility,
    ) -> Result<Self>;

    fn refresh_filter(&mut self);
}
```

production `SelectorApp`에서 전체 source `rows` field를 제거한다. test도
실제 in-memory schema v2 DB에 fixture를 넣어 `SearchIndex`를 만들며 별도
mock search semantics를 만들지 않는다.

## 9. 주요 흐름과 pseudocode

### 9.1 document 생성

```rust
fn load_document(
    tx: &Transaction<'_>,
    session_key: SessionKey,
) -> Result<Option<SearchDocument>> {
    let Some(session) = query_session(tx, session_key).optional()? else {
        return Ok(None);
    };

    let mut commands = Vec::new();
    let mut outputs = Vec::new();
    for event in query_execs_ordered_by_event_index(tx, session_key)? {
        commands.push(event.command);
        outputs.push(event.output);
    }

    Ok(Some(SearchDocument {
        session_key,
        first_message: session.first_message,
        cwd: session.cwd.unwrap_or_default(),
        repository_url: session.repository_url.unwrap_or_default(),
        branch: session.branch.unwrap_or_default(),
        timestamp: session.timestamp.clone().unwrap_or_default(),
        date: session
            .timestamp
            .as_deref()
            .map(|value| value.chars().take(10).collect())
            .unwrap_or_default(),
        exec_command: commands.join("\n"),
        exec_output: outputs.join("\n"),
    }))
}
```

### 9.2 incremental sync

canonical mutation이 먼저 일어나므로 dirty trigger가 `dirty=1`로 만든다.

```rust
fn apply_delta(tx: &Transaction<'_>, delta: &IndexDelta) -> Result<()> {
    let deleted = delta
        .deleted_session_keys
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for session_key in &deleted {
        tx.execute(
            "DELETE FROM sessions_fts WHERE rowid = ?1",
            params![session_key],
        )?;
    }

    for session_key in normalized_unique(&delta.touched_session_keys) {
        if deleted.contains(&session_key) {
            continue;
        }
        let document = load_document(tx, session_key)?
            .with_context(|| format!("touched session {session_key} is missing"))?;
        tx.execute(INSERT_OR_REPLACE_ALL_FTS_COLUMNS, document.params())?;
    }
    Ok(())
}
```

Contentless-Delete UPDATE는 모든 user column을 제공해야 한다. 구현은 이
제약을 단순하게 지키기 위해 `INSERT OR REPLACE`와 명시적 rowid를 사용한다.

### 9.3 preflight, repair와 transaction

```rust
fn build_index(options: &IndexOptions) -> Result<IndexOutcome> {
    let root = absolute_lexical_path(&options.sessions_root)?;
    let mut conn = store::open_configured_connection(&options.output)?;
    fts::verify_runtime(&conn)?;
    let state = schema::detect_schema(&conn)?;
    let mode = choose_mode(&state, &root, options.rebuild)?;
    let stored = load_stored_fingerprints_for(mode, &state, &conn)?;
    let plan = scan_sources(&root, &stored, mode.is_rebuild())?;

    let tx = store::begin_immediate(&mut conn)?;
    let fts_mode = match state {
        SchemaState::CanonicalV1 { .. } if mode == Incremental => {
            fts::create_schema(&tx)?;
            FtsSyncMode::Populate
        }
        SchemaState::Current { .. } if mode == Incremental => fts::preflight(&tx)?,
        // apply_canonical(Rebuild)가 새 target schema와 빈 FTS table을 생성한다.
        _ => FtsSyncMode::Populate,
    };

    let mut delta = apply_canonical(mode, &tx, &root, &plan)?;

    match fts_mode {
        FtsSyncMode::Delta => fts::apply_delta(&tx, &delta)?,
        FtsSyncMode::Populate => fts::populate_all(&tx)?,
        FtsSyncMode::Rebuild => fts::rebuild(&tx)?,
    }

    fts::verify_invariants(
        &tx,
        !matches!(fts_mode, FtsSyncMode::Delta),
    )?;
    fts::mark_clean(&tx)?;
    store::verify_invariants(&tx)?;
    store::set_schema_version(&tx)?;
    let counts = store::query_counts(&tx)?;
    tx.commit()?;
    normalize_delta(&mut delta);
    Ok(outcome(mode, plan, counts, delta))
}
```

`preflight`의 판정 순서:

```text
fts_sync_state singleton/dirty 확인
→ dirty=1이면 Rebuild
→ sessions와 sessions_fts rowid 집합 양방향 차집합 확인
→ 차이가 있으면 Rebuild
→ 차이가 없으면 Delta
```

`Populate`는 migration 또는 canonical full rebuild가 이미 만든 빈 FTS
table에 모든 session document를 채운다. `Rebuild`는 canonical table은
보존하고 여섯 trigger, `sessions_fts`,
shadow table과 `fts_sync_state`만 drop/create한 뒤 모든 session document를
채운다. repair도 현재 canonical/index mutation과 같은 transaction 안에서
끝난다. repair 실패 시 transaction 전체를 rollback한다.

### 9.4 FTS invariant

```rust
fn verify_invariants(
    tx: &Transaction<'_>,
    check_internal_index: bool,
) -> Result<()> {
    if check_internal_index {
        // migration, full rebuild와 FTS-only repair에서 실행
        tx.execute(
            "INSERT INTO sessions_fts(sessions_fts, rank)
             VALUES('integrity-check', 0)",
            [],
        )?;
    }

    let missing = tx.query_row(
        "SELECT count(*)
         FROM sessions s
         LEFT JOIN sessions_fts f ON f.rowid = s.session_key
         WHERE f.rowid IS NULL",
        [],
        get_i64,
    )?;
    let extra = tx.query_row(
        "SELECT count(*)
         FROM sessions_fts f
         LEFT JOIN sessions s ON s.session_key = f.rowid
         WHERE s.session_key IS NULL",
        [],
        get_i64,
    )?;
    ensure!(missing == 0 && extra == 0, "FTS row identity mismatch");
    Ok(())
}
```

Contentless table은 canonical text를 저장하지 않으므로 token과 source text를
직접 byte 비교할 수 없다. 대신 다음 조합이 correctness boundary다.

- 모든 canonical write가 dirty trigger를 실행
- 같은 transaction에서 production sync 후에만 dirty를 0으로 전환
- rowid set 양방향 일치
- FTS internal integrity-check
- schema object exact validation

전체 FTS `integrity-check`는 migration, canonical full rebuild와 FTS-only
repair 뒤에는 실행하지만 healthy delta마다 실행하지 않는다. 정상 incremental
refresh가 전체 token corpus 크기에 비례하지 않게 하기 위한 결정이다.

FTS shadow table을 외부 도구가 직접 변경하는 것은 지원하지 않는다. query에서
`SQLITE_CORRUPT` 또는 `SQLITE_CORRUPT_VTAB`이 나타나면 status가
`select-codex-session index --rebuild`를 안내한다. forced rebuild가
canonical table과 FTS를 함께 다시 만들고 internal integrity-check까지
실행한다. busy, I/O, permission 같은 다른 SQLite error를 corruption으로
오인해 자동 rebuild하지 않는다.

### 9.5 search query

```rust
fn search(&self, input: &str, scope: SearchScope) -> Result<Vec<SearchHit>> {
    if input.trim().is_empty() {
        return query_unranked_sessions(&self.conn, self.view);
    }

    ensure_clean_state(&self.conn)?;
    let ast = parse_query(input)?;
    let match_query = compile_match(&ast, scope)?;

    query_map(
        &self.conn,
        SEARCH_SQL,
        params![
            match_query,
            i64::from(self.view.include_subsessions),
            i64::from(self.view.include_empty_messages),
        ],
        map_search_hit,
    )
}
```

exact ranked SQL:

```sql
SELECT
    s.path, s.id, s.timestamp, s.cwd, s.repository_url, s.branch,
    s.first_message, s.is_subsession, sessions_fts.rank
FROM sessions_fts
JOIN sessions AS s ON s.session_key = sessions_fts.rowid
WHERE sessions_fts MATCH ?1
  AND sessions_fts.rank MATCH
      'bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)'
  AND (?2 = 1 OR s.is_subsession = 0)
  AND (?3 = 1 OR s.has_nonempty_first_message = 1)
ORDER BY sessions_fts.rank ASC, s.timestamp DESC, s.session_key DESC
```

### 9.6 selector 오류와 lifecycle

`SearchIndex`는 read-only connection 하나를 `SelectorApp`과 같은 lifetime으로
소유한다. `open`할 때 exact schema, clean state와 canonical/FTS rowid 집합을
한 번 검사한다. query마다 DB를 다시 열거나 전체 rowid 집합을 scan하지 않고
O(1) singleton dirty-state만 다시 확인한다. 외부 canonical write는 dirty
trigger로 검출된다.

```rust
fn refresh_filter(&mut self) {
    match self.search_index.search(&self.query, self.search_scope) {
        Ok(rows) => {
            self.filtered = rows.into_iter().map(|hit| hit.row).collect();
            self.list_state.select((!self.filtered.is_empty()).then_some(0));
            self.message_scroll = 0;
            self.status = None;
        }
        Err(error) => {
            // query와 직전 결과/selection/scroll은 유지
            self.status = Some(format_search_error(error));
        }
    }
}
```

scope 전환에서 오류가 나도 새 scope 값은 유지해 사용자가 query를 수정할 수
있게 한다. DB write는 selector lifecycle에 없다.

## 10. 단계별 구현 순서

### Phase 0: baseline 고정

1. 현재 checkout에서 공통 quality gate를 실행한다.
2. current substring, scope, exec visibility, v1 schema와 incremental delta
   characterization test가 green인지 확인한다.
3. bundled SQLite version/FTS compile option을 test로 기록한다.

커밋 가능 조건:

- production 동작 변경 없음
- characterization test만 추가했다면 처음부터 green
- 전체 gate green

권장 commit:

```text
test: characterize pre-FTS search and schema
```

### Phase 1: schema v2와 v1 migration

1. schema v2 DDL/validation test를 red로 작성한다.
2. `SchemaState::CanonicalV1` detection test를 red로 작성한다.
3. FTS runtime version/compile option test를 red로 작성한다.
4. `fts_sync_state`, virtual table, shadow table과 trigger DDL을 구현한다.
5. schema version을 2로 올린다.
6. v1 same-root migration transaction을 구현한다.
7. forced rebuild와 rollback 회귀 test를 통과시킨다.

커밋 가능 조건:

- v1 DB가 canonical reparse 없이 v2가 됨
- unknown/future overwrite 방지 유지
- exact schema test green
- 전체 gate green

권장 commit:

```text
feat: add FTS5 schema and v1 migration
```

### Phase 2: document build와 transactional sync

1. document column/exec ordering test를 red로 작성한다.
2. full FTS population을 구현한다.
3. insert/update/delete delta test를 red로 작성한다.
4. `touched_session_keys` 기반 Contentless-Delete sync를 구현한다.
5. dirty trigger와 clean transition test를 통과시킨다.
6. FTS 실패 rollback test를 통과시킨다.

커밋 가능 조건:

- canonical/FTS atomicity 확인
- unchanged source/document는 rewrite하지 않음
- all delta cases green
- 전체 gate green

권장 commit:

```text
feat: synchronize FTS documents incrementally
```

### Phase 3: health check와 FTS-only repair

1. dirty state와 missing/extra rowid test를 red로 작성한다.
2. preflight와 invariant를 구현한다.
3. canonical table을 보존하는 FTS-only rebuild를 구현한다.
4. shadow corruption의 query 오류와 forced rebuild 복구 test를 작성한다.
5. repair 실패 rollback을 확인한다.

커밋 가능 조건:

- dirty/missing/extra mismatch가 다음 index에서 repair됨
- shadow corruption이 forced rebuild로 repair됨
- canonical `session_key`가 repair 전후 동일
- 전체 gate green

권장 commit:

```text
feat: detect and repair stale FTS indexes
```

### Phase 4: query compiler와 ranked search API

1. parser/escape/AND/OR/prefix/phrase test를 red로 작성한다.
2. scope column filter test를 작성한다.
3. injection 문자열을 bind parameter로만 처리하는 test를 작성한다.
4. query compiler를 구현한다.
5. read-only `SearchIndex`와 BM25 SQL을 구현한다.
6. ranking/tie-break/view filter test를 통과시킨다.

커밋 가능 조건:

- raw FTS operator가 주입되지 않음
- 모든 fixture/scope/ranking test green
- 전체 gate green

권장 commit:

```text
feat: query FTS index with scoped BM25 ranking
```

### Phase 5: selector 통합

1. `SearchScope::Exec` cycle/label/help test를 red로 작성한다.
2. application이 schema v2 `SearchIndex`를 열도록 변경한다.
3. selector가 FTS result를 사용하도록 변경한다.
4. query/DB 오류 상태 보존 test를 통과시킨다.
5. exec visibility와 search independence test를 통과시킨다.
6. v0/v1 `--no-refresh` actionable error를 integration test로 고정한다.

커밋 가능 조건:

- selector search production 경로에 memory scan 없음
- 기존 key/lifecycle 회귀 test green
- 전체 gate green

권장 commit:

```text
feat: use FTS5 search in the selector
```

### Phase 6: benchmark, 문서와 handoff

1. deterministic benchmark fixture와 script를 추가한다.
2. release mode benchmark를 실행하고 결과를 handoff에 기록한다.
3. README의 search/schema/migration 설명을 갱신한다.
4. `docs/fts5-handoff.md`의 progress와 완료 결과를 갱신한다.
5. 최종 quality/package/manual gate를 실행한다.

권장 commit:

```text
docs: document FTS5 search and validation
```

## 11. red/green과 커밋 정책

각 phase는 다음 순서를 지킨다.

```text
test 작성
→ 대상 test를 실행해 예상한 이유로 red 확인
→ 같은 phase의 production 구현
→ 대상 test green
→ scripts/check-before-commit.sh
→ cargo build --release
→ git diff --check
→ test와 구현을 함께 green commit
```

- red 상태를 commit하지 않는다.
- schema version만 먼저 올리는 commit을 만들지 않는다.
- FTS DDL만 있고 sync가 없는 중간 commit을 만들지 않는다.
- migration과 rollback test는 같은 phase/commit에 둔다.
- benchmark threshold 실패를 무시하고 문서만 완료 처리하지 않는다.
- 사용자 요청 없이 publish/tag/release하지 않는다.

## 12. pre-commit과 CI 자동화

기존 single source of truth를 유지한다.

```text
scripts/check-before-commit.sh
  cargo fmt --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-targets --all-features

.githooks/pre-commit
  → scripts/check-before-commit.sh

.github/workflows/ci.yml
  → scripts/check-before-commit.sh
```

FTS unit/integration test는 ignored 처리하지 않아 local hook과 CI에서 항상
실행한다. 성능 benchmark만 `#[ignore = "release benchmark"]`로 두고
`scripts/benchmark-fts.sh`가 release mode로 명시 실행한다.

script 변경이 있으면 다음도 실행한다.

```bash
bash -n scripts/check-before-commit.sh
bash -n scripts/benchmark-fts.sh
bash -n scripts/install-git-hooks.sh
```

## 13. 구체적 테스트 계획

### 13.1 schema/runtime unit test

`src/indexer/schema.rs`, `src/indexer/fts.rs`:

- `bundled_sqlite_supports_contentless_delete_fts5`
  - `sqlite_version() >= 3.43.0`
  - `sqlite_compileoption_used('ENABLE_FTS5') == 1`
- `schema_v2_uses_contentless_delete_with_expected_columns`
  - create schema
  - `user_version == 2`
  - FTS 8 column 순서와 DDL option exact assertion
  - `sessions_fts_content` absence
- `schema_v2_has_exact_dirty_triggers`
  - 여섯 name/event/body exact assertion
- `canonical_v1_is_detected_as_migratable`
  - current v1 fixture
  - `CanonicalV1 { sessions_root }`
- `modified_v1_is_not_treated_as_migratable`
  - extra column 또는 trigger
  - `Unknown`
- `future_schema_is_still_refused`
  - `user_version=3`
  - overwrite 없음

### 13.2 document/sync unit test

`src/indexer/fts.rs`:

- `document_maps_metadata_and_exec_in_event_order`
  - command `cargo test`, `git status`
  - output `테스트 성공`, `clean`
  - exact newline join
- `full_population_uses_session_key_as_rowid`
  - two sessions
  - exact FTS rowid set equals session_key set
- `delta_insert_adds_searchable_document`
- `delta_session_update_replaces_all_columns`
- `delta_exec_update_rebuilds_command_and_output`
- `delta_exec_delete_removes_old_tokens`
- `delta_session_delete_removes_fts_row`
- `unchanged_incremental_index_does_not_dirty_fts`
- `canonical_write_sets_dirty_until_sync_is_complete`
- `fts_failure_rolls_back_canonical_and_fts_changes`

각 delta test는 mutation 전에는 old term만, commit 후에는 new term만 match함을
assert한다.

### 13.3 migration/repair integration test

`src/indexer.rs`:

- `schema_v1_migrates_without_reparsing_unchanged_sources`
  - valid v1 DB와 unchanged JSONL
  - `parsed_files == 0`
  - `user_version == 2`
  - metadata/exec term match
- `failed_v1_migration_preserves_version_one_database`
  - injected FTS insert failure
  - v1 table/data/version exact preservation
- `dirty_fts_is_rebuilt_on_next_incremental_index`
  - external canonical UPDATE로 dirty=1
  - next index 후 new text만 match, key 유지
- `missing_fts_row_is_repaired_without_canonical_rebuild`
- `extra_fts_row_is_repaired_without_canonical_rebuild`
- `fts_only_repair_preserves_session_and_exec_keys`
- `corrupt_fts_query_requests_forced_rebuild`
- `forced_rebuild_repairs_corrupt_fts`
- `forced_rebuild_recreates_canonical_and_fts_together`

### 13.4 query compiler unit test

`src/indexer/search.rs`:

- `bare_terms_compile_to_and_prefix_phrases`
  - `fix read`
  - exact `("fix"* AND "read"*)`
- `quoted_text_compiles_to_exact_phrase`
- `spaced_pipe_compiles_or_groups`
- `unclosed_quote_is_valid_interactive_phrase`
- `quotes_and_backslashes_are_escaped`
- `leading_trailing_and_repeated_or_are_rejected`
- `punctuation_only_query_is_rejected`
- `raw_fts_operators_are_quoted_as_text`
  - `NOT`, `repo:main`, `{cwd}`, `a-b`, `foo*`
  - SQL structure에 주입되지 않음
- `scope_wraps_only_expected_columns`
  - 7 scope exact output

### 13.5 검색 fixture와 exact assertion

in-memory schema v2 DB에 다음 session을 넣는다.

```text
A:
  first_message = "README parser 오류 수정"
  cwd = "/home/user/work/codex-session-selector"
  repository_url = "https://github.com/widehyo1/codex-session-selector.git"
  branch = "feature/fts-search"
  timestamp = "2026-07-27T01:02:03Z"
  command = "cargo test --all-targets"
  output = "테스트 성공"

B:
  first_message = "검색기능 벤치마크 작성"
  cwd = "/tmp/benchmark"
  repository_url = "ssh://git@example.com/team/search.git"
  branch = "main"
  timestamp = "2026-07-26T01:02:03Z"
  command = "rg --files src"
  output = "src/indexer.rs"
```

test:

- `korean_prefix_matches_without_ascii_lowercasing`
  - query `검색`, scope message
  - B만 반환
- `korean_middle_of_token_does_not_match`
  - query `기능`, scope message
  - empty
- `path_components_are_searchable`
  - query `codex sess`, scope cwd
  - A만 반환
- `repository_url_components_are_searchable`
  - query `github wide`, scope repo
  - A만 반환
- `command_and_output_are_searchable_in_exec_scope`
  - `cargo all` → A
  - `"테스트 성공"` → A
- `all_scope_can_combine_message_and_exec_terms`
  - `read cargo` → A
- `exec_visibility_does_not_change_search_results`
  - hidden/shown 각각 same ordered path
- `date_scope_uses_yyyy_mm_dd_tokens`
  - `2026 07 27` → A
- `bm25_prefers_message_over_exec_output`
  - 같은 term을 A message, B output에 배치
  - A rank가 더 작고 먼저 반환
- `equal_rank_uses_timestamp_then_session_key`
- `view_filters_apply_before_results_are_returned`
  - subsession/blank-message 각 option matrix
- `middle_substring_behavior_intentionally_changes`
  - `read`는 README match
  - `ead`는 empty

### 13.6 selector unit test

기존 selector test를 schema v2-backed `SearchIndex`로 전환하고 다음을 추가한다.

- `search_scope_cycles_through_exec`
- `exec_scope_label_is_exec`
- `successful_refresh_selects_first_ranked_result`
- `empty_fts_result_clears_selection`
- `query_error_preserves_previous_results_and_selection`
- `database_error_preserves_previous_results_and_selection`
- `selector_toggle_does_not_refresh_fts_results`
- `search_mode_e_remains_query_text`
- `replay_return_preserves_query_scope_focus_and_ranked_results`

### 13.7 CLI integration test

`tests/cli.rs`:

- `index_creates_schema_v2_with_fts`
- `incremental_index_updates_fts_search_content`
- `selector_no_refresh_rejects_schema_v1_with_action`
- `selector_default_refresh_migrates_schema_v1`
- `index_rebuild_repairs_corrupt_fts`
- 기존 help/version/index summary assertion 유지

TUI integration에서 terminal input이 어려운 항목은 selector state unit test와
아래 manual smoke로 나눈다.

### 13.8 benchmark

`tests/fts_benchmark.rs`는 고정 seed의 synthetic corpus를 1k, 10k, 50k
session으로 만든다. 각 session은 metadata, 한국어/ASCII first message와
평균 exec 5개(command 120 bytes, output 1 KiB)를 가진다.

release benchmark가 기록할 값:

- canonical-only DB build time/size
- canonical+FTS full build time/size
- 한 session+exec 변경 incremental time
- 8개 query × 30회 warm run의 median/p95
- all/message/exec scope별 result count

로컬 reference gate:

- 10k warm query p95 `<= 100 ms`
- 50k warm query p95 `<= 250 ms`
- 10k one-session incremental sync `<= full FTS rebuild의 20%`
- FTS 포함 DB size `<= canonical-only DB의 3.0배`
- 모든 반복의 result path/order가 동일

환경과 commit SHA, CPU, SQLite version, corpus size와 결과를
`docs/fts5-handoff.md`에 기록한다. threshold를 넘으면 구현 완료로 처리하지
않고 `EXPLAIN QUERY PLAN`, prefix index 크기와 exec output 비중을 측정해
같은 계획 안에서 조정한다. tokenizer나 column 범위를 임의로 바꾸지는 않고
계획 문서를 먼저 갱신한다.

실행:

```bash
scripts/benchmark-fts.sh
```

script는 내부적으로 다음을 실행한다.

```bash
cargo test --release --test fts_benchmark -- --ignored --nocapture
```

### 13.9 manual TUI smoke

1. 한국어/message/path/repo/command/output fixture로 새 DB를 만든다.
2. selector를 열고 header가 `search: all`인지 확인한다.
3. `/`, `read`를 입력해 README session이 첫 결과인지 확인한다.
4. `ead`로 바꾸면 결과가 비는지 확인한다.
5. `"readme parser" | 검색`을 입력해 OR 결과를 확인한다.
6. search 중 `Tab`을 반복해 `exec`까지 순환하는지 확인한다.
7. exec scope에서 `cargo`, `"테스트 성공"`을 각각 검색한다.
8. normal mode에서 `e`를 눌러 hidden/shown을 바꿔도 결과와 selection이
   유지되는지 확인한다.
9. result를 replay하고 돌아와 query, exec scope, selection과 focus가
   유지되는지 확인한다.
10. 별도 process에서 canonical row를 수정한 뒤 현재 selector가 오류 status를
    표시하고 종료되지 않는지 확인한다.
11. selector를 종료해 refresh한 뒤 새 text 검색과 key 보존을 확인한다.

## 14. package와 문서 변경

### 14.1 `Cargo.toml`/`Cargo.lock`

- package version `0.3.0` 유지
- Rust `1.97` 유지
- `rusqlite 0.40.1` bundled 유지
- 새 dependency/feature 없음
- lockfile는 변경되지 않아야 함

### 14.2 README

다음을 현재 동작 기준으로 갱신한다.

- selector scope 목록에 `exec`
- restricted query 문법과 예
- token prefix/phrase/OR와 중간 substring 차이
- 한국어 token boundary
- exec visibility와 search independence
- schema v2/FTS object
- Contentless-Delete와 stable rowid
- v1 automatic migration
- `--no-refresh` old schema 오류와 해결 명령
- dirty detection/FTS-only repair
- FTS5가 아직 없다는 기존 문구 제거

### 14.3 handoff

구현 전에 작성된 `docs/fts5-handoff.md`의 progress ledger와 완료 보고에
다음을 실제 결과로 갱신한다.

- 구현 commit과 최종 file map
- schema v2 exact object 목록
- v1 migration/rollback 결과
- query grammar와 ranking weight
- fixture/test 결과
- benchmark 환경과 수치
- manual smoke 결과
- 남은 비범위
- release를 수행하지 않았다는 상태

## 15. 최종 검증 명령

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
scripts/benchmark-fts.sh
bash -n scripts/check-before-commit.sh
bash -n scripts/benchmark-fts.sh
bash -n scripts/install-git-hooks.sh
git diff --check
```

release binary smoke:

```bash
tmp_root="$(mktemp -d)"
cargo install --path . --root "$tmp_root"
"$tmp_root/bin/select-codex-session" --version
"$tmp_root/bin/select-codex-session" index \
  --sessions-root tests/fixtures \
  --output "$tmp_root/index.sqlite3" \
  --rebuild
```

temporary path는 검증 후 명시한 exact path만 제거한다.

## 16. 완료 조건

다음을 모두 만족해야 완료다.

- schema v2 exact DDL과 validation이 구현됨
- SQLite `>=3.43.0`과 FTS5 capability가 확인됨
- v1 same-root migration이 reparse 없이 성공함
- migration/repair/index 실패가 DB 전체를 rollback함
- FTS rowid와 `session_key`가 항상 일치함
- session/exec insert/update/delete가 같은 transaction에서 FTS에 반영됨
- dirty/internal/rowid mismatch가 검출되고 다음 index에서 repair됨
- 한국어/path/URL/command/output fixture가 expected result를 반환함
- AND/OR/prefix/phrase/escape 규칙이 exact test로 고정됨
- BM25 weight와 tie-break가 deterministic함
- selector가 production에서 FTS search를 사용함
- exec visibility가 검색 결과에 영향을 주지 않음
- v0/v1 `--no-refresh` 오류가 actionable함
- 기존 canonical/replay/TUI/CLI 회귀 test가 통과함
- benchmark gate가 통과하고 handoff에 기록됨
- README와 handoff가 현재 구현을 설명함
- final quality/release build/diff gate가 모두 통과함
- package version, dependency, publish/tag/release가 변경되지 않음

## 17. 후속 작업 경계

이 계획 완료 후에도 다음은 별도 승인과 별도 계획이 필요하다.

- snippet/highlight와 match 위치 UI
- query history와 saved search
- NOT/NEAR/parentheses/raw advanced syntax
- exact-term 선택 operator
- typo/fuzzy/semantic search
- custom 한국어 tokenizer
- background/debounced async query
- result pagination/limit
- exec kind/name/call_id 검색
- shadow-table 직접 변경까지 감지하는 cryptographic content digest
- package publish, tag와 GitHub release

후속 기능을 쉽게 만든다는 이유로 이번 구현에서 비범위 state, schema column,
dependency 또는 hidden query 문법을 미리 추가하지 않는다.
