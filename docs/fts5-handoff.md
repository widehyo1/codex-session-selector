# FTS5 검색 강화 구현 Handoff

## 1. 문서 상태와 목적

- 상태: 구현 전 실행 handoff
- 대상 기능: 3번 FTS5 검색 강화
- 상세 설계 source of truth:
  [`fts5-implementation-plan.md`](./fts5-implementation-plan.md)
- 목표 package version: `0.3.0` 유지
- 목표 index schema version: `2`

이 문서는 다른 구현 세션이 저장소 탐색과 이미 끝난 설계 판단을 반복하지 않고
계획을 순서대로 실행하기 위한 참고자료다. exact DDL, query grammar, type/function
signature, pseudocode, test 이름, benchmark threshold와 완료 조건은 계획 문서가
우선한다.

구현 세션은 production code를 바꾸기 전에 계획 문서 1~17절을 처음부터
끝까지 읽는다. handoff만 읽거나 일부 계획 section만 발췌해 구현하지 않는다.

이 파일은 구현 완료 뒤 새 파일로 대체하지 않는다. 아래 progress ledger,
검증 결과와 완료 보고 section을 실제 결과로 갱신해 최종 구현 handoff로
사용한다.

## 2. 다음 구현 세션의 임무

현재 canonical incremental SQLite index와 in-memory selector 검색을 다음
구조로 전환한다.

```text
canonical schema v1
  → same-root refresh에서 canonical row 재parse 없는 schema v2 migration

canonical session + exec mutation
  → 같은 BEGIN IMMEDIATE transaction의 Contentless-Delete FTS5 sync

selector query
  → persistent read-only connection의 scoped FTS5 MATCH
  → BM25 relevance
  → timestamp/session_key deterministic tie-break
```

최종 사용자-visible 핵심 동작:

```text
search scope
  all → message → cwd → branch → repo → date → exec → all

bare terms
  whitespace AND + implicit token prefix

"quoted phrase"
  exact token phrase

left | right
  whitespace-delimited OR group

exec search
  command/output을 항상 index/search
  selector exec visibility는 replay 표시만 제어
```

불일치와 복구:

```text
canonical external write
  → dirty trigger
  → 다음 index에서 FTS-only rebuild

missing/extra FTS rowid
  → 다음 index에서 FTS-only rebuild

FTS shadow corruption
  → query error가 index --rebuild 안내
  → forced canonical+FTS rebuild와 integrity-check
```

## 3. 문서 우선순위

구현 중 지시나 관찰이 충돌하면 다음 순서로 판단한다.

1. 구현 세션에서 사용자가 새로 내린 지시
2. `docs/fts5-implementation-plan.md`
3. 이 handoff
4. `docs/implementation-plan-authoring-guidelines.md`
5. 현재 production code와 test가 보여주는 실제 baseline
6. `docs/canonical-incremental-index-plan.md`
7. `docs/canonical-incremental-index-handoff.md`
8. `docs/tui-exec-toggle-plan.md`
9. `docs/tui-exec-toggle-handoff.md`
10. 현재 README

계획과 handoff가 충돌하면 계획을 따른다. 계획과 실제 코드 배치가 달라도
계획의 제품/schema 결정을 즉석에서 바꾸지 않는다. 현재 동작을 characterization
test로 확인한 뒤 계획의 최종 계약을 만족하는 최소 변경을 한다.

계획의 전제가 사실이 아니어서 schema, tokenizer, query 의미, migration,
ranking 또는 오류 정책을 바꿔야만 구현할 수 있으면 작업을 멈추고 다음을
보고한다.

```text
계획의 전제
현재 코드/SQLite에서 관찰한 사실
재현 명령과 test
영향받는 계약
가능한 대안
```

## 4. 시작 상태

### 4.1 production 기준 commit

handoff 작성 시점:

```text
5e73d9590c11e788aaabfce80c31b2f39be264d8
5e73d95 feat: add canonical incremental session index
```

이 commit에는 선행 기능이 모두 들어 있다.

- one-binary application
- selector/replay exec visibility toggle
- canonical schema v1
- incremental fingerprint scan
- stable source/session/exec key
- `IndexDelta`와 `touched_session_keys`
- canonical rebuild/incremental rollback

### 4.2 package/runtime baseline

```text
package: codex-session-selector 0.3.0
binary: select-codex-session 하나
edition: 2024
rust-version: 1.97
rustc: 1.97.1
rusqlite: 0.40.1, bundled
libsqlite3-sys: 0.38.1
bundled SQLite: 3.53.2
current schema user_version: 1
target schema user_version: 2
```

system `sqlite3` CLI는 이 환경에서 bundled library와 다를 수 있다.
Contentless-Delete capability 판단에 system CLI version을 사용하지 않는다.

### 4.3 handoff 작성 시 worktree

