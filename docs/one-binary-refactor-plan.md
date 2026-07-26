# One-Binary Refactor Implementation Plan

## 1. 문서 상태

- 상태: 구현 준비 완료
- 대상 버전: `0.3.0`
- 최종 설치 바이너리: `select-codex-session`
- 구현 범위: 기존 세 바이너리의 기능을 단일 바이너리와 서브커맨드로 통합
- 기능 보존 기준: 현재 `main` 바이너리인 `select-codex-session`의 정상 동작과 기존 SQLite 스키마 및 검색 결과를 유지

관련 문서:

- [계획 작성 지침](./implementation-plan-authoring-guidelines.md)
- [다음 세션 구현 handoff](./one-binary-refactor-handoff.md)

이 문서의 코드 블록은 구현자가 그대로 Rust 코드로 옮길 수 있도록 작성한
pseudocode다. 타입과 함수 이름은 특별한 사유가 없다면 그대로 사용한다.

## 2. 범위

### 2.1 구현 대상

다음 CLI 표면을 구현한다.

```text
select-codex-session [SELECTOR_OPTIONS]
select-codex-session index [INDEX_OPTIONS]
select-codex-session replay [REPLAY_OPTIONS] [PATH|-]
```

동작은 다음과 같다.

```text
서브커맨드 없음
  → 현재 selector를 실행
  → 기본적으로 내부 indexer로 DB를 갱신
  → 선택한 세션을 내부 replay 모듈로 재생

index
  → 기존 record-codex-session-info와 동일하게 DB를 생성

replay
  → 기존 codex-replay-tui와 동일하게 JSONL/JSON을 재생
```

다음 호환 옵션은 이번 변경에서 유지한다.

```text
--record-command COMMAND
--replay-command COMMAND
```

- 옵션을 지정하지 않으면 단일 바이너리 내부 구현을 호출한다.
- 옵션을 지정하면 현재와 동일한 인자 규약으로 외부 명령을 실행한다.
- 외부 명령은 executable 경로 하나로 취급한다. 공백으로 명령과 인자를
  분리하거나 셸을 경유하지 않는다.

### 2.2 명시적 비범위

다음은 구현하지 않는다.

- TUI에서 `--include-exec`를 변경하는 토글 키
- selector 또는 replay 실행 중 `include_exec` 상태 변경
- exec 포함 여부를 selector와 replay 사이에서 가변 상태로 공유하는 기능
- FTS5
- FTS5 Contentless-Delete 테이블
- SQLite schema version 또는 migration framework
- 증분 인덱싱
- 파일 변경 감지
- 검색 ranking, prefix, phrase, trigram 검색
- exec command/output을 대상으로 한 selector 검색
- 현재 부분 문자열 검색 알고리즘 변경
- `sessions` 또는 `exec_events` 스키마 변경

이번 변경에서 사용하는 SQLite 기능은 현재와 동일하다.

```text
일반 table
일반 index
transaction
DROP TABLE / CREATE TABLE
INSERT
SELECT ... ORDER BY
```

`rusqlite`의 `bundled` feature와 dependency version도 변경하지 않는다.
FTS5 및 Contentless-Delete 설계와 구현은 별도 계획으로 작성한다.

## 3. 고정된 호환성 결정

### 3.1 유지되는 기본 경로

```text
sessions root: ~/.codex/sessions
SQLite DB:     ~/codex-session-info.sqlite3
```

### 3.2 유지되는 selector 동작

- 실행 전에 DB를 갱신한다.
- `--no-refresh`를 지정하면 갱신하지 않는다.
- DB에서 세션을 timestamp 내림차순으로 읽는다.
- 세션이 없으면 TUI를 열지 않고 오류를 반환한다.
- `Enter`로 replay를 열고 replay 종료 후 기존 selector 상태로 돌아온다.
- selector 복귀 시 선택, query, search scope, pane focus를 유지한다.
- `--print-path`이면 선택한 경로를 stdout에 출력하고 replay를 실행하지 않는다.
- `--include-exec`이면 refresh와 replay 시작 양쪽에 현재와 동일하게 적용한다.

### 3.3 유지되는 index 동작

- JSONL 파일을 전체 스캔한다.
- 기본적으로 subsession과 빈 first message를 제외한다.
- `sessions`를 drop/create한 뒤 전체 데이터를 다시 삽입한다.
- `--include-exec`이면 `exec_events`를 drop/create하고 채운다.
- `--include-exec`이 없으면 `exec_events`를 제거한다.
- 현재 stdout summary 문구를 유지한다.

### 3.4 유지되는 replay 동작

- 경로 입력, `-`, 인자 없는 stdin 입력을 모두 지원한다.
- raw JSONL과 JSON array를 지원한다.
- 기존 user, assistant, legacy exec 및 새 exec tool-call 정규화를 유지한다.
- `--include-exec` 기본값은 `false`다.
- 현재 키 바인딩, pane focus, fullscreen, clipboard 동작을 유지한다.

### 3.5 의도적으로 변경되는 CLI

다음 standalone executable은 최종 산출물에서 제거한다.

```text
record-codex-session-info
codex-replay-tui
```

대체 명령은 다음과 같다.

```text
record-codex-session-info ARGS
  → select-codex-session index ARGS

codex-replay-tui ARGS
  → select-codex-session replay ARGS
```

기존 DB 파일은 migration 없이 그대로 읽을 수 있어야 한다.

## 4. 목표 파일 구조

기존 `src/lib.rs`의 session parsing, filtering 및 SQLite 함수는 이번
변경에서 분할하지 않는다. 불필요한 대규모 이동을 피하고 binary entrypoint
코드만 애플리케이션 모듈로 옮긴다.

```text
src/
  main.rs
  lib.rs
  application.rs
  cli.rs
  indexer.rs
  terminal.rs
  selector/
    mod.rs
  replay/
    mod.rs
  test_support.rs       # #[cfg(test)]에서만 compile

docs/
  one-binary-refactor-plan.md

scripts/
  check-before-commit.sh
  install-git-hooks.sh

.githooks/
  pre-commit

.github/
  workflows/
    ci.yml
```

파일별 책임은 다음과 같다.

| 파일 | 책임 |
|---|---|
| `src/main.rs` | OS argument를 받아 library entrypoint 호출 |
| `src/lib.rs` | 기존 core 함수 유지, 새 모듈 선언, `run_from_args` 제공 |
| `src/cli.rs` | root/index/replay 인자 파싱 및 help 문자열 |
| `src/application.rs` | command dispatch와 selector-index-replay orchestration |
| `src/indexer.rs` | 기존 recorder의 use-case와 summary 생성 |
| `src/terminal.rs` | ratatui init/restore의 단일 안전 경계 |
| `src/selector/mod.rs` | 기존 selector state, render, event loop |
| `src/replay/mod.rs` | 기존 replay parser, state, render, event loop |
| `src/test_support.rs` | 공통 임시 session/SQLite test fixture |
| `scripts/check-before-commit.sh` | local hook와 CI가 공유하는 전체 quality gate |
| `scripts/install-git-hooks.sh` | repo-local `core.hooksPath` 안전 설정 |
| `.githooks/pre-commit` | commit 전에 공통 검사 script 실행 |
| `.github/workflows/ci.yml` | push/PR에서 같은 공통 검사 script 실행 |

기존 파일은 최종 단계에서 제거한다.

