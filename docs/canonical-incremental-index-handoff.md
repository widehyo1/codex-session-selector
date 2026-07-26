# Canonical and Incremental SQLite Index Implementation Handoff

## 목적

다음 구현 세션에서
[`canonical-incremental-index-plan.md`](./canonical-incremental-index-plan.md)를
그대로 실행하기 위한 기능 2.5 handoff다.

상세 schema DDL, type과 function signature, pseudocode, exact CLI 문구,
test 이름, benchmark와 완료 조건의 source of truth는 계획 문서다. 이
handoff는 현재 worktree, 구현 순서, 반드시 유지할 계약과 phase별 확인
사항을 실행 관점에서 요약한다.

구현 세션은 production code를 바꾸기 전에 계획 문서 1~18절을 처음부터
끝까지 읽는다. handoff만 읽고 구현하거나 계획의 일부 section만 발췌해
적용하지 않는다.

## 다음 구현 세션의 임무

현재 full rebuild SQLite index를 다음 구조로 전환한다.

```text
모든 정상 session/exec를 저장하는 canonical DB
  +
file fingerprint 기반 incremental refresh
  +
stable session/exec integer identity
  +
legacy DB transactional migration
  +
selector read-time view filtering
```

최종 핵심 동작:

```text
첫 index 또는 legacy DB
  → 모든 JSONL을 canonical schema로 transactional rebuild

두 번째 무변경 index
  → metadata만 scan, JSONL parse 0, DB row write 0

신규/변경 JSONL
  → 해당 file만 parse하고 stable key를 유지하며 diff 적용

삭제 JSONL
  → source/session/exec를 같은 transaction에서 정리

selector
  → canonical DB를 읽되 기존 default visibility를 query에서 유지
```

후속 3번 FTS5가 사용할 stable key와 `IndexDelta`까지만 제공한다. FTS table,
tokenizer, search API 또는 FTS synchronization code는 구현하지 않는다.

## 문서 우선순위

구현 중 지시나 관찰이 충돌하면 다음 순서로 판단한다.

1. 구현 세션에서 사용자가 새로 내린 지시
2. `docs/canonical-incremental-index-plan.md`
3. 이 handoff
4. `docs/implementation-plan-authoring-guidelines.md`
5. 현재 production code와 test가 보여주는 실제 baseline
6. 현재 README
7. 완료된 2번 TUI 계획과 handoff
8. 완료된 one-binary 계획과 handoff

계획과 handoff가 충돌하면 계획을 따른다. 계획과 현재 코드 배치가 다르면
현재 동작을 확인한 뒤 계획의 최종 계약을 만족하도록 최소 범위에서
적용한다. 기능 결정을 새로 만들거나 계획의 exact schema를 즉석에서
바꾸지 않는다.

현재 code/test에서 계획의 전제가 사실이 아닌 것을 발견하면 다음 순서를
따른다.

1. characterization test로 실제 동작을 증명한다.
2. 계획의 사용자-visible 계약을 유지할 수 있는지 확인한다.
3. schema, option 의미, identity 또는 오류 정책을 바꿔야만 해결된다면
   구현을 멈추고 사용자에게 차이를 보고한다.

## 시작 상태

### Production 기준 commit

```text
c03f174a597b41eaa97212a03457ac3635cb3668
feat: toggle exec entries in tui
```

package 상태:

```text
package version: 0.3.0
binary target: select-codex-session 하나
edition: 2024
rust-version: 1.97
rusqlite: 0.40.1, bundled
libsqlite3-sys: 0.38.1
bundled SQLite: 3.53.2
```

### 현재 worktree

handoff 작성 시점의 expected 상태:

```text
 M Cargo.lock
 M Cargo.toml
 M README.md
 M src/replay/mod.rs
 M src/selector/mod.rs
?? docs/canonical-incremental-index-plan.md
?? docs/canonical-incremental-index-handoff.md
```

파일별 의미:

- `Cargo.toml`
  - 최소 Rust를 `1.85`에서 `1.97`로 변경
  - `rusqlite`를 `0.32`에서 `0.40.1`로 변경