handoff 생성 전:

```text
?? docs/fts5-implementation-plan.md
```

handoff 생성 후 expected:

```text
?? docs/fts5-implementation-plan.md
?? docs/fts5-handoff.md
```

두 문서는 사용자가 요청한 구현 참고자료다. reset, restore, checkout 또는
삭제하지 않는다. 구현 세션에서 다른 사용자 변경이 함께 보이면 해당 변경도
보존하고 겹치는 hunk만 주의해서 편집한다.

### 4.4 검증된 baseline

2026-07-27, commit `5e73d95`에서 실행:

```text
scripts/check-before-commit.sh
  cargo fmt --check: pass
  cargo clippy --all-targets --all-features -- -D warnings: pass
  cargo test --all-targets --all-features: pass

library:
  74 passed
  1 ignored release-only canonical benchmark
  0 failed

CLI integration:
  9 passed
  0 failed
```

구현 시작 시 다음을 다시 실행한다.

```bash
git status --short
git diff --check
rustc --version
cargo tree -i libsqlite3-sys
scripts/check-before-commit.sh
```

baseline이 실패하면 FTS code를 추가하기 전에 원인을 확인한다. 기존 사용자
변경을 원복해서 green으로 만들지 않는다.

## 5. 구현 전 반드시 읽을 파일

계획 전체를 읽은 뒤 다음 파일의 현재 내용을 확인한다.

```text
Cargo.toml
Cargo.lock
README.md
src/lib.rs
src/indexer.rs
src/indexer/schema.rs
src/indexer/store.rs
src/indexer/scan.rs
src/application.rs
src/selector/mod.rs
src/ui_state.rs
src/test_support.rs
tests/cli.rs
tests/fixtures/session.jsonl
scripts/check-before-commit.sh
scripts/install-git-hooks.sh
.githooks/pre-commit
.github/workflows/ci.yml
docs/implementation-plan-authoring-guidelines.md
docs/fts5-implementation-plan.md
```

선행 기능의 의도를 확인할 때만 다음을 참고한다.

```text
docs/canonical-incremental-index-plan.md
docs/canonical-incremental-index-handoff.md
docs/tui-exec-toggle-plan.md
docs/tui-exec-toggle-handoff.md
```

선행 계획의 “FTS를 구현하지 않는다”는 당시 phase의 비범위다. 현재 작업은
승인된 별도 FTS 계획을 실행하므로 그 negative boundary만 제거한다. stable
identity, canonical storage와 exec visibility 계약은 유지한다.

## 6. 현재 코드 지도

line number는 handoff 작성 시점 기준이다. 편집 후 달라질 수 있으므로 symbol
이름으로 다시 찾는다.

### 6.1 `src/indexer.rs`

핵심:

```text
IndexDelta                    line 22
IndexMode                     line 33
build_index                   line 60
choose_mode                   line 88
format_summary                line 130
```

현재 transaction:

```text
detect schema
→ choose rebuild/incremental
→ fingerprint load
→ source scan
→ BEGIN IMMEDIATE
→ apply_rebuild 또는 apply_incremental
→ canonical verify_invariants
→ user_version=1
→ count
→ commit
→ delta normalize
```

FTS phase는 canonical mutation 뒤, canonical invariant와 commit 전에 들어간다.
`IndexDelta`는 commit 뒤에 normalize되므로 FTS delta sync는
`touched_session_keys`와 deleted key를 자체 sort/dedup해야 한다. sync를 위해
commit 전 delta normalization으로 기존 result contract를 우발적으로 바꾸지
않는다.

### 6.2 `src/indexer/schema.rs`

핵심:

```text
SCHEMA_VERSION=1             line 9
TABLE_DDL                    line 11
INDEX_DDL                    line 62
SchemaState                  line 70
detect_schema                line 86
validate_current             line 223
create_schema                line 389
drop_user_schema             line 400
```

현재 중요한 전제:

- version 1만 current
- exact user table set은 canonical table 4개
- trigger/view는 모두 unexpected
- 모든 canonical table은 STRICT
- `drop_user_schema`는 object 목록을 먼저 수집하고 trigger/view/table 순으로
  `DROP ... IF EXISTS`함

FTS 구현 시 주의:

- version 상수를 먼저 2로 바꾸면 valid v1이 바로 Unknown이 된다.
  `CanonicalV1` exact validator를 먼저 만들고 detection 분기를 함께 바꾼다.
- FTS virtual/shadow table은 `sqlite_%`가 아니므로 `user_tables()` 결과에
  나타난다.
- schema v2에서는 계획의 여섯 trigger만 허용하지만 schema v1에 trigger가
  있으면 계속 Unknown이다.
