# 메타/콘텐츠 검색 분리 구현 Handoff

## 구현 진행 기록

- schema/package: v3 / 0.4.0으로 갱신했다.
- canonical `message_events`와 역할별 content FTS 문서를 추가했다.
- selector는 metadata 및 content:all/user/agent/exec target을 Tab으로 순환한다.
- v1/v2 index는 refresh 시 source를 다시 읽어 v3으로 마이그레이션한다.
- v2 전용 unit/CLI/benchmark fixture와 assertion을 v3 계약에 맞춰 갱신했다.
- 검증: `cargo test --all-targets --all-features`, `cargo build --release`,
  `bash scripts/benchmark-fts.sh`, `cargo clippy --all-targets --all-features -- -D warnings` 통과.

## 1. 문서 상태와 목적

- 상태: 구현 대기 handoff
- 대상 기능: selector 메타 검색과 대화 콘텐츠 FTS 검색 분리
- 상세 설계 source of truth:
  [`content-search-implementation-plan.md`](./content-search-implementation-plan.md)
- production 기준 commit: `cccee9b`
- 시작 package version: `0.3.0`
- 목표 package version: `0.4.0`
- 시작 index schema version: `2`
- 목표 index schema version: `3`

이 문서는 다음 구현 세션이 저장소 탐색과 이미 끝난 제품·schema 판단을
반복하지 않고 계획을 실행하기 위한 작업 지시서다. exact DDL, type/function
signature, pseudocode, test 이름, benchmark threshold와 완료 조건은 계획
문서가 우선한다.

구현 세션은 production code를 바꾸기 전에 계획 문서 1~17절을 처음부터
끝까지 읽는다. 이 handoff만 읽거나 일부 계획 section만 발췌해 구현하지
않는다.

이 파일은 구현 완료 뒤 삭제하거나 새 handoff로 교체하지 않는다. 아래
progress ledger, 검증 결과와 완료 보고를 실제 결과로 갱신해 최종 handoff로
사용한다.

## 2. 다음 구현 세션의 임무

현재 schema v2의 metadata+exec 혼합 FTS 검색을 다음 구조로 전환한다.

```text
metadata search
  → SessionRow 여섯 field
  → v0.2.0 case-insensitive all-term substring
  → timestamp/session_key 순서 유지

content search
  → canonical USER/AGENT message + exec command/output
  → session 단위 Contentless-Delete FTS5
  → all/user/agent/exec scope
  → BM25 + deterministic tie-break

schema v1/v2 refresh
  → 모든 stable JSONL 재parse
  → 기존 session/exec key 보존
  → message_events backfill
  → schema v3 content FTS populate
```

최종 selector search target 순서는 고정한다.

```text
metadata
→ content:all
→ content:user
→ content:agent
→ content:exec
→ metadata
```

사용자-visible 핵심 계약:

- selector 초기 target은 `metadata`
- content에 처음 들어가면 `all`
- `Tab` while searching은 위 다섯 target 순환
- metadata는 substring AND이고 query syntax error가 없음
- content는 기존 prefix/phrase/OR 문법 유지
- exec command/output은 물리 FTS column을 분리하고 논리 `exec`로 묶음
- exec visibility는 검색 결과에 영향 없음
- 오른쪽 pane은 계속 first message만 표시
- matched 본문 preview/highlight는 구현하지 않음

## 3. 문서 우선순위

구현 중 지시나 관찰이 충돌하면 다음 순서로 판단한다.

1. 구현 세션에서 사용자가 새로 내린 지시
2. `docs/content-search-implementation-plan.md`
3. 이 handoff
4. `docs/implementation-plan-authoring-guidelines.md`
5. 현재 production code와 test가 보여주는 실제 baseline
6. `docs/fts5-implementation-plan.md`
7. `docs/fts5-handoff.md`
8. `docs/canonical-incremental-index-plan.md`
9. `docs/canonical-incremental-index-handoff.md`
10. 현재 README

계획과 handoff가 충돌하면 계획을 따른다. 이전 FTS 계획과 현재 계획이
충돌하면 현재 content-search 계획을 따른다. 이전 계획의 stable row identity,
Contentless-Delete lifecycle, query escaping, dirty repair와 rollback 계약은
유지하되 metadata를 FTS에 포함한다는 결정과 일곱 field scope는 교체 대상이다.

