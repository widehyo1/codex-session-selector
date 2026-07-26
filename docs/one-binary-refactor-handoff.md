# One-Binary Refactor Handoff

## 목적

다음 세션에서
[`one-binary-refactor-plan.md`](./one-binary-refactor-plan.md)를 그대로
구현하기 위한 실행 handoff다. 상세 설계와 pseudocode의 source of truth는
계획 문서이며, 이 문서는 시작 상태, 실행 순서와 주의사항을 요약한다.

계획 문서를 수정하거나 새 설계를 시작하기 전에 반드시 전체를 읽는다.

## 다음 세션의 임무

기존 세 바이너리를 다음 단일 CLI로 통합한다.

```text
select-codex-session [SELECTOR_OPTIONS]
select-codex-session index [INDEX_OPTIONS]
select-codex-session replay [REPLAY_OPTIONS] [PATH|-]
```

최종 설치 파일은 `select-codex-session` 하나다.

## 문서 우선순위

충돌이 있으면 다음 순서로 판단한다.

1. 다음 세션에서 사용자가 새로 내린 지시
2. `one-binary-refactor-plan.md`
3. 이 handoff
4. 현재 README
5. 현재 구현의 세부 배치

계획 작성 방식이나 2번·3번의 후속 계획을 작성할 때는
[`implementation-plan-authoring-guidelines.md`](./implementation-plan-authoring-guidelines.md)를
사용한다.

## 현재 작업 트리

handoff 작성 시점의 상태:

```text
 M README.md
?? docs/
```

파일별 의미:

- `README.md`: 직전 작업에서 현재 3-binary 구현 기준으로 `--include-exec`,
  recorder/replay 사용법 및 SQLite schema를 최신화했다.
- `docs/one-binary-refactor-plan.md`: 구현 준비가 끝난 상세 계획이다.
- `docs/implementation-plan-authoring-guidelines.md`: 향후 계획 작성 기준이다.
- `docs/one-binary-refactor-handoff.md`: 이 문서다.

현재 production Rust source, `Cargo.toml`, `Cargo.lock`,
`install-bundle.sh`에는 one-binary refactor가 적용되지 않았다.
quality script, tracked pre-commit hook와 GitHub Actions workflow도 아직
생성되지 않았고 local `core.hooksPath`도 설정되지 않았다.

기존 변경은 사용자 요청으로 만들어진 작업이므로 보존한다. 특히 README를
원복하지 말고, 구현 마지막 단계에서 현재 내용을 단일 바이너리 CLI에 맞게
재작성한다.

## Baseline

마지막 production-code 검증 결과:

```text
cargo fmt --check
  pass

cargo test --all-targets --all-features
  21 passed, 0 failed

cargo clippy --all-targets --all-features -- -D warnings
  fail: CollectOptions의 clippy::derivable_impls
```

계획과 handoff는 Markdown만 추가했으므로 production baseline은 동일하다.
clippy 실패는 알려진 bootstrap 항목이며 one-binary 구현 결함이 아니다.
다음 세션 시작 시 fmt와 test baseline을 다시 확인한다.

```bash
git status --short
git diff --check
cargo fmt --check
cargo test --all-targets --all-features
```

fmt 또는 test가 실패하면 refactor를 시작하기 전에 원인을 확인한다. 기존
사용자 변경을 reset하거나 restore하지 않는다. clippy는 계획 13.2절의
`#[derive(Default)]` 변경 후 반드시 green이어야 한다.

## 확정 결정

추가 질문 없이 다음 결정을 따른다.

- package version은 `0.3.0`으로 올린다.
- `select-codex-session`만 Cargo binary target으로 남긴다.
- subcommand가 없으면 현재 selector를 실행한다.
- `index`는 기존 `record-codex-session-info` 기능을 제공한다.
- `replay`는 기존 `codex-replay-tui` 기능을 제공한다.
- 기본 refresh는 외부 process가 아니라 내부 indexer 함수를 호출한다.
- 기본 replay는 외부 process가 아니라 내부 replay 함수를 호출한다.
- `--record-command`, `--replay-command`는 optional compatibility override로
  유지한다.
- 외부 override를 지정했을 때의 argument 순서와 exit-status 처리를
  유지한다.