- contentless FTS에는 `_content` shadow table이 없어야 한다.
- virtual table은 STRICT가 아니므로 canonical STRICT 검사 loop에 넣지 않는다.
- shadow table의 내부 column schema를 application contract로 재정의하지
  않는다. exact object name/set과 parent virtual DDL을 검사한다.
- FTS virtual table을 shadow table보다 먼저 drop해야 한다. 수집한 shadow
  name에 대한 후속 `DROP TABLE IF EXISTS`는 no-op이어야 한다.

### 6.3 `src/indexer/store.rs`

핵심:

```text
SessionView                  line 20
open_configured_connection  line 44
load_fingerprints           line 58
begin_immediate             line 103
apply_rebuild               line 107
apply_incremental           line 126
apply_source                line 143
apply_exec_diff             line 319
delete_source               line 412
record_session_deletion     line 433
verify_invariants           line 449
load_sessions_with_view     line 508
normalize_delta             line 565
set_schema_version          line 579
```

FTS sync가 소비할 현재 보장:

- inserted session key는 `RETURNING session_key`
- update는 같은 session key 유지
- exec update는 같은 exec key 유지
- session delete 전에 `record_session_deletion`이 session/exec key를 delta에
  기록
- exec change는 해당 session key를 `touched_session_keys`에 추가
- source cascade가 canonical child row를 제거
- rebuild/incremental 모두 caller의 같은 transaction 사용

`open_configured_connection`은 parent directory를 만들고 read/write connection을
연다. selector의 read-only `SearchIndex`에는 재사용하지 말고
`SQLITE_OPEN_READ_ONLY` 기반 별도 open helper를 사용한다.

### 6.4 `src/application.rs`

핵심:

```text
run_select                  line 32
refresh_database            line 74
replay_selected             line 132
```

현재 `run_select`:

```text
optional refresh
→ load_sessions_with_view로 모든 row load
→ empty면 error
→ SelectorApp::new(rows, visibility)
→ selector/replay loop
```

목표:

```text
optional refresh/migration
→ schema v2 read-only SearchIndex::open(db, view)
→ empty query result load
→ empty면 기존 no sessions error
→ SelectorApp::new(search_index, visibility)
```

`--no-refresh` old schema의 actionable error는 terminal 진입 전에 발생해야
CLI integration test가 pseudo-terminal 없이 확인할 수 있다.

### 6.5 `src/selector/mod.rs`

핵심:

```text
SearchScope                 line 28
SelectorApp                 line 43
SelectorApp::new            line 59
refresh_filter              line 99
next_search_scope           line 116
toggle_exec_visibility      line 175
handle_key                  line 238
render footer/help          line 509/548
filter_sessions_by_scope    line 653
SearchScope impl            line 693
```

현재 app는 `rows`와 `filtered`를 모두 소유하고 query마다 in-memory lowercase
substring scan을 한다. 목표 app는 persistent `SearchIndex`와 `filtered`만
소유한다.

반드시 유지:

- 성공한 query/scope refresh는 첫 result 선택
- 성공한 empty result는 selection `None`
- successful refresh는 message scroll 0
- normal `e`는 visibility만 toggle
- search `e`는 query 문자
- replay round-trip에서 query/scope/focus/selection 유지
- help modal key priority

오류 refresh는 query/scope를 유지하고 직전 filtered/selection/scroll을
보존한다. footer status만 갱신한다.

### 6.6 `src/lib.rs`

다음 public helper는 삭제하거나 의미를 바꾸지 않는다.

```text
SessionRow                  line 41
ExecEvent                   line 53
session_date               line 206
searchable_text             line 213
filter_sessions             line 226
load_sessions              line 430
```

`searchable_text`와 `filter_sessions`는 public compatibility API로 남는다.
selector production 경로만 FTS로 교체한다.

### 6.7 test support와 CLI test

`SessionFixture`는 unique temp root와 다음 helper를 제공한다.

```text
write_session_with_exec
write_named_session
write_no_meta
```

Drop이 temp root를 정리한다. FTS fixture도 같은 pattern을 사용한다. process-wide
고정 DB path를 사용해 parallel test를 깨뜨리지 않는다.

`tests/cli.rs`의 현재 `Fixture`와 exact summary helper를 확장한다. summary
문구는 계획에서 변경 대상으로 지정하지 않았으므로 schema version/FTS 추가
때문에 index stdout을 임의로 바꾸지 않는다.

## 7. 절대 경계

### 7.1 반드시 구현

- schema v2
- exact Contentless-Delete FTS DDL
- `session_key == sessions_fts.rowid`
- first message/metadata/date/exec command/output document
- `unicode61 remove_diacritics 2`
- prefix `2 3`, detail full, columnsize 1
- dirty-state table과 여섯 canonical mutation trigger
- v1 exact detection과 same-root in-place migration
- Delta/Populate/Rebuild FTS sync mode
- canonical+FTS single transaction
- FTS-only dirty/rowid repair
- forced rebuild corruption recovery
- restricted AND/OR/prefix/phrase query compiler
- SQL bind parameter escaping
- seven search scope
- fixed BM25 weight
- timestamp/session_key tie-break
- persistent read-only search connection
- selector error-state preservation
- 한국어/path/URL/command/output tests
- benchmark, README와 최종 handoff 갱신

