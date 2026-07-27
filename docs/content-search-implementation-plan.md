# 메타/콘텐츠 검색 분리 구현 계획

## 1. 문서 상태와 목표 버전

- 상태: 구현 준비 완료
- 대상 기능: selector 메타 검색과 대화 콘텐츠 FTS 검색 분리
- 목표 package version: `0.4.0`
- 목표 index schema version: `3`
- 기준 commit: `cccee9b` (`fix: align tui paging and end scroll`)
- 선행 schema:
  - schema v1 canonical incremental index
  - schema v2 Contentless-Delete FTS5 session search
- 공개 시점: 이 계획의 구현, 검증과 handoff 완료 이후 별도 승인 시점

이 문서는 구현자가 추가 제품·schema 결정을 하지 않고 red/green workflow를
실행할 수 있는 기준이다. 계획 작성 단계에서는 production code, schema,
package version을 변경하지 않는다.

## 2. 목표와 사용자에게 보이는 최종 동작

1. selector의 `/` 검색은 `metadata`와 `content` 두 검색 모드를 제공한다.
2. search mode에서 `Tab`은 다음 순서로 검색 대상을 순환한다.

   ```text
   metadata
   → content:all
   → content:user
   → content:agent
   → content:exec
   → metadata
   ```

3. selector를 처음 열었을 때 검색 대상은 `metadata`다. `content`로 처음
   이동할 때의 scope는 `all`이다.
4. `metadata`는 다음 여섯 값만 대상으로 v0.2.0의 case-insensitive
   all-term substring 검색을 수행한다.

   ```text
   first_message
   cwd
   repository_url
   branch
   timestamp
   date
   ```

5. `content`는 session당 하나의 FTS5 문서를 검색한다.
6. content scope의 대상은 다음과 같다.

   | scope | 검색 대상 |
   | --- | --- |
   | `all` | USER 본문, AGENT 본문, exec command, exec output |
   | `user` | USER 본문 |
   | `agent` | AGENT 본문 |
   | `exec` | exec command와 exec output |

7. `exec_command`와 `exec_output`은 FTS의 물리 column으로 분리하지만,
   selector에는 하나의 `exec` scope만 노출한다.
8. USER/AGENT 메시지는 replay가 표시하는 두 형식을 모두 포함한다.
   - `event_msg`의 `user_message`와 `agent_message`
   - `response_item`의 `message` 중 role `user`와 `assistant`
9. 공백으로 나눈 content 일반 검색어는 AND token prefix이고,
   `"quoted phrase"`는 token phrase, 공백으로 분리한 `|`는 OR다.
10. non-empty content 결과는 BM25 relevance, timestamp 내림차순,
    `session_key` 내림차순으로 정렬한다.
11. metadata 결과와 모든 empty-query 결과는 timestamp 내림차순,
    `session_key` 내림차순으로 정렬한다.
12. 검색 대상 전환과 query 수정이 성공하면 첫 결과를 선택한다. 결과가
    없으면 selection은 `None`이다.
13. 잘못된 content query는 TUI를 종료하지 않는다. 직전 성공 결과,
    selection과 message scroll을 유지하고 footer에 기존 query error를
    표시한다.
14. metadata 검색에는 query syntax error가 없다. 따옴표와 `|`도 일반
    substring 문자의 일부다.
15. selector 오른쪽 pane은 계속 선택한 session의 first message만 표시한다.
    어떤 USER/AGENT/exec 본문이 match했는지 preview하거나 highlight하지
    않는다.

## 3. 구현 범위와 명시적 비범위

### 3.1 구현 범위

- schema v3 canonical `message_events` table
- schema v1/v2 → v3 source-reparse migration
- USER/AGENT record 정규화 규칙의 indexer/replay 공유
- changed source의 message event key-preserving diff
- session 단위 content-only Contentless-Delete FTS5 document
- `all`, `user`, `agent`, `exec` content scope
- v0.2.0 metadata substring filter의 selector production 경로 복원
- selector search target state와 `Tab` 순환
- metadata와 content의 서로 다른 query/정렬/error 의미
- schema/FTS dirty state, row identity와 repair 동작
- 한국어, USER/AGENT, command/output, path/repository metadata fixture
- schema migration, incremental update, search, selector, CLI 회귀 test
- schema v3 데이터 크기, full/migration/index 시간과 query latency benchmark
- README, CLI help, TUI help와 구현 handoff

### 3.2 명시적 비범위

- match한 event의 본문 preview, snippet 또는 highlight
- `SearchHit`에 matched role, event index 또는 excerpt 추가
- preview를 위한 event 단위 FTS table이나 external-content FTS table
- USER/AGENT label 문자열 자체를 FTS token으로 삽입
- `phase`를 canonical DB에 저장하거나 검색
- system, developer 또는 일반 tool message 검색
- exec `kind`, `name`, `call_id`, `session_id` 검색
- metadata의 field별 scope
- metadata BM25 ranking, prefix/phrase/OR 문법
- content의 first_message/cwd/repository/branch/timestamp/date 검색
- typo tolerance, substring trigram, stemming, fuzzy 또는 vector 검색
- raw FTS5 query, `NOT`, `NEAR`, 괄호와 사용자 지정 column filter
- 검색 결과 pagination, background thread, query history와 disk persistence
- replay pane 내부 검색
- 새 CLI search subcommand
- 새 dependency, Cargo feature 또는 Rust minimum version 변경
- publish, tag 또는 GitHub release

`message_events`는 deferred preview를 위한 선행 schema가 아니다. schema v3의
content FTS 문서를 canonical 원본에서 재구성하고 변경 event를 증분 동기화하기
위해 이번 기능 자체에 필요하다. preview 전용 key, query, state와 UI는 추가하지
않는다.

## 4. 현재 동작 및 호환성 계약

### 4.1 현재 baseline

- package: `0.3.0`
- Rust minimum: `1.97`
- `rusqlite`: `0.40.1`, `bundled` feature
- lockfile `libsqlite3-sys`: `0.38.1`
- bundled SQLite: `3.53.2`
- current schema: `PRAGMA user_version = 2`
- canonical tables:
  - `index_metadata`
  - `source_files`
  - `sessions`
  - `exec_events`
- stable identities:
  - `source_files.source_key`
  - `sessions.session_key`
  - `exec_events.exec_key`
- FTS identity:

  ```text
  sessions_fts.rowid == sessions.session_key
  ```

- 현재 FTS column:

  ```text
  first_message
  cwd
  repository_url
  branch
  timestamp
  date
  exec_command
  exec_output
  ```