계획의 전제가 사실이 아니어서 schema, message 정규화, migration, query 의미,
ranking 또는 selector 상태 전이를 바꿔야만 구현할 수 있으면 작업을 멈추고
다음을 보고한다.

```text
계획의 전제
현재 code/test/SQLite에서 관찰한 사실
재현 명령과 failing test
영향받는 계약
계획을 유지할 수 없는 이유
가능한 대안
```

## 4. 시작 상태

### 4.1 production 기준

handoff 작성 시점:

```text
cccee9b212e7f11fd23edefafa0c3f17c4674d09
cccee9b fix: align tui paging and end scroll
```

이 commit에는 다음 선행 기능이 들어 있다.

- one-binary application
- selector/replay exec visibility toggle
- canonical incremental schema v1
- schema v2 FTS5 session search
- stable source/session/exec identity와 `IndexDelta`
- persistent read-only `SearchIndex`
- half-page `d/u`와 focused-pane bottom `G`

### 4.2 package/runtime baseline

2026-07-27 사용자 `rustup update` 이후:

```text
package: codex-session-selector 0.3.0
binary: select-codex-session
edition: 2024
rust-version: 1.97
rustc: 1.97.1
cargo: 1.97.1
clippy: 0.1.97
rusqlite: 0.40.1, bundled
libsqlite3-sys: 0.38.1
bundled SQLite: 3.53.2
current schema user_version: 2
target schema user_version: 3
```

별도 `1.97.0-x86_64-unknown-linux-gnu` toolchain도 설치되어 있지만 active
default stable은 `1.97.1`이다. 구현 명령은 default `cargo`를 사용한다.
system `sqlite3` CLI version은 bundled SQLite capability의 근거로 사용하지
않는다.

### 4.3 handoff 작성 시 worktree

handoff 생성 전:

```text
?? docs/content-search-implementation-plan.md
```

handoff 생성 후 expected:

```text
?? docs/content-search-implementation-plan.md
?? docs/content-search-handoff.md
```

두 파일은 사용자가 요청한 구현 문서다. reset, restore, checkout 또는
삭제하지 않는다. 구현 시작 시 다른 사용자 변경이 함께 보이면 보존하고,
겹치는 hunk만 주의해서 편집한다.

### 4.4 검증된 baseline

2026-07-27, commit `cccee9b`, Rust `1.97.1`에서:

```text
cargo fmt --check
  pass

cargo clippy --all-targets --all-features -- -D warnings
  pass

cargo test --all-targets --all-features
  library: 101 passed, 1 ignored, 0 failed
  main: 0 tests
  CLI integration: 10 passed, 0 failed
  FTS benchmark target: 1 ignored, 0 failed

cargo build --release
  pass

git diff --check
  pass
```

구현 시작 시 다시 실행한다.

```bash
git status --short
git diff --check
rustc --version
cargo --version
cargo clippy --version
cargo tree -i libsqlite3-sys
bash scripts/check-before-commit.sh
cargo build --release
```

baseline이 실패하면 기능 code를 추가하기 전에 원인을 확인한다. 기존 사용자
변경을 원복해서 green으로 만들지 않는다.

## 5. 구현 전 반드시 읽을 파일

계획 전체를 읽은 뒤 다음 파일의 현재 내용을 확인한다.

```text
Cargo.toml
Cargo.lock
README.md
src/lib.rs
src/application.rs
src/cli.rs
src/replay/mod.rs
src/selector/mod.rs
src/indexer.rs
src/indexer/scan.rs
src/indexer/schema.rs
src/indexer/store.rs
src/indexer/fts.rs
src/indexer/search.rs
src/test_support.rs
tests/cli.rs
tests/fts_benchmark.rs
scripts/check-before-commit.sh
scripts/benchmark-fts.sh
.githooks/pre-commit
.github/workflows/ci.yml
docs/implementation-plan-authoring-guidelines.md
docs/content-search-implementation-plan.md
```

이전 설계의 이유를 확인할 때만 다음을 참고한다.

```text
docs/fts5-implementation-plan.md
docs/fts5-handoff.md
docs/canonical-incremental-index-plan.md
docs/canonical-incremental-index-handoff.md
docs/tui-exec-toggle-plan.md
docs/tui-exec-toggle-handoff.md
```

## 6. 현재 코드 지도

line number는 `cccee9b` 기준이다. 편집 뒤 달라지므로 symbol 이름으로 다시
찾는다.