### 7.2 절대 구현하지 않음

- replay JSONL search
- snippet/highlight UI
- raw FTS query pass-through
- NOT/NEAR/parentheses/user column syntax
- fuzzy/vector/semantic search
- custom tokenizer/형태소 분석
- pagination/result limit/background worker
- query history/config persistence
- new search CLI subcommand
- exec kind/name/call_id/session_id search
- canonical table column 변경
- cryptographic content digest
- 새 dependency/Cargo feature
- Rust minimum/package version 변경
- publish/tag/GitHub release

비범위 기능을 위한 state, schema column, dependency 또는 hidden syntax를
미리 넣지 않는다.

## 8. 구현 결정 요약

여기서는 실행 중 자주 확인할 결정만 요약한다. exact 내용은 계획 5~9절을
따른다.

### 8.1 document

session당 FTS row 하나:

```text
rowid              session_key
first_message      weight 10.0
cwd                weight 4.0
repository_url     weight 4.0
branch             weight 5.0
timestamp          weight 2.0
date               weight 2.0
exec_command       weight 1.5
exec_output        weight 0.25
```

exec는 `event_index` 순으로 각각 newline join한다. NULL metadata는 empty
string이다. date는 timestamp 첫 10 Unicode scalar다.

### 8.2 query

```text
fix read
  → ("fix"* AND "read"*)

"readme parser" | cargo test
  → ("readme parser") OR ("cargo"* AND "test"*)
```

- prefix marker `*`는 escaped quote 밖에 둔다.
- user input은 항상 FTS string으로 double-quote escape한다.
- query 전체는 bind parameter다.
- 공백 양쪽의 `|`만 OR다.
- unclosed quote는 interactive phrase로 허용한다.
- punctuation-only atom은 오류다.
- raw FTS operator는 literal text다.

### 8.3 ranking

```text
bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)
→ rank ASC
→ timestamp DESC
→ session_key DESC
```

empty query는 FTS MATCH/rank를 사용하지 않는다.

### 8.4 visibility

exec command/output은 visibility와 관계없이 항상 검색한다. selector normal
`e`는 다음 replay visibility만 바꾸고 search result를 refresh하지 않는다.
search mode의 `e`는 query 문자다.

## 9. schema와 transaction 실행표

### 9.1 새 DB, legacy DB, root change, `--rebuild`

```text
scan all source
→ BEGIN IMMEDIATE
→ drop old user schema
→ create schema v2(canonical + empty FTS + state + trigger)
→ populate canonical rows(dirty=1)
→ populate all FTS documents
→ rowid invariant
→ FTS internal integrity-check
→ mark clean
→ canonical invariant
→ user_version=2
→ commit
```

FTS mode는 `Populate`다. 이미 빈 FTS table을 만들었으므로 다시 drop/create하지
않는다.

### 9.2 exact v1, same root, normal refresh

```text
load v1 fingerprint
→ unchanged source parse 0
→ BEGIN IMMEDIATE
→ create FTS/state/trigger extension(dirty=1)
→ canonical incremental mutation
→ populate all FTS documents
→ full FTS verification
→ mark clean
→ user_version=2
→ commit
```

FTS mode는 `Populate`다. 실패하면 version 1과 기존 data/object를 모두
보존한다.

### 9.3 healthy v2 incremental

```text
preflight dirty=0 + rowid set equal
→ canonical delta mutation(trigger가 dirty=1)
→ deleted FTS row DELETE
→ touched existing document INSERT OR REPLACE(all columns)
→ rowid invariant
→ mark clean
→ commit
```

FTS mode는 `Delta`다. healthy delta에서 전체 FTS integrity-check를 실행하지
않는다.

### 9.4 dirty 또는 rowid mismatch v2

```text
preflight → Rebuild
→ canonical delta mutation
→ FTS/state/trigger만 drop
→ FTS/state/trigger recreate
→ all documents populate
→ full integrity-check
→ mark clean
→ commit
```

canonical table/key는 보존한다.

### 9.5 shadow corruption

일반 refresh마다 전체 token integrity scan을 하지 않는다.

```text
search MATCH
→ SQLITE_CORRUPT 또는 SQLITE_CORRUPT_VTAB
→ selector는 종료하지 않고 forced rebuild 안내

select-codex-session index --rebuild
→ canonical+FTS rebuild
→ full integrity-check
```

busy, read-only, I/O, permission 오류를 corruption으로 분류해 자동 rebuild하지
않는다.

