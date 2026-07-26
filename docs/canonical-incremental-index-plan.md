# Canonical and Incremental SQLite Index Implementation Plan

## 1. 문서 상태

- 상태: 구현 준비 완료
- 기능 번호: `2.5`
- 대상 package version: `0.3.0`
- 최종 설치 binary: `select-codex-session`
- 시작 production commit: `c03f174a597b41eaa97212a03457ac3635cb3668`
- 선행 기능: 2번 TUI exec visibility toggle
- 후속 기능: 3번 FTS5 검색 강화

이 문서는 모든 Codex session JSONL을 canonical SQLite schema에 저장하고,
변경된 file만 다시 읽는 증분 index를 구현하기 위한 계획이다. 구현자는 이
문서에 적힌 schema, option migration, transaction 경계와 오류 정책을
변경하거나 추가 설계 판단을 하지 않고 phase 순서대로 실행한다.

관련 문서:

- [계획 작성 지침](./implementation-plan-authoring-guidelines.md)
- [다음 구현 세션 handoff](./canonical-incremental-index-handoff.md)
- [2번 TUI exec visibility toggle 계획](./tui-exec-toggle-plan.md)
- [2번 구현 handoff](./tui-exec-toggle-handoff.md)
- [one-binary refactor 계획](./one-binary-refactor-plan.md)

### 1.1 승인된 prerequisite upgrade

계획 작성 중 사용자 승인에 따라 다음 upgrade를 먼저 적용했다.

| 항목 | 이전 | 계획 baseline |
| --- | --- | --- |
| local stable Rust | `1.96.1` | `1.97.1` |
| package `rust-version` | `1.85` | `1.97` |
| `rusqlite` | `0.32.1` | `0.40.1` |
| `libsqlite3-sys` | `0.30.1` | `0.38.1` |
| bundled SQLite | `3.46.0` | `3.53.2` |

`Cargo.toml`은 `rusqlite = { version = "0.40.1", features = ["bundled"] }`를
사용한다. SQLite를 system library에 연결하거나 `bundled` feature를
제거하지 않는다.

Rust 1.97에서 새로 발생한 `clippy::collapsible_if`를 해결하기 위해
`src/replay/mod.rs`와 `src/selector/mod.rs`의 중첩 event 조건을 let-chain으로
바꿨다. 이는 동작 보존 prerequisite이며 기능 2.5 구현 phase에 포함하지
않는다.

최신화 직후 baseline:

```text
cargo fmt --check
  pass

cargo clippy --all-targets --all-features -- -D warnings
  pass

cargo test --all-targets --all-features
  49 library tests passed
  4 CLI integration tests passed
  0 failed
```

`rust-version = "1.85"`였던 이전 manifest는 실제 lockfile과 일치하지 않았다.
기존 dependency 중 일부가 Rust 1.86~1.88을 요구해 Rust 1.85 전체 test가
실패했다. 이번에 최소 Rust를 1.97로 명시했으므로 구현 중 이를 다시 낮추거나
별도 MSRV downgrade 작업을 하지 않는다.

### 1.2 공식 기술 기준

계획은 다음 공식 문서의 동작을 사용한다.

- rusqlite `0.40.1` release:
  <https://github.com/rusqlite/rusqlite/releases/tag/v0.40.1>
- SQLite STRICT tables:
  <https://sqlite.org/stricttables.html>
- SQLite UPSERT:
  <https://sqlite.org/lang_upsert.html>
- SQLite transaction:
  <https://sqlite.org/lang_transaction.html>
- SQLite `PRAGMA user_version`:
  <https://sqlite.org/pragma.html#pragma_user_version>

STRICT table은 SQLite `3.37.0`부터 지원된다. baseline의 bundled SQLite
`3.53.2`는 이 계획이 사용하는 STRICT, UPSERT, foreign key, transaction과
`RETURNING`을 모두 지원한다.

## 2. 목표와 사용자에게 보이는 최종 동작

### 2.1 Canonical index

한 SQLite file에는 option과 무관하게 발견된 모든 정상 session을 저장한다.

- 일반 session
- subsession/subagent session
- first user message가 없거나 공백뿐인 session
- 각 session의 command execution event

`sessions`와 `exec_events` table은 항상 존재한다. `--include-exec` 여부에
따라 table을 만들거나 제거하지 않는다.

canonical은 저장 범위를 뜻한다. selector가 기본적으로 보여 주는 범위는
현재와 동일하다.

- subsession은 기본적으로 숨긴다.
- first message가 비어 있는 session은 기본적으로 숨긴다.
- exec visibility는 selector/replay의 in-memory TUI 상태이며 DB 저장 범위와
  관계없다.

### 2.2 Incremental refresh

첫 실행 또는 legacy DB migration은 전체 JSONL을 읽는다. 그 뒤 기본
refresh는 다음만 처리한다.

- 신규 JSONL parse와 insert
- fingerprint가 바뀐 JSONL parse와 update
- 사라진 JSONL의 source/session/exec row delete
- 변경되지 않은 JSONL의 metadata 확인

변경되지 않은 file은 열거나 JSON parse하지 않고 SQLite row도 쓰지 않는다.

### 2.3 Stable identity

같은 source path가 유지되는 동안 다음 key를 유지한다.

- `source_files.source_key`
- `sessions.session_key`
- 같은 session과 `event_index`를 가진 `exec_events.exec_key`

JSONL 내용, first message, repository metadata, command 또는 output이
바뀌어도 natural key가 같으면 integer key는 바뀌지 않는다.

다음 경우 key 변경을 허용한다.

- file rename 또는 다른 path로 이동
- `--rebuild` forced rebuild
- legacy schema에서 canonical schema로 최초 rebuild
- sessions root 변경으로 인한 automatic rebuild

후속 FTS5 구현은 forced/automatic full rebuild 때 FTS도 함께 rebuild해야
한다.

### 2.4 CLI 최종 동작

새 option:

```text
select-codex-session index --rebuild
select-codex-session --include-subsessions
select-codex-session --include-empty-messages
```

- `index --rebuild`는 fingerprint를 무시하고 transactional full rebuild를
  수행한다.
- root selector의 `--include-subsessions`는 canonical DB에서 subsession도
  load한다.
- root selector의 `--include-empty-messages`는 first message가 공백이거나
  없는 session도 load한다.
- root selector의 기존 `--include-exec`는 initial TUI exec visibility만
  결정한다.

기존 index option은 제거하지 않는다.

```text
index --include-subsessions
index --include-empty-messages
index --include-exec
```

세 option은 script compatibility를 위해 parse하고 성공시키지만 canonical
저장 범위를 바꾸지 않는다. runtime warning은 출력하지 않는다. `--help`와
README에서 compatibility option이며 canonical index가 항상 해당 data를
저장한다고 설명한다.