### 6.1 `src/lib.rs`

핵심:

```text
SessionRow                       line 39
ExecEvent                        line 54
ParsedSessionFile                line 81
parse_session_file_data          line 109
searchable_text                  line 214
filter_sessions                  line 227
collect_exec_event               line 443 부근
value_to_text                    line 550 부근
```

현재 parser 계약:

- first message는 첫 `event_msg.user_message`
- `include_exec=false`면 meta+first message를 얻은 뒤 early exit
- `include_exec=true`면 EOF까지 읽고 exec call/output을 pairing
- invalid JSON line은 warning 후 skip
- response_item USER/AGENT는 canonical 저장하지 않음
- `filter_sessions`는 이미 목표 metadata semantics를 구현함

변경 시 주의:

- response_item user를 first message로 승격하지 않는다.
- public `CollectedSessionData`에 deferred preview용 message field를
  추가하지 않는다.
- canonical stable parse에서는 EOF까지 message를 수집해야 한다.
- exec pairing과 legacy exec parsing 결과를 바꾸지 않는다.

### 6.2 `src/replay/mod.rs`

핵심:

```text
PayloadEvent                     line 14
NormalizedEvent                  line 48
load_entries_from_str            line 510 부근
normalize_record                 line 591
normalize_response_item          line 621
to_entry                         line 680 부근
```

현재 replay가 인식하는 message:

- bare payload `user_message`/`agent_message`
- `event_msg.user_message`/`agent_message`
- `response_item.message` role `user`/`assistant`

새 `session_event.rs`는 message 인식과 response content text 변환만 공유한다.
exec normalization, call/output pairing, `Entry`, phase suffix와 visibility는
replay에 남긴다.

Phase 0 refactor 뒤 기존 replay test의 summary/detail/phase가 달라지면
refactor를 진행하지 말고 원인을 해결한다.

### 6.3 `src/indexer/schema.rs`

핵심:

```text
SCHEMA_VERSION                  line 11
TABLE_DDL                       line 14
INDEX_DDL                       line 65
SchemaState                     line 73
detect_schema                   line 90
validate_v1                     line 233
validate_current                line 255
validate_canonical_tables       line 280 부근
validate_fts_table              line 400 부근
validate_fts_triggers           line 430 부근
create_schema                   line 560
drop_user_schema                line 570 부근
```

주의:

- version 상수만 먼저 3으로 바꾸면 valid v2가 Unknown이 된다.
- v1 exact validator는 direct v3 migration을 위해 유지한다.
- v2 exact validator를 `FtsV2` 판별용으로 유지한다.
- v3 canonical table set에 `message_events`를 포함한다.
- FTS virtual/shadow table은 STRICT canonical loop에 넣지 않는다.
- message role CHECK는 `pragma_table_info`로 보이지 않으므로 normalized
  `sqlite_schema.sql`도 확인한다.
- FTS virtual table을 shadow table보다 먼저 drop한다.
- migration용 v2 FTS drop과 full user schema drop을 혼동하지 않는다.

### 6.4 `src/indexer/store.rs`

핵심:

```text
StoredCounts                    line 30 부근
apply_rebuild                   line 107 부근
apply_incremental               line 130 부근
apply_source                    line 145 부근
apply_exec_diff                 line 319
insert_exec                     line 364
verify_invariants               line 400 부근
normalize_delta                 line 450 부근
```

message diff는 exec diff와 같은 stable child-row 패턴을 따른다.

```text
load existing by event_index
→ same event unchanged: no write
→ same event changed: key-preserving UPDATE
→ new event_index: INSERT
→ stale event_index: DELETE
→ any mutation: touched_session_keys에 session_key 추가
```

`UNIQUE(session_key,event_index)` autoindex를 사용하므로 별도
message-session index를 추가하지 않는다.

### 6.5 `src/indexer.rs`와 `scan.rs`

핵심:

```text
IndexDelta                      indexer.rs line 24
build_index                     indexer.rs line 62
choose_mode                     indexer.rs line 106
format_summary                  indexer.rs line 146 부근
scan_sources(force_all)         scan.rs line 80 부근
```

현재 build flow:

```text
schema detect
→ mode/stored fingerprint
→ scan
→ BEGIN IMMEDIATE
→ canonical apply
→ FTS delta/populate/rebuild
→ FTS invariant/clean
→ canonical invariant/version/count
→ commit
```