## 10. FTS5 구현 시 주의사항

### 10.1 Contentless-Delete

- 최소 SQLite `3.43.0`
- `content=''`와 `contentless_delete=1` 둘 다 필요
- rowid 없는 INSERT 금지
- UPDATE는 모든 user column을 제공해야 함
- 계획대로 `INSERT OR REPLACE`에 rowid와 8개 column을 모두 전달
- DELETE는 rowid로 수행
- FTS column을 SELECT하면 contentless이므로 NULL임
- result text는 반드시 canonical `sessions` join에서 읽음
- old contentless special delete command를 사용하지 않음

### 10.2 runtime capability

bundled baseline이 충분해도 code/test에서 확인한다.

```sql
SELECT sqlite_version();
SELECT sqlite_compileoption_used('ENABLE_FTS5');
```

version string은 `(major, minor, patch)` integer tuple로 parse한다.
lexicographic string 비교를 사용하지 않는다.

### 10.3 schema validation

- v1 exact validator와 v2 exact validator를 분리
- v1 extra object/column은 migratable로 인정하지 않음
- v2 dirty value 0/1은 둘 다 schema-valid
- clean 여부는 schema detection이 아니라 FTS health 책임
- trigger body validation에서 SQLite가 보존한 whitespace/trailing semicolon만
  normalize
- tokenizer/option 값 차이는 current로 인정하지 않음
- FTS internal shadow column layout을 hard-code하지 않음

### 10.4 dirty lifecycle

```text
schema create             dirty=1
canonical insert/update   dirty=1
canonical delete          dirty=1
FTS sync success          아직 1
FTS verification success dirty=0
commit                    clean DB
```

`mark_clean`은 singleton UPDATE affected row가 정확히 1인지 확인하고 최종
value가 0인지 assert한다. verification 전에 0으로 바꾸지 않는다.

### 10.5 delta

- deleted session key부터 FTS DELETE
- deleted set에 있는 touched key는 reload하지 않음
- 나머지 touched key는 canonical row가 반드시 있어야 함
- exec-only update/delete도 whole session document rebuild
- unchanged incremental에서는 FTS DML과 dirty transition이 없어야 함
- duplicate touched key는 local sort/dedup

### 10.6 repair

FTS-only rebuild helper는 canonical table을 drop하지 않는다. full canonical
rebuild와 이름/호출 경로를 분명히 한다.

```text
populate_all
  이미 존재하는 빈 FTS에 insert

rebuild
  FTS/state/trigger drop/create + populate_all
```

## 11. query/search 구현 시 주의사항

### 11.1 parser와 compiler 분리

parser AST와 FTS compiler를 분리해 다음을 독립 test한다.

- quote/backslash lexing
- OR group validation
- prefix/phrase atom
- scope column wrapper
- FTS double-quote escaping

사용자 입력을 SQL 또는 raw MATCH syntax에 직접 이어 붙이지 않는다.

### 11.2 read-only connection

`SearchIndex::open`:

```text
SQLITE_OPEN_READ_ONLY
→ busy timeout
→ runtime capability
→ schema v2 exact detection
→ dirty=0
→ canonical/FTS rowid set equality
```

`SelectorApp::new`가 open된 index에 empty query를 실행해 initial rows와
selection을 만든다.

query마다:

```text
dirty singleton O(1) check
→ parse/compile
→ bound MATCH query
```

query마다 전체 rowid set이나 integrity-check를 실행하지 않는다.

### 11.3 result row

contentless FTS column을 map하지 않는다. `sessions_fts.rowid`로 canonical
`sessions`를 join하고 기존 `SessionRow` field를 같은 방식으로 구성한다.

view filter:

```sql
(?include_subsessions = 1 OR is_subsession = 0)
(?include_empty = 1 OR has_nonempty_first_message = 1)
```

### 11.4 selector state

success:

```text
filtered 교체
first/None selection
message scroll 0
status clear
```

query/compiler/DB error:

```text
query 유지
scope 유지
filtered 유지
selection 유지
scroll 유지
status만 error
TUI 계속 실행
```

normal `e`는 query를 실행하지 않는다.

## 12. 목표 파일 배치

```text
src/indexer/fts.rs
  runtime, DDL lifecycle, SearchDocument, sync/health/repair

src/indexer/search.rs
  SearchScope, query AST/parser/compiler, SearchIndex

src/indexer/schema.rs
  v1/v2 detection과 exact object validation

src/indexer/store.rs
  canonical mutation 유지, 필요한 internal row loader만 제공

src/indexer.rs
  migration/sync mode와 transaction orchestration

src/application.rs
  SearchIndex open과 SelectorApp construction

src/selector/mod.rs
  ranked result state와 exec scope UI

src/test_support.rs
  deterministic FTS fixture

tests/fts_benchmark.rs
scripts/benchmark-fts.sh
README.md
docs/fts5-handoff.md
```

