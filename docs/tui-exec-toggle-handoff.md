# TUI Exec Visibility Toggle Implementation Handoff

## 목적

다음 세션에서
[`tui-exec-toggle-plan.md`](./tui-exec-toggle-plan.md)를 그대로 구현하기
위한 실행 handoff다.

상세 설계, exact 문구, 함수 시그니처, pseudocode, test assertion과 manual
smoke 절차의 source of truth는 계획 문서다. 이 handoff는 시작 상태,
절대 경계, 구현 순서와 자주 놓치기 쉬운 계약을 요약한다.

다음 세션은 production code를 변경하기 전에 계획 문서를 처음부터 끝까지
읽는다. 일부 section만 발췌해 구현하지 않는다.

## 다음 세션의 임무

selector와 replay TUI에 runtime exec visibility toggle을 구현한다.

최종 핵심 동작:

```text
selector normal e
  → 다음 replay의 exec visibility 변경

replay e
  → JSONL 재읽기 없이 현재 timeline의 exec entry 표시/숨김

internal replay 종료
  → 최종 visibility를 selector로 반환
```

`--include-exec`는 기존 CLI option으로 유지하며 TUI 초기 visibility를
결정한다. root selector에서 이 option이 최초 index refresh에도 전달되는
현재 계약은 유지한다.

## 문서 우선순위

구현 중 지시가 충돌하면 다음 순서로 판단한다.

1. 다음 세션에서 사용자가 새로 내린 지시
2. `docs/tui-exec-toggle-plan.md`
3. 이 handoff
4. `docs/implementation-plan-authoring-guidelines.md`
5. 현재 production code와 test가 보여주는 baseline
6. 현재 README의 사용자 설명
7. 완료된 one-binary refactor 계획과 handoff

계획과 handoff 사이에 충돌이 있으면 계획을 따른다. 계획과 현재 코드의
세부 배치가 다르면 기능 결정을 새로 만들지 말고 현재 실제 동작을 먼저
확인한 뒤 계획의 최종 계약을 만족하도록 적용한다.

## 시작 상태

handoff 작성 시점의 production 기준 commit은 다음이다.

```text
e0623e5 refactor: consolidate into one binary
```

package 상태:

```text
Cargo.toml package version: 0.3.0
Cargo.lock local package version: 0.3.0
binary target: select-codex-session 하나
Rust minimum: 1.85
```

작성 시점 production baseline:

```text
cargo fmt --check
  pass

cargo clippy --all-targets --all-features -- -D warnings
  pass

cargo test --all-targets --all-features
  36 library tests passed
  4 CLI integration tests passed
  0 failed

git diff --check
  pass
```

handoff 작성 과정에서 추가된 사용자 문서는 다음 두 개다.

```text
docs/tui-exec-toggle-plan.md
docs/tui-exec-toggle-handoff.md
```

다음 세션 시작 시 `git status --short`로 실제 상태를 다시 확인한다.
문서가 untracked이거나 다른 사용자 변경이 함께 있어도 reset, restore 또는
삭제하지 않는다.

## version과 공개 정책

이 정책은 구현 편의를 위해 변경할 수 없다.

- 2번 구현 전후 package version은 `0.3.0`이다.
- `Cargo.toml`과 `Cargo.lock`의 local package version을 올리지 않는다.
- `0.4.0` 또는 다른 intermediate version을 만들지 않는다.
- 2번 기능만 완료한 상태를 crates.io에 publish하지 않는다.
- 2번 기능만 완료한 상태로 release tag나 GitHub release를 만들지 않는다.
- 별도 canonical/증분 index 기능과 3번 FTS5까지 모두 완료하고 통합 검증을
  통과한 뒤 `0.3.0`으로 공개한다.
- `cargo build --release`와 임시 root의 `cargo install --path`는 local
  artifact 검증이며 공개 행위가 아니다.
- local install 검증에서 `select-codex-session --version`은 exact하게
  `select-codex-session 0.3.0`을 출력해야 한다.

이번 구현에서 `Cargo.toml` 또는 `Cargo.lock`이 바뀌면 dependency와 version
변경이 우발적으로 들어갔는지 확인한다. 계획된 package 변경은 없다.

## 구현 전 반드시 읽을 현재 파일

계획 전체를 읽은 뒤 다음 production/test 파일의 현재 내용을 확인한다.