```text
src/bin/select-codex-session.rs
src/bin/record-codex-session-info.rs
src/bin/codex-replay-tui.rs
```

## 5. Cargo 및 설치 구조

`Cargo.toml`은 자동 binary 탐색을 끄고 단일 target만 명시한다.

```toml
[package]
name = "codex-session-selector"
version = "0.3.0"
autobins = false

[[bin]]
name = "select-codex-session"
path = "src/main.rs"
```

기존 세 개의 `[[bin]]` 항목은 제거한다.

`install-bundle.sh`는 다음 한 파일만 설치한다.

```bash
cargo build --release --manifest-path "$selector_dir/Cargo.toml"
install -m 755 \
  "$selector_dir/target/release/select-codex-session" \
  "$bin_dir/select-codex-session"
```

설치 스크립트는 기존 standalone binary를 자동 삭제하지 않는다. 삭제는
복구가 어려운 동작이며, 다른 설치 경로의 파일 소유권을 판단할 수 없기
때문이다. README의 upgrade note에서 수동 정리 대상만 안내한다.

## 6. 공통 타입과 함수 시그니처

### 6.1 Library entrypoint

`src/lib.rs`에 다음 모듈과 entrypoint를 추가한다.

```rust
mod application;
mod cli;
mod indexer;
mod replay;
mod selector;
mod terminal;

#[cfg(test)]
mod test_support;

pub fn run_from_args<I>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = String>,
{
    let action = cli::parse_args(args.into_iter())?;

    match action {
        cli::CliAction::Run(command) => application::run(command),
        cli::CliAction::PrintHelp(topic) => {
            println!("{}", cli::help_text(topic));
            Ok(())
        }
        cli::CliAction::PrintVersion => {
            println!(
                "select-codex-session {}",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
    }
}
```

`src/main.rs`는 로직을 갖지 않는다.

```rust
fn main() -> anyhow::Result<()> {
    codex_session_selector::run_from_args(std::env::args().skip(1))
}
```

인자 파서 내부에서 `std::process::exit`를 호출하지 않는다. help/version도
`CliAction`으로 반환하여 unit test가 프로세스를 종료하지 않고 검증할 수
있게 한다.

### 6.2 CLI command 타입

`src/cli.rs`에 다음 타입을 둔다.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
    Run(Command),
    PrintHelp(HelpTopic),
    PrintVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpTopic {
    Root,
    Index,
    Replay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Select(SelectOptions),
    Index(IndexOptions),
    Replay(ReplayOptions),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectOptions {
    pub db: PathBuf,
    pub refresh: bool,
    pub print_path: bool,
    pub include_exec: bool,
    // None이면 내부 indexer를 호출한다.
    pub record_command: Option<String>,
    // None이면 내부 replay를 호출한다.
    pub replay_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexOptions {
    pub output: PathBuf,
    pub sessions_root: PathBuf,
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
    pub include_exec: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayOptions {
    // None 또는 Some("-")이면 stdin을 읽는다.
    pub input: Option<PathBuf>,
    pub include_exec: bool,
}
```

기본값 생성 중 `HOME`이 없을 수 있으므로 `Default` trait 대신 fallible
constructor를 사용한다.

```rust
impl SelectOptions {
    pub(crate) fn defaults() -> anyhow::Result<Self> {
        Ok(Self {
            db: home_dir()?.join("codex-session-info.sqlite3"),
            refresh: true,
            print_path: false,
            include_exec: false,
            record_command: None,
            replay_command: None,
        })
    }
}

impl IndexOptions {
    pub(crate) fn defaults() -> anyhow::Result<Self> {
        let home = home_dir()?;
        Ok(Self {
            output: home.join("codex-session-info.sqlite3"),
            sessions_root: home.join(".codex").join("sessions"),
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        })
    }
}

impl ReplayOptions {
    pub(crate) fn defaults() -> Self {
        Self {
            input: None,
            include_exec: false,
        }
    }
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

fn expand_home(value: &str) -> anyhow::Result<PathBuf> {
    if value == "~" {
        return home_dir();
    }

    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }

    Ok(PathBuf::from(value))
}
```

## 7. CLI 파싱 명세

### 7.1 Root dispatch 알고리즘

첫 번째 인자만 subcommand 판별에 사용한다. root option 뒤에 subcommand를
두는 형식은 지원하지 않는다.

```text
지원:
  select-codex-session index --include-exec
  select-codex-session replay --include-exec file.jsonl

지원하지 않음:
  select-codex-session --include-exec replay file.jsonl
```

구현은 다음 순서를 따른다.

```rust
pub(crate) fn parse_args(
    mut args: impl Iterator<Item = String>,
) -> anyhow::Result<CliAction> {
    let Some(first) = args.next() else {
        return Ok(CliAction::Run(Command::Select(
            SelectOptions::defaults()?
        )));
    };

    match first.as_str() {
        "index" => parse_index_args(args),
        "replay" => parse_replay_args(args),
        "-h" | "--help" => {
            Ok(CliAction::PrintHelp(HelpTopic::Root))
        }
        "-V" | "--version" => Ok(CliAction::PrintVersion),
        _ => parse_select_args(std::iter::once(first).chain(args)),
    }
}
```

현재 parser처럼 help/version을 만나면 뒤의 인자를 소비하지 않고 즉시
해당 action을 반환한다. 값을 받는 option은 다음 공통 함수로 검사한다.

```rust
fn required_value(
    args: &mut impl Iterator<Item = String>,
    option: &str,
    noun: &str,
) -> anyhow::Result<String> {
    args.next()
        .ok_or_else(|| anyhow::anyhow!(
            "{option} requires a {noun}"
        ))
}
```

경로 option도 빈 문자열을 별도로 금지하지 않는다. 현재와 동일하게 빈
문자열은 `PathBuf::from("")`으로 처리되고 실제 파일 작업에서 오류가
발생한다.

### 7.2 Selector 옵션 파싱

```rust
fn parse_select_args(
    args: impl Iterator<Item = String>,
) -> anyhow::Result<CliAction> {
    let mut options = SelectOptions::defaults()?;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => {
                options.db = expand_home(&required_value(
                    &mut args,
                    &arg,
                    "path",
                )?)?;
            }
            "--record-command" => {
                options.record_command = Some(required_value(
                    &mut args,
                    &arg,
                    "command",
                )?);
            }
            "--replay-command" => {
                options.replay_command = Some(required_value(
                    &mut args,
                    &arg,
                    "command",
                )?);
            }
            "--no-refresh" => options.refresh = false,
            "--print-path" => options.print_path = true,
            "--include-exec" => options.include_exec = true,
            "-h" | "--help" => {
                return Ok(CliAction::PrintHelp(HelpTopic::Root));
            }
            "-V" | "--version" => {
                return Ok(CliAction::PrintVersion);
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }

    Ok(CliAction::Run(Command::Select(options)))
}
```

### 7.3 Index 옵션 파싱

```rust
fn parse_index_args(
    mut args: impl Iterator<Item = String>,
) -> anyhow::Result<CliAction> {
    let mut options = IndexOptions::defaults()?;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                options.output = expand_home(&required_value(
                    &mut args,
                    &arg,
                    "path",
                )?)?;
            }
            "--sessions-root" => {
                options.sessions_root = expand_home(&required_value(
                    &mut args,
                    &arg,
                    "path",
                )?)?;
            }
            "--include-subsessions" => {
                options.include_subsessions = true;
            }
            "--include-empty-messages" => {
                options.include_empty_messages = true;
            }
            "--include-exec" => options.include_exec = true,
            "-h" | "--help" => {
                return Ok(CliAction::PrintHelp(HelpTopic::Index));
            }
            "-V" | "--version" => {
                return Ok(CliAction::PrintVersion);
            }
            _ => anyhow::bail!("unknown index argument: {arg}"),
        }
    }

    Ok(CliAction::Run(Command::Index(options)))
}
```

### 7.4 Replay 옵션 파싱

현재와 동일하게 input은 최대 하나만 허용하고 `-`만 option처럼 생긴
특수 경로로 인정한다.

```rust
fn parse_replay_args(
    mut args: impl Iterator<Item = String>,
) -> anyhow::Result<CliAction> {
    let mut options = ReplayOptions::defaults();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--include-exec" => options.include_exec = true,
            "-h" | "--help" => {
                return Ok(CliAction::PrintHelp(HelpTopic::Replay));
            }
            "-V" | "--version" => {
                return Ok(CliAction::PrintVersion);
            }
            "-" => set_replay_input(&mut options, PathBuf::from("-"))?,
            _ if arg.starts_with('-') => {
                anyhow::bail!("unknown replay argument: {arg}");
            }
            _ => set_replay_input(&mut options, PathBuf::from(arg))?,
        }
    }

    Ok(CliAction::Run(Command::Replay(options)))
}