module 경계가 순환 dependency를 만들면 `SessionView`와 `SearchScope`의 소유
위치를 계획대로 유지하면서 import 방향을 단순화한다.

```text
indexer.rs
  → schema/store/fts/search module 선언

fts
  → canonical type/Transaction

search
  → SessionRow, SessionView, schema detection

selector
  → SearchIndex/SearchScope
```

## 13. 실행 순서

계획 10절의 phase를 순서대로 실행한다. schema/query/UI를 한 번에 섞은 큰
commit을 만들지 않는다.

### 13.1 Phase 0: baseline 고정

작업:

- 시작 명령 재실행
- 현재 substring/scope/visibility/schema/delta characterization 확인
- bundled version/FTS compile option test 추가

대상 test 예:

```bash
cargo test bundled_sqlite_supports_contentless_delete_fts5
cargo test search_remains_case_insensitive_all_terms_substring_match
cargo test canonical_schema_uses_user_version_one
```

production 변경 없이 처음부터 green인 characterization는 독립 commit 가능하다.

권장 commit:

```text
test: characterize pre-FTS search and schema
```

### 13.2 Phase 1: schema v2와 v1 migration

red:

- v2 DDL/object
- v1 migratable state
- invalid v1 unknown
- runtime capability
- migration parse 0
- migration rollback

green:

- `fts.rs` skeleton과 DDL
- v1/v2 validator
- schema version 2
- Populate mode migration

먼저 v1 detection을 green으로 만든 뒤 version 상수를 올린다. migration
transaction test 없이 schema version 변경을 commit하지 않는다.

권장 commit:

```text
feat: add FTS5 schema and v1 migration
```

### 13.3 Phase 2: document와 delta sync

red:

- metadata/exec exact document
- event order
- session insert/update/delete
- exec update/delete
- no-op no FTS write
- rollback

green:

- document loader
- `populate_all`
- Contentless-Delete delta
- dirty/clean lifecycle

권장 commit:

```text
feat: synchronize FTS documents incrementally
```

### 13.4 Phase 3: health와 repair

red:

- dirty state
- missing FTS rowid
- extra FTS rowid
- repair key preservation
- corrupt query error
- forced rebuild corruption recovery

green:

- preflight
- rowid invariant
- FTS-only `rebuild`
- full-path integrity-check
- corruption error mapping

권장 commit:

```text
feat: detect and repair stale FTS indexes
```

### 13.5 Phase 4: compiler와 search API

red:

- AND prefix
- exact phrase
- spaced OR
- unclosed quote
- escaping/operator injection
- punctuation-only error
- seven scope
- Korean/path/URL/command/output
- BM25/tie-break/view filter

green:

- AST/parser/compiler
- read-only `SearchIndex`
- empty/ranked SQL

권장 commit:

```text
feat: query FTS index with scoped BM25 ranking
```

### 13.6 Phase 5: selector 통합

red:

- Exec scope cycle/label
- success/empty/error refresh state
- visibility independence
- search-mode e
- replay round-trip state
- old schema `--no-refresh`

green:

- app-owned `SearchIndex`
- selector result refresh
- help/footer/README-facing labels
- actionable error

기존 `filter_sessions_by_scope`는 selector production에서 제거한다. public
`filter_sessions`는 유지한다.

권장 commit:

```text
feat: use FTS5 search in the selector
```

### 13.7 Phase 6: benchmark와 문서

작업:

- ignored release benchmark
- benchmark shell wrapper
- 1k/10k/50k 측정
- README 갱신
- 이 handoff progress/result section 갱신
- full automated/manual/package gate

권장 commit:

```text
docs: document FTS5 search and validation
```

## 14. red/green과 commit 규칙

각 구현 단위:

```text
test 작성
→ 대상 test가 계획한 이유로 실패하는지 확인
→ production 구현
→ 대상 test green
→ scripts/check-before-commit.sh
→ cargo build --release
→ git diff --check
→ test+implementation green commit
```

금지:

- red commit
- schema version만 바꾼 commit
- sync 없는 FTS DDL commit
- rollback test 없는 migration commit
- 전체 app rewrite
- unrelated formatting/rename
- user-owned change restore
- benchmark 실패를 문서에서 숨김

red 실행 결과는 commit하지 않지만 최종 handoff/PR 설명에 phase별 예상 실패
원인을 기록한다.

## 15. test 실행 체크리스트

exact test 이름과 assertion은 계획 13절을 따른다. 아래는 실행 중 누락 방지용
그룹이다.

### 15.1 runtime/schema