- `Cargo.lock`
  - `rusqlite 0.40.1`
  - `libsqlite3-sys 0.38.1`
  - bundled SQLite 관련 transitive dependency 갱신
- `README.md`
  - 최소 Rust `1.97`
  - bundled SQLite `3.53.2` 표시
- `src/replay/mod.rs`
  - Rust 1.97 Clippy에 맞춘 동작 보존 let-chain
- `src/selector/mod.rs`
  - Rust 1.97 Clippy에 맞춘 동작 보존 let-chain
- `docs/canonical-incremental-index-plan.md`
  - 승인된 기능 2.5 상세 계획
- `docs/canonical-incremental-index-handoff.md`
  - 이 실행 handoff

이 변경은 사용자 승인으로 만들어졌다. reset, restore, checkout, 삭제 또는
dependency downgrade를 하지 않는다. 다른 사용자 변경이 추가되어 있어도
함께 원복하지 않는다.

### Prerequisite baseline

계획 작성 세션에서 확인한 결과:

```text
rustc --version
  rustc 1.97.1 (8bab26f4f 2026-07-14)

rustup check
  stable up to date: 1.97.1
  rustup up to date: 1.29.0

cargo tree -i libsqlite3-sys
  libsqlite3-sys v0.38.1
  └── rusqlite v0.40.1
      └── codex-session-selector v0.3.0

cargo fmt --check
  pass

cargo clippy --all-targets --all-features -- -D warnings
  pass

cargo test --all-targets --all-features
  49 library tests passed
  4 CLI integration tests passed
  0 failed

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
cargo tree -i libsqlite3-sys
scripts/check-before-commit.sh
```

baseline 실패 시 기능 2.5 code를 추가하기 전에 원인을 확인한다. 기존
사용자 변경을 원복해서 green으로 만들지 않는다.

## Version과 공개 정책

다음 정책은 구현 편의를 위해 바꾸지 않는다.

- package version은 계속 `0.3.0`이다.
- `Cargo.toml`과 `Cargo.lock`의 local package version을 올리지 않는다.
- binary target을 추가하거나 이름을 바꾸지 않는다.
- 최소 Rust는 `1.97`을 유지한다.
- `rusqlite 0.40.1`과 `bundled` feature를 유지한다.
- system SQLite linking으로 바꾸지 않는다.
- 기능 2.5만 완료한 상태를 crates.io에 publish하지 않는다.
- release tag 또는 GitHub release를 만들지 않는다.
- local `cargo build --release`와 temp root의 install/smoke는 공개 행위가
  아니다.
- 2번, 2.5번과 3번 FTS5 통합 완료 전까지 `0.3.0`을 공개하지 않는다.

실행 금지:

```text
cargo publish
git tag
gh release create
```

## 구현 전 반드시 읽을 파일

계획 전체를 읽은 뒤 다음 파일의 현재 내용을 확인한다.

```text
Cargo.toml
Cargo.lock
README.md
src/lib.rs
src/indexer.rs
src/application.rs
src/cli.rs
src/selector/mod.rs
src/replay/mod.rs
src/test_support.rs
tests/cli.rs
tests/fixtures/session.jsonl
scripts/check-before-commit.sh
.github/workflows/ci.yml
.githooks/pre-commit
```

특히 다음 현재 계약을 code와 test에서 확인한다.

- `parse_session_file_data`의 line 처리와 exec pairing
- `should_include_row`의 subsession/blank-message 의미
- `session_jsonl_paths`의 year/month/day traversal
- `recreate_database_with_exec`의 legacy DDL과 transaction 분리
- `load_sessions`의 column과 timestamp 정렬
- `refresh_database`가 `include_exec`를 indexer에 전달하는 현재 동작
- external recorder argument 순서
- selector/replay `e` toggle과 visibility 복귀
- current exact summary test

## 절대 경계

### 반드시 구현