```text
src/lib.rs
src/ui_state.rs                 # 구현 전에는 없음
src/application.rs
src/cli.rs
src/selector/mod.rs
src/replay/mod.rs
src/terminal.rs
tests/cli.rs
tests/fixtures/session.jsonl
README.md
Cargo.toml
Cargo.lock
scripts/check-before-commit.sh
```

`src/lib.rs`에서는 module 선언 위치만 변경 대상이다. session parser와
SQLite 함수는 읽어서 regression boundary를 확인하되 수정하지 않는다.

## 확정 결정

추가 질문 없이 다음 결정을 구현한다.

### Shared state

- `src/ui_state.rs`를 추가한다.
- `ExecVisibility` variant는 `Hidden`, `Shown` 두 개다.
- `from_include_exec`, `is_shown`, `toggle`, `label`을 계획 signature대로
  구현한다.
- `Default`는 구현하지 않는다.
- global state, interior mutability, channel 또는 mutex를 사용하지 않는다.

### Key binding

- modifier 없는 소문자 `e`만 toggle이다.
- selector normal mode에서는 toggle이다.
- selector search mode에서는 query 문자다.
- selector/replay help에서는 무시한다.
- replay timeline/detail fullscreen에서는 toggle이다.
- 대문자 `E`, `Ctrl-e`, `Alt-e`는 새 toggle binding이 아니다.

### Selector 의미

- selector의 `e`는 session row를 filter하지 않는다.
- 다음 replay에 전달할 visibility만 변경한다.
- query, scope, focus, selection과 scroll을 유지한다.
- internal replay의 final visibility를 받아 header를 갱신한다.
- replay가 오류로 끝나면 replay 진입 전 visibility를 유지한다.

### Replay 의미

- JSONL/JSON input은 한 번만 읽고 parse한다.
- supported User, Agent, Exec entry를 모두 `all_entries`에 저장한다.
- hidden/shown view는 `visible_indices: Vec<usize>`로 표현한다.
- toggle 때 entry를 clone하거나 input을 다시 parse하지 않는다.
- exec call과 matching output은 initial visibility와 관계없이 결합한다.
- unmatched output과 unsupported record 처리 규칙은 유지한다.

### Display index와 selection

- internal identity는 `all_entries` index다.
- 화면 `#NNNN`과 detail `index:`는 visible position을 사용한다.
- hidden/shown 각각 `#0000`부터 조밀하게 번호를 매긴다.
- 선택한 non-exec entry가 계속 visible이면 같은 entry를 유지한다.
- 숨긴 selected exec 뒤의 첫 visible entry를 우선 선택한다.
- 뒤에 없으면 앞의 마지막 visible entry를 선택한다.
- visible entry가 없으면 selection은 `None`이다.
- `None` 상태에서 entry가 다시 보이면 첫 entry를 선택한다.
- toggle은 detail scroll과 transient status만 reset하고 focus/fullscreen은
  유지한다.

### Help와 fullscreen

- help overlay는 modal이다.
- help 상태에서 `Esc`와 `?`는 help만 닫는다.
- help 상태에서 `q`와 `Ctrl-C`는 TUI를 종료한다.
- 그 외 key는 뒤 상태를 변경하지 않는다.
- selector help의 `Esc`가 selector를 종료하던 현재 동작은 의도적으로
  help-only close로 바꾼다.
- replay help를 fullscreen 위에서 닫아도 fullscreen은 유지한다.

### Internal/external orchestration

- `replay::run`은 `Result<ExecVisibility>`를 반환한다.
- standalone `replay` command는 final visibility를 버리고 정상 종료한다.
- internal selector replay는 final visibility를 selector에 반영한다.
- external replay에는 실행 직전 visibility로 `--include-exec` argument를
  정한다.
- external replay 성공 후 selector state는 실행 전과 동일하다.
- child와 visibility를 주고받는 stdout/file protocol을 추가하지 않는다.
- external recorder는 original CLI bool과 기존 argument order를 유지한다.

### UI 문구

다음 exact 상태 label을 사용한다.

```text
exec: hidden
exec: shown
```

header/footer/help/CLI help의 전체 exact 형식은 계획 5.10절을 그대로
사용한다. 표현을 줄이거나 동의어로 바꾸지 않는다.