- selector의 선택, query, scope와 focus는 replay 복귀 후 유지한다.
- 기존 `sessions`, `exec_events` schema와 전체 rebuild 방식을 유지한다.
- 현재 메모리 기반 부분 문자열 검색을 유지한다.
- 기존 standalone binary는 최종 Cargo package에서 제거한다.
- 설치 스크립트는 기존 설치 위치의 standalone binary를 자동 삭제하지
  않는다.
- 공통 shell script를 fmt/clippy/test quality gate의 single source로 둔다.
- tracked pre-commit hook와 GitHub Actions가 같은 script를 호출한다.
- 모든 phase는 green 상태에서만 commit한다.
- 새 behavior test의 red는 commit하지 않고 같은 phase 안에서 green으로
  전환한다.

## 절대 비범위

다음 기능을 구현하거나 준비 명목으로 schema/state를 추가하지 않는다.

- `--include-exec` TUI toggle key
- 실행 중 `include_exec` 변경
- FTS5
- Contentless-Delete
- search ranking 또는 query 문법 변경
- exec command/output 검색
- 증분 index
- DB migration framework
- SQLite dependency/feature 변경

완료 후에도 2번과 3번은 defer 상태다.

## 실행 순서

상세 코드는 계획 문서의 해당 절을 사용한다.

### 1. 계획 전체 확인

```text
2. 범위
3. 호환성 결정
4~12. 구조와 pseudocode
13. quality gate와 red/green 정책
14. 구현 순서
15~17. 테스트와 package 검증
19. 완료 조건
```

계획과 현재 코드가 충돌하면 정상 동작을 바꾸지 않는 범위에서 현재 코드의
실제 동작을 우선 확인한다. 기능 결정을 새로 만들지 않는다.

### 2. Quality automation bootstrap

계획 13절을 먼저 구현한다.

- `CollectOptions`를 `#[derive(Default)]`로 전환하고 동등성 test 추가
- `scripts/check-before-commit.sh`
- `.githooks/pre-commit`
- `scripts/install-git-hooks.sh`
- `.github/workflows/ci.yml`
- executable mode 설정
- shell syntax 검사
- local hook 설치

실행:

```bash
bash -n \
  scripts/check-before-commit.sh \
  scripts/install-git-hooks.sh \
  .githooks/pre-commit

scripts/install-git-hooks.sh
scripts/check-before-commit.sh
```

hook installer는 local `.git/config`를 변경한다. 실행 환경이 `.git` 쓰기를
제한하면 승인을 요청하여 installer를 실행하고, hook 설치를 생략하거나
global git config로 우회하지 않는다.

전체 검사가 green인 상태에서만 bootstrap commit을 만든다. 이후 모든
commit은 hook를 통과해야 한다.

### 3. Baseline test 보강

제품 코드를 이동하기 전에 계획 15절의 누락 test를 현재 파일 위치에
추가한다.

게이트:

```bash
scripts/check-before-commit.sh
```

### 4. Indexer 추출

- `src/indexer.rs` 생성
- 기존 recorder 본문을 `build_index`와 `format_summary`로 이동
- standalone recorder는 임시 thin wrapper로 유지

게이트:

```bash
scripts/check-before-commit.sh
cargo run --quiet --bin record-codex-session-info -- --help
```

### 5. Replay 추출

- `src/replay/mod.rs` 생성
- parser, model, TUI와 test를 동작 변경 없이 이동
- standalone replay는 임시 thin wrapper로 유지

게이트:

```bash
scripts/check-before-commit.sh
cargo run --quiet --bin codex-replay-tui -- --help
```

### 6. Selector 추출

- `src/selector/mod.rs` 생성
- state, render, event loop와 test 이동
- terminal lifecycle을 `src/terminal.rs`로 통합

게이트:

```bash
scripts/check-before-commit.sh
```

가능하면 fixture HOME에서 selector/replay 왕복을 수동 확인한다.

### 7. 통합 CLI와 application 구현

- `src/cli.rs`
- `src/application.rs`
- `src/main.rs`
- `src/lib.rs::run_from_args`

기본 경로는 내부 indexer/replay를 사용하고 external override만
`std::process::Command`를 사용한다.

게이트:

```bash
scripts/check-before-commit.sh
cargo run --quiet --bin select-codex-session -- --help
cargo run --quiet --bin select-codex-session -- index --help
cargo run --quiet --bin select-codex-session -- replay --help
```

### 8. 단일 binary 전환