- canonical `index_metadata`, `source_files`, `sessions`, `exec_events`
- `PRAGMA user_version = 1`
- stable `source_key`, `session_key`, `exec_key`
- size/mtime fingerprint
- 신규/변경/삭제/unchanged scan plan
- active file two-attempt stability 검사
- `BEGIN IMMEDIATE` 단일 write transaction
- foreign key cascade
- legacy/current/future/unknown schema detection
- legacy와 root 변경 full rebuild
- `index --rebuild`
- selector view option
- index include compatibility no-op
- exact incremental summary
- deterministic `IndexDelta`
- correctness/rollback/benchmark/manual smoke

### 절대 구현하지 않음

- FTS5 또는 Contentless-Delete
- `sessions_fts`, `exec_events_fts` 또는 유사 table
- tokenizer, ranking, phrase/prefix/query syntax
- exec selector 검색
- SQLite trigger 기반 production synchronization
- replay SQLite source
- filesystem watcher/background refresh
- content hashing dependency
- WAL 강제
- multi-root DB
- non-UTF-8 path format 개선
- package version/release 변경

향후 기능을 쉽게 만든다는 이유로 empty FTS table, search metadata, callback,
hook, trigger 또는 placeholder state를 추가하지 않는다.

## 확정 결정 요약

### Canonical 저장

indexer는 항상 다음 의미로 parse한다.

```rust
CollectOptions {
    include_subsessions: true,
    include_empty_messages: true,
    include_exec: true,
}
```

index subcommand의 기존 세 include option은 parse에 성공하지만 저장 결과를
바꾸지 않는다.

### Selector view

canonical DB는 superset을 저장하고 selector query가 표시 범위를 결정한다.

```rust
SessionView {
    include_subsessions: false,
    include_empty_messages: false,
}
```

가 default다. root CLI에 두 include option을 추가한다.

`--include-exec`는 TUI initial visibility만 결정하고 canonical DB schema나
row 집합을 바꾸지 않는다.

### Identity

```text
source natural key: path
session natural key: source_key
exec natural key: session_key + event_index
surrogate key: source_key/session_key/exec_key INTEGER PRIMARY KEY
```

session `id`와 exec `call_id`는 unique key가 아니다.

같은 natural key의 update에서 surrogate key를 바꾸지 않는다. forced
rebuild, legacy migration, root 변경 또는 file rename에서는 key 변경을
허용한다.

### Fingerprint

```text
file size
modified seconds since UNIX epoch
modified nanoseconds
```

content hash를 추가하지 않는다. 동일 size와 mtime으로 바꾼 file은 감지할
수 없으며 `index --rebuild`가 복구 경로다.

### Active file

신규/변경 file은 parse 전후 metadata가 같은지 확인하고, 다르면 한 번 더
parse한다. 두 번째도 다르면 unstable이다.

- 기존 unstable file: 이전 DB state 유지
- 신규 unstable file: DB row 없음
- 다음 refresh에서 재시도
- hard metadata/open/read 오류: 전체 operation 실패

### Transaction

connection:

```text
busy timeout: 5 seconds
foreign_keys: ON
```

write:

```text
BEGIN IMMEDIATE
→ rebuild 또는 delta apply
→ foreign_key_check/quick_check/invariant
→ user_version = 1
→ COMMIT
```

SQL 또는 invariant 실패는 전체 rollback이다.

### Schema policy

```text
Empty                  → rebuild
Legacy                 → rebuild
Current, same root     → incremental
Current, changed root  → rebuild
Future                 → always error
Unknown                → error, explicit --rebuild만 허용
```

future schema는 `--rebuild`로도 덮지 않는다.

### External recorder

새 root view option이 true일 때만 external recorder에 전달한다.

```text
--output DB
--include-subsessions
--include-empty-messages
--include-exec
```

위 순서를 사용한다. false option은 생략한다. 기존 사용자가 새 option을
쓰지 않을 때 current exact argument를 유지한다.

## 목표 file 배치

계획 6절을 그대로 사용한다.