- [ ] bundled SQLite version
- [ ] FTS5 compile option
- [ ] schema v2 exact DDL
- [ ] no `_content` shadow
- [ ] exact trigger set
- [ ] v1 migratable
- [ ] malformed v1 unknown
- [ ] future schema refusal

### 15.2 migration/transaction

- [ ] v1 same-root parse 0
- [ ] v1 rollback preserves version/data
- [ ] full rebuild v2
- [ ] root change rebuild
- [ ] unknown requires rebuild
- [ ] future always refuses

### 15.3 document/delta

- [ ] metadata/date mapping
- [ ] exec event order
- [ ] insert
- [ ] session update
- [ ] exec update
- [ ] exec delete/stale token removal
- [ ] session delete
- [ ] unchanged no write
- [ ] failure rollback

### 15.4 health/repair

- [ ] dirty trigger
- [ ] clean transition
- [ ] missing rowid
- [ ] extra rowid
- [ ] FTS-only rebuild
- [ ] canonical key preservation
- [ ] corrupt query action
- [ ] forced rebuild repair

### 15.5 parser/search

- [ ] AND prefix
- [ ] quoted phrase
- [ ] spaced OR
- [ ] unclosed quote
- [ ] quote/backslash
- [ ] invalid OR
- [ ] punctuation-only
- [ ] raw operator literal
- [ ] seven scope
- [ ] Korean prefix/middle negative
- [ ] path/repository URL
- [ ] command/output
- [ ] cross-column all
- [ ] ranking/tie-break
- [ ] view filter

### 15.6 selector/CLI

- [ ] Exec scope UI
- [ ] success/empty/error state
- [ ] normal/search `e`
- [ ] visibility independence
- [ ] replay return
- [ ] v1 no-refresh error
- [ ] default refresh migration
- [ ] current summary/help regression

## 16. benchmark

계획 13.8절의 corpus와 threshold를 변경하지 않는다.

```bash
scripts/benchmark-fts.sh
```

기록할 환경:

```text
date/timezone
commit SHA
rustc
rusqlite/libsqlite3-sys
SELECT sqlite_version()
CPU/model
build profile
corpus seed/size
```

기록할 결과:

| corpus | canonical-only size/time | FTS size/time | incremental | query median | query p95 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1k | pending | pending | pending | pending | pending |
| 10k | pending | pending | pending | pending | pending |
| 50k | pending | pending | pending | pending | pending |

gate:

```text
10k warm p95 <= 100ms
50k warm p95 <= 250ms
10k one-session sync <= full FTS rebuild 20%
FTS DB <= canonical-only DB 3.0x
result path/order deterministic
```

threshold 실패 시 계획을 조용히 완화하지 않는다. query plan, prefix size와
exec output 비중을 측정하고 제품/schema 결정을 바꿔야 하면 사용자에게
보고한다.

## 17. manual smoke

계획 13.9절을 그대로 실행하고 여기 결과를 기록한다.

| 항목 | 결과 | 비고 |
| --- | --- | --- |
| `read` README match | pending | |
| `ead` non-match | pending | |
| phrase/OR | pending | |
| scope `exec` 순환 | pending | |
| command search | pending | |
| output search | pending | |
| visibility independence | pending | |
| replay round-trip state | pending | |
| dirty external write error/refresh | pending | |
| key preservation | pending | |

자동 test로 대체할 수 없는 TUI 항목은 실제 pseudo-terminal에서 확인한다.

## 18. final quality와 package 검증

자동 gate:

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

local install:

```bash
install_root="$(mktemp -d)"
cargo install --path . --root "$install_root"
test -x "$install_root/bin/select-codex-session"
test "$(find "$install_root/bin" -maxdepth 1 -type f | wc -l)" -eq 1
test "$("$install_root/bin/select-codex-session" --version)" = \
  "select-codex-session 0.3.0"
```

검증 후 생성된 exact temp root만 정리한다. broad path나 unresolved variable에
recursive delete를 실행하지 않는다.

공개 작업은 금지한다.

```text
cargo publish
git tag
gh release create
```

## 19. 구현 중 중단하고 보고할 조건

다음은 임의 판단으로 우회하지 않는다.

- bundled SQLite가 계획의 `3.53.2`/FTS5 capability와 다름
- Contentless-Delete DDL이 bundled runtime에서 실패
- 예상 shadow object set이 SQLite `3.53.2`에서 다름
- contentless `INSERT OR REPLACE`/DELETE 의미가 계획과 다름
- v1 exact schema를 canonical row reparse 없이 migration할 수 없음
- trigger dirty-state와 current transaction order가 충돌
- read-only connection에서 계획의 MATCH/rank query가 동작하지 않음
- fixed BM25 SQL이 계획한 deterministic order를 만들 수 없음
- 계획의 Korean/path/query fixture 기대가 tokenizer 실제 동작과 다름
- benchmark threshold를 만족하려면 tokenizer, indexed column 또는 query
  의미를 바꿔야 함