### 2.5 Summary output

index 성공 출력은 option에 따라 갈라지던 기존 문구를 다음 exact 형식으로
교체한다.

```text
updated canonical index at {output}: scanned {scanned_files} jsonl files; parsed {parsed_files} ({new_files} new, {changed_files} changed), kept {unchanged_files} unchanged, removed {deleted_files} deleted, deferred {unstable_files} unstable, skipped {skipped_files}; stored {session_rows} sessions and {exec_rows} exec events
```

fresh DB, legacy migration, root 변경 또는 `--rebuild`에는 첫 단어만 바꾼다.

```text
rebuilt canonical index at {output}: scanned ...
```

정의:

- `scanned_files`: directory traversal이 반환한 현재 JSONL 수
- `parsed_files`: 안정된 fingerprint로 parse한 신규+변경 file 수
- `new_files`: DB fingerprint가 없고 parse를 완료한 file 수
- `changed_files`: 기존 fingerprint와 달라 parse를 완료한 file 수
- `unchanged_files`: fingerprint가 같아 열지 않은 file 수
- `deleted_files`: DB에는 있었지만 scan 결과에 없는 file 수
- `unstable_files`: 두 번의 안정성 확인에 모두 실패해 이번 transaction에서
  변경하지 않은 file 수
- `skipped_files`: 성공 후 `source_files.parse_status = 'skipped'`인 현재
  file 총수
- `session_rows`, `exec_rows`: commit 후 table 전체 row 수

summary 숫자는 transaction commit 후 query한 값만 사용한다.

## 3. 구현 범위와 명시적 비범위

### 3.1 구현 범위

- canonical/superset SQLite schema
- schema version detection
- legacy DB의 transactional full rebuild migration
- sessions root 변경 detection과 automatic rebuild
- source file fingerprint
- 신규·변경 file parse
- 삭제 file 정리
- session/exec stable integer identity
- insert/update/delete delta
- no-op refresh에서 parse/write 생략
- forced rebuild
- selector view filtering
- 기존 include 계열 option migration
- rollback, concurrent writer와 unstable file 처리
- 정확한 summary와 help/README 갱신
- correctness test와 performance benchmark

### 3.2 명시적 비범위

다음은 구현하거나 준비 명목으로 schema/API를 추가하지 않는다.

- FTS5 virtual table
- Contentless-Delete
- tokenizer, prefix, phrase, ranking 또는 query syntax
- exec command/output selector 검색
- selector search algorithm 변경
- replay source를 SQLite로 변경
- filesystem watcher 또는 background daemon
- periodic refresh
- content hash 또는 새 hashing dependency
- symlink traversal 정책 변경
- session JSONL directory layout 변경
- non-UTF-8 path 저장 형식 변경
- DB encryption, compression 또는 WAL mode 강제
- multiple sessions root를 한 DB에 합치는 기능
- package version 변경
- binary target 변경
- crate publish, release tag 또는 GitHub release

특히 후속 FTS를 위해 `sessions_fts`, `exec_events_fts`, trigger, tokenizer
설정 또는 search query API를 미리 만들지 않는다.

## 4. 현재 동작과 호환성 계약

### 4.1 현재 index 흐름

현재 `src/indexer.rs::build_index`는 option을 `CollectOptions`로 변환하고 모든
file을 수집한 뒤 `recreate_database_with_exec`를 호출한다.

현재 DB write는 다음 순서다.

```text
DROP sessions
CREATE sessions
INSERT all sessions
CREATE indexes
include_exec이면 DROP/CREATE/INSERT exec_events
include_exec가 아니면 DROP exec_events
```

session insert와 exec insert는 각각 transaction이지만 DDL 전체와 두 table
전체를 묶는 하나의 transaction은 아니다. 중간 실패 시 partially rebuilt
DB가 남을 수 있다.

### 4.2 유지할 parser 동작

- session path discovery는 `sessions_root/year/month/day/*.jsonl`만 사용한다.
- 발견한 path는 정렬한다.
- 빈 line은 건너뛴다.
- invalid JSON line은 warning을 stderr에 출력하고 다음 line을 처리한다.
- 최초 `session_meta`를 사용한다.
- 최초 `event_msg/user_message`를 first message로 사용한다.
- current legacy/custom/function exec record mapping을 유지한다.
- hard file open/read 오류는 `Result::Err`로 반환한다.
- `session_meta`가 없는 file은 session을 만들지 않는다.
- timestamp 내림차순 selector 정렬을 유지한다.

canonical parse는 exec를 항상 수집하기 위해 file 끝까지 읽는다. 현재
`include_exec = false`에서 first message를 찾은 뒤 조기 종료하는 최적화는
canonical index path에서만 제거된다. 기존 public collection helper의 option
동작은 유지한다.

### 4.3 유지할 selector/replay 동작

- 기본 selector 결과에는 현재와 동일하게 subsession과 빈 first message가
  없다.
- 검색은 현재 in-memory case-insensitive all-term substring match다.
- selector 검색 scope와 정렬은 바꾸지 않는다.
- `e` toggle은 DB를 rebuild하지 않는다.
- replay는 JSONL을 직접 읽는다.
- selector/replay visibility 전달과 복귀 상태를 유지한다.
- external replay command argument 계약을 유지한다.

### 4.4 Legacy DB read

`--no-refresh`는 기존 7-column `sessions` table도 계속 읽는다.

legacy table에는 `is_subsession`과 empty-message 상태 column이 없으므로
`--no-refresh`로 legacy DB를 읽을 때는 DB에 이미 저장된 row를 그대로
반환한다. 새 selector include option은 legacy DB에 없는 row를 복원할 수
없다. canonical filtering을 보장하려면 refresh 또는 `index --rebuild`를
실행해야 한다.

### 4.5 의도적인 호환성 변경

| 이전 | 이후 |
| --- | --- |
| 기본 index는 filtered session만 저장 | canonical index는 모든 정상 session 저장 |
| `--include-exec`에 따라 exec table 생성/삭제 | exec table이 항상 존재 |
| index include option이 DB row 집합 결정 | root selector include option이 view 결정 |
| 모든 refresh가 full rebuild | 변경 file만 update |
| index 성공 문구가 include-exec에 따라 달라짐 | canonical incremental summary |
| DB에 schema version 없음 | `PRAGMA user_version = 1` |

기존 column 이름은 유지하고 새 column을 뒤에 추가한다. 기존 client가 column
이름을 지정한 SELECT를 수행하면 계속 동작한다. `SELECT *`의 새 trailing
column은 intentional schema change다.

## 5. 확정한 기술과 구조 결정

### 5.1 Dependency와 SQLite feature

추가 dependency와 Cargo feature는 없다.