```text
src/indexer.rs
src/indexer/scan.rs
src/indexer/schema.rs
src/indexer/store.rs
```

- orchestration과 summary는 `src/indexer.rs`
- filesystem/fingerprint/stable parse는 `scan.rs`
- exact DDL과 schema detection은 `schema.rs`
- connection/transaction/rebuild/delta/load는 `store.rs`

`src/lib.rs`의 existing public API를 삭제하지 않는다. 필요하면 새 module의
behavior-preserving wrapper로 남긴다. canonical per-file parser를 indexer가
사용할 수 있도록 visibility만 필요한 만큼 넓힌다.

## 실행 순서

계획 13절의 Phase 0~9를 순서대로 실행한다. phase를 합치거나 incremental을
canonical schema보다 먼저 구현하지 않는다.

### 1. Phase 0: Prerequisite upgrade 고정

계획 1.1절과 13절 Phase 0을 사용한다.

확인:

```bash
git diff -- Cargo.toml Cargo.lock README.md src/replay/mod.rs src/selector/mod.rs
rustc --version
cargo tree -i libsqlite3-sys
scripts/check-before-commit.sh
```

expected:

```text
rust-version = 1.97
rusqlite = 0.40.1 + bundled
libsqlite3-sys = 0.38.1
SQLite = 3.53.2
```

prerequisite가 이미 적용되어 있으므로 다시 dependency update를 실행하지
않는다. newer dependency를 추가로 찾거나 package 전체를 update하지 않는다.

commit을 수행하는 workflow라면 이 prerequisite와 let-chain을 첫 green
commit으로 분리한다. 사용자가 commit을 요청하지 않은 workflow라면
worktree에서 보존하고 완료 보고에 별도 변경 묶음으로 명시한다.

### 2. Phase 1: Characterization과 fixture

계획 13절 Phase 1과 14.1~14.3절을 사용한다.

production behavior를 바꾸기 전에 다음을 current green test로 고정한다.

```text
legacy sessions-only load
legacy sessions+exec load
invalid JSON line warning 뒤 정상 row
subsession/blank-message filter
exec event_index/output pairing
legacy summary exact text
```

기존 `tests/fixtures/session.jsonl`을 무조건 교체하지 않는다. canonical
fixture가 별도 file 집합을 요구하면 `test_support`가 temp root에 생성하도록
확장한다.

### 3. Phase 2: Schema module과 detection

계획 7.3절, 8절, 9.1~9.2절의 exact schema를 사용한다.

추가:

```text
src/indexer/schema.rs
```

반드시 test할 상태:

```text
Empty
Legacy
Current
Future
Unknown
```

schema를 간소화하거나 기존 table column 순서를 바꾸지 않는다. 기존
`sessions` 7개 column과 `exec_events` 8개 column이 먼저 오고 internal
column은 뒤에 추가된다.

이 phase에서 FTS object는 없어야 한다.

### 4. Phase 3: Canonical full rebuild

계획 5.2절, 9.3절, 10.5~10.6절을 사용한다.

먼저 correctness를 위해 canonical full rebuild를 완성한다.

```text
모든 정상 session 저장
subsession 저장
blank-message session 저장
모든 exec 저장
skipped source fingerprint 저장
단일 transaction
stable key column 생성
```

이 phase가 끝날 때 incremental optimization이 없어도 canonical schema와
rollback은 green이어야 한다.

legacy DB row를 copy/mutate하지 말고 source JSONL을 다시 parse한다.

### 5. Phase 4: Fingerprint와 scan plan

계획 5.4~5.5절과 10.2~10.3절을 사용한다.

추가:

```text
src/indexer/scan.rs
```

DB write와 분리해 다음 classification을 deterministic test한다.

```text
new
changed
unchanged
deleted
unstable
skipped
```

directory traversal이 실패한 경우 deleted path를 적용하지 않는다.

`Metadata::modified()`가 UNIX epoch 이전이면 오류다. path는 현재
`to_string_lossy()` behavior를 유지한다.

### 6. Phase 5: Incremental store와 stable identity