- 현재 selector search scope:

  ```text
  all → message → cwd → branch → repo → date → exec
  ```

- 현재 canonical parser는 첫 `event_msg.user_message`와 exec event만 저장한다.
- 현재 replay parser는 `event_msg`와 `response_item` 양쪽의 USER/AGENT
  message를 표시한다.
- 현재 `exec_command`와 `exec_output`은 event index 순으로 newline join되어
  FTS에 이미 포함된다.
- 현재 `filter_sessions`/`searchable_text`는 v0.2.0 metadata 검색 의미를
  유지하지만 selector production 경로에서는 사용하지 않는다.

### 4.2 유지할 동작

- `first_message`는 계속 첫 `event_msg.user_message`만 뜻한다.
  `response_item.message(role=user)`를 first message로 승격하지 않는다.
- `SessionRow`, `CollectOptions`, `collect_rows`, `collect_session_data`,
  `load_sessions`, legacy DB helper의 공개 API 의미
- `--include-subsessions`, `--include-empty-messages` read-time view filter
- empty query에서 최신 session 우선 정렬
- 성공한 query/scope 갱신 시 첫 row 선택
- 빈 결과 selection `None`
- search `Enter`, `Esc`, `Backspace`와 문자 입력
- search mode에서 `e`는 query 문자이고 normal mode에서만 exec visibility
  toggle
- `ExecVisibility`와 `--include-exec`는 다음 replay의 표시만 제어
- exec visibility 변경은 search result/query/selection을 바꾸지 않음
- replay에서 selector로 돌아온 뒤 query, 검색 대상, focus, selection과
  exec visibility 유지
- invalid query 또는 DB error 때 직전 성공 결과와 selection 유지
- changed JSONL의 기존 source/session/동일 event key 보존
- root 변경 또는 `--rebuild`의 full rebuild
- unknown/future schema overwrite 방지
- 한 `BEGIN IMMEDIATE` transaction과 실패 시 전체 rollback
- dirty FTS 또는 FTS/canonical rowid 불일치의 다음 refresh repair
- replay가 동일 JSON/JSONL 입력에서 표시하는 USER/AGENT/exec entry와 label

### 4.3 의도적으로 달라지는 동작

| 현재 schema v2 | 구현 후 schema v3 |
| --- | --- |
| 모든 검색 field가 FTS | metadata는 in-memory substring, content만 FTS |
| first message를 `message` scope로 FTS 검색 | first message는 metadata 검색 대상 |
| `all/message/cwd/branch/repo/date/exec` | `metadata/content:all/user/agent/exec` |
| USER/AGENT 전체 본문 미저장 | replay 대상 USER/AGENT 본문 canonical 저장 |
| FTS `all`이 metadata와 exec를 혼합 | content `all`은 대화와 exec만 검색 |
| schema v1→v2는 source 재parse 불필요 | v1/v2→v3은 모든 stable source 재parse |
| canonical child는 exec만 존재 | `message_events` child 추가 |

의도적 검색 차이 예:

```text
query: ead
metadata first_message: "Fix README"
→ metadata에서는 substring으로 match
→ content에서는 token 중간 문자열이므로 match하지 않음

query: docker alpha
cwd: /work/docker
branch: feature/alpha
→ metadata에서는 두 term이 한 row의 서로 다른 field에 있어도 match

query: retry parser
USER message: "retry this"
AGENT message: "parser failed"
→ content:all에서는 한 session FTS document 안의 AND이므로 match
→ content:user와 content:agent에서는 각각 두 term이 없으므로 match하지 않음
```

## 5. 확정한 기술·구조 결정

### 5.1 검색 책임 분리

metadata는 이미 selector가 보유하는 `Vec<SessionRow>`에
`filter_sessions`를 적용한다.

```rust
let terms = query
    .split_whitespace()
    .map(str::to_lowercase)
    .collect::<Vec<_>>();

let matches = terms
    .iter()
    .all(|term| searchable_text(row).contains(term));
```

`searchable_text` field와 join 순서는 변경하지 않는다.

```text
first_message
cwd
repository_url
branch
timestamp
date
```

content만 SQLite FTS5를 사용한다. metadata query에서는 FTS runtime query,
query parser, BM25를 호출하지 않는다.

### 5.2 USER/AGENT 정규화

새 `src/session_event.rs`가 USER/AGENT record 인식과 response content의 text
변환을 소유한다. indexer와 replay가 이 함수를 공유한다.

| source record | role mapping | content |
| --- | --- | --- |
| bare `user_message` payload | `User` | string `message` |
| bare `agent_message` payload | `Agent` | string `message` |
| `event_msg.user_message` | `User` | string `message` |
| `event_msg.agent_message` | `Agent` | string `message` |
| `response_item.message`, role `user` | `User` | `content`의 normalized text |
| `response_item.message`, role `assistant` | `Agent` | `content`의 normalized text |

bare payload는 replay 입력 호환을 위한 것이며 session JSONL index에서는
top-level Codex record만 만난다.

`response_item.content` text 변환은 현재 replay의 `value_text`와 동일하다.

- string은 그대로 사용
- array는 non-empty child text를 newline join
- object는 string `text`, 그다음 string `output`, 둘 다 없으면 JSON string
- number/bool은 JSON string
- null은 empty string

알 수 없는 role, malformed message와 다른 record type은 message로 만들지
않는다. 빈 content는 canonical event로 저장하되 FTS에는 유효 token을
추가하지 않는다.

JSONL physical line의 zero-based line index를 `event_index`로 사용한다.
현재 exec event와 같은 기준이다. replay가 중복된 `event_msg`와
`response_item`을 둘 다 표시하면 index도 둘 다 별도 message event로
저장하고 FTS aggregate에 포함한다.

### 5.3 canonical message identity

`message_events`의 안정 identity는 다음과 같다.

```text
UNIQUE(session_key, event_index)
```

동일 source path의 같은 line index가 content 또는 role만 바뀌면
`message_key`를 보존해 UPDATE한다. event가 새로 생기면 INSERT, 없어지면
DELETE한다. source/session 삭제는 foreign-key cascade다.

`phase`, session path와 session id는 저장하지 않는다. 현재 검색 문서 구성에
필요하지 않고 sessions join으로 path/id를 얻을 수 있다.

### 5.4 FTS document와 column

session당 정확히 하나의 content FTS document를 유지한다.

```text
sessions_fts.rowid == sessions.session_key
```