| 기술 | version/feature | 용도 |
| --- | --- | --- |
| Rust | 최소 `1.97`, edition `2024` | scan, fingerprint, delta |
| rusqlite | `0.40.1`, `bundled` | connection, transaction, query |
| SQLite | bundled `3.53.2` | STRICT, UPSERT, FK, user_version |
| anyhow | `1` | context를 포함한 오류 전달 |

`rusqlite_migration`, hashing crate, filesystem watcher crate를 추가하지 않는다.
schema가 한 번 바뀌고 legacy data를 source JSONL에서 다시 만들 수 있으므로
custom migration framework 대신 exact schema detection과 rebuild를 쓴다.

### 5.2 Canonical 저장 정책

indexer는 다음 고정 option으로 parse한다.

```rust
let canonical_options = CollectOptions {
    include_subsessions: true,
    include_empty_messages: true,
    include_exec: true,
};
```

index CLI의 compatibility include bool은 parser나 DB writer에 전달하지
않는다.

### 5.3 Natural key와 surrogate key

- source natural key: lossy UTF-8 path TEXT
- session natural key: `source_key` 1:1
- exec natural key: `(session_key, event_index)`
- FTS용 surrogate key: `session_key`, `exec_key`

session `id`는 optional이며 duplicate 가능성을 배제할 근거가 없으므로 unique
key로 쓰지 않는다. `call_id`도 legacy/custom record에서 없거나 command와
output이 별도 line이므로 exec unique key로 쓰지 않는다.

path 저장은 현재 `to_string_lossy()` behavior를 유지한다. 서로 다른
non-UTF-8 path가 같은 lossy 문자열로 충돌할 수 있는 기존 제한은 이번 범위에서
해결하지 않는다. UNIQUE 충돌은 전체 transaction을 rollback하고 path를
포함한 오류를 반환한다.

### 5.4 Fingerprint

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    pub size: u64,
    pub modified_secs: i64,
    pub modified_nanos: u32,
}
```

fingerprint는 file size와 `Metadata::modified()`를 UNIX epoch 기준 seconds와
nanoseconds로 분리해 저장한다.

선택 이유:

- no-op refresh에서 file content를 읽지 않는다.
- Codex JSONL은 정상적으로 append되므로 size 또는 mtime이 바뀐다.
- 새 dependency가 없다.
- scan cost가 file 수에 비례하고 content byte 수에 비례하지 않는다.

명시적 제한:

- 동일 size로 내용을 바꾸고 mtime까지 이전 값으로 복원하면 변경을 감지하지
  못한다.
- UNIX epoch 이전 mtime은 오류로 처리한다.
- 이 제한의 복구 방법은 `index --rebuild`다.

content hash는 매 refresh마다 모든 byte를 읽어야 완전한 detection이 되므로
증분 목표와 충돌해 사용하지 않는다. metadata가 바뀐 뒤에만 hash하는 방식은
동일 metadata 변경을 검출하지 못하므로 추가 가치가 없다.

### 5.5 Active file 안정성

각 신규·변경 file은 다음 절차로 최대 두 번 parse한다.

```text
metadata before
read and parse entire file
metadata after
before == after이면 stable
다르면 한 번 즉시 retry
두 번째도 다르면 unstable
```

unstable 기존 file은 이전 DB row와 fingerprint를 그대로 유지한다. unstable
신규 file은 DB에 row를 만들지 않는다. 둘 다 다음 refresh에서 다시
candidate가 된다.

hard metadata/open/read 오류는 unstable로 낮추지 않고 전체 operation을
실패시킨다. DB transaction은 시작하지 않거나 rollback한다.

### 5.6 Transaction과 concurrency

connection 초기화:

```rust
conn.busy_timeout(Duration::from_secs(5))?;
conn.execute_batch("PRAGMA foreign_keys = ON;")?;
```

write는 `TransactionBehavior::Immediate` 하나로 묶는다.

```text
BEGIN IMMEDIATE
schema create/rebuild 또는 delta apply
row count와 invariant 확인
PRAGMA user_version = 1
COMMIT
```

5초 안에 writer lock을 얻지 못하면 오류를 반환한다. 자동 retry loop는
추가하지 않는다. reader는 SQLite snapshot behavior에 따라 이전 commit을
계속 읽을 수 있다.

parse는 DB lock을 오래 보유하지 않도록 transaction 전에 한다. transaction
안에서 source fingerprint를 다시 읽어 concurrent indexer가 먼저 commit한
경우에도 현재 key와 row를 기준으로 idempotent하게 delta를 적용한다.

### 5.7 Journal mode

WAL을 강제하지 않는다. 기존 DB의 journal mode를 유지한다. 새 DB도 SQLite
default journal mode를 사용한다. WAL은 별도 concurrency/cleanup 정책이
필요하므로 비범위다.

## 6. 목표 file 구조와 책임

```text
src/
  application.rs
  cli.rs
  indexer.rs
  indexer/
    scan.rs
    schema.rs
    store.rs
  lib.rs
  test_support.rs
tests/
  cli.rs
docs/
  canonical-incremental-index-plan.md
README.md
Cargo.toml
Cargo.lock
```

책임:

- `src/indexer.rs`
  - public orchestration
  - mode 결정
  - `IndexSummary`, `IndexOutcome`, formatting
- `src/indexer/scan.rs`
  - path scan
  - fingerprint
  - stable parse retry
  - 신규/변경/삭제/unchanged plan 생성
- `src/indexer/schema.rs`
  - DDL constant
  - schema detection
  - legacy/current/future/unknown classification
  - schema create/drop
- `src/indexer/store.rs`
  - connection configuration
  - transaction
  - rebuild/incremental apply
  - stable key와 delta
  - canonical/legacy session load
- `src/lib.rs`
  - `SessionRow`, `ExecEvent`
  - existing parser와 public collection/search API
  - canonical indexer가 사용할 per-file parse visibility
- `src/cli.rs`
  - `--rebuild`
  - root view include option
  - compatibility help
- `src/application.rs`
  - refresh와 load policy wiring
  - external recorder argument
- `src/test_support.rs`
  - legacy/canonical fixture helper
  - append/rewrite/delete helper
- `tests/cli.rs`
  - CLI help와 end-to-end index assertions
- `README.md`
  - canonical schema, incremental behavior, option migration, recovery

기존 `src/lib.rs`의 public function을 삭제하지 않는다. DB 관련 구현을
`store.rs`로 옮겨도 기존 public function은 behavior-preserving wrapper로
남긴다.

## 7. 목표 type과 function signature

### 7.1 CLI option

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectOptions {
    pub db: PathBuf,
    pub refresh: bool,
    pub print_path: bool,
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
    pub include_exec: bool,
    pub record_command: Option<String>,
    pub replay_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexOptions {
    pub output: PathBuf,
    pub sessions_root: PathBuf,
    pub rebuild: bool,
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
    pub include_exec: bool,
}
```