계획 7.5~7.6절과 10.4~10.8절을 사용한다.

추가:

```text
src/indexer/store.rs
```

per-file 기존 row를 읽어 Rust에서 diff한다. 모든 exec를 지웠다가 다시
insert하지 않는다. 그래야 unchanged exec의 `exec_key`가 유지된다.

필수 red/green:

```text
no-op parse/write 0
session update key 유지
exec update key 유지
stale exec delete
deleted source cascade
sorted unique delta
transaction rollback
```

`IndexDelta.touched_session_keys`에는 session 또는 child exec가
insert/update/delete된 parent key를 넣는다.

### 7. Phase 6: Migration과 forced rebuild

계획 9절, 12절과 13절 Phase 6을 사용한다.

CLI:

```text
index --rebuild
```

필수:

```text
legacy automatic rebuild
root changed automatic rebuild
unknown explicit rebuild
future always reject
forced rebuild logical equality
```

`--rebuild` 구현에 filesystem delete를 쓰지 않는다. exact output DB를
SQLite transaction 안에서 drop/create한다.

### 8. Phase 7: Selector view와 option migration

계획 11절을 사용한다.

root:

```text
--include-subsessions
--include-empty-messages
```

index:

```text
--include-subsessions      accepted compatibility no-op
--include-empty-messages   accepted compatibility no-op
--include-exec             accepted compatibility no-op
```

index compatibility option은 runtime warning을 출력하지 않는다. help와
README로 의미를 설명한다.

internal refresh가 root `include_exec`를 canonical index storage option으로
복사하지 않게 한다. replay visibility 전달은 유지한다.

legacy `--no-refresh` DB는 stored row를 그대로 읽는다. 없는 subsession 또는
blank row를 복원하려고 JSONL을 읽지 않는다.

### 9. Phase 8: Summary, help, README와 benchmark

계획 2.5절의 exact summary를 사용한다.

```text
updated canonical index at ...
rebuilt canonical index at ...
```

counter 의미를 바꾸거나 생략하지 않는다.

README:

- canonical DB와 view 차이
- incremental refresh
- new root option
- compatibility index option
- `--rebuild`
- metadata fingerprint 제한
- legacy migration
- FTS 비범위

benchmark는 계획 14.6절의 100/1,000/10,000 file fixture, warm-up 1회,
측정 5회 median을 사용한다. generated data를 repository에 추가하지 않는다.

### 10. Phase 9: 최종 통합 검증

계획 14.7~14.8절과 17절을 전부 확인한다.

## Red/green 실행 규칙

각 production phase:

```text
새 test 작성
→ 대상 test를 실행해 예상 이유의 red 확인
→ 같은 phase의 production 구현
→ 대상 test green
→ scripts/check-before-commit.sh
→ git diff --check
→ green 상태만 commit 가능
```

red 자체를 commit하지 않는다. 긴 작업을 중단해도 실패하는 code/test를
임시 commit하지 않는다.

characterization test가 production 변경 없이 처음부터 green이면 독립
commit이 가능하다.

## Test 실행 체크리스트

상세 test 이름과 assertion은 계획 14.1~14.5절을 사용한다.

### Schema

- exact column name/type/order
- primary/unique key
- FK와 `ON DELETE CASCADE`
- STRICT table
- `PRAGMA user_version = 1`
- metadata singleton
- FTS/trigger negative assertion

### Incremental

- unchanged file open/parse 0
- no-op DB row write 0
- one-file append parse 1
- one-file deletion parse 0/delete 1
- skipped file fingerprint cache
- stale exec deletion
- session/exec stable key
- sorted unique delta

### Error와 rollback

- traversal error는 delete 미적용
- hard file error는 전체 실패
- unstable file은 이전 state 유지
- SQL trigger fault는 transaction rollback
- legacy rebuild fault는 legacy DB 보존
- writer lock timeout은 DB 변경 없음
- future schema는 no overwrite

### CLI와 compatibility