v1/v2 migration에서는 `force_all=true`로 모든 stored source를
`changed_sources`에 넣되 `apply_incremental`을 사용해 기존 key를 보존한다.
unstable path가 하나라도 있으면 transaction 전에 migration을 중단한다.

`IndexMode::Rebuild`를 migration으로 사용하지 않는다. rebuild는 key 보존을
보장하지 않는다.

### 6.6 `src/indexer/fts.rs`

핵심:

```text
FTS_DDL                         line 10
FTS_TRIGGER_DDL                 line 33
SearchDocument                  line 90
preflight                       line 150 부근
load_document                   line 165
populate_all                    line 230 부근
apply_delta                     line 250 부근
verify_invariants               line 280 부근
```

현재 metadata 여섯 field와 exec 두 field를 가진 8-column document를 계획의
4-column content document로 교체한다.

```text
user_content
agent_content
exec_command
exec_output
```

row identity, Contentless-Delete option, tokenizer, prefix index, dirty state와
internal integrity-check는 유지한다. dirty trigger는 sessions 3 + exec 3 +
message 3, 총 9개다.

### 6.7 `src/indexer/search.rs`

핵심:

```text
SearchScope                     line 23
SearchHit                       line 34
parse_query                     line 77 부근
compile_match                   line 150 부근
SearchIndex                     line 72
SearchIndex::open               line 190 부근
SearchIndex::search             line 224
```

유지:

- restricted parser
- quote escaping과 bind parameter
- bare prefix, phrase와 spaced `|`
- query error 문구
- read-only persistent connection
- view filter
- rank/timestamp/session_key tie-break
- dirty/row identity check

교체:

- `SearchScope` → `ContentScope`
- 8-column compiler → four content column compiler
- `search` → `search_content`
- `all_sessions` API 추가
- BM25 → `4.0, 4.0, 1.5, 0.25`

`SearchHit`에 evidence/preview field를 추가하지 않는다.

### 6.8 `src/selector/mod.rs`

핵심:

```text
SelectorApp                     line 39
SelectorApp::new                line 58
refresh_filter                  line 108
next_search_scope               line 130
search key handling             line 360 부근
render_footer                   line 560 부근
render_help                     line 600 부근
SearchScope UI impl             line 730
```

목표:

```text
all_rows: Vec<SessionRow>
search_target: SearchTarget
filtered: Vec<SessionRow>
```

metadata는 `filter_sessions(&all_rows, query)`, content는
`search_index.search_content(query, scope)`다. 성공 시 first selection/reset,
오류 시 기존 결과/selection/scroll 유지라는 현재 transition을 보존한다.

오른쪽 pane은 selected `SessionRow.first_message`만 사용한다. content match
본문을 JSONL이나 DB에서 추가로 읽지 않는다.

### 6.9 application, CLI, README와 tests

- `application.rs`
  - `SearchIndex::open`과 selector/replay loop 유지
  - schema v3 no-refresh error만 갱신
- `cli.rs`
  - search `Tab` 설명 갱신
- `README.md`
  - schema v2/8-column FTS 설명을 schema v3/two-mode로 교체
- `test_support.rs`
  - multi-message role fixture 추가
- `tests/cli.rs`
  - schema v3와 v2 no-refresh/migration assertion
- `tests/fts_benchmark.rs`
  - multi-message corpus와 v2 migration fixture

## 7. 변경하면 안 되는 계약

### 7.1 metadata와 first message

- metadata field는 정확히 여섯 개다.
- metadata 검색은 v0.2.0 substring AND다.
- metadata는 BM25나 FTS query parser를 사용하지 않는다.
- first message 추출 규칙은 바꾸지 않는다.
- default empty-message/subsession view filter를 바꾸지 않는다.

### 7.2 content semantics

- USER와 AGENT는 replay가 표시하는 message만 포함한다.
- role `assistant`는 AGENT다.
- system/developer/일반 tool message는 검색하지 않는다.
- USER/AGENT label 문자열은 indexed content에 삽입하지 않는다.
- empty message event는 저장하되 검색 token은 만들지 않는다.
- content `all`의 AND term은 session document 전체에서 만족할 수 있다.
- exec command/output은 분리 저장하고 scope에서 함께 검색한다.

### 7.3 migration과 identity