fn set_replay_input(
    options: &mut ReplayOptions,
    input: PathBuf,
) -> anyhow::Result<()> {
    if options.input.replace(input).is_some() {
        anyhow::bail!("only one input path may be provided");
    }
    Ok(())
}
```

### 7.5 Help 출력

root help에는 selector options와 subcommands를 함께 표시한다.

```text
Usage:
  select-codex-session [SELECTOR_OPTIONS]
  select-codex-session index [INDEX_OPTIONS]
  select-codex-session replay [REPLAY_OPTIONS] [PATH|-]

Commands:
  index    Build the SQLite session index
  replay   Replay one JSONL/JSON session
```

`help_text`는 문자열을 반환하여 snapshot 성격의 exact test가 가능하게 한다.

```rust
pub(crate) fn help_text(topic: HelpTopic) -> String {
    match topic {
        HelpTopic::Root => root_help(),
        HelpTopic::Index => index_help(),
        HelpTopic::Replay => replay_help(),
    }
}
```

도움말 함수 시그니처는 다음으로 고정한다.

```rust
fn root_help() -> String;
fn index_help() -> String;
fn replay_help() -> String;
```

세 문자열은 끝에 newline을 포함하지 않는다. 출력하는 쪽에서
`println!`을 한 번 사용한다. `index --version`과 `replay --version`도
standalone helper 이름이 아니라 통합 바이너리 버전을 출력한다.

각 함수는 아래 text의 `{version}`을 `env!("CARGO_PKG_VERSION")`으로
치환한 문자열을 반환한다.

Root help:

```text
select-codex-session {version}

Open a TUI for choosing a local Codex session and replaying its JSONL timeline.

Usage:
  select-codex-session [SELECTOR_OPTIONS]
  select-codex-session index [INDEX_OPTIONS]
  select-codex-session replay [REPLAY_OPTIONS] [PATH|-]

Commands:
  index                          Build the SQLite session index
  replay                         Replay one JSONL/JSON session

Selector options:
      --db PATH                  SQLite index path
                                 default: ~/codex-session-info.sqlite3
      --record-command COMMAND   Optional external recorder override
                                 default: internal indexer
      --replay-command COMMAND   Optional external replay override
                                 default: internal replay
      --no-refresh               Do not rebuild the SQLite index before opening
      --include-exec             Index and show command execution records
      --print-path               Print the selected JSONL path instead of replaying
  -h, --help                     Show this help
  -V, --version                  Show version

Keys:
  Enter                          replay selected session, then return here
  /                              interactive search
  Tab                            switch pane focus; while searching, cycle scope
  h/l or Left/Right              horizontal scroll in sessions pane
  y                              copy `codex resume <session-id>` to clipboard
  q, Esc, Ctrl-C                 quit
```

Index help:

```text
select-codex-session {version}

Build a SQLite index from local Codex session JSONL files.

Usage:
  select-codex-session index [INDEX_OPTIONS]

Options:
  -o, --output PATH              SQLite output path
                                 default: ~/codex-session-info.sqlite3
      --sessions-root PATH       Codex sessions root
                                 default: ~/.codex/sessions
      --include-subsessions      Include subagent/subsession records
      --include-empty-messages   Include sessions without a first user message
      --include-exec             Also create and populate exec_events
                                 default: disabled
  -h, --help                     Show this help
  -V, --version                  Show version

Default schema:
  sessions(path, id, timestamp, cwd, repository_url, branch, first_message)

With --include-exec:
  exec_events(session_path, session_id, event_index, call_id, kind, name, command, output)
```

Replay help:

```text
select-codex-session {version}

Replay a Codex session JSONL file in a terminal UI.

Usage:
  select-codex-session replay [REPLAY_OPTIONS] [PATH|-]

Options:
      --include-exec     Show command execution records in the timeline
                         default: hidden
  -h, --help             Show this help
  -V, --version          Show version

Input:
  PATH    Raw Codex JSONL file or a JSON array of preprocessed events
  -       Read JSONL/JSON from stdin
  omitted Read JSONL/JSON from stdin

Keys:
  Tab                switch focus between timeline/detail
  j/k or Up/Down     move or scroll, depending on focus
  d/u or Page keys   page move or page scroll
  g/G                first/last event or detail top/bottom
  1/2/f              fullscreen controls
  y                  copy detail pane to clipboard
  q, Esc, Ctrl-C     quit
```

## 8. Indexer 모듈

### 8.1 타입과 함수

`src/indexer.rs`는 stdout이나 프로세스 종료를 직접 제어하지 않는다.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexSummary {
    pub session_rows: usize,
    pub exec_rows: usize,
    pub total_files: usize,
    pub skipped: usize,
    pub output: PathBuf,
    pub exec_enabled: bool,
}

pub(crate) fn build_index(
    options: &IndexOptions,
) -> anyhow::Result<IndexSummary> {
    let collect_options = CollectOptions {
        include_subsessions: options.include_subsessions,
        include_empty_messages: options.include_empty_messages,
        include_exec: options.include_exec,
    };

    let data = collect_session_data(
        &options.sessions_root,
        collect_options,
    )?;

    recreate_database_with_exec(
        &options.output,
        &data.rows,
        options
            .include_exec
            .then_some(data.exec_events.as_slice()),
    )?;

    Ok(IndexSummary {
        session_rows: data.rows.len(),
        exec_rows: data.exec_events.len(),
        total_files: data.total_files,
        skipped: data.skipped,
        output: options.output.clone(),
        exec_enabled: options.include_exec,
    })
}

pub(crate) fn format_summary(summary: &IndexSummary) -> String {
    if summary.exec_enabled {
        format!(
            "wrote {} session rows and {} exec rows to {} \
             from {} jsonl files; skipped {} filtered or invalid sessions",
            summary.session_rows,
            summary.exec_rows,
            summary.output.display(),
            summary.total_files,
            summary.skipped,
        )
    } else {
        format!(
            "wrote {} session rows to {} from {} jsonl files; \
             skipped {} filtered or invalid sessions; \
             exec indexing disabled",
            summary.session_rows,
            summary.output.display(),
            summary.total_files,
            summary.skipped,
        )
    }
}
```