- root view option parse/help
- `index --rebuild`
- index include option accepted
- default selector 결과 유지
- root `--include-exec`가 storage를 바꾸지 않음
- external exact argument 순서
- exact summary

### TUI regression

- selector `e` normal/search/help 동작
- replay `e`
- selector/replay visibility 전달
- replay가 JSONL을 직접 읽음
- selector query/scope/focus/selection 유지

## Schema 구현 시 주의사항

계획 8절의 DDL을 복사해 사용한다. 동등해 보이는 다른 schema로 바꾸지 않는다.

특히:

- `source_files`에는 skipped stable file도 row가 있다.
- `sessions.path`는 UNIQUE다.
- `sessions.source_key`는 UNIQUE FK다.
- `exec_events(session_key, event_index)`는 UNIQUE다.
- `INTEGER PRIMARY KEY`를 사용하고 `AUTOINCREMENT`를 추가하지 않는다.
- existing public column 순서를 먼저 유지한다.
- `has_nonempty_first_message`는 `trim().is_empty()`의 반대다.
- path index는 path UNIQUE와 중복 생성하지 않는다.
- timestamp/cwd와 exec session path/id index만 생성한다.
- schema version의 single source는 `PRAGMA user_version`이다.
- 별도 schema version column을 만들지 않는다.

Current detection은 table 이름만 확인하지 않는다. required column, key와
foreign key를 검사한다. missing ordinary index는 repair할 수 있지만 table/key
shape가 다르면 Current가 아니다.

## No-op와 key 보존 확인

기능 2.5의 핵심 acceptance는 단순히 결과 row가 같은 것이 아니다.

no-op 전후:

```text
source/session/exec key 동일
all content 동일
parsed_files = 0
delta = empty
row write = 0
```

changed file 전후:

```text
source path 동일 → source_key 동일
source session 동일 → session_key 동일
같은 event_index → exec_key 동일
실제로 바뀐 content만 UPDATE
사라진 event_index만 DELETE
새 event_index만 INSERT
```

test instrumentation 때문에 production global mutable counter를 넣지 않는다.
scan/store helper return 값이나 test connection trace처럼 test에서
deterministic하게 관찰 가능한 경계를 사용한다.

## Benchmark 실행과 기록

계획 14.6절을 그대로 따른다.

완료 보고에 다음 표를 채운다.

| files | fresh median | no-op median | one-change median | delete median | DB bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 |  |  |  |  |  |
| 1,000 |  |  |  |  |  |
| 10,000 |  |  |  |  |  |

측정 환경:

```text
OS:
CPU:
filesystem:
rustc:
profile: release
SQLite:
```

완료를 막는 조건:

- unchanged가 fresh보다 빠르지 않음
- no-op `parsed_files != 0`
- one-change `parsed_files != 1`
- delete가 JSONL을 parse함
- canonical DB가 이전 include-all+exec DB size의 1.5배 초과

benchmark가 gate를 넘으면 수치를 숨기거나 threshold를 바꾸지 않는다.
schema/index 또는 write algorithm을 확인한다.

## Manual smoke

실제 사용자 DB와 sessions root를 사용하지 않는다.

`mktemp -d`로 temp root를 만들고 plan fixture를 복사/생성한다.

실행:

```bash
select-codex-session index --sessions-root TEMP_ROOT --output TEMP_DB
select-codex-session index --sessions-root TEMP_ROOT --output TEMP_DB
select-codex-session --db TEMP_DB --no-refresh
select-codex-session --db TEMP_DB --no-refresh --include-subsessions
select-codex-session --db TEMP_DB --no-refresh --include-empty-messages
select-codex-session index --sessions-root TEMP_ROOT --output TEMP_DB --rebuild
```

확인:

- 첫 실행 `rebuilt`
- 두 번째 실행 `updated`, parsed 0
- default selector row set
- view option별 추가 row
- `e` toggle과 replay 복귀
- rebuild row content equality
- no FTS table

temp directory 제거가 필요하면 exact `mktemp` 결과 path를 확인한 뒤에만
삭제한다.