column 순서, source와 BM25 weight는 고정한다.

| 순서 | FTS column | canonical source | weight |
| ---: | --- | --- | ---: |
| 0 | `user_content` | User message content의 event index 순 newline join | 4.0 |
| 1 | `agent_content` | Agent message content의 event index 순 newline join | 4.0 |
| 2 | `exec_command` | exec command의 event index 순 newline join | 1.5 |
| 3 | `exec_output` | exec output의 event index 순 newline join | 0.25 |

USER와 AGENT는 같은 중요도로 시작한다. command는 output보다 높게 두어 긴
command output이 `all` relevance를 과도하게 지배하지 않게 한다.

metadata column을 FTS에서 제거한다. `exec_command`/`exec_output`은 기존
loader와 test를 재사용하기 위해 합치지 않는다. selector compiler만 두 column을
`exec` scope로 묶는다.

session 단위 document를 사용하므로 `content:all`의 AND term은 서로 다른
USER/AGENT/exec event에 존재해도 match할 수 있다. 이것이 대략적인 키워드로
session을 찾는 제품 의도다.

event 단위 FTS는 exact matched-event preview에는 유리하지만, AND 의미와 rank
merge 규칙이 달라지고 이번 preview 비범위에 필요하지 않으므로 도입하지 않는다.

### 5.5 FTS runtime, tokenizer와 query

현재와 같은 Contentless-Delete FTS5를 유지한다.

```text
minimum SQLite: 3.43.0
content=''
contentless_delete=1
tokenize='unicode61 remove_diacritics 2'
prefix='2 3'
detail=full
columnsize=1
```

현재 bundled SQLite `3.53.2`와 FTS5 compile option 검사를 유지한다. 구현
시작 시 `Cargo.lock`의 `libsqlite3-sys`와 다음 runtime query를 다시 확인한다.

```sql
SELECT sqlite_version();
SELECT sqlite_compileoption_used('ENABLE_FTS5');
```

미지원 runtime exact error는 유지한다.

```text
SQLite 3.43.0 or newer with FTS5 is required; found <version>
```

구현 전 확인할 공식 문서:

- [FTS5 Contentless-Delete tables](https://www.sqlite.org/fts5.html#contentless_delete_tables)
- [FTS5 query syntax](https://www.sqlite.org/fts5.html#full_text_query_syntax)
- [FTS5 BM25](https://www.sqlite.org/fts5.html#the_bm25_function)
- [SQLite 3.43.0 release](https://www.sqlite.org/releaselog/3_43_0.html)

content query parser, escaping, AND/OR/prefix/phrase 규칙과 error 문구는 schema
v2 구현을 그대로 유지한다. metadata는 이 parser를 호출하지 않는다.

scope compiler:

```text
all   → column filter 없음
user  → {user_content} : (...)
agent → {agent_content} : (...)
exec  → {exec_command exec_output} : (...)
```

### 5.6 ranking과 정렬

non-empty content query:

```sql
rank MATCH 'bm25(4.0, 4.0, 1.5, 0.25)'
```

```sql
ORDER BY
    sessions_fts.rank ASC,
    sessions.timestamp DESC,
    sessions.session_key DESC
```

metadata와 empty query:

```sql
ORDER BY sessions.timestamp DESC, sessions.session_key DESC
```

metadata filter는 이 순서로 load된 `all_rows`의 상대 순서를 바꾸지 않는다.
결과 개수 제한은 두지 않는다.

## 6. 목표 schema와 migration

### 6.1 schema version과 exact canonical DDL

```rust
pub(crate) const SCHEMA_VERSION: i64 = 3;
const CANONICAL_V1_VERSION: i64 = 1;
const FTS_V2_VERSION: i64 = 2;
```

기존 네 canonical table은 column 변경 없이 유지한다. 다음 table을 추가한다.

```sql
CREATE TABLE message_events (
    event_index INTEGER NOT NULL CHECK (event_index >= 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'agent')),
    content TEXT NOT NULL,
    message_key INTEGER PRIMARY KEY,
    session_key INTEGER NOT NULL
        REFERENCES sessions(session_key) ON DELETE CASCADE,
    UNIQUE (session_key, event_index)
) STRICT;
```

별도 ordinary index는 추가하지 않는다. `UNIQUE(session_key, event_index)`의
SQLite autoindex가 session별 event index scan을 지원한다.

### 6.2 exact FTS DDL

```sql
CREATE TABLE fts_sync_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    dirty INTEGER NOT NULL CHECK (dirty IN (0, 1))
) STRICT;

INSERT INTO fts_sync_state(singleton, dirty) VALUES (1, 1);

CREATE VIRTUAL TABLE sessions_fts USING fts5(
    user_content,
    agent_content,
    exec_command,
    exec_output,
    content='',
    contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3',
    detail=full,
    columnsize=1
);
```

### 6.3 exact dirty trigger set

session insert/delete는 FTS row identity를 바꾸고 session update는 외부 writer의
예상하지 못한 mutation을 보수적으로 감지하므로 기존 세 trigger를 유지한다.
exec와 message mutation trigger를 합쳐 총 아홉 개다.

```sql
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

CREATE TRIGGER message_events_fts_dirty_ai
AFTER INSERT ON message_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER message_events_fts_dirty_au
AFTER UPDATE ON message_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER message_events_fts_dirty_ad
AFTER DELETE ON message_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;
```

### 6.4 schema state와 exact validation

```rust
pub(crate) enum SchemaState {
    Empty,
    Legacy,
    CanonicalV1 { sessions_root: PathBuf },
    FtsV2 { sessions_root: PathBuf },
    Current { sessions_root: PathBuf },
    Future { version: i64 },
    Unknown { version: i64, reason: String },
}
```

schema v3 validation은 다음을 exact하게 검사한다.

- 기존 canonical table column/STRICT/UNIQUE/foreign key
- `message_events` column 순서, type, NOT NULL, role CHECK와 STRICT
- `UNIQUE(session_key, event_index)`
- `message_events.session_key` cascading foreign key
- `fts_sync_state` column, STRICT와 valid singleton
- `sessions_fts`의 네 user column 순서와 모든 FTS option
- Contentless-Delete 예상 shadow table set
- 아홉 dirty trigger의 name, target, event와 body
- unexpected user table/view/trigger 없음
- `sessions_fts_content`가 없음

schema v1과 v2 validation은 migration source 판별을 위해 기존 exact validation을
보존한다.

### 6.5 v1/v2 → v3 migration

v1과 v2에는 USER/AGENT 전체 본문이 없으므로 기존 DB row만으로 v3를 채울 수
없다. 같은 sessions root이고 `--rebuild`가 아니어도 모든 JSONL source를
stable parse한다.

parse는 write transaction 전에 수행한다.

```rust
let force_all = matches!(
    schema_state,
    SchemaState::CanonicalV1 { .. } | SchemaState::FtsV2 { .. }
);
let plan = scan_sources(&root, &stored, force_all)?;

if force_all && !plan.unstable_paths.is_empty() {
    bail!(
        "cannot migrate index schema to 3 while {} source files are changing; retry the index refresh",
        plan.unstable_paths.len()
    );
}
```

migration의 모든 stored path는 `changed_sources`, 새 path는 `new_sources`가
된다. unstable source가 하나라도 있으면 transaction을 시작하지 않고 기존
DB를 그대로 둔다.

v2 migration transaction:

```text
BEGIN IMMEDIATE
→ v2 FTS trigger/table/state drop
→ message_events table 생성
→ v3 FTS/state/아홉 trigger 생성(dirty=1)
→ canonical incremental apply
   - sessions와 exec key-preserving diff
   - message key-preserving diff
→ 모든 session의 v3 FTS document populate
→ FTS internal integrity/row identity 확인
→ dirty=0
→ canonical foreign key/quick check/invariant 확인
→ PRAGMA user_version=3
→ COMMIT
```

v1 migration은 기존 FTS drop 단계만 생략하고 나머지는 같다. migration 중
실패하면 DDL과 data mutation을 모두 rollback하여 원래 v1/v2 DB를 보존한다.

legacy schema, sessions root 변경과 explicit `--rebuild`는 기존처럼 전체
schema를 drop/create하고 source를 full parse한다. rebuild는 새 key를 할당할
수 있다.

schema v1/v2 DB를 `--no-refresh` selector로 열면 write/fallback 없이 다음
오류로 종료한다.

```text
search index schema 3 is required; refresh the index or run `select-codex-session index`
```

## 7. 목표 파일 구조와 파일별 책임

```text
src/
  lib.rs
  session_event.rs               # 새 파일
  application.rs
  cli.rs
  replay/mod.rs
  selector/mod.rs
  indexer.rs
  indexer/
    scan.rs
    schema.rs
    store.rs
    fts.rs
    search.rs
  test_support.rs
tests/
  cli.rs
  fts_benchmark.rs
scripts/
  benchmark-fts.sh
  check-before-commit.sh
docs/
  content-search-implementation-plan.md
  content-search-handoff.md       # 구현 완료 시 추가
README.md
Cargo.toml
Cargo.lock
```

- `session_event.rs`
  - USER/AGENT role type
  - event_msg/response_item/bare payload message 정규화
  - response item content text 변환
- `lib.rs`
  - `MessageEvent`와 `ParsedSessionFile.message_events`
  - 기존 first message와 exec collection 유지
  - 각 JSONL line에서 shared message normalizer 호출
- `replay/mod.rs`
  - shared message normalizer를 replay `Entry`로 변환
  - exec record 정규화와 pairing은 기존 소유권 유지
- `indexer/schema.rs`
  - schema v1/v2/v3 detection과 exact validation
  - message table 및 v3 FTS lifecycle orchestration
- `indexer/store.rs`
  - stored message load와 key-preserving diff
  - counts/invariants에 message table 포함
- `indexer/fts.rs`
  - content-only `SearchDocument`
  - message/exec aggregate와 FTS sync/repair
- `indexer/search.rs`
  - `ContentScope`
  - 기존 restricted query compiler
  - all session load와 content-only ranked search
- `indexer.rs`
  - v1/v2 force-all parse migration과 transaction 순서
  - message delta/count summary
- `selector/mod.rs`
  - `all_rows`, `SearchTarget`과 `Tab` state transition
  - metadata filter/content FTS dispatch
- `application.rs`
  - schema v3 search backend open; control flow는 유지
- `test_support.rs`
  - role별 다중 message와 exec fixture
- `tests/cli.rs`
  - schema version/object와 no-refresh migration error
- `tests/fts_benchmark.rs`
  - 다중 message corpus, migration/build/size/query 측정
- README/CLI/TUI help
  - 두 검색 모드, scope, 문법, schema v3와 migration 설명

## 8. 타입과 함수 시그니처

### 8.1 shared message normalizer

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageRole {
    User,
    Agent,
}

impl MessageRole {
    pub(crate) fn as_str(self) -> &'static str;
    pub(crate) fn from_str(value: &str) -> Result<Self>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedMessage {
    pub role: MessageRole,
    pub phase: Option<String>,
    pub content: String,
}

pub(crate) fn normalize_message_record(
    value: &serde_json::Value,
) -> Option<NormalizedMessage>;

fn value_to_text(value: &serde_json::Value) -> String;
```

`phase`는 replay label 호환을 위해 normalizer output에는 존재하지만
`MessageEvent`나 DB에는 저장하지 않는다.

`MessageRole::from_str`은 `user`와 `agent`만 받고 그 외 값에 다음 오류를
반환한다.

```text
invalid canonical message role <value>
```

### 8.2 canonical parsed message

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageEvent {
    pub event_index: usize,
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedSessionFile {
    pub(crate) row: SessionRow,
    pub(crate) exec_events: Vec<ExecEvent>,
    pub(crate) message_events: Vec<MessageEvent>,
}
```

`parse_session_file(path)`는 early-exit 최적화를 유지할 수 있다. canonical
index의 `stable_parse`는 기존처럼 `parse_session_file_data(path, true)`를
호출하며, `include_exec=true`일 때 EOF까지 읽어 exec와 모든 message를
수집한다. 공개 `collect_session_data`는 기존 API대로 message list를 노출하지
않는다.

### 8.3 index delta와 counts

```rust
pub(crate) type MessageKey = i64;

pub(crate) struct IndexDelta {
    // 기존 session/exec field 유지
    pub inserted_message_keys: Vec<MessageKey>,
    pub updated_message_keys: Vec<MessageKey>,
    pub deleted_message_keys: Vec<MessageKey>,
    pub touched_session_keys: Vec<SessionKey>,
}

pub(crate) struct StoredCounts {
    pub skipped_files: usize,
    pub session_rows: usize,
    pub exec_rows: usize,
    pub message_rows: usize,
}

pub(crate) struct IndexSummary {
    // 기존 field 유지
    pub message_rows: usize,
}
```

message diff가 하나라도 발생하면 해당 `session_key`를
`touched_session_keys`에 추가한다. 최종 `normalize_delta`가 모든 key vector를
sort/dedup한다.

index summary exact suffix는 다음으로 바꾼다.

```text
stored <sessions> sessions, <messages> messages and <execs> exec events
```

### 8.4 FTS document

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchDocument {
    pub session_key: SessionKey,
    pub user_content: String,
    pub agent_content: String,
    pub exec_command: String,
    pub exec_output: String,
}

pub(crate) fn load_document(
    tx: &Transaction<'_>,
    session_key: SessionKey,
) -> Result<Option<SearchDocument>>;
```

### 8.5 search model

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentScope {
    All,
    User,
    Agent,
    Exec,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchHit {
    pub row: SessionRow,
    pub rank: f64,
}

pub(crate) fn compile_match(
    query: &SearchQuery,
    scope: ContentScope,
) -> Result<String, QueryError>;

impl SearchIndex {
    pub(crate) fn open(path: &Path, view: SessionView) -> Result<Self>;

    pub(crate) fn all_sessions(&self) -> Result<Vec<SessionRow>>;

    pub(crate) fn search_content(
        &self,
        input: &str,
        scope: ContentScope,
    ) -> Result<Vec<SearchHit>>;
}
```

`SearchHit`에는 preview/evidence field를 추가하지 않는다.

### 8.6 selector search state

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchTarget {
    Metadata,
    Content(ContentScope),
}

impl SearchTarget {
    fn next(self) -> Self;
    fn label(self) -> &'static str;
}

pub(crate) struct SelectorApp {
    search_index: SearchIndex,
    all_rows: Vec<SessionRow>,
    total_rows: usize,
    filtered: Vec<SessionRow>,
    query: String,
    search_target: SearchTarget,
    // 기존 나머지 field 유지
}

impl SelectorApp {
    pub(crate) fn new(
        search_index: SearchIndex,
        exec_visibility: ExecVisibility,
    ) -> Result<Self>;

    fn refresh_filter(&mut self);
    fn next_search_target(&mut self);
}
```

exact label:

```text
metadata
content:all
content:user
content:agent
content:exec
```

## 9. 주요 흐름 및 알고리즘 pseudocode

### 9.1 JSONL parse

```rust
fn parse_session_file_data(
    path: &Path,
    include_exec: bool,
) -> Result<Option<ParsedSessionFile>> {
    let mut meta = None;
    let mut first_message = None;
    let mut exec_events = Vec::new();
    let mut message_events = Vec::new();

    for (line_number, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let value = match parse_json_line(line?) {
            Ok(value) => value,
            Err(error) => {
                warn_invalid_json(path, line_number + 1, error);
                continue;
            }
        };

        collect_first_session_meta_without_changing_existing_rules(&value, &mut meta);
        collect_first_event_msg_user_without_changing_existing_rules(
            &value,
            &mut first_message,
        );

        if include_exec {
            collect_exec_event(/* existing pairing state */);
            if let Some(message) = normalize_message_record(&value) {
                message_events.push(MessageEvent {
                    event_index: line_number,
                    role: message.role,
                    content: message.content,
                });
            }
        }

        if !include_exec && meta.is_some() && first_message.is_some() {
            break;
        }
    }

    let Some(mut row) = meta else {
        return Ok(None);
    };
    row.first_message = first_message.unwrap_or_default();
    attach_session_id_to_execs(&mut exec_events, row.id.clone());

    Ok(Some(ParsedSessionFile {
        row,
        exec_events,
        message_events,
    }))
}
```

### 9.2 message key-preserving diff

```rust
fn apply_message_diff(
    tx: &Transaction<'_>,
    session_key: SessionKey,
    parsed: &[MessageEvent],
    delta: &mut IndexDelta,
) -> Result<()> {
    let mut existing = load_messages_by_event_index(tx, session_key)?;

    for event in parsed {
        match existing.remove(&event.event_index) {
            None => {
                let key = insert_message(tx, session_key, event)?;
                delta.inserted_message_keys.push(key);
                delta.touched_session_keys.push(session_key);
            }
            Some(stored) if stored.event != *event => {
                update_message(tx, stored.key, event)?;
                delta.updated_message_keys.push(stored.key);
                delta.touched_session_keys.push(session_key);
            }
            Some(_) => {}
        }
    }

    for stored in existing.into_values() {
        delete_message(tx, stored.key)?;
        delta.deleted_message_keys.push(stored.key);
        delta.touched_session_keys.push(session_key);
    }
    Ok(())
}
```

`apply_source`는 session key를 결정한 뒤 exec diff와 message diff를 같은
transaction에서 순서대로 실행한다.

### 9.3 FTS document build

```rust
fn load_document(
    tx: &Transaction<'_>,
    session_key: SessionKey,
) -> Result<Option<SearchDocument>> {
    if !session_exists(tx, session_key)? {
        return Ok(None);
    }

    let mut users = Vec::new();
    let mut agents = Vec::new();
    for message in query_messages_ordered_by_event_index(tx, session_key)? {
        match message.role {
            MessageRole::User => users.push(message.content),
            MessageRole::Agent => agents.push(message.content),
        }
    }

    let mut commands = Vec::new();
    let mut outputs = Vec::new();
    for exec in query_execs_ordered_by_event_index(tx, session_key)? {
        commands.push(exec.command);
        outputs.push(exec.output);
    }

    Ok(Some(SearchDocument {
        session_key,
        user_content: users.join("\n"),
        agent_content: agents.join("\n"),
        exec_command: commands.join("\n"),
        exec_output: outputs.join("\n"),
    }))
}
```

FTS delta/delete/populate/rebuild 순서는 schema v2 구현을 유지하며 문서 field만
네 content column으로 바꾼다.

### 9.4 selector refresh

```rust
fn refresh_filter(&mut self) {
    let result = match self.search_target {
        SearchTarget::Metadata => {
            Ok(filter_sessions(&self.all_rows, &self.query))
        }
        SearchTarget::Content(scope) => {
            self.search_index
                .search_content(&self.query, scope)
                .map(|hits| hits.into_iter().map(|hit| hit.row).collect())
        }
    };

    match result {
        Ok(rows) => {
            self.filtered = rows;
            self.list_state
                .select((!self.filtered.is_empty()).then_some(0));
            self.message_scroll = 0;
            self.status = None;
        }
        Err(error) => {
            // 직전 filtered, selection과 scroll을 바꾸지 않는다.
            self.status = Some(format_search_error(&error));
        }
    }
}
```

### 9.5 search target transition

```rust
fn next(self) -> Self {
    match self {
        Self::Metadata => Self::Content(ContentScope::All),
        Self::Content(ContentScope::All) => Self::Content(ContentScope::User),
        Self::Content(ContentScope::User) => Self::Content(ContentScope::Agent),
        Self::Content(ContentScope::Agent) => Self::Content(ContentScope::Exec),
        Self::Content(ContentScope::Exec) => Self::Metadata,
    }
}
```

`Tab`은 target을 먼저 갱신하고 같은 query로 `refresh_filter`를 호출한다.
query가 비어 있으면 모든 target이 같은 timestamp-order row를 보여 준다.

## 10. 오류 처리, 상태 소유권과 lifecycle

### 10.1 상태 소유권

- `SearchIndex`
  - read-only SQLite connection
  - `SessionView`
  - content query와 all-session load
- `SelectorApp`
  - `all_rows`
  - 현재 `query`
  - `SearchTarget`
  - 직전 성공 `filtered`
  - selection/focus/scroll/status
- canonical DB
  - USER/AGENT/exec 검색 원본
  - FTS sync state
- replay
  - JSONL을 직접 읽는 기존 lifecycle
  - selector 검색 state를 소유하지 않음

### 10.2 query 오류

- metadata query: syntax error 없음
- content query parser error:
  - `filtered`, selection, message scroll 유지
  - exact prefix `search query error: `
- SQLite corruption:
  - 기존 corruption suffix와 rebuild 안내 유지
- 그 외 search DB error:
  - 기존 refresh/rebuild 안내 유지
- 다음 성공 query/target 전환은 status를 지움

### 10.3 index/migration 오류

- source open/read/parse fatal error: transaction 전에 종료
- migration 중 unstable source: transaction 전에 exact migration error
- DDL/canonical/FTS/invariant error: transaction rollback
- FTS dirty/row identity mismatch:
  - schema v3 normal refresh에서 content FTS-only rebuild
  - canonical key와 message key 보존
- unknown schema:
  - `index --rebuild` 없이는 overwrite하지 않음
- future schema:
  - 항상 overwrite 거부

### 10.4 selector/replay lifecycle

selector가 생성될 때 schema v3 validation과 clean/row identity 검사를 하고
`all_rows`를 한 번 load한다. selector가 열린 동안 외부 process가 canonical
DB를 바꾸는 것은 지원하지 않는다. 기존 persistent read-only connection
정책을 유지한다.

replay에 진입하고 돌아와도 `all_rows`, query, target와 결과를 재생성하지
않는다. 다음 selector process 시작 또는 explicit refresh가 DB 변경을 반영한다.

## 11. 단계별 구현 순서

### Phase 0. baseline characterization와 shared normalizer 추출

1. 현 replay message normalization test를 characterization test로 보강한다.
2. `session_event.rs`에 message normalization을 이동한다.
3. replay가 shared function을 사용하도록 바꾼다.
4. replay entry output과 phase label이 byte-for-byte 유지되는지 확인한다.
5. canonical schema/search 동작은 변경하지 않는다.

Quality gate:

```bash
cargo test replay::
cargo test session_event::
bash scripts/check-before-commit.sh
cargo build --release
git diff --check
```

커밋 가능 조건:

- shared normalizer test와 전체 gate green
- selector/index/schema diff 없음
- commit 예: `refactor: share session message normalization`

### Phase 1. schema v3와 metadata/content 검색 기능

1. parser/index store regression test를 먼저 red로 만든다.
2. `MessageEvent`, `MessageRole`, parsed message list를 추가한다.
3. `message_events` DDL, exact validation과 invariants를 추가한다.
4. message key-preserving diff와 delta/count를 구현한다.
5. v1/v2 force-all source parse migration을 구현한다.
6. `SearchDocument`와 BM25를 four-column content로 변경한다.
7. `ContentScope`와 compiler mapping을 구현한다.
8. v3 FTS populate/delta/repair와 role scope test를 통과시킨다.
9. `SearchTarget`, `all_rows`와 metadata/content dispatch를 구현한다.
10. `Tab` 순환, footer/help label과 selector state test를 통과시킨다.
11. schema, storage, FTS, search backend와 selector를 함께 바꿔 중간 commit도
    완전한 schema v3 계약으로 끝낸다.

Quality gate:

```bash
cargo test indexer::schema
cargo test indexer::store
cargo test indexer::fts
cargo test indexer::search
cargo test indexer::tests
cargo test selector::
cargo test application::
cargo test --test cli
bash scripts/check-before-commit.sh
cargo build --release
git diff --check
```

커밋 가능 조건:

- v1/v2 migration rollback/key-preservation test green
- schema v3 exact validation green
- FTS row identity가 모든 session과 일치
- metadata-only token이 content FTS에서 match하지 않음
- role scope가 다른 role의 unique token을 반환하지 않음
- 다섯 target 순환과 content default `all` test green
- preview/highlight가 render되지 않는 negative test green
- commit 예: `feat: split metadata and content session search`

### Phase 2. benchmark, package와 문서

1. benchmark corpus를 multi-message schema v3 데이터로 갱신한다.
2. package version을 `0.4.0`으로 갱신하고 lockfile을 반영한다.
3. README, CLI help, TUI help와 schema 설명을 갱신한다.
4. `content-search-handoff.md`에 실제 commit, test, benchmark 결과를 기록한다.
5. manual tmux smoke를 수행한다.

Quality gate:

```bash
bash -n scripts/check-before-commit.sh
bash -n scripts/benchmark-fts.sh
bash scripts/check-before-commit.sh
cargo build --release
bash scripts/benchmark-fts.sh
git diff --check
```

커밋 가능 조건:

- README/help/schema/package가 실제 구현과 일치
- benchmark gate와 manual smoke 통과
- commit 예: `docs: document metadata and content search`

## 12. red/green 및 커밋 정책

모든 phase는 green-only commit이다.

```text
대상 test 추가
→ 예상한 이유로 red 확인
→ 같은 phase production code 구현
→ 대상 test green
→ 공통 pre-commit gate
→ release build와 diff check
→ test와 구현 함께 commit
```

- 실패 test만 커밋하지 않는다.
- schema v3 DDL만 있고 parser/store/FTS가 없어 전체 test가 깨지는 중간
  commit을 만들지 않는다.
- migration은 schema/data/FTS/validation이 한 commit에서 green이어야 한다.
- Phase 0 characterization test가 처음부터 green이면 refactor와 같은 commit
  또는 별도 green commit이 가능하다.
- 각 phase에서 사용자 소유의 관련 없는 working-tree 변경을 포함하지 않는다.
- commit 전 실제 staged diff를 검토한다.

## 13. pre-commit/CI 자동화

현재 자동화가 지침의 single-source-of-truth 구조를 이미 만족하므로 새 hook이나
workflow를 추가하지 않는다.

```text
scripts/check-before-commit.sh
.githooks/pre-commit
.github/workflows/ci.yml
```

공통 script는 다음을 계속 실행한다.

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

최종 구현에서는 추가로 다음을 실행한다.

```bash
bash -n scripts/check-before-commit.sh
bash -n scripts/benchmark-fts.sh
cargo build --release
git diff --check
```

CI와 hook에 검사 명령을 중복 추가하지 않는다. 새 test target은 기존
`cargo test --all-targets --all-features`에 포함되게 한다.

## 14. unit, integration, benchmark와 manual test

### 14.1 shared normalizer unit test

`src/session_event.rs`:

- `event_msg_user_and_agent_are_normalized`
  - USER `message="need parser"`와 AGENT `message="done"`
  - role/content exact equality
- `response_item_user_and_assistant_content_arrays_are_normalized`
  - `content=[{"type":"input_text","text":"alpha"},{"text":"beta"}]`
  - exact content `"alpha\nbeta"`
- `bare_payload_messages_preserve_replay_compatibility`
  - bare user/agent payload와 phase
  - role/content/phase exact equality
- `unknown_roles_and_malformed_messages_are_ignored`
  - role system, missing role, event message non-string body
  - 모두 `None`
- `empty_message_is_preserved`
  - exact empty string event

`src/replay/mod.rs`:

- 기존 event_msg/response_item timeline entry test 유지
- `shared_message_normalizer_preserves_phase_labels`
  - exact summary `USER (analysis)`/`AGENT (final)` 등 현재 formatter 결과
- exec pairing/output test 결과 불변

### 14.2 parser/store/schema test

`src/lib.rs`:

- `parser_collects_all_replay_user_and_agent_messages`
  - session_meta, event_msg USER/AGENT, response_item user/assistant, exec
  - exact four message roles/content/event indexes
- `first_message_remains_first_event_msg_user`
  - response_item user가 먼저 있어도 later event_msg user가 first message
- `message_collection_runs_to_eof_for_canonical_parse`
  - first message 이후 later agent/user가 모두 존재
- `public_parse_session_file_keeps_existing_early_result`
  - public row 의미 불변

`src/indexer/schema.rs`:

- `schema_v3_has_exact_message_and_content_fts_columns`
- `schema_v3_has_nine_dirty_triggers`
- `schema_v3_message_foreign_key_cascades`
- `schema_v2_is_detected_as_migration_source`
- `schema_v3_rejects_role_check_or_fts_column_drift`
- `contentless_schema_has_no_fts_content_shadow_table`

`src/indexer/store.rs` 또는 `src/indexer.rs`:

- `message_insert_update_delete_preserves_expected_keys`
  - 같은 event index content 변경은 message key 보존/updated delta
  - 새 event는 inserted key
  - 제거 event는 deleted key
- `source_delete_cascades_messages_and_touches_session`
- `unchanged_refresh_preserves_message_keys_and_has_empty_delta`
- `stored_counts_include_message_rows`
- `v2_migration_reparses_all_sources_and_preserves_session_and_exec_keys`
- `v1_migration_reparses_all_sources_and_populates_messages`
- `migration_assigns_stable_message_keys`
- `migration_with_unstable_source_rolls_back_before_schema_change`
- `migration_failure_preserves_v2_schema_and_rows`
- `root_change_still_performs_full_rebuild`

exact migration assertion:

```text
before: user_version=2, known session_key/exec_key
after:  user_version=3, same session_key/exec_key,
        message_events count/role/content exact,
        sessions_fts row count == sessions count,
        fts_sync_state.dirty == 0
```

### 14.3 FTS/search unit test

- `search_document_joins_user_agent_and_exec_by_event_index`
- `all_scope_matches_terms_across_user_and_agent_columns`
- `user_scope_excludes_agent_only_term`
- `agent_scope_excludes_user_only_term`
- `exec_scope_searches_command_and_output`
- `metadata_fields_are_not_content_searchable`
- `first_message_is_content_searchable_only_when_present_in_message_events`
- `message_update_refreshes_only_touched_session_document`
- `message_delete_removes_stale_fts_tokens`
- `dirty_message_mutation_triggers_fts_rebuild`
- `content_scope_compiles_exact_column_filters`
- 기존 query escaping/phrase/OR/Korean tests 유지
- deterministic BM25 tie-break test를 four-column weight로 갱신

metadata negative fixture:

```text
cwd: /work/metadata-needle
USER: "unrelated conversation"
```

assertion:

```text
metadata query "metadata-needle" → 1 row
content:all query "metadata-needle" → 0 rows
```

role fixture:

```text
USER:  "useronly shared"
AGENT: "agentonly shared"
EXEC command: "exec-command-only"
EXEC output:  "exec-output-only"
```

scope별 exact result count를 assertion한다.

### 14.4 selector TestBackend test

- `search_target_cycles_metadata_and_content_scopes`
- `new_selector_defaults_to_metadata`
- `entering_content_defaults_to_all`
- `metadata_search_uses_substring_and_all_terms`
- `content_search_uses_fts_prefix_and_rank`
- `empty_query_has_same_rows_in_every_target`
- `invalid_content_query_preserves_results_selection_and_scroll`
- `switching_invalid_content_query_to_metadata_recovers`
- `replay_return_preserves_search_target`
- `exec_visibility_toggle_does_not_change_content_results`
- `search_footer_renders_exact_target_label`
- `selector_message_pane_still_renders_first_message`
- `selector_does_not_render_match_preview_or_highlight`

마지막 negative test는 content-only token이 later AGENT message에 있고 first
message에는 없도록 만든다. search result는 나오지만 오른쪽 pane에는 기존
first message만 있고 later token이나 `matched` label이 없어야 한다.

### 14.5 CLI integration test

`tests/cli.rs`:

- `index_creates_schema_v3_with_messages_and_content_fts`
- `selector_no_refresh_rejects_schema_v2_with_action`
- `normal_refresh_migrates_schema_v2_to_v3`
- `index_summary_reports_message_count`
- 기존 selector/replay/legacy/root/future schema test 유지

### 14.6 benchmark

`tests/fts_benchmark.rs` corpus는 각 session에 다음을 넣는다.

```text
1 session_meta
6 USER messages
6 AGENT messages
2 exec command/output pairs
```

크기:

```text
1,000 sessions
10,000 sessions
50,000 sessions
```

v2 migration benchmark fixture는 `e09808a`의 exact schema v2 DDL을 test-only
상수로 보존하고, generated corpus에서 `source_files`, `sessions`,
`exec_events`와 기존 8-column FTS row를 채운다. migration command 실행 전에
`PRAGMA user_version = 2`와 schema v2 object set을 assertion한다. production
code에 v2 schema 생성 API를 추가하지 않는다.

측정:

- clean schema v3 full build time
- v2→v3 migration time
- source JSONL total bytes
- schema v3 DB bytes
- unchanged incremental refresh time
- `content:all/user/agent/exec` warm query median/p95

gate:

```text
schema v3 DB bytes <= source JSONL bytes * 3
unchanged incremental <= full build time의 20%
10k warm content query p95 <= 100ms
50k warm content query p95 <= 250ms
모든 scope query result count가 fixture 기대값과 일치
```

benchmark 명령과 exit-status:

```bash
bash scripts/benchmark-fts.sh
```

script는 다음 command를 `exec`하고 non-zero를 그대로 반환한다.

```bash
cargo test --release --test fts_benchmark -- --ignored --nocapture
```

### 14.7 tmux manual smoke

실제 구현 후 isolated tmux server를 사용한다.

```bash
tmux -L codex-session-selector-content-search new-session \
  -d -x 120 -y 36 \
  'target/release/select-codex-session --db /absolute/path/to/schema-v3.sqlite3 --no-refresh'
```

확인 순서:

1. `/` 입력 후 footer가 `search: metadata`인지 capture-pane으로 확인한다.
2. cwd 또는 branch에만 존재하는 substring을 입력해 metadata 결과를 확인한다.
3. `Tab` 후 `content:all` label과 content 결과를 확인한다.
4. 차례로 `content:user`, `content:agent`, `content:exec`로 이동해 각 role
   전용 fixture token의 결과 수를 확인한다.
5. content 결과에서 오른쪽 pane이 match preview가 아닌 first message를
   유지하는지 확인한다.
6. `Enter`로 search를 accept하고 replay 진입/복귀 후 query와 target label이
   유지되는지 확인한다.
7. normal mode `e`가 결과 수를 바꾸지 않고 replay visibility만 바꾸는지
   확인한다.
8. `q`로 정상 종료하고 tmux pane의 exit status가 `0`인지 확인한다.

capture와 정리:

```bash
tmux -L codex-session-selector-content-search capture-pane -p -S -200
tmux -L codex-session-selector-content-search kill-server
```

## 15. package 및 문서 변경

### 15.1 package

- `Cargo.toml`: `0.3.0` → `0.4.0`
- `Cargo.lock`: package version 동기화
- dependency와 feature 변경 없음
- Rust minimum `1.97` 유지

### 15.2 README

다음을 갱신한다.

- search key와 target 순환
- metadata substring AND 의미
- content prefix/phrase/OR 문법
- content role scope와 default `all`
- exec visibility와 검색 독립성
- first-message pane이 match preview가 아님
- schema v3 table 목록과 key 안정성
- v1/v2→v3 full source reparse migration
- FTS four-column document, tokenizer, dirty trigger 수와 repair
- `--no-refresh` schema v3 요구사항

### 15.3 CLI/TUI help

exact 핵심 문구:

```text
Tab                            switch pane focus; while searching, cycle metadata/content scope
```

TUI help Search section:

```text
/               interactive search
Tab             cycle metadata/content:all/user/agent/exec
Enter           accept search
Esc             leave search/help or quit
```

search footer:

```text
search: metadata /<query>
search: content:all /<query>
```

### 15.4 handoff

구현 완료 시 `docs/content-search-handoff.md`를 추가하고 다음을 기록한다.

- 최종 commit과 schema/package version
- 실제 변경 파일
- red/green test 목록
- migration/rollback/key-preservation 결과
- benchmark 환경과 수치
- tmux smoke 결과
- 계획과 구현의 차이가 있다면 승인 근거
- deferred preview 경계

## 16. 완료 조건

다음을 모두 만족해야 완료다.

1. schema v3가 모든 replay 대상 USER/AGENT message를 canonical 저장한다.
2. v1/v2 migration이 모든 source를 재parse하고 기존 session/exec key를
   보존한다.
3. changed source에서 동일 event index message key가 보존된다.
4. metadata는 정확히 여섯 field의 v0.2.0 substring AND 검색이다.
5. content FTS에는 metadata가 포함되지 않는다.
6. content `all/user/agent/exec` scope가 exact target만 검색한다.
7. content default scope가 `all`이고 selector 초기 target은 `metadata`다.
8. empty query, ordering, selection, error recovery, replay return과 exec
   visibility 호환성 test가 통과한다.
9. selector가 matched content preview/highlight를 표시하지 않는다.
10. dirty/row identity mismatch가 다음 refresh에서 content FTS-only repair된다.
11. README, CLI help, TUI help와 schema 설명이 실제 구현과 일치한다.
12. package version이 `0.4.0`, schema version이 `3`이다.
13. 다음 명령이 모두 exit `0`이다.

    ```bash
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features
    cargo build --release
    bash scripts/benchmark-fts.sh
    git diff --check
    ```

14. isolated `tmux -L codex-session-selector-content-search` smoke가 통과한다.
15. handoff 문서가 실제 결과를 기록한다.

## 17. 후속 작업 경계

matched USER/AGENT/exec 본문 preview는 별도 계획으로만 진행한다. 후속 작업은
사용자 승인 전 다음을 추가하지 않는다.

- matched event identity
- event excerpt/snippet/highlight
- selector pane layout 변경
- event 단위 FTS 또는 external-content FTS
- `SearchHit` evidence field
- preview navigation과 copy action

schema v3의 `message_events`는 content FTS 원본으로만 사용한다. 후속 preview가
필요해졌을 때 현재 session 단위 FTS로 role만 식별할지, 별도 event-level
index를 둘지, restricted query와 동일한 highlight 정확도를 어떻게 보장할지는
그 계획에서 비용/효과를 다시 결정한다.