`application`이 `format_summary` 결과를 한 번만 출력한다. selector의 자동
refresh와 직접 `index` 서브커맨드 모두 같은 함수를 사용한다.

## 9. Terminal lifecycle

selector와 replay가 같은 프로세스에서 순차 실행되므로 terminal 초기화와
복원을 공통 함수로 감싼다.

```rust
pub(crate) fn with_terminal<T>(
    run: impl FnOnce(
        &mut ratatui::DefaultTerminal,
    ) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);

    ratatui::restore();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::cursor::Show
    );

    result
}
```

규칙은 다음과 같다.

- selector 화면을 떠날 때 항상 `with_terminal`이 terminal을 복원한다.
- JSONL 파일 read/parse는 replay terminal 초기화 전에 수행한다.
- selector terminal이 복원된 뒤 replay terminal을 새로 초기화한다.
- replay가 끝나면 replay terminal을 복원한 뒤 selector를 새로 초기화한다.
- selector의 `App` 값은 terminal 재초기화와 무관하게 application loop에
  계속 보관한다.

panic 시 terminal 복원까지 보장하는 RAII guard 도입은 이번 범위가 아니다.
현재 정상/`Result::Err` 경로의 복원만 유지한다.

## 10. Selector 모듈

### 10.1 이동 대상

기존 `src/bin/select-codex-session.rs`에서 다음을
`src/selector/mod.rs`로 이동한다.

```text
Mode
PaneFocus
SearchScope
AppAction
App
event loop
render 함수
filter_sessions_by_scope
clipboard 함수
selector 관련 tests
```

다음 항목은 selector 모듈에서 제거한다.

```text
main
Args
Args::parse
refresh_database
record_command_args
default_record_command
default_record_command_from_exe
print_help
home_dir
expand_home
restore_terminal
process::Command import
```

### 10.2 외부 인터페이스

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectorAction {
    Quit,
    OpenReplay(PathBuf),
}

pub(crate) struct SelectorApp {
    // 기존 App 필드를 이름과 타입 변경 없이 이동한다.
}

impl SelectorApp {
    pub(crate) fn new(rows: Vec<SessionRow>) -> Self;
}

pub(crate) fn run(
    app: &mut SelectorApp,
) -> anyhow::Result<SelectorAction> {
    terminal::with_terminal(|terminal| {
        run_event_loop(terminal, app)
    })
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut SelectorApp,
) -> anyhow::Result<SelectorAction>;
```

`include_exec`는 `SelectorApp`에 추가하지 않는다. 이번 변경에서는 CLI의
고정된 `SelectOptions.include_exec`이며 application이 refresh와 replay
시작에만 사용한다.

### 10.3 현재 검색 알고리즘 유지

검색은 DB FTS로 변경하지 않고 기존 메모리 필터를 유지한다.

```rust
fn filter_sessions_by_scope(
    rows: &[SessionRow],
    query: &str,
    scope: SearchScope,
) -> Vec<SessionRow> {
    let terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();

    if terms.is_empty() {
        return rows.to_vec();
    }

    rows.iter()
        .filter(|row| {
            let haystack = scope.text(row).to_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .cloned()
        .collect()
}

impl SearchScope {
    fn text(self, row: &SessionRow) -> String {
        match self {
            Self::All => [
                row.first_message.as_str(),
                row.cwd.as_deref().unwrap_or_default(),
                row.repository_url.as_deref().unwrap_or_default(),
                row.branch.as_deref().unwrap_or_default(),
                row.timestamp.as_deref().unwrap_or_default(),
                &session_date(row),
            ]
            .join("\n"),
            Self::FirstMessage => row.first_message.clone(),
            Self::Cwd => row.cwd.clone().unwrap_or_default(),
            Self::Branch => row.branch.clone().unwrap_or_default(),
            Self::Repository => {
                row.repository_url.clone().unwrap_or_default()
            }
            Self::Date => session_date(row),
        }
    }
}
```

다음 검색 대상도 변경하지 않는다.

```text
all
message
cwd
branch
repo
date
```

exec command/output은 검색 대상에 추가하지 않는다.

## 11. Replay 모듈

### 11.1 이동 대상

기존 `src/bin/codex-replay-tui.rs`의 replay 로직 전체를
`src/replay/mod.rs`로 이동한다.

다음만 제거하거나 대체한다.

```text
main                  → application::run에서 호출
ReplayArgs            → cli::ReplayOptions 사용
ReplayArgs::parse     → cli::parse_replay_args 사용
print_help            → cli::replay_help 사용
restore_terminal      → terminal::with_terminal 사용
```

parser, entry model, event loop, render 함수와 unit test는 동일 모듈로
이동한다. 이번 refactor에서 replay를 추가 하위 파일로 재분할하지 않는다.

### 11.2 외부 인터페이스

```rust
pub(crate) fn run(
    options: &ReplayOptions,
) -> anyhow::Result<()> {
    let input = read_input(options.input.as_deref())?;
    let entries = load_entries_from_str(
        &input,
        options.include_exec,
    )?;

    terminal::with_terminal(|terminal| {
        run_event_loop(terminal, ReplayApp::new(entries))
    })
}

fn read_input(path: Option<&Path>) -> anyhow::Result<String>;

fn load_entries_from_str(
    input: &str,
    include_exec: bool,
) -> anyhow::Result<Vec<Entry>>;
```

`read_input`은 다음과 같이 구현한다. `None`과 `Some("-")`를 동일하게
stdin으로 처리한다.

```rust
fn read_input(path: Option<&Path>) -> anyhow::Result<String> {
    match path {
        Some(path) if path != Path::new("-") => {
            std::fs::read_to_string(path)
                .with_context(|| {
                    format!("failed to read {}", path.display())
                })
        }
        Some(_) | None => {
            let mut stdin = std::io::stdin().lock();
            let input = read_input_from_reader(&mut stdin)?;

            if input.trim().is_empty() {
                anyhow::bail!(
                    "usage: select-codex-session replay \
                     <events.json|events.jsonl>\n       \
                     jq -c . a.json | \
                     select-codex-session replay"
                );
            }

            Ok(input)
        }
    }
}

fn read_input_from_reader(
    reader: &mut impl std::io::Read,
) -> anyhow::Result<String> {
    let mut input = String::new();
    reader
        .read_to_string(&mut input)
        .context("failed to read stdin")?;
    Ok(input)
}
```

`load_entries_from_str`의 알고리즘은 변경하지 않는다.

```text
입력을 JSON array 또는 JSON/JSONL stream으로 파싱
  ↓
각 record를 NormalizedEvent로 변환
  ↓
Payload event:
  include_exec=false이고 ExecCommandEnd이면 제외
  나머지는 Entry로 변환
  ↓
Exec tool call:
  include_exec=false이면 제외
  true이면 Entry로 추가하고 call_id → entry index 저장
  ↓
Exec tool output:
  기존 call_id entry가 있으면 output 추가
```

실행 중 `include_exec`를 바꾸거나 entries를 다시 필터링하는 코드는 추가하지
않는다.

## 12. Application orchestration

### 12.1 Command dispatch

`src/application.rs`의 root 함수는 다음과 같다.

```rust
pub(crate) fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Select(options) => run_select(options),
        Command::Index(options) => run_index(options),
        Command::Replay(options) => replay::run(&options),
    }
}