- current user change와 같은 hunk를 안전하게 보존할 수 없음
- package version/dependency/Rust minimum 변경이 필요함

다음은 blocker가 아니다.

- test helper 추가 필요
- module import/visibility 정리
- 계획 signature의 작은 ownership 조정
- Clippy가 동작 보존 refactor를 요구
- test fixture 생성 코드가 길어짐

작은 ownership 조정도 사용자-visible 계약, DDL, query semantics와 transaction
순서를 바꾸지 않아야 한다.

## 20. 작업 트리 안전

- 시작/각 phase/최종에 `git status --short` 확인
- 기존 문서와 사용자 변경 보존
- unrelated file formatting 금지
- `git reset --hard`, broad `git checkout --`, broad `git restore` 금지
- red code 임시 commit 금지
- destructive cleanup은 exact temp path만
- commit은 사용자가 명시적으로 요청한 경우에만 수행

handoff와 계획이 untracked인 상태에서 구현을 시작할 수 있다. 이를 “clean
baseline”으로 만들기 위해 문서를 삭제하거나 자동 commit하지 않는다.

## 21. progress ledger

구현 세션은 phase가 끝날 때 이 표를 실제 상태로 갱신한다.

| Phase | 상태 | commit | 대상 test | 전체 gate | 비고 |
| --- | --- | --- | --- | --- | --- |
| 0 baseline | pending | - | pending | baseline pass | |
| 1 schema/migration | pending | - | pending | pending | |
| 2 document/sync | pending | - | pending | pending | |
| 3 health/repair | pending | - | pending | pending | |
| 4 query/search | pending | - | pending | pending | |
| 5 selector | pending | - | pending | pending | |
| 6 benchmark/docs | pending | - | pending | pending | |

commit하지 않는 workflow라면 commit column에 `not requested`를 적고 diff
scope를 기록한다.

## 22. 완료 조건

계획 16절의 모든 항목을 충족해야 완료다. 핵심:

- schema v2 exact
- runtime capability 확인
- v1 parse-0 migration
- canonical/FTS atomic rollback
- stable FTS rowid
- complete insert/update/delete sync
- dirty/rowid repair
- forced corruption recovery
- Korean/path/URL/command/output fixture
- safe restricted query grammar
- deterministic BM25/tie-break
- selector production FTS
- visibility independence
- actionable old-schema error
- unit/integration/manual/benchmark green
- README와 이 handoff 갱신
- package/dependency/version 유지
- no publish/tag/release

하나라도 충족하지 않으면 partial completion으로 보고하고 FTS5 구현 완료라고
표현하지 않는다.

## 23. 완료 보고 형식

구현 완료 시 이 section 아래에 실제 값을 기록하고 최종 사용자 보고에도 같은
내용을 요약한다.

### 23.1 변경

```text
새 module/file:
변경 production file:
변경 test/fixture:
README/help:
사용자-visible behavior:
```

### 23.2 schema/migration

```text
SQLite runtime:
user_version:
FTS DDL/object:
v1 migration parse count:
root change:
forced rebuild:
rollback:
```

### 23.3 incremental/repair

```text
no-op FTS writes:
session insert/update/delete:
exec insert/update/delete:
dirty repair:
rowid repair:
corruption recovery:
key preservation:
```

### 23.4 query/selector

```text
grammar:
scope:
ranking:
Korean/path/URL:
command/output:
visibility:
error state:
```

### 23.5 검증

```text
phase red 원인:
unit/integration count:
fmt:
clippy:
release build:
diff check:
benchmark:
manual smoke:
local install/version:
```

### 23.6 공개 정책

```text
package version 0.3.0 유지:
dependency/Rust minimum 유지:
publish/tag/release 없음:
```

## 24. 구현 세션에 전달할 시작 지시

다른 세션에는 다음과 함께 계획과 handoff 두 파일을 제공한다.

```text
docs/fts5-implementation-plan.md와 docs/fts5-handoff.md를 처음부터 끝까지
읽고, handoff의 시작 상태를 현재 worktree에서 재검증한 뒤 계획 Phase 0부터
순서대로 구현하라.

계획이 source of truth다. schema/query/ranking/transaction 결정을 바꾸지
말고, 각 phase에서 red를 확인한 뒤 green implementation과 전체 quality
gate를 완료하라. 기존 사용자 변경을 보존하고, 계획과 실제 환경이 충돌해
제품 또는 schema 결정을 바꿔야 하면 구현을 멈추고 근거와 대안을 보고하라.

구현 완료 뒤 docs/fts5-handoff.md의 progress ledger, benchmark, manual smoke,
완료 보고를 실제 결과로 갱신하라. package version은 0.3.0으로 유지하고
publish/tag/release를 수행하지 마라.
```