- schema v1/v2는 모든 stable source를 다시 parse해야 한다.
- migration은 source scan 뒤 한 `BEGIN IMMEDIATE` transaction이다.
- 기존 `source_key`, `session_key`, 동일 exec `exec_key`를 보존한다.
- 동일 message event index가 변경되면 `message_key`를 보존한다.
- migration unstable source는 silent skip하지 않는다.
- migration 실패는 기존 v1/v2 DB를 완전히 보존한다.
- root 변경/explicit rebuild만 full rebuild다.

### 7.4 selector lifecycle

- initial search target은 metadata다.
- content initial scope는 all이다.
- search target은 replay 왕복 뒤 유지된다.
- normal `e`는 result/query/selection을 바꾸지 않는다.
- search mode `e`는 query 문자다.
- invalid content query는 직전 성공 상태를 보존한다.
- help modal, focus, paging, `G`, clipboard와 replay return 동작을 유지한다.

### 7.5 deferred preview

다음을 구현하지 않는다.

- match excerpt/snippet/highlight
- matched role/event id를 `SearchHit`에 넣기
- event-level/external-content FTS
- selector pane 추가 또는 layout 변경
- preview navigation/copy
- preview 준비용 schema/key/query/state

`message_events`는 현재 content FTS 원본이므로 허용된다. `phase` 저장은
현재 FTS에 필요하지 않으므로 금지한다.

## 8. 구현 실행 순서

### Phase 0. shared message normalizer

계획 11절 Phase 0을 그대로 실행한다.

1. replay의 event_msg/response_item/bare payload 동작을 characterization test로
   고정한다.
2. test가 기존 code에서 green인지 확인한다.
3. `src/session_event.rs`를 추가한다.
4. `MessageRole`, `NormalizedMessage`, `normalize_message_record`,
   `value_to_text`를 이동한다.
5. replay message branch가 shared normalizer를 사용하게 한다.
6. exec normalization은 이동하지 않는다.
7. 대상 test와 전체 gate를 실행한다.

targeted commands:

```bash
cargo test replay::
cargo test session_event::
bash scripts/check-before-commit.sh
cargo build --release
git diff --check
```

commit 전 확인:

- replay output/phase label 변화 없음
- selector/index/schema 변화 없음
- new dependency 없음
- green-only

예상 commit:

```text
refactor: share session message normalization
```

### Phase 1. schema v3와 metadata/content 검색

이 phase는 canonical message, schema, FTS, search API와 selector를 하나의
green commit으로 끝낸다. schema v3 DDL만 먼저 commit하거나 old selector가
없는 FTS column을 query하는 중간 commit을 만들지 않는다.

순서:

1. plan 14.1~14.4의 parser/schema/store/search/selector test를 red로 추가한다.
2. canonical parser에 `MessageEvent` collection을 추가한다.
3. `message_events` DDL과 exact schema validation을 추가한다.
4. message key-preserving diff/delta/count/invariant를 추가한다.
5. `SchemaState::FtsV2`와 v1/v2 force-all migration을 추가한다.
6. 4-column FTS document/trigger/BM25/compiler를 추가한다.
7. `ContentScope`, `all_sessions`, `search_content`를 구현한다.
8. selector `SearchTarget`과 metadata/content dispatch를 구현한다.
9. footer/help/key handling과 error transition을 갱신한다.
10. CLI schema/migration test를 갱신한다.
11. targeted test와 전체 gate를 실행한다.

targeted commands:

```bash
cargo test session_event::
cargo test replay::
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

commit 전 DB 확인:

```text
PRAGMA user_version = 3
message_events exact DDL
sessions_fts exact 4-column DDL
sessions_fts_content 없음
dirty trigger 9개
message/session foreign key cascade
sessions_fts row count == sessions row count
fts_sync_state singleton dirty=0
foreign_key_check empty
quick_check = ok
```

commit 전 behavior 확인:

```text
metadata-only token:
  metadata match
  content:all no match

role-only token:
  content:all match
  matching role scope match
  other role scope no match

exec command/output:
  content:exec와 content:all match

right pane:
  first message 유지
  matched later message preview 없음