fn run_index(options: IndexOptions) -> anyhow::Result<()> {
    let summary = indexer::build_index(&options)?;
    println!("{}", indexer::format_summary(&summary));
    Ok(())
}
```

### 12.2 Selector 전체 알고리즘

```rust
fn run_select(options: SelectOptions) -> anyhow::Result<()> {
    if options.refresh {
        refresh_database(&options)?;
    }

    let rows = load_sessions(&options.db)?;
    if rows.is_empty() {
        anyhow::bail!(
            "no sessions found in {}",
            options.db.display()
        );
    }

    let mut app = SelectorApp::new(rows);

    if options.print_path {
        match selector::run(&mut app)? {
            SelectorAction::OpenReplay(path) => {
                println!("{}", path.display());
            }
            SelectorAction::Quit => {}
        }
        return Ok(());
    }

    loop {
        match selector::run(&mut app)? {
            SelectorAction::Quit => return Ok(()),
            SelectorAction::OpenReplay(path) => {
                match replay_selected(&options, path) {
                    Ok(()) => app.clear_status(),
                    Err(error) => {
                        // Replay 실패는 selector 전체를 종료하지 않는다.
                        app.set_status(error.to_string());
                    }
                }
            }
        }
    }
}
```

`SelectorApp.status`는 private field로 유지하고 다음 setter를 제공한다.

```rust
impl SelectorApp {
    pub(crate) fn clear_status(&mut self) {
        self.status = None;
    }

    pub(crate) fn set_status(
        &mut self,
        status: impl Into<String>,
    ) {
        self.status = Some(status.into());
    }
}
```

### 12.3 내부/외부 refresh 선택

```rust
fn refresh_database(
    options: &SelectOptions,
) -> anyhow::Result<()> {
    if let Some(program) = options.record_command.as_deref() {
        return run_external_refresh(program, options);
    }

    let defaults = IndexOptions::defaults()?;
    let index_options = IndexOptions {
        output: options.db.clone(),
        include_exec: options.include_exec,
        ..defaults
    };

    let summary = indexer::build_index(&index_options)?;
    println!("{}", indexer::format_summary(&summary));
    Ok(())
}
```

외부 recorder 인자 규약은 현재와 동일하게 유지한다.

```rust
fn external_record_args(
    db: &Path,
    include_exec: bool,
) -> Vec<String> {
    let mut args = vec![
        "--output".to_string(),
        db.to_string_lossy().to_string(),
    ];

    if include_exec {
        args.push("--include-exec".to_string());
    }

    args
}

fn run_external_refresh(
    program: &str,
    options: &SelectOptions,
) -> anyhow::Result<()> {
    let status = std::process::Command::new(program)
        .args(external_record_args(
            &options.db,
            options.include_exec,
        ))
        .status()?;

    if !status.success() {
        anyhow::bail!(
            "{} failed with status {}",
            program,
            status.code().unwrap_or(1),
        );
    }

    Ok(())
}
```

외부 recorder 인자에서는 현재 구현과 동일하게 DB 경로에
`to_string_lossy()`를 적용한다. non-UTF-8 경로 지원 개선은 이번
refactor 범위에 포함하지 않는다.

### 12.4 내부/외부 replay 선택

```rust
fn replay_selected(
    select_options: &SelectOptions,
    path: PathBuf,
) -> anyhow::Result<()> {
    if let Some(program) = select_options.replay_command.as_deref() {
        return run_external_replay(
            program,
            &path,
            select_options.include_exec,
        );
    }

    replay::run(&ReplayOptions {
        input: Some(path),
        include_exec: select_options.include_exec,
    })
}

fn external_replay_args(
    path: &Path,
    include_exec: bool,
) -> Vec<OsString> {
    let mut args = Vec::new();

    if include_exec {
        args.push(OsString::from("--include-exec"));
    }

    args.push(path.as_os_str().to_owned());
    args
}

fn run_external_replay(
    program: &str,
    path: &Path,
    include_exec: bool,
) -> anyhow::Result<()> {
    let status = std::process::Command::new(program)
        .args(external_replay_args(path, include_exec))
        .status()?;

    if status.success() {
        return Ok(());
    }

    anyhow::bail!(
        "{} exited with status {}",
        program,
        status.code().unwrap_or(1),
    )
}
```

내부 replay 오류와 외부 replay의 non-zero status는 모두
`replay_selected`의 `Err`로 정규화한다. selector는 해당 오류를 status에
표시하고 계속 실행한다.

## 13. Commit quality gate와 red/green 정책

### 13.1 결정

- 모든 phase commit은 green 상태여야 한다.
- red test만 담은 commit은 만들지 않는다.
- 각 phase에서 먼저 targeted test의 예상 실패를 확인한 뒤 같은 phase에서
  구현하여 green으로 만든다.
- commit 직전에 공통 quality script가 fmt, clippy, 전체 test를 실행한다.
- local pre-commit hook와 GitHub Actions는 공통 shell script를 호출한다.
- 검사 명령을 hook 또는 workflow YAML 안에 중복 작성하지 않는다.

phase별 순서는 다음으로 고정한다.

```text
characterization test라면 기존 코드에서 green 확인
또는 새 behavior test라면 targeted red 확인
  ↓
production code 구현
  ↓
targeted test green 확인
  ↓
scripts/check-before-commit.sh 전체 통과
  ↓
test + implementation을 하나의 green commit으로 기록
```

red 실행은 test가 실제로 요구사항을 검증함을 확인하기 위한 로컬 단계다.
phase 의미는 red commit이 아니라 test 이름, code diff와 commit message로
보존한다.

### 13.2 현재 clippy baseline 정규화

현재 baseline 결과는 다음과 같다.

```text
cargo fmt --check
  → pass

cargo test --all-targets --all-features
  → 21 passed

cargo clippy --all-targets --all-features -- -D warnings
  → fail: clippy::derivable_impls at CollectOptions
```

hook를 활성화하기 전에 다음 동작 보존 변경으로 clippy baseline을 green으로
만든다.

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct CollectOptions {
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
    pub include_exec: bool,
}

// 기존 impl Default for CollectOptions 블록은 제거한다.
```

derive가 기존의 세 `false` 기본값과 동일함을 test로 고정한다.

```rust
#[test]
fn collect_options_default_disables_all_optional_data() {
    let options = CollectOptions::default();

    assert!(!options.include_subsessions);
    assert!(!options.include_empty_messages);
    assert!(!options.include_exec);
}
```

이 변경과 quality automation 파일을 bootstrap phase의 첫 green commit으로
묶는다. commit 전에 새 hook를 설치하고 공통 script를 직접 실행한다.

### 13.3 공통 검사 script

`scripts/check-before-commit.sh`를 검사 명령의 single source of truth로
사용한다.

```bash
#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

cd "$repo_root"

cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

순서는 format 오류를 가장 먼저 빠르게 반환하고, clippy compilation
artifact를 test가 재사용하도록 `fmt → clippy → test`로 고정한다.

### 13.4 Version-controlled pre-commit hook

`.githooks/pre-commit`에는 검사 코드를 넣지 않고 공통 script만 실행한다.

```bash
#!/usr/bin/env bash
set -euo pipefail

hook_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$hook_dir/.." && pwd)"