`IndexOptions`의 세 include field는 compatibility parse test와 external API
shape를 위해 유지하지만 `build_index`는 읽지 않는다.

### 7.2 View policy

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SessionView {
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
}
```

### 7.3 Schema state

```rust
pub(crate) const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaState {
    Empty,
    Legacy,
    Current { sessions_root: PathBuf },
    Future { version: i64 },
    Unknown { version: i64, reason: String },
}
```

`Empty`는 `user_version = 0`이고 user table이 없는 DB다. `Legacy`는
`user_version = 0`이며 현재 7-column `sessions` table을 가진 DB다.

### 7.4 Scan type

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    pub size: u64,
    pub modified_secs: i64,
    pub modified_nanos: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseStatus {
    Indexed,
    Skipped,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedSource {
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub session: Option<ParsedSessionFile>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanPlan {
    pub scanned_files: usize,
    pub new_sources: Vec<ParsedSource>,
    pub changed_sources: Vec<ParsedSource>,
    pub unchanged_paths: Vec<PathBuf>,
    pub deleted_paths: Vec<PathBuf>,
    pub unstable_paths: Vec<PathBuf>,
}
```

`ParsedSessionFile`은 `pub(crate)`로 바꾸고 `row`와 `exec_events`를
indexer가 읽을 수 있게 한다.

### 7.5 Delta와 summary

```rust
pub(crate) type SessionKey = i64;
pub(crate) type ExecKey = i64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IndexDelta {
    pub inserted_session_keys: Vec<SessionKey>,
    pub updated_session_keys: Vec<SessionKey>,
    pub deleted_session_keys: Vec<SessionKey>,
    pub inserted_exec_keys: Vec<ExecKey>,
    pub updated_exec_keys: Vec<ExecKey>,
    pub deleted_exec_keys: Vec<ExecKey>,
    pub touched_session_keys: Vec<SessionKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexMode {
    Incremental,
    Rebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexSummary {
    pub mode: IndexMode,
    pub scanned_files: usize,
    pub parsed_files: usize,
    pub new_files: usize,
    pub changed_files: usize,
    pub unchanged_files: usize,
    pub deleted_files: usize,
    pub unstable_files: usize,
    pub skipped_files: usize,
    pub session_rows: usize,
    pub exec_rows: usize,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexOutcome {
    pub summary: IndexSummary,
    pub delta: IndexDelta,
}
```

모든 key vector는 ascending numeric order로 정렬하고 중복을 제거한 뒤
반환한다. 후속 FTS test가 deterministic하게 사용할 수 있어야 한다.

### 7.6 Function signature

```rust
pub(crate) fn build_index(options: &IndexOptions) -> Result<IndexOutcome>;

pub(crate) fn format_summary(summary: &IndexSummary) -> String;

pub(crate) fn detect_schema(conn: &Connection) -> Result<SchemaState>;

pub(crate) fn load_fingerprints(
    conn: &Connection,
) -> Result<BTreeMap<PathBuf, StoredSource>>;

pub(crate) fn scan_sources(
    sessions_root: &Path,
    stored: &BTreeMap<PathBuf, StoredSource>,
    force_all: bool,
) -> Result<ScanPlan>;

pub(crate) fn stable_parse(path: &Path) -> Result<StableParse>;

pub(crate) fn apply_incremental(
    tx: &Transaction<'_>,
    root: &Path,
    plan: &ScanPlan,
) -> Result<IndexDelta>;

pub(crate) fn apply_rebuild(
    tx: &Transaction<'_>,
    root: &Path,
    plan: &ScanPlan,
) -> Result<IndexDelta>;

pub(crate) fn load_sessions_with_view(
    db_path: &Path,
    view: SessionView,
) -> Result<Vec<SessionRow>>;
```

`StableParse`은 `Stable(ParsedSource)` 또는 `Unstable(PathBuf)` 두 variant를
갖는다. hard I/O 오류는 enum variant가 아니라 `Err`다.

## 8. Exact canonical schema

### 8.1 Connection pragma

모든 canonical write connection에서 transaction 전에 실행한다.

```sql
PRAGMA foreign_keys = ON;
```

schema version은 다음 pragma만 single source of truth로 사용한다.

```sql
PRAGMA user_version = 1;
```

별도 `schema_version` column을 만들지 않는다.

### 8.2 Metadata

```sql
CREATE TABLE index_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    sessions_root TEXT NOT NULL
) STRICT;
```

항상 row 하나만 존재한다.

```sql
INSERT INTO index_metadata (singleton, sessions_root)
VALUES (1, ?1);
```

### 8.3 Source files

```sql
CREATE TABLE source_files (
    source_key INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    modified_secs INTEGER NOT NULL CHECK (modified_secs >= 0),
    modified_nanos INTEGER NOT NULL
        CHECK (modified_nanos >= 0 AND modified_nanos < 1000000000),
    parse_status TEXT NOT NULL
        CHECK (parse_status IN ('indexed', 'skipped'))
) STRICT;
```

`source_files`에는 session meta가 없어 skipped된 stable file도 저장한다. 해당
file이 unchanged이면 다음 refresh에서 다시 parse하지 않는다. file이
append/수정되면 fingerprint가 달라져 다시 parse한다.

### 8.4 Sessions

기존 7개 column 순서를 먼저 유지하고 trailing internal column을 추가한다.

```sql
CREATE TABLE sessions (
    path TEXT NOT NULL,
    id TEXT,
    timestamp TEXT,
    cwd TEXT,
    repository_url TEXT,
    branch TEXT,
    first_message TEXT NOT NULL,
    session_key INTEGER PRIMARY KEY,
    source_key INTEGER NOT NULL UNIQUE
        REFERENCES source_files(source_key) ON DELETE CASCADE,
    is_subsession INTEGER NOT NULL
        CHECK (is_subsession IN (0, 1)),
    has_nonempty_first_message INTEGER NOT NULL
        CHECK (has_nonempty_first_message IN (0, 1)),
    UNIQUE (path)
) STRICT;

CREATE INDEX sessions_timestamp_idx ON sessions(timestamp);
CREATE INDEX sessions_cwd_idx ON sessions(cwd);
```

`has_nonempty_first_message`는 정확히 다음 Rust 식의 결과다.

```rust
!row.first_message.trim().is_empty()
```

`path` UNIQUE가 기존 `sessions_path_idx` 역할을 하므로 중복 path index는
만들지 않는다.

### 8.5 Exec events

기존 8개 column 순서를 먼저 유지한다.