## 절대 비범위

다음 기능이나 준비성 변경을 구현하지 않는다.

- `sessions` 또는 `exec_events` schema 변경
- canonical/superset index
- SQLite 전체 rebuild 제거
- 증분 index, fingerprint, upsert 또는 migration framework
- stable DB/FTS row identity
- subsession/empty-message TUI toggle
- selector exec count 표시
- exec command/output selector 검색
- FTS5, Contentless-Delete, tokenizer, ranking 또는 query 문법
- visibility disk/config/DB persistence
- 새 dependency 또는 Cargo feature
- Rust minimum version 변경
- package version 변경
- crates.io publish
- release tag 또는 GitHub release

특히 향후 기능을 쉽게 만든다는 이유로 schema column, metadata table,
fingerprint, row key나 FTS table을 미리 추가하지 않는다.

## 실행 순서

계획 10절의 phase를 순서대로 실행한다.

### 1. 시작 상태 재검증

```bash
git status --short
git diff --check
cargo fmt --check
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

baseline이 실패하면 기능 구현 전에 원인을 확인한다. 사용자 변경을
원복하여 green으로 만들지 않는다.

### 2. Phase 1: shared visibility type

계획 7.1절과 10절 Phase 1을 사용한다.

```text
src/ui_state.rs 추가
src/lib.rs module 선언
bool 변환/toggle/label test
```

target test:

```bash
cargo test ui_state::tests::
```

이 phase에서 selector, replay, CLI와 SQLite 동작은 바꾸지 않는다.

### 3. Phase 2: standalone replay dynamic filter

계획 5.6~5.8, 7.3, 8.3~8.6과 10절 Phase 2를 사용한다.

핵심 순서:

```text
behavior test red
  → Entry.index/title을 summary 모델로 전환
  → 모든 entry parse
  → visible_indices filter
  → selection 복원
  → replay e/help/fullscreen key 처리
  → replay::run final visibility 반환
  → existing call site compile 수정
  → replay tests green
```

target test:

```bash
cargo test replay::tests::
cargo test application::tests::
```

이 phase에서는 selector 왕복을 아직 구현하지 않는다. selector에서 호출한
replay final visibility는 계획대로 명시적으로 버리고 Phase 3에서 연결한다.

### 4. Phase 3: selector와 application 왕복

계획 5.2~5.5, 7.2, 7.4, 8.1~8.2, 8.7과 10절 Phase 3을 사용한다.

핵심 순서:

```text
selector behavior test red
  → SelectorApp state/constructor/getter/setter
  → pure key handler와 modal help
  → selector header/footer/help
  → current state로 replay 실행
  → internal final state 반영
  → external state 유지
  → selector/application tests green
```

target test:

```bash
cargo test selector::tests::
cargo test application::tests::
```

root refresh는 TUI 생성 전에 original `SelectOptions.include_exec`를 사용해야
한다. TUI에서 `e`를 눌렀다고 DB를 다시 만들지 않는다.

### 5. Phase 4: CLI, README와 package 검증

계획 5.10, 10절 Phase 4와 14절을 사용한다.

```text
CLI help integration test red
  → root/replay help 변경
  → README 변경
  → version 0.3.0 유지 확인
  → full quality gate
  → release build와 local install
  → manual TUI smoke
```

target test:

```bash
cargo test cli::tests::
cargo test --test cli
```

`Cargo.toml`과 `Cargo.lock`을 version 동기화 명목으로 수정하지 않는다.

## Red/green과 commit 규칙

각 phase에서 다음 순서를 지킨다.

```text
새 test 작성
  ↓
예상한 이유의 red 확인
  ↓
같은 phase에서 production 구현
  ↓
target test green
  ↓
scripts/check-before-commit.sh green
  ↓