- `autobins = false`
- 단일 `[[bin]]` target
- 기존 `src/bin` entrypoint 제거
- version 및 lockfile 갱신
- install script 단일 파일 설치

`src/bin/*.rs`는 Cargo가 자동으로 binary로 발견할 수 있다. 파일을
남기면서 `[[bin]]` 항목만 제거하는 것으로는 충분하지 않으므로 계획대로
`autobins = false`와 entrypoint 제거를 모두 적용한다.

### 9. README와 migration 문서 갱신

현재 README 변경을 보존하면서 명령 예시를 다음처럼 전환한다.

```text
record-codex-session-info → select-codex-session index
codex-replay-tui          → select-codex-session replay
```

현재 `--include-exec` 설명의 기능 의미는 유지한다.

### 10. 최종 검증

```bash
scripts/check-before-commit.sh
cargo build --release
git diff --check
```

계획 16절의 manual smoke test와 17절의 임시 설치 검증도 실행한다.

최종 Cargo metadata의 binary 목록:

```json
["select-codex-session"]
```

## 구현 시 주의사항

### Commit과 red/green

- characterization test는 기존 코드에서 green임을 먼저 확인한다.
- 새 behavior test는 해당 test만 실행하여 요구사항 때문에 red인지 확인한다.
- compile error나 fixture 오류는 유효한 red로 간주하지 않는다.
- red test만 별도 commit하지 않는다.
- 같은 phase에서 구현 후 targeted test를 green으로 만든다.
- `scripts/check-before-commit.sh`가 green인 경우에만 commit한다.
- `--no-verify`로 검사 실패를 우회하여 commit하지 않는다.
- phase 의미는 test 이름과 commit message에 기록한다.

### Terminal

- selector terminal을 restore한 뒤 replay terminal을 init한다.
- replay 종료 후 terminal을 restore하고 selector를 다시 init한다.
- 내부 replay 오류가 발생해도 selector loop는 종료하지 않는다.
- `--print-path`는 replay를 호출하지 않는다.

### 상태

- `include_exec`는 `SelectOptions`와 `ReplayOptions`의 시작 옵션으로만 둔다.
- `SelectorApp`에 toggle state를 추가하지 않는다.
- selector `App` 자체는 replay 왕복 동안 재생성하지 않는다.

### SQLite와 검색

- indexer는 기존 `collect_session_data`와
  `recreate_database_with_exec`를 그대로 사용한다.
- `load_sessions`와 `filter_sessions_by_scope`의 결과를 바꾸지 않는다.
- FTS table이나 migration metadata table을 만들지 않는다.
- `--include-exec`가 없을 때 `exec_events`를 제거하는 현재 동작을 유지한다.

### 외부 compatibility override

- 기본값은 `None`이며 내부 구현을 뜻한다.
- 명시적으로 지정한 경우에만 외부 program을 실행한다.
- command 문자열을 셸로 해석하지 않는다.
- external recorder DB path 변환은 현재처럼 `to_string_lossy()`를 사용한다.
- external replay path는 `OsString`으로 전달한다.

### 기존 변경 보호

- `git reset --hard`, broad restore 또는 checkout으로 현재 변경을 지우지 않는다.
- README와 docs를 새로 생성해 덮어쓰기보다 현재 내용을 기준으로 수정한다.
- 관련 없는 사용자 변경이 추가로 발견되면 보존하고 작업 범위를 좁힌다.

## 완료 보고 형식

다음 세션의 최종 보고에는 최소한 다음을 포함한다.

- 단일 binary와 새 subcommand 구현 결과
- 제거된 standalone binary
- 호환 유지 사항
- 변경한 주요 파일
- 실행한 자동 test와 개수
- pre-commit hook 설치와 공통 quality gate 결과
- clippy `-D warnings` 결과
- red 확인 후 green으로 전환한 behavior test
- manual TUI smoke test 결과
- 임시 설치에서 확인한 binary 목록
- 2번과 3번을 구현하지 않았다는 확인
- 남은 위험이나 미검증 항목

## 다음 세션에 전달할 구현 지시

아래 문장을 구현 요청과 함께 사용할 수 있다.

```text
docs/one-binary-refactor-handoff.md와
docs/one-binary-refactor-plan.md를 순서대로 모두 읽고,
현재 사용자 변경을 보존하면서 계획 전체를 구현하고 검증하라.
2번 TUI toggle과 3번 FTS 검색 강화는 구현하지 말라.
```