exec "$repo_root/scripts/check-before-commit.sh"
```

두 shell 파일은 executable mode로 저장한다.

```bash
chmod +x \
  scripts/check-before-commit.sh \
  scripts/install-git-hooks.sh \
  .githooks/pre-commit
```

### 13.5 Hook installer

Git은 clone만으로 tracked hook를 자동 활성화하지 않으므로
`scripts/install-git-hooks.sh`를 제공한다. 기존 local hook 설정을 조용히
덮어쓰지 않는다.

```bash
#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

cd "$repo_root"

existing="$(git config --local --get core.hooksPath || true)"

if [[ -n "$existing" && "$existing" != ".githooks" ]]; then
  echo "core.hooksPath is already set to: $existing" >&2
  echo "refusing to overwrite the existing local hook path" >&2
  exit 1
fi

git config --local core.hooksPath .githooks

actual="$(git config --local --get core.hooksPath)"
if [[ "$actual" != ".githooks" ]]; then
  echo "failed to configure core.hooksPath" >&2
  exit 1
fi

echo "configured core.hooksPath=.githooks"
```

bootstrap phase에서 다음 순서로 활성화한다.

```bash
bash -n \
  scripts/check-before-commit.sh \
  scripts/install-git-hooks.sh \
  .githooks/pre-commit

scripts/install-git-hooks.sh
scripts/check-before-commit.sh
```

### 13.6 GitHub Actions

`.github/workflows/ci.yml`도 공통 script만 호출한다.

```yaml
name: CI

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: Run commit quality gate
        run: bash scripts/check-before-commit.sh
```

local hook는 commit 전 feedback을 제공하고, GitHub Actions는 hook가 설치되지
않았거나 `--no-verify`로 우회된 경우에도 같은 검사를 강제한다.

### 13.7 자동화 test

bootstrap phase에서 다음을 확인한다.

```bash
bash -n \
  scripts/check-before-commit.sh \
  scripts/install-git-hooks.sh \
  .githooks/pre-commit

test "$(git config --local --get core.hooksPath)" = ".githooks"

scripts/check-before-commit.sh
.githooks/pre-commit
```

`.githooks/pre-commit` 실행은 `exec`로 공통 script를 호출하므로 두 번째 전체
검사가 된다. 최초 bootstrap 검증에서만 직접 실행하고, 이후에는 실제
commit마다 hook가 자동 실행한다.

## 14. 구현 순서

각 단계는 독립 커밋으로 만들고 해당 단계에서 전체 테스트가 통과해야 한다.

### 단계 0: Quality automation bootstrap

- `CollectOptions`를 `#[derive(Default)]`로 변경하고 동등성 test 추가
- `scripts/check-before-commit.sh` 생성
- `.githooks/pre-commit` 생성
- `scripts/install-git-hooks.sh` 생성
- `.github/workflows/ci.yml` 생성
- shell syntax 검사
- local `core.hooksPath=.githooks` 설정
- 공통 script와 hook 직접 실행
- green 상태에서 bootstrap commit 생성

이 단계 이후 모든 commit은 설치된 hook를 통과해야 한다.

### 단계 1: 동작 고정 테스트

- 현재 코드 위치에 누락된 CLI/parser/orchestration test 추가
- 제품 코드 이동 없음
- characterization test가 기존 코드에서 green인지 확인
- 공통 quality gate 통과 후 commit

### 단계 2: Indexer 추출

- `src/indexer.rs` 생성
- recorder `main`을 `indexer::build_index` 호출로 축소
- summary 문자열 exact test 추가
- 기존 standalone recorder는 아직 유지
- targeted test와 공통 quality gate 통과 후 commit

### 단계 3: Replay 추출

- `src/replay/mod.rs` 생성
- replay parser/TUI를 이동
- 기존 standalone replay `main`은 `replay::run`만 호출
- 기존 replay tests를 새 모듈로 이동
- targeted test와 공통 quality gate 통과 후 commit

### 단계 4: Selector 추출

- `src/selector/mod.rs` 생성
- selector state/render/event loop 이동
- 기존 selector `main`은 임시로 application 함수를 호출
- targeted test와 공통 quality gate 통과 후 commit

### 단계 5: 통합 CLI와 application 구현

- `src/cli.rs`, `src/application.rs`, `src/terminal.rs` 생성
- `src/lib.rs::run_from_args` 추가
- `src/main.rs` 생성
- 기본 refresh/replay를 내부 호출로 전환
- 명시적 external override는 유지
- 새 behavior test의 red를 먼저 확인하고 구현 후 green으로 전환
- 공통 quality gate 통과 후 commit

### 단계 6: 단일 binary 전환

- `autobins = false`
- 단일 `[[bin]]` target 설정
- 기존 `src/bin` entrypoint 제거
- package version을 `0.3.0`으로 갱신
- `install-bundle.sh`를 단일 파일 설치로 변경
- package test와 공통 quality gate 통과 후 commit

### 단계 7: 문서 및 package 검증

- README를 단일 binary와 subcommand 기준으로 갱신
- legacy command mapping 추가
- `cargo package` 및 임시 root 설치 검증
- 공통 quality gate 통과 후 commit

## 15. 테스트 계획

### 15.1 CLI parser tests

`src/cli.rs`에 다음 test를 구현한다.

```rust
#[test]
fn no_args_defaults_to_selector() {
    let action = parse_args(std::iter::empty::<String>()).unwrap();

    let CliAction::Run(Command::Select(options)) = action else {
        panic!("expected selector");
    };

    assert!(options.refresh);
    assert!(!options.print_path);
    assert!(!options.include_exec);
    assert_eq!(options.record_command, None);
    assert_eq!(options.replay_command, None);
}

#[test]
fn selector_parses_current_options() {
    let action = parse_args(
        [
            "--db",
            "/tmp/sessions.sqlite3",
            "--no-refresh",
            "--print-path",
            "--include-exec",
            "--record-command",
            "/tmp/recorder",
            "--replay-command",
            "/tmp/replay",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap();

    let CliAction::Run(Command::Select(options)) = action else {
        panic!("expected selector");
    };

    assert_eq!(
        options.db,
        PathBuf::from("/tmp/sessions.sqlite3")
    );
    assert!(!options.refresh);
    assert!(options.print_path);
    assert!(options.include_exec);
    assert_eq!(
        options.record_command.as_deref(),
        Some("/tmp/recorder")
    );
    assert_eq!(
        options.replay_command.as_deref(),
        Some("/tmp/replay")
    );
}

#[test]
fn index_subcommand_parses_legacy_recorder_options() {
    let action = parse_args(
        [
            "index",
            "--output",
            "/tmp/index.sqlite3",
            "--sessions-root",
            "/tmp/sessions",
            "--include-subsessions",
            "--include-empty-messages",
            "--include-exec",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap();

    let CliAction::Run(Command::Index(options)) = action else {
        panic!("expected index");
    };

    assert_eq!(
        options.output,
        PathBuf::from("/tmp/index.sqlite3")
    );
    assert_eq!(
        options.sessions_root,
        PathBuf::from("/tmp/sessions")
    );
    assert!(options.include_subsessions);
    assert!(options.include_empty_messages);
    assert!(options.include_exec);
}

#[test]
fn replay_subcommand_accepts_stdin_and_include_exec() {
    let action = parse_args(
        ["replay", "--include-exec", "-"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();

    assert_eq!(
        action,
        CliAction::Run(Command::Replay(ReplayOptions {
            input: Some(PathBuf::from("-")),
            include_exec: true,
        }))
    );
}

#[test]
fn replay_rejects_multiple_inputs() {
    let error = parse_args(
        ["replay", "a.jsonl", "b.jsonl"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("only one input path")
    );
}

#[test]
fn root_option_before_subcommand_is_not_reinterpreted() {
    let error = parse_args(
        ["--include-exec", "replay", "a.jsonl"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown argument"));
}

#[test]
fn help_is_returned_without_process_exit() {
    assert_eq!(
        parse_args(["index", "--help"].into_iter().map(str::to_owned))
            .unwrap(),
        CliAction::PrintHelp(HelpTopic::Index)
    );
}
```