```sql
CREATE TABLE exec_events (
    session_path TEXT NOT NULL,
    session_id TEXT,
    event_index INTEGER NOT NULL CHECK (event_index >= 0),
    call_id TEXT,
    kind TEXT NOT NULL,
    name TEXT,
    command TEXT NOT NULL,
    output TEXT NOT NULL,
    exec_key INTEGER PRIMARY KEY,
    session_key INTEGER NOT NULL
        REFERENCES sessions(session_key) ON DELETE CASCADE,
    UNIQUE (session_key, event_index)
) STRICT;

CREATE INDEX exec_events_session_path_idx
ON exec_events(session_path);

CREATE INDEX exec_events_session_id_idx
ON exec_events(session_id);
```

`event_index`는 현재 parser의 zero-based JSONL line index를 유지한다.

### 8.6 금지 schema

다음 이름 또는 동등한 object를 만들지 않는다.

```text
sessions_fts
exec_events_fts
fts_config
fts_sync_state
search_metadata
```

trigger도 만들지 않는다.

## 9. Schema detection과 migration

### 9.1 Detection 순서

```text
PRAGMA user_version 읽기
user table 이름 읽기
version > 1이면 Future
version == 1이면 exact required table/column 검사
version == 0이고 user table 없음이면 Empty
version == 0이고 legacy sessions shape이면 Legacy
나머지는 Unknown
```

Current schema는 table 존재만 보지 않고 `PRAGMA table_info`와
`PRAGMA foreign_key_list`로 required column, primary key, unique key와 FK를
검사한다. unexpected extra non-FTS column은 허용하지 않는다. index 이름은
복구 가능하므로 missing index가 있으면 transaction 안에서 다시 만든다.

### 9.2 Mode 결정

| 상태 | 기본 실행 | `--rebuild` |
| --- | --- | --- |
| Empty | rebuild | rebuild |
| Legacy | rebuild | rebuild |
| Current, root 동일 | incremental | rebuild |
| Current, root 다름 | rebuild | rebuild |
| Future | error | error |
| Unknown | error와 `--rebuild` 안내 | rebuild |

Future schema는 `--rebuild`로도 덮어쓰지 않는다. newer binary가 만든 DB를
older binary가 파괴하지 않게 한다.

Unknown schema는 사용자가 명시적으로 `--rebuild`한 경우에만 교체한다.

### 9.3 Legacy migration

legacy row를 직접 변환하지 않는다. source JSONL을 다시 parse하여 canonical
DB를 만든다. legacy table에는 fingerprint, filter 상태와 stable identity가
없으므로 in-place row copy는 정확하지 않다.

모든 source scan과 stable parse가 성공한 뒤 `BEGIN IMMEDIATE` 안에서 기존
table을 drop하고 canonical schema와 row를 만든다. create/insert/invariant
검사 중 하나라도 실패하면 rollback하여 legacy DB를 그대로 유지한다.

### 9.4 Root 변경

`index_metadata.sessions_root`는 CLI에서 받은 path를 absolute lexical path로
정규화해 저장한다.

- 현재 working directory와 relative path를 join한다.
- `.`와 `..` component를 lexical하게 정리한다.
- filesystem `canonicalize()`는 path가 없거나 symlink일 때 의미를 바꿀 수
  있어 사용하지 않는다.

저장 root와 요청 root가 다르면 full rebuild한다. 서로 다른 root의 row를 한
DB에 합치지 않는다.

## 10. Incremental algorithm

### 10.1 Orchestration pseudocode

```rust
pub(crate) fn build_index(options: &IndexOptions) -> Result<IndexOutcome> {
    ensure_parent_directory(&options.output)?;
    let root = absolute_lexical_path(&options.sessions_root)?;
    let mut conn = open_configured_connection(&options.output)?;
    let state = detect_schema(&conn)?;
    let mode = choose_mode(&state, &root, options.rebuild)?;

    let stored = match mode {
        IndexMode::Incremental => load_fingerprints(&conn)?,
        IndexMode::Rebuild => BTreeMap::new(),
    };

    // Directory enumeration or hard read failure happens before DB mutation.
    let plan = scan_sources(&root, &stored, mode == IndexMode::Rebuild)?;

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let delta = match mode {
        IndexMode::Incremental => apply_incremental(&tx, &root, &plan)?,
        IndexMode::Rebuild => apply_rebuild(&tx, &root, &plan)?,
    };

    verify_invariants(&tx)?;
    set_user_version(&tx, SCHEMA_VERSION)?;
    let counts = query_committed_counts_before_commit(&tx)?;
    tx.commit()?;

    Ok(IndexOutcome {
        summary: summary_from(mode, &plan, counts, &options.output),
        delta: normalize_delta(delta),
    })
}
```

`query_committed_counts_before_commit`라는 이름은 transaction의 최종 state를
query한다는 의미다. 값은 commit이 성공한 뒤에만 사용자에게 반환한다.

### 10.2 Scan

```rust
pub(crate) fn scan_sources(
    root: &Path,
    stored: &BTreeMap<PathBuf, StoredSource>,
    force_all: bool,
) -> Result<ScanPlan> {
    let mut paths = session_jsonl_paths(root)?;
    paths.sort();

    let current: BTreeSet<PathBuf> = paths.iter().cloned().collect();
    let deleted_paths = stored
        .keys()
        .filter(|path| !current.contains(*path))
        .cloned()
        .collect();

    for path in paths {
        let current_fingerprint = fingerprint(&path)?;
        if !force_all
            && stored
                .get(&path)
                .is_some_and(|old| old.fingerprint == current_fingerprint)
        {
            unchanged_paths.push(path);
            continue;
        }

        match stable_parse(&path)? {
            StableParse::Stable(parsed) => classify_new_or_changed(parsed),
            StableParse::Unstable(path) => unstable_paths.push(path),
        }
    }

    Ok(plan_with_sorted_paths)
}
```

directory traversal이 중간에 실패하면 deleted set을 적용하지 않고 전체
operation을 실패시킨다.

### 10.3 Stable parse

```rust
pub(crate) fn stable_parse(path: &Path) -> Result<StableParse> {
    for _attempt in 0..2 {
        let before = fingerprint(path)?;
        let session = parse_session_file_data(path, true)?;
        let after = fingerprint(path)?;

        if before == after {
            return Ok(StableParse::Stable(ParsedSource {
                path: path.to_path_buf(),
                fingerprint: after,
                session,
            }));
        }
    }

    Ok(StableParse::Unstable(path.to_path_buf()))
}
```

invalid JSON warning behavior는 parse 함수가 그대로 소유한다.

### 10.4 Deleted source

transaction 안에서 삭제 전에 affected key를 읽는다.

```text
session_key 조회
exec_key 목록 조회
delta.deleted_*에 기록
DELETE source_files WHERE path = ?
FK cascade로 sessions와 exec_events 삭제
```

삭제된 session key는 `touched_session_keys`에도 넣는다.

### 10.5 Upsert source와 session

source:

```sql
INSERT INTO source_files (
    path, file_size, modified_secs, modified_nanos, parse_status
) VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(path) DO UPDATE SET
    file_size = excluded.file_size,
    modified_secs = excluded.modified_secs,
    modified_nanos = excluded.modified_nanos,
    parse_status = excluded.parse_status
RETURNING source_key;
```

parse result가 `None`이면 기존 session key와 exec key를 delta에 기록한 뒤
session을 delete한다. source row는 `parse_status = 'skipped'`로 유지한다.

session이 있으면 source key로 existing row를 조회한다.

- row가 없으면 insert하고 inserted key 기록
- content field가 하나라도 다르면 update하고 updated key 기록
- 모든 field가 같으면 session write를 생략

session insert:

```sql
INSERT INTO sessions (
    path, id, timestamp, cwd, repository_url, branch, first_message,
    source_key, is_subsession, has_nonempty_first_message
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
RETURNING session_key;
```

session update는 `session_key`를 set하지 않는다.

### 10.6 Exec diff

변경 file의 existing exec rows를 `(event_index, StoredExec)` BTreeMap으로
읽는다. parsed event와 event_index로 비교한다.

- parsed에만 있으면 insert
- 양쪽에 있고 content가 다르면 update
- 양쪽에 있고 content가 같으면 write 없음
- existing에만 있으면 delete

exec insert:

```sql
INSERT INTO exec_events (
    session_path, session_id, event_index, call_id,
    kind, name, command, output, session_key
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
RETURNING exec_key;
```

exec update는 `exec_key`, `session_key`, `event_index`를 바꾸지 않는다.
session path/id가 바뀌면 해당 exec row도 content change로 update한다.

exec delete는 stale key를 delta에 기록한 뒤 key로 삭제한다.

### 10.7 Touched session

다음 중 하나면 session key를 `touched_session_keys`에 넣는다.

- session insert/update/delete
- 해당 session의 exec insert/update/delete

후속 FTS가 session metadata와 exec content를 하나의 search document로 합칠
경우 이 목록만 보면 된다.

### 10.8 No-op guarantee

모든 file fingerprint가 같고 deletion이 없으면:

- JSONL open 0회
- JSON parse 0회
- `source_files`, `sessions`, `exec_events` write 0회
- `IndexDelta` 모든 vector empty
- summary의 `parsed_files = 0`
- `unchanged_files = scanned_files`

schema/index repair가 필요하지 않은 current DB라는 전제다.

## 11. Selector view와 option migration

### 11.1 Canonical query

```sql
SELECT path, id, timestamp, cwd, repository_url, branch, first_message
FROM sessions
WHERE (?1 = 1 OR is_subsession = 0)
  AND (?2 = 1 OR has_nonempty_first_message = 1)
ORDER BY timestamp DESC
```

`?1`은 `include_subsessions`, `?2`는 `include_empty_messages`다.

### 11.2 CLI mapping

| old workflow | new canonical workflow |
| --- | --- |
| `index --include-subsessions` 후 `--no-refresh` | root `--include-subsessions` |
| `index --include-empty-messages` 후 `--no-refresh` | root `--include-empty-messages` |
| `index --include-exec` | option 없이도 exec는 DB에 저장 |
| root `--include-exec` | 그대로 initial TUI visibility |

index compatibility option은 help에서 다음 exact description을 사용한다.

```text
--include-subsessions      Compatibility option; canonical index always stores subsessions
--include-empty-messages   Compatibility option; canonical index always stores empty messages
--include-exec             Compatibility option; canonical index always stores exec events
--rebuild                  Ignore fingerprints and rebuild the canonical index
```

root help:

```text
--include-subsessions      Show subsessions from the canonical index
--include-empty-messages   Show sessions without a non-empty first message
```

### 11.3 Internal refresh

현재 `refresh_database`가 root `include_exec`를 `IndexOptions`로 복사하는
동작을 제거한다. internal refresh의 index include compatibility field는
모두 false여도 결과가 canonical이어야 한다.

### 11.4 External recorder

external recorder는 canonical contract를 강제할 수 없으므로 기존 option
contract를 유지한다. argument 순서:

```text
--output DB
--include-subsessions        # root option이 true일 때만
--include-empty-messages     # root option이 true일 때만
--include-exec               # root option이 true일 때만
```

기존 사용자가 새 root option을 쓰지 않으면 현재 exact arguments
`--output DB` 또는 `--output DB --include-exec`가 유지된다.

external recorder가 legacy DB를 만들면 loader는 schema를 감지해 legacy read
path를 사용한다.

## 12. 오류 처리와 lifecycle

### 12.1 오류와 DB 결과

| 오류 | DB 결과 |
| --- | --- |
| sessions root read 실패 | 변경 없음 |
| file metadata/open/read 실패 | 변경 없음 |
| file 두 번 변경 | 그 file만 이전 state 유지, operation 성공 |
| unknown schema, no rebuild | 변경 없음, 오류 |
| future schema | 변경 없음, 오류 |
| writer lock timeout | 변경 없음, 오류 |
| SQL constraint/insert 실패 | transaction 전체 rollback |
| invariant 실패 | transaction 전체 rollback |
| commit 실패 | SQLite가 보고한 state를 오류로 반환 |

### 12.2 Invariant

commit 전에 다음 exact query를 확인한다.

```sql
PRAGMA foreign_key_check;
PRAGMA quick_check;
```

- `foreign_key_check` row가 하나라도 있으면 오류
- `quick_check` 첫 row가 exact `ok`가 아니면 오류
- `index_metadata` row count가 1이 아니면 오류
- indexed source 수와 session 수가 같지 않으면 오류
- skipped source에는 session이 없어야 한다

indexed source/session count:

```sql
SELECT count(*) FROM source_files WHERE parse_status = 'indexed';
SELECT count(*) FROM sessions;
```

### 12.3 Interrupted process

process가 parse 중 종료되면 DB transaction을 시작하지 않았으므로 기존 DB가
남는다. transaction 중 종료되면 SQLite rollback journal이 atomicity를
보장한다. temp DB file을 따로 만들거나 rename하지 않는다.

### 12.4 Forced rebuild safety

`--rebuild`는 사용자가 지정한 exact output DB만 대상으로 한다. parent
directory나 다른 DB를 삭제하지 않는다. filesystem `remove_file`을 사용하지
않고 SQLite transaction 안에서 user objects를 drop/create한다.

future schema는 forced rebuild로도 덮지 않는다.

## 13. 단계별 구현 순서

각 phase는 이전 phase green 이후 시작한다.

### Phase 0: Prerequisite upgrade 고정

이미 적용한 Rust/rusqlite upgrade와 let-chain 변경을 검증한다.

검증:

```bash
rustc --version
cargo tree -i libsqlite3-sys
scripts/check-before-commit.sh
```