```

예상 commit:

```text
feat: split metadata and content session search
```

### Phase 2. benchmark, package, docs와 handoff 완료

1. benchmark corpus와 v2 migration fixture를 계획대로 갱신한다.
2. `Cargo.toml` package version을 `0.4.0`으로 올린다.
3. Cargo 명령으로 `Cargo.lock` local package version을 동기화한다.
4. README, root CLI help와 TUI help를 갱신한다.
5. release benchmark를 실행한다.
6. isolated tmux smoke를 실행한다.
7. 이 handoff의 ledger와 실제 결과를 채운다.
8. 전체 quality gate를 실행한다.

commands:

```bash
bash -n scripts/check-before-commit.sh
bash -n scripts/benchmark-fts.sh
bash scripts/check-before-commit.sh
cargo build --release
bash scripts/benchmark-fts.sh
git diff --check
```

예상 commit:

```text
docs: document metadata and content search
```

package version 변경은 이 phase의 계획된 변경이다. dependency, feature,
edition와 rust-version은 바꾸지 않는다.

## 9. Red/green과 commit 운영

각 phase:

```text
test 작성
→ 대상 test가 계획한 이유로 red인지 확인
→ production 구현
→ 대상 test green
→ full gate green
→ staged diff 검토
→ 구현+test 함께 commit
```

금지:

- red commit
- schema v3 일부만 있는 commit
- `--no-verify`로 실패 gate 숨기기
- unrelated 사용자 변경 포함
- 계획 문서나 handoff를 구현 편의 때문에 삭제
- `git reset --hard`, broad checkout/restore
- package publish/tag/release

red 결과는 세션 기록이나 final handoff에 요약할 수 있지만 broken git
history로 남기지 않는다.

## 10. 자동 검증과 test matrix

source of truth:

```text
scripts/check-before-commit.sh
```

이 script는:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

최종 추가 gate:

```bash
cargo build --release
bash scripts/benchmark-fts.sh
git diff --check
```

필수 test 그룹은 계획 14절의 exact 이름과 fixture를 사용한다.

### Normalization

- event_msg USER/AGENT
- response_item user/assistant content array
- bare replay payload
- unknown/malformed role negative
- empty message
- phase label replay regression

### Canonical/schema

- 모든 replay message EOF collection
- first message compatibility
- exact message DDL/role CHECK/FK/UNIQUE
- key-preserving insert/update/delete
- cascade와 touched session
- unchanged empty delta
- v1/v2 migration reparse/key preservation/rollback
- unstable migration abort

### Search

- all/user/agent/exec scope
- cross-column AND in content:all
- metadata excluded from FTS
- first message only if canonical message event
- stale token delete
- dirty rebuild
- Korean prefix/middle-token negative
- phrase/OR/escaping
- BM25 tie-break

### Selector

- exact target cycle
- metadata initial/content all default
- metadata substring
- content FTS
- empty query consistency
- invalid query state preservation/recovery
- replay return state
- exec visibility independence
- exact footer label
- first-message pane/no-preview negative

### CLI/benchmark

- schema v3 object/count
- v2 no-refresh actionable error
- normal v2→v3 refresh
- message count summary
- 1k/10k/50k corpus gate

## 11. Manual tmux smoke

계획 14.7절의 isolated server name을 그대로 사용한다.

```text
codex-session-selector-content-search
```

binary:

```bash
target/release/select-codex-session
```

검증 항목:

1. `/` → `search: metadata`
2. metadata-only cwd/branch substring match
3. `Tab` → `content:all`
4. 다음 Tab에서 user/agent/exec exact 순환
5. role별 unique token result
6. content result에서도 오른쪽 first message만 표시
7. search accept 후 replay 왕복 target/query 유지
8. normal `e`로 result count 불변
9. `q` exit status 0

capture:

```bash
tmux -L codex-session-selector-content-search capture-pane -p -S -200
```

종료:

```bash
tmux -L codex-session-selector-content-search kill-server
```

실패하면 capture output, 입력 key sequence, DB fixture와 exit status를 handoff에
기록하고 완료 처리하지 않는다.

## 12. 자주 놓치기 쉬운 함정

1. `first_message`를 response_item user로 바꾸지 않는다.
2. `include_exec=false` public fast path와 canonical EOF parse를 혼동하지 않는다.
3. replay와 indexer에 서로 다른 message text normalization을 복제하지 않는다.
4. USER/AGENT label을 content 문자열에 prefix하지 않는다.
5. message `phase`를 DB에 저장하지 않는다.
6. metadata query에 FTS parser를 적용하지 않는다.
7. content FTS에 first_message나 cwd를 남기지 않는다.
8. `content:exec`에서 command/output 중 하나만 검색하지 않는다.
9. v2 migration을 FTS-only migration으로 처리하지 않는다.
10. force-all scan 뒤 full rebuild를 호출해 stable key를 잃지 않는다.
11. migration에서 unstable existing source를 unchanged로 남기지 않는다.
12. message mutation 뒤 `touched_session_keys`를 빼먹지 않는다.
13. FTS delete 뒤 extra/missing row identity를 검증한다.
14. metadata filter 전에 all rows의 deterministic order를 깨지 않는다.
15. invalid content query 때 filtered/selection/scroll을 reset하지 않는다.
16. search target 전환 성공 때 selection reset 계약을 빼먹지 않는다.
17. preview를 위해 JSONL을 selector에서 다시 열지 않는다.
18. package `0.4.0` 변경을 Phase 1 code commit에 섞지 않는다.

## 13. 중단하고 보고할 조건

다음은 즉석 설계 변경 없이 중단한다.

- replay가 표시하지만 계획 normalizer mapping으로 표현할 수 없는 USER/AGENT
  record가 실제 fixture에서 발견됨
- 한 physical JSONL line에서 둘 이상의 message event가 생겨
  `UNIQUE(session_key,event_index)`가 성립하지 않음
- schema v2 exact detection 없이 migration source를 안전하게 구분할 수 없음
- v2→v3 migration에서 session/exec key 보존이 불가능함
- Contentless-Delete 4-column UPDATE/DELETE가 bundled SQLite에서 계획과 다름
- role scope를 raw FTS exposure 없이 compiler로 표현할 수 없음
- metadata/content target 순환이 기존 key mode와 충돌
- benchmark gate가 반복 가능한 release run에서 실패
- baseline이나 existing user change가 기능 diff와 겹쳐 안전하게 분리할 수 없음

보고 형식:

```text
blocker:
baseline:
reproduction:
observed:
expected by plan:
affected phase/contract:
safe options:
```

## 14. Progress ledger

구현 세션은 phase 종료 때 이 표를 실제 상태로 갱신한다.

| Phase | 상태 | commit | targeted tests | full gate | 비고 |
| --- | --- | --- | --- | --- | --- |
| 0 shared normalizer | pending | - | - | - | - |
| 1 schema/search/UI | pending | - | - | - | - |
| 2 benchmark/docs/package | pending | - | - | - | - |

허용 상태:

```text
pending
in progress
blocked
complete
```

## 15. 최종 검증 결과

구현 완료 시 placeholder를 실제 값으로 교체한다.

```text
rustc:
cargo:
SQLite:
package version:
schema version:

cargo fmt --check:
cargo clippy --all-targets --all-features -- -D warnings:
cargo test --all-targets --all-features:
cargo build --release:
bash scripts/benchmark-fts.sh:
git diff --check:

library tests:
CLI integration tests:
ignored tests:
tmux smoke:
```

benchmark:

```text
environment:
1k full/migration/size/query:
10k full/migration/size/query:
50k full/migration/size/query:
gate:
```

## 16. 완료 보고 형식

최종 응답과 handoff summary는 다음 순서로 작성한다.

```text
outcome
  package/schema version과 사용자-visible 동작

commits
  phase별 commit hash와 subject

schema/migration
  message row identity
  v1/v2 key preservation
  rollback/dirty repair

search/UI
  metadata semantics
  content scope/ranking
  state/error compatibility
  preview 미구현 확인

verification
  test count
  release build
  benchmark
  tmux smoke

deferred
  matched content preview/highlight
```

완료라고 보고하기 전에 계획 16절의 15개 완료 조건을 하나씩 확인한다.

## 17. 구현 세션 시작용 prompt

다음 세션에는 아래 요청으로 시작할 수 있다.

```text
docs/content-search-implementation-plan.md와
docs/content-search-handoff.md를 처음부터 끝까지 읽고 계획을 구현해.

handoff의 시작 baseline을 먼저 재검증하고, Phase 0부터 red/green과
green-only commit 정책을 지켜 진행해. 계획의 schema, migration,
metadata/content 검색 의미와 preview defer 경계를 임의로 바꾸지 마.

각 phase 완료 시 handoff progress ledger를 실제 commit/test 결과로
갱신하고, 전체 quality gate와 release benchmark 및 tmux -L smoke까지
완료해.
```