추가 오류 test:

```text
--db 값 누락
--output 값 누락
--sessions-root 값 누락
unknown selector option
unknown index option
unknown replay option
```

### 15.2 Indexer tests

기존 `collect_session_data_omits_exec_by_default_and_collects_when_enabled`와
`recreate_database_creates_exec_table_only_when_requested`를 유지한다.

다음 use-case test를 추가한다.

```rust
#[test]
fn build_index_without_exec_preserves_default_schema() {
    let fixture = SessionFixture::new();
    fixture.write_session_with_exec();
    let db = fixture.path("index.sqlite3");

    let summary = build_index(&IndexOptions {
        output: db.clone(),
        sessions_root: fixture.sessions_root(),
        include_subsessions: false,
        include_empty_messages: false,
        include_exec: false,
    })
    .unwrap();

    assert_eq!(summary.session_rows, 1);
    assert_eq!(summary.exec_rows, 0);
    assert!(!summary.exec_enabled);

    let connection = rusqlite::Connection::open(db).unwrap();
    assert_eq!(table_count(&connection, "sessions"), 1);
    assert!(!table_exists(&connection, "exec_events"));
    assert!(!table_exists(&connection, "sessions_fts"));
}

#[test]
fn build_index_with_exec_preserves_current_exec_schema() {
    let fixture = SessionFixture::new();
    fixture.write_session_with_exec();
    let db = fixture.path("index.sqlite3");

    let summary = build_index(&IndexOptions {
        output: db.clone(),
        sessions_root: fixture.sessions_root(),
        include_subsessions: false,
        include_empty_messages: false,
        include_exec: true,
    })
    .unwrap();

    assert_eq!(summary.session_rows, 1);
    assert_eq!(summary.exec_rows, 3);
    assert!(summary.exec_enabled);

    let connection = rusqlite::Connection::open(db).unwrap();
    assert_eq!(table_count(&connection, "sessions"), 1);
    assert_eq!(table_count(&connection, "exec_events"), 3);
    assert!(!table_exists(&connection, "sessions_fts"));
}

#[test]
fn format_summary_matches_legacy_output() {
    let summary = IndexSummary {
        session_rows: 2,
        exec_rows: 5,
        total_files: 3,
        skipped: 1,
        output: PathBuf::from("/tmp/index.sqlite3"),
        exec_enabled: true,
    };

    assert_eq!(
        format_summary(&summary),
        "wrote 2 session rows and 5 exec rows to \
         /tmp/index.sqlite3 from 3 jsonl files; \
         skipped 1 filtered or invalid sessions"
    );
}
```

`SessionFixture`는 test-only helper로 구현하고 `Drop`에서 자신이 생성한
정확한 임시 디렉터리만 제거한다. workspace root나 광범위한 경로를
삭제하지 않는다.

#### Test fixture 구현

`src/test_support.rs`에 다음 helper를 둔다.

```rust
pub(crate) struct SessionFixture {
    root: PathBuf,
}

fn write_jsonl_with_exec(path: &Path) {
    // src/lib.rs의 기존 test helper 본문을 내용 변경 없이 이동한다.
}

impl SessionFixture {
    pub(crate) fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);

        let unique = format!(
            "codex-session-selector-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(
                1,
                std::sync::atomic::Ordering::Relaxed,
            ),
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub(crate) fn sessions_root(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub(crate) fn write_session_with_exec(&self) -> PathBuf {
        let day = self
            .sessions_root()
            .join("2026")
            .join("07")
            .join("26");
        std::fs::create_dir_all(&day).unwrap();
        let path = day.join("session.jsonl");

        // src/lib.rs tests의 기존 write_jsonl_with_exec가 생성하는
        // session_meta, user_message, legacy exec 1개,
        // custom_tool_call/output 1쌍, function_call/output 1쌍을
        // 내용 변경 없이 이 공통 helper로 이동한다.
        write_jsonl_with_exec(&path);
        path
    }
}

impl Drop for SessionFixture {
    fn drop(&mut self) {
        // root는 new()가 생성한 정확한 임시 하위 경로다.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn table_exists(
    connection: &rusqlite::Connection,
    table: &str,
) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM sqlite_master
                WHERE type = 'table' AND name = ?1
            )",
            rusqlite::params![table],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
}

pub(crate) fn table_count(
    connection: &rusqlite::Connection,
    table: &str,
) -> i64 {
    // SQL identifier bind는 지원되지 않으므로 허용 목록으로 제한한다.
    let sql = match table {
        "sessions" => "SELECT count(*) FROM sessions",
        "exec_events" => "SELECT count(*) FROM exec_events",
        _ => panic!("unsupported fixture table: {table}"),
    };

    connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}
```

기존 `src/lib.rs` test module의 `write_jsonl_with_exec`는 복사본을 남기지
않고 `src/test_support.rs`로 이동한다. 해당 helper가 생성하는 exec row가
정확히 3개라는 기존 assertion을 유지한다.

### 15.3 External command argument tests

실제 외부 프로세스를 실행하지 않고 인자 생성 함수를 검증한다.

```rust
#[test]
fn external_record_args_match_legacy_contract() {
    assert_eq!(
        external_record_args(
            Path::new("/tmp/index.sqlite3"),
            false,
        ),
        vec![
            "--output".to_string(),
            "/tmp/index.sqlite3".to_string(),
        ]
    );

    assert_eq!(
        external_record_args(
            Path::new("/tmp/index.sqlite3"),
            true,
        ),
        vec![
            "--output".to_string(),
            "/tmp/index.sqlite3".to_string(),
            "--include-exec".to_string(),
        ]
    );
}

#[test]
fn external_replay_args_match_legacy_contract() {
    assert_eq!(
        external_replay_args(
            Path::new("/tmp/session.jsonl"),
            true,
        ),
        vec![
            OsString::from("--include-exec"),
            OsString::from("/tmp/session.jsonl"),
        ]
    );
}
```

### 15.4 Replay tests

기존 replay unit test를 이름과 assertion을 변경하지 않고 이동한다.

```text
loads_raw_codex_jsonl_events
loads_preprocessed_json_array_events
loads_response_item_exec_tool_calls_as_exec_entries
loads_function_call_exec_command_as_exec_entry
extracts_exec_command_from_tool_input
excludes_exec_entries_unless_requested
```

기존 `replay_args_enable_exec_only_when_requested`는 CLI module test로
대체한다.

다음 stdin/path test를 추가한다. stdin은 process-global이므로
`read_input`을 직접 stdin에 결합하지 않고 reader helper를 분리한다.

```rust
fn read_input_from_reader(
    reader: &mut impl std::io::Read,
) -> anyhow::Result<String>;

#[test]
fn reader_input_preserves_jsonl() {
    let mut input =
        br#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#
            .as_slice();

    assert_eq!(
        read_input_from_reader(&mut input).unwrap(),
        r#"{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#
    );
}
```