expected:

```text
rustc 1.97.1
libsqlite3-sys v0.38.1
rusqlite v0.40.1
```

Cargo build output의 bundled SQLite constant가 `3.53.2`인지 확인한다.

이 prerequisite는 기능 구현과 분리된 green commit으로 만들 수 있다.

### Phase 1: Characterization과 fixture

production 변경 전에 다음 현재 behavior test를 추가한다.

- legacy sessions-only schema load
- legacy sessions+exec schema load
- parser invalid line warning 후 정상 row
- subsession/blank message current filter
- exec event_index와 output pairing
- current summary exact text

처음부터 green인 characterization test만 commit한다.

### Phase 2: Schema module과 detection

`schema.rs`와 exact DDL을 추가한다.

red:

- empty DB classification
- legacy classification
- future/unknown rejection
- canonical table shape와 key/FK

green 후에도 production `build_index`는 아직 legacy full rebuild path를
사용해도 된다. 새 schema helper가 독립 test로 검증되면 commit한다.

### Phase 3: Canonical full rebuild

indexer가 모든 session/exec를 저장하도록 전환한다.

red:

- default option으로 subsession, blank session, exec가 모두 DB에 존재
- include option 조합과 canonical row가 동일
- full rebuild failure rollback
- stable key column 존재

이 phase에서 legacy/current detection에 따라 모두 full rebuild해도 된다.
incremental은 다음 phase다.

### Phase 4: Fingerprint와 scan plan

`scan.rs`, `FileFingerprint`, `ScanPlan`, two-attempt stability를 구현한다.

red:

- unchanged/new/changed/deleted classification
- skipped stable file이 unchanged 처리
- active append가 unstable 처리
- hard I/O error propagation

DB write와 연결하지 않은 deterministic unit test로 먼저 green을 만든다.

### Phase 5: Incremental store와 stable identity

`apply_incremental`과 per-file diff를 구현한다.

red:

- no-op write/parse zero
- changed session key 유지
- existing exec key 유지
- stale exec delete
- deleted source cascade와 delta
- SQL failure rollback

Phase 5 완료 시 기본 refresh를 incremental로 전환한다.

### Phase 6: Migration과 forced rebuild

`--rebuild`, root 변경 rebuild, legacy migration, unknown/future policy를
연결한다.

red:

- legacy rebuild
- root 변경 rebuild
- unknown requires explicit rebuild
- future always rejected
- forced rebuild logical equality

### Phase 7: View option migration

root selector option, canonical load query, index compatibility help와 external
argument를 구현한다.

red:

- default filter 유지
- 각 root include option
- index include option no-op canonical equality
- root `--include-exec`가 DB schema에 영향 없음
- external exact argument order

### Phase 8: Summary, docs와 benchmark

exact summary, README, help, benchmark를 완성한다.

### Phase 9: 최종 통합 검증

14절의 자동/수동 검증과 15절 완료 조건을 모두 확인한다.

## 14. Test와 quality gate

### 14.1 Unit test 목록

`src/indexer/schema.rs`:

```text
empty_database_is_detected
legacy_sessions_only_database_is_detected
legacy_database_with_exec_is_detected
current_schema_requires_exact_keys_and_foreign_keys
future_schema_is_rejected_even_for_rebuild
unknown_schema_requires_explicit_rebuild
canonical_schema_uses_user_version_one
```

`src/indexer/scan.rs`:

```text
fingerprint_uses_size_seconds_and_nanoseconds
unchanged_file_is_not_parsed
new_changed_deleted_and_unchanged_are_classified
stable_skipped_file_is_cached
file_changed_during_both_attempts_is_deferred
hard_file_error_aborts_scan
```

`src/indexer/store.rs`:

```text
canonical_rebuild_stores_all_session_classes_and_exec
canonical_rebuild_ignores_compatibility_include_options
unchanged_refresh_preserves_all_keys_and_writes_nothing
session_update_preserves_session_key
exec_update_preserves_exec_key
stale_exec_rows_are_deleted
deleted_source_cascades_and_reports_keys
skipped_source_removes_previous_session
failed_incremental_transaction_rolls_back_all_tables
legacy_rebuild_failure_preserves_legacy_database
root_change_forces_rebuild
forced_rebuild_matches_fresh_database
delta_keys_are_sorted_and_unique
```

`src/cli.rs`와 `src/application.rs`:

```text
root_parses_session_view_options
index_parses_rebuild
index_include_options_remain_accepted
internal_refresh_does_not_forward_visibility_to_canonical_storage
external_refresh_forwards_true_view_options_in_exact_order
```

### 14.2 Fixture

기본 canonical fixture는 최소 다음 5개 file을 갖는다.

```text
normal.jsonl
  session_meta + non-empty user message + three exec forms

subsession.jsonl
  source.subagent 또는 thread_source=subagent

empty.jsonl
  session_meta, no non-empty user message

invalid-line.jsonl
  invalid JSON line 뒤 valid session_meta와 user message

no-meta.jsonl
  event만 있고 session_meta 없음
```

metadata에는 다음 문자를 포함한다.

- 한글 first message: `증분 인덱스 확인`
- 공백을 포함한 cwd
- repository URL
- slash와 quote를 포함한 command
- multiline exec output

### 14.3 Exact assertion

테스트는 row count만 확인하지 않는다.

- exact DDL column/name/type/not-null/PK
- exact `PRAGMA user_version`
- exact FK target와 cascade
- key before/after equality
- changed/deleted delta exact vector
- summary exact string
- default/root option query exact path set
- rollback 전후 table dump equality
- negative `sessions_fts`/trigger absence

### 14.4 Integration test

`tests/cli.rs`에서 실제 binary를 실행한다.

```text
index_fresh_build_reports_rebuilt_canonical_summary
index_second_run_reports_zero_parsed_files
index_changed_and_deleted_files_report_incremental_counts
index_rebuild_forces_full_parse
selector_view_options_are_listed_in_root_help
index_help_marks_include_options_as_compatibility
```

SQLite assertion은 temp DB를 `rusqlite::Connection`으로 열어 수행한다.

### 14.5 Fault injection

production에 test-only global flag를 넣지 않는다. rollback test는 temp DB에
다음 trigger를 설치해 write를 실패시킨다.

```sql
CREATE TRIGGER fail_session_update
BEFORE UPDATE ON sessions
BEGIN
    SELECT RAISE(ABORT, 'injected session update failure');
END;
```

canonical schema detection이 unexpected trigger를 허용하도록 하지 않는다.
fault test는 schema detection을 통과한 뒤 transaction 적용 helper를 직접
호출한다. public CLI unknown-schema 정책과 섞지 않는다.

### 14.6 Performance benchmark

새 dependency 없이 ignored Rust test 또는 test helper binary로 synthetic
fixture를 생성한다. generated fixture와 DB는 temp directory에 두고
repository에 commit하지 않는다.