## 각 phase quality gate

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

공통 script를 사용할 때:

```bash
scripts/check-before-commit.sh
git diff --check
```

최종:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
git diff --check
```

quality command를 `.githooks/pre-commit`, CI YAML 또는 새 script에 복제하지
않는다.

## 중단하거나 사용자 판단을 요청할 조건

다음은 임의로 결정하지 않는다.

- existing code가 계획의 natural key uniqueness를 반증함
- path lossiness collision을 실제 fixture에서 발견함
- current JSONL이 append-only fingerprint 전제를 반복적으로 깨뜨림
- future schema를 지원하거나 downgrade해야 함
- external recorder compatibility를 위해 argument 의미 변경이 필요함
- canonical schema가 benchmark size gate를 넘고 계획 DDL 변경이 필요함
- FTS 준비 변경 없이는 correctness를 달성할 수 없다고 판단함
- package version 또는 release 정책 변경이 필요함

safe한 characterization, test, measurement까지 수행한 뒤 concrete evidence와
선택지를 보고한다.

다음은 blocker가 아니다.

- 작업량이 큼
- release build가 오래 걸림
- benchmark fixture 생성 시간이 김
- test가 계획한 이유로 red
- legacy migration에 full parse가 필요함

## 작업 트리 안전

- 시작 시 `git status --short`를 기록한다.
- 기존 변경과 새 변경을 구분한다.
- 사용자 변경을 reset/restore하지 않는다.
- destructive git command를 사용하지 않는다.
- generated DB, benchmark fixture와 temp install을 repository에 추가하지
  않는다.
- `.gitignore` 변경으로 generated artifact를 숨기기 전에 artifact 생성
  위치를 temp로 고친다.
- unrelated formatting 또는 dependency update를 섞지 않는다.

## 완료 조건

계획 17절의 모든 항목을 충족해야 완료다. 핵심만 다시 요약하면:

- canonical superset DB
- always-present sessions/exec table
- schema version 1
- stable session/exec identity
- no-op parse/write 0
- changed-only parse
- deletion cleanup
- active file safety
- atomic rollback
- legacy/root rebuild
- future/unknown safety
- forced rebuild
- default selector compatibility
- root view option
- index include compatibility no-op
- exact summary와 delta
- FTS negative boundary
- package/version/dependency baseline 유지
- unit/integration/manual/benchmark green
- fmt/clippy/test/release/diff green
- no publish/tag/release

하나라도 충족하지 않으면 partial completion으로 보고하고 기능 2.5 완료라고
표현하지 않는다.

## 완료 보고 형식

최종 보고에는 다음을 포함한다.

### 변경

```text
새 module/file
변경한 production file
변경한 test/fixture
README/help 변경
사용자-visible behavior
```

### Schema와 migration

```text
user_version
table/key/FK
legacy migration 결과
root change 결과
forced rebuild 결과
rollback test
```

### Incremental correctness

```text
no-op parsed/write count
new/changed/deleted count
session key 보존
exec key 보존
stale exec deletion
unstable file 결과
IndexDelta 결과
```

### Compatibility

```text
default selector row set
view option
index compatibility option
root exec visibility
external recorder arguments
TUI/replay regression
FTS negative assertion
```

### 검증

```text
phase별 red 원인
최종 unit/integration test 수
fmt
clippy
release build
git diff --check
manual smoke
benchmark 표와 측정 환경
```

### 공개 정책

```text
package version 0.3.0 유지
publish/tag/release 없음
```

## 다음 작업 경계

기능 2.5 완료 후 자동으로 3번 FTS5 구현을 시작하지 않는다.

다음 세션은 별도 승인과 별도 FTS5 계획을 사용한다. 기능 3은 다음 contract만
소비한다.

```text
sessions.session_key
exec_events.exec_key
IndexDelta
touched_session_keys
canonical rebuild mode
```

FTS table, Contentless-Delete, tokenizer, query syntax, ranking, transaction
sync와 mismatch repair는 모두 기능 3이 소유한다.