### 15.5 Selector tests

기존 selector test를 이동하고 유지한다.

```text
metadata_plain_uses_requested_column_order
scrolled_segments_preserve_text_after_horizontal_offset
search_scope_cycles_through_all_fields
search_scope_filters_only_selected_field
resume_command_uses_session_id
```

기존 binary argument test는 `cli.rs` test로 대체한다.

검색 결과가 FTS로 바뀌지 않았음을 다음 test로 고정한다.

```rust
#[test]
fn search_remains_case_insensitive_all_terms_substring_match() {
    let rows = sample_rows();

    let filtered = filter_sessions_by_scope(
        &rows,
        "FIX read",
        SearchScope::All,
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].first_message, "Fix README parser");
}

#[test]
fn exec_text_is_not_a_selector_search_scope() {
    assert_eq!(
        SearchScope::All.next(),
        SearchScope::FirstMessage
    );
    assert_eq!(
        SearchScope::Repository.next(),
        SearchScope::Date
    );
    assert_eq!(SearchScope::Date.next(), SearchScope::All);
}
```

### 15.6 Binary integration tests

`tests/cli.rs`에서 빌드된 단일 executable을 호출한다. 새 test dependency는
추가하지 않고 표준 라이브러리의 `std::process::Command`를 사용한다.

```rust
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_select-codex-session")
}

#[test]
fn root_help_lists_subcommands() {
    let output = Command::new(binary())
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("index"));
    assert!(stdout.contains("replay"));
    assert!(stdout.contains("--include-exec"));
}

#[test]
fn index_help_lists_legacy_recorder_options() {
    let output = Command::new(binary())
        .args(["index", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--sessions-root"));
    assert!(stdout.contains("--include-subsessions"));
    assert!(stdout.contains("--include-empty-messages"));
}

#[test]
fn replay_help_lists_current_replay_options() {
    let output = Command::new(binary())
        .args(["replay", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--include-exec"));
    assert!(stdout.contains("[PATH|-]"));
}
```

TUI 자체는 pseudo-terminal이 필요하므로 자동 integration test에서 실행하지
않는다. 아래 manual smoke test로 검증한다.

## 16. Manual smoke test

임시 HOME과 fixture를 사용하여 실제 사용자 HOME의 DB를 변경하지 않는다.

```bash
smoke_root="$(mktemp -d)"
mkdir -p "$smoke_root/home/.codex/sessions/2026/07/26"
cp tests/fixtures/session.jsonl \
  "$smoke_root/home/.codex/sessions/2026/07/26/session.jsonl"

HOME="$smoke_root/home" \
  target/debug/select-codex-session
```

확인 항목:

1. 시작 시 index summary가 한 번 출력된다.
2. selector에 fixture session이 표시된다.
3. 검색, focus, 이동 키가 기존과 동일하다.
4. `Enter`를 누르면 replay가 열린다.
5. replay 종료 후 selector의 선택과 query가 유지된다.
6. selector 종료 후 cursor와 terminal echo가 정상이다.

서브커맨드 smoke test:

```bash
HOME="$smoke_root/home" \
  target/debug/select-codex-session index \
  --output "$smoke_root/index.sqlite3"

target/debug/select-codex-session replay \
  tests/fixtures/session.jsonl

target/debug/select-codex-session replay \
  --include-exec \
  tests/fixtures/session.jsonl
```

검증 후 임시 디렉터리만 제거한다.

```bash
rm -rf "$smoke_root"
```

`smoke_root`가 `mktemp -d`의 non-empty 결과인지 확인한 뒤에만 위 제거
명령을 실행한다.

## 17. Package 검증

### 17.1 전체 자동 검사

```bash
scripts/check-before-commit.sh
cargo build --release
git diff --check
```

### 17.2 Binary target 수 확인

```bash
cargo metadata --no-deps --format-version 1 |
  jq '[
    .packages[0].targets[]
    | select(.kind | index("bin"))
    | .name
  ]'
```

기대 결과:

```json
["select-codex-session"]
```

### 17.3 임시 설치 확인

```bash
install_root="$(mktemp -d)"
cargo install --path . --root "$install_root"
find "$install_root/bin" -maxdepth 1 -type f -printf '%f\n'
```

기대 결과:

```text
select-codex-session
```

다음 파일이 생성되면 실패다.

```text
record-codex-session-info
codex-replay-tui
```

## 18. README 변경 명세

README는 다음 구조로 변경한다.

```text
Binaries
  select-codex-session 하나만 설명

Quick Start
  select-codex-session
  select-codex-session --include-exec

Index
  select-codex-session index ...

Replay
  select-codex-session replay ...

Migration
  기존 standalone 명령 → subcommand 매핑

SQLite Schema
  현재 sessions/exec_events만 설명
  FTS 언급 없음

Development
  scripts/install-git-hooks.sh
  scripts/check-before-commit.sh
  green-only commit policy
```

다음 설명은 제거한다.

```text
세 바이너리를 설치한다는 설명
record-codex-session-info를 직접 설치하는 명령
codex-replay-tui를 직접 설치하는 명령
기본 selector가 sibling recorder를 탐색한다는 설명
기본 selector가 PATH의 replay binary를 호출한다는 설명
```

external override 설명은 advanced compatibility 항목으로 이동한다.

## 19. 완료 조건

다음 조건을 모두 만족해야 구현 완료로 판단한다.

- Cargo package에 binary target이 정확히 하나다.
- `cargo install` 결과가 `select-codex-session` 하나다.
- `core.hooksPath`가 `.githooks`로 설정되어 있다.
- pre-commit hook와 GitHub Actions가 같은 공통 검사 script를 호출한다.
- 공통 검사 script가 fmt, clippy `-D warnings`, 전체 test를 순서대로 실행한다.
- 모든 phase commit은 green quality gate를 통과한다.
- 새 behavior test는 같은 phase 안에서 red 확인 후 green으로 커밋한다.
- 서브커맨드 없는 실행이 현재 selector 동작을 유지한다.
- 기본 refresh가 외부 helper 없이 동작한다.
- 기본 replay가 외부 helper 없이 동작한다.
- `--record-command`와 `--replay-command`를 명시하면 외부 helper를 사용한다.
- `index`가 기존 recorder와 동일한 schema와 row를 생성한다.
- `replay`가 기존 replay와 동일한 입력 및 event를 처리한다.
- `--include-exec`가 현재 command-line 동작만 유지한다.
- 새 TUI toggle key가 없다.
- 새 FTS table이 없다.
- selector 검색 결과와 scope 순서가 변경되지 않는다.
- replay 종료 후 selector state가 유지된다.
- 정상 및 오류 `Result` 경로에서 terminal이 복원된다.
- 기존 unit test와 새 test가 모두 통과한다.
- README와 실제 `--help`가 일치한다.

## 20. 후속 작업 경계

이 refactor 완료 후에도 2번과 3번은 자동으로 시작하지 않는다. 각각 별도
요청, 별도 설계 문서, 별도 변경 세트로 진행한다.

이번 refactor에서 만들어지는 module boundary는 후속 변경이 core CLI,
terminal lifecycle 및 packaging과 섞이지 않게 하는 목적만 갖는다. 후속
기능을 미리 구현하거나 DB schema를 선행 변경하지 않는다.