규모:

```text
100 files
1,000 files
10,000 files
```

각 file:

- session_meta 1개
- user message 1개
- exec command/output pair 10개
- output 각 1 KiB

release build에서 각 case를 warm-up 1회 후 5회 실행하고 median을 기록한다.

측정:

- fresh rebuild elapsed
- unchanged refresh elapsed
- one-file append elapsed
- one-file deletion elapsed
- DB byte size
- parsed/new/changed/unchanged/deleted counter

완료 gate:

- unchanged refresh의 `parsed_files = 0`
- one-file append의 `parsed_files = 1`
- one-file deletion의 `parsed_files = 0`, `deleted_files = 1`
- 1,000/10,000 file에서 unchanged median이 fresh rebuild median보다 빨라야
  한다.
- 10,000 file canonical DB가 동일 fixture의 이전 include-all+exec DB
  size의 1.5배를 넘으면 schema/index 구성을 재검토하고 완료 처리하지 않는다.

wall-clock 절대 시간은 hardware-dependent이므로 CI assertion으로 만들지 않고
implementation handoff에 측정 환경과 median을 기록한다.

### 14.7 Manual smoke

실제 `~/.codex/sessions` 대신 복사한 temp root와 temp DB를 사용한다.

```bash
select-codex-session index --sessions-root TEMP_ROOT --output TEMP_DB
select-codex-session index --sessions-root TEMP_ROOT --output TEMP_DB
select-codex-session --db TEMP_DB --no-refresh
select-codex-session --db TEMP_DB --no-refresh --include-subsessions
select-codex-session --db TEMP_DB --no-refresh --include-empty-messages
select-codex-session index --sessions-root TEMP_ROOT --output TEMP_DB --rebuild
```

확인:

- 두 번째 index가 parsed 0
- default selector 결과가 현재와 동일
- view option이 해당 hidden row만 추가
- `e` toggle과 replay 복귀 상태 유지
- forced rebuild 뒤 key 변경은 허용되지만 row content 동일
- 실제 사용자 default DB를 삭제하거나 덮지 않음

### 14.8 공통 quality gate

각 phase:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
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

`.githooks/pre-commit`, CI workflow와 `scripts/check-before-commit.sh`의 command
구성은 바꾸지 않는다.

## 15. Red/green과 commit 정책

green-only commit 정책을 사용한다.

```text
phase의 regression test 작성
→ 대상 test가 계획한 이유로 red인지 확인
→ 같은 phase에서 production 구현
→ 대상 test green
→ 전체 quality gate green
→ test와 구현을 함께 commit
```

권장 commit 경계:

```text
chore: update rust and bundled sqlite dependencies
test: characterize legacy index behavior
feat: add versioned canonical index schema
feat: rebuild canonical session index
feat: update canonical index incrementally
feat: migrate index options to selector views
docs: describe canonical incremental indexing
```

red 상태, partially migrated DB fixture 또는 generated benchmark data를
commit하지 않는다.

## 16. Package와 문서

### 16.1 Version과 release

package version은 `0.3.0`을 유지한다. 기능 2.5 완료만으로 publish하지 않는다.

다음을 실행하지 않는다.

```text
cargo publish
git tag
gh release create
```

2번, 2.5번과 3번 FTS5를 모두 완료하고 통합 검증을 통과한 뒤 별도 release
계획에서 공개한다.

### 16.2 README

다음을 갱신한다.

- 최소 Rust `1.97`
- bundled SQLite `3.53.2`
- canonical schema 전체
- default view와 stored superset 차이
- incremental refresh
- `--rebuild`
- root view include option
- index compatibility option migration
- fingerprint 제한과 forced rebuild 복구
- legacy DB automatic migration
- FTS가 아직 구현되지 않았다는 경계

### 16.3 CLI help

root/index help exact text를 test한다. replay help는 기능 2.5에서 변경하지
않는다.

## 17. 완료 조건

다음을 모두 만족해야 기능 2.5 완료다.

- canonical DB가 모든 정상 session class와 exec event를 저장한다.
- `sessions`와 `exec_events`가 항상 존재한다.
- `PRAGMA user_version = 1`이다.
- stable source/session/exec integer identity가 incremental update에서 유지된다.
- no-op refresh가 JSONL을 parse하지 않고 DB row를 쓰지 않는다.
- 신규/변경 file만 parse한다.
- 삭제 file의 source/session/exec가 한 transaction에서 제거된다.
- unstable file이 기존 DB state를 손상하지 않는다.
- hard scan/parse/SQL 오류가 partial update를 남기지 않는다.
- legacy DB가 source JSONL로 transactional rebuild된다.
- future schema를 덮어쓰지 않는다.
- unknown schema는 explicit rebuild 없이는 덮어쓰지 않는다.
- sessions root 변경이 automatic rebuild를 수행한다.
- `--rebuild`가 fingerprint를 무시한다.
- default selector row set이 기존과 동일하다.
- root include option이 canonical hidden row를 표시한다.
- root `--include-exec`는 DB 저장 범위를 바꾸지 않는다.
- index include option은 accepted compatibility no-op다.
- external recorder의 기존 argument 계약이 유지된다.
- summary가 exact counter를 출력한다.
- `IndexDelta`가 sorted unique stable key를 반환한다.
- FTS table, trigger, tokenizer와 search API가 없다.
- replay가 계속 JSONL을 직접 읽는다.
- package version은 `0.3.0`이다.
- Rust `1.97`, rusqlite `0.40.1`, bundled SQLite `3.53.2` baseline이다.
- unit/integration/manual test가 통과한다.
- benchmark gate를 통과하고 결과를 handoff에 기록한다.
- fmt, clippy, full test, release build와 `git diff --check`가 통과한다.
- publish, tag와 GitHub release를 만들지 않았다.

## 18. 후속 FTS5 경계

기능 3은 기능 2.5 완료 뒤 다음만 소비한다.

- `sessions.session_key`
- `exec_events.exec_key`
- `IndexDelta`
- `touched_session_keys`
- canonical rebuild mode

기능 3 계획이 소유할 항목:

- FTS search document와 indexed column
- tokenizer와 한글/path/URL/command 처리
- query syntax, escaping, AND/OR/prefix/phrase
- ranking과 tie-break
- Contentless-Delete DDL
- canonical transaction 안의 FTS apply 순서
- FTS mismatch detection과 repair
- query latency benchmark

기능 2.5의 `apply_incremental`은 transaction reference와 delta를 사용하므로
기능 3이 canonical mutation 뒤, commit 전에 FTS mutation을 추가할 수 있다.
그러나 기능 2.5 구현에서는 그 hook, callback, FTS state 또는 empty FTS
table을 만들지 않는다.