git diff --check green
```

- red 상태를 commit하지 않는다.
- 기존 assertion을 약화하거나 test를 삭제하지 않는다.
- characterization test만 처음부터 green인 독립 commit이 될 수 있다.
- commit 생성은 다음 세션의 사용자 지시가 허용할 때만 한다.
- commit을 만들더라도 publish/tag/release 권한으로 해석하지 않는다.
- 계획의 권장 commit message를 사용할 수 있지만 history보다 green 상태가
  우선이다.

## 구현 중 주요 회귀 위험

### `load_entries_from_str`

기존 bool parameter를 제거하면 기존 replay test call site가 모두
compile error가 난다. test를 삭제하지 말고 새 all-entry parser 계약에
맞춰 고친다.

### Entry numbering

`Entry.index`를 그대로 canonical index로 재사용하면 hidden view에 번호
공백이 생긴다. 계획대로 `Entry.summary`와 render-time visible index를
분리한다.

### Exec output correlation

initial hidden에서도 exec tool call을 `exec_by_call_id`에 등록해야 matching
output이 보존된다. visibility 조건을 call 등록 앞에 두지 않는다.

### `replay::run` return type

return type 변경 후 다음 call site를 모두 확인한다.

```text
application::run의 Command::Replay branch
application::replay_selected
selector application loop
replay unit/manual entry points
```

standalone command만 return state를 버리고 selector internal replay는
반드시 반영한다.

### Help key priority

Ctrl-C 처리 뒤 help modal branch를 normal/search/fullscreen branch보다
앞에 둔다. 그렇지 않으면 `e`, movement, Tab 또는 fullscreen key가 overlay
뒤 상태를 바꾼다.

### Root refresh와 dynamic state

root `--include-exec` original bool은 refresh에 계속 사용한다. selector
toggle은 replay visibility만 바꾸며 DB schema를 바꾸지 않는다.

### External override

external recorder argument는 original CLI bool, external replay argument는
실행 직전 TUI state다. 두 bool의 시점이 다르므로 하나로 다시 합치지 않는다.

### User-owned 변경

working tree의 기존 변경과 새 변경을 구분한다. unrelated file을 format,
restore, stage 또는 commit하지 않는다.

## 자동 검증

각 phase:

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

기존 negative test에서 다음을 유지한다.

```text
default index: exec_events 없음
--include-exec index: exec_events 현재 row 수 유지
sessions_fts 없음
selector search scope에 exec 없음
```

## Manual smoke

계획 13.7절의 모든 단계를 실행한다. 요약만 보고 일부를 생략하지 않는다.

최소 확인 대상:

```text
direct replay hidden → shown → hidden
--include-exec direct replay initial shown
timeline/detail fullscreen에서 e
help modal에서 e/j/Tab/1 무시
exec-only empty selection 전이
selector search에서 e가 query 문자
selector e로 다음 replay initial state 변경
replay 변경 state가 selector로 복귀
selector query/scope/focus/selection 유지
```

TUI 자동 integration test로 대체할 수 없는 부분은 실제 pseudo-terminal에서
확인하고 결과를 구현 handoff 또는 PR 설명에 기록한다.

## Local package 검증

계획 14.4절의 제한된 임시 install root를 사용한다.

```bash
install_root="$(mktemp -d)"
cargo install --path . --root "$install_root"

test -x "$install_root/bin/select-codex-session"
test "$(find "$install_root/bin" -maxdepth 1 -type f | wc -l)" -eq 1
test "$("$install_root/bin/select-codex-session" --version)" = \
  "select-codex-session 0.3.0"

rm -rf "$install_root"
```

이 단계에서 `cargo publish`, `git tag`, `gh release create` 또는 이에
준하는 공개 작업을 실행하지 않는다.

## 완료 조건과 다음 handoff

계획 15절의 모든 항목을 충족해야 2번 기능 구현 완료다.

완료 보고에는 다음을 포함한다.

- 변경한 파일과 사용자 visible behavior
- phase별 red 원인과 최종 green test
- 전체 fmt/clippy/test/release build 결과
- manual TUI smoke 결과
- local install binary와 `0.3.0` version 확인
- SQLite/index/FTS가 변경되지 않았다는 negative test 결과
- publish/tag/release를 수행하지 않았다는 확인
- 남은 작업이 canonical/증분 index 계획과 3번 FTS5라는 경계

2번 완료 후 다음 구현에 바로 들어가지 않는다. 별도
`Canonical and Incremental SQLite Index Implementation Plan`을 먼저
작성·승인하고 구현한다. 그 뒤 3번 FTS5 계획을 실행한다.

최종 `0.3.0` 공개는 다음 세 조건을 모두 만족한 별도 release 단계의
책임이다.

```text
2번 TUI exec visibility toggle 완료
canonical/증분 index 완료
3번 FTS5 검색 강화 완료
```
