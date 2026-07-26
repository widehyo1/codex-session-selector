# TUI Exec Visibility Toggle Implementation Plan

## 1. 문서 상태와 목표 버전

- 문서 상태: 구현 준비 완료
- 작성 기준일: 2026-07-26
- 기준 commit: `e0623e5` (`refactor: consolidate into one binary`)
- 기준 package version: `0.3.0`
- 2번 구현 중 package version: `0.3.0` 유지
- 통합 공개 목표 version: `0.3.0`
- 공개 시점: 2번, canonical/증분 index와 3번 FTS5까지 모두 완료한 이후
- 구현 blocker: 없음

작성 시점 working tree는 clean이다. 현재 baseline은 다음과 같다.

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

이 문서는 기능 코드가 아니라 2번 TUI exec visibility toggle을 구현하기 위한
계획이다. 구현자는 이 문서의 결정을 변경하거나 추가 설계 판단을 하지 않고
순서대로 실행한다.

## 2. 목표와 사용자에게 보이는 최종 동작

`select-codex-session`의 selector와 replay TUI에 `e` 키로 command execution
entry 표시 여부를 실행 중 변경하는 기능을 추가한다.

최종 사용자 동작은 다음과 같다.

1. `--include-exec` 없이 selector 또는 replay를 시작하면 exec entry는
   `hidden` 상태다.
2. `--include-exec`를 지정하면 exec entry는 `shown` 상태다.
3. selector normal mode에서 `e`를 누르면 현재 visibility가 바뀐다.
   selector의 session 목록 자체는 바뀌지 않고, 다음에 여는 replay의 초기
   visibility가 바뀐다.
4. replay에서 `e`를 누르면 JSONL을 다시 읽거나 다시 parse하지 않고 현재
   timeline에서 exec entry를 즉시 표시하거나 숨긴다.
5. internal replay에서 visibility를 바꾼 후 selector로 돌아오면 변경된
   visibility가 selector에 유지된다. 다음 replay도 그 상태로 시작한다.
6. selector와 replay header에 각각 다음 exact 상태 문구가 항상 표시된다.

   ```text
   exec: hidden
   exec: shown
   ```

7. selector search mode에서 입력한 `e`는 검색어에 추가되며 toggle로
   처리되지 않는다.
8. help overlay가 열려 있을 때 `e`를 포함한 일반 키는 뒤의 화면 상태를
   변경하지 않는다.
9. replay timeline 또는 detail fullscreen 상태에서는 `e`가 계속 toggle로
   동작한다.
10. exec entry를 숨기거나 다시 표시해도 가능한 경우 현재 선택한 non-exec
    entry를 유지한다.

## 3. 구현 범위와 명시적 비범위

### 3.1 구현 범위

- selector와 replay가 함께 사용하는 `ExecVisibility` 상태
- selector normal mode의 `e` key binding
- replay normal/fullscreen의 `e` key binding
- CLI `--include-exec`와 초기 visibility 연결
- selector → internal replay → selector visibility 왕복
- replay 입력을 한 번만 parse하고 `all_entries`와 `visible_indices`를
  분리하는 모델
- toggle 시 visible index rebuild와 selection 복원
- selector와 replay의 help overlay modal key 처리
- header, footer, TUI help, CLI help와 README 상태 문구 변경
- 상태 전이 unit test와 manual pseudo-terminal smoke test
- package version `0.3.0` 유지와 중간 공개 금지

### 3.2 명시적 비범위

다음 항목은 이번 구현에 포함하지 않는다.

- `sessions` 또는 `exec_events` SQLite schema 변경
- SQLite 전체 rebuild 제거
- canonical/superset index
- 증분 index, file fingerprint, `UPSERT`, schema version 또는 migration
- `index --include-exec`의 저장 의미 변경
- subsession 또는 빈 first message의 TUI toggle
- selector session 목록에 exec count 또는 exec 내용을 표시하는 기능
- selector 검색에 exec command/output을 추가하는 기능
- FTS5, Contentless-Delete, ranking 또는 query 문법 변경
- visibility를 config file이나 SQLite에 영구 저장하는 기능
- 서로 다른 process 실행 사이의 visibility 유지
- 외부 replay process에서 변경한 visibility를 parent selector로 반환하는
  별도 protocol
- 새로운 dependency, Cargo feature 또는 최소 Rust version 변경
- crate publish, release tag 생성 또는 GitHub release

기능 완료 후에도 SQLite는 현재처럼 refresh 시 전체 rebuild한다. 이번
기능은 replay가 JSONL을 직접 읽는 현재 구조만 사용한다.

## 4. 현재 동작과 호환성 계약

### 4.1 현재 동작 baseline

- `SelectOptions.include_exec`와 `ReplayOptions.include_exec` 기본값은
  `false`다.
- root `--include-exec`는 selector 시작 전 refresh에서 `exec_events`
  table을 만들고, 선택한 replay에도 같은 bool을 전달한다.
- `index --include-exec`는 `exec_events` table 생성과 row 저장을
  제어한다.
- `replay --include-exec`는 replay parse 중 exec entry를 포함한다.
- `load_entries_from_str(input, false)`는 exec entry를 parse 결과에서
  제거하고, `true`이면 포함한다.
- `ReplayApp.entries`에는 화면에 보이는 entry만 들어 있다.
- replay를 닫으면 `ReplayApp`은 소멸한다.
- `SelectorApp`은 application loop 바깥에서 한 번 생성되므로 선택, query,
  search scope와 pane focus는 replay 복귀 후 유지된다.
- selector help overlay가 열려 있어도 현재는 normal-mode key가 뒤에서
  처리되며 `Esc`는 selector를 종료한다.
- replay help overlay도 `Esc` 외 일반 key가 뒤에서 처리된다.
- replay fullscreen에서는 모든 기존 key가 처리된다.
- replay entry title과 detail의 `index:`는 현재 화면에 포함된 entry를
  `0`부터 조밀하게 번호 매긴다.
- replay는 SQLite `exec_events`를 읽지 않고 선택한 JSONL을 직접 읽는다.

### 4.2 유지할 호환성

- `--include-exec` option 이름과 parse 위치를 유지한다.
- `--include-exec`가 없으면 최초 화면에서 exec entry가 숨겨진다.
- `--include-exec`가 있으면 최초 화면에서 exec entry가 표시된다.
- root `--include-exec`는 최초 refresh에 계속 전달된다.
- `index --include-exec`의 schema와 output summary를 변경하지 않는다.
- external recorder에 전달하는 argument 순서를 유지한다.
- external replay에는 replay를 여는 시점의 visibility에 따라
  `--include-exec`를 전달한다.
- user/agent/exec normalization, exec tool call/output 결합, unsupported
  record 무시 규칙과 invalid JSON 오류를 유지한다.
- 표시되는 entry title은 현재와 같이 `#0000`부터 조밀하게 번호 매긴다.
- selector의 session filtering, query, scope, focus, scroll, clipboard와
  `--print-path` 동작을 유지한다.
- replay의 focus, scroll, fullscreen, clipboard와 종료 동작을 유지한다.
- SQLite schema, row 내용과 rebuild transaction 순서를 변경하지 않는다.
- 설치 binary는 `select-codex-session` 하나를 유지한다.

### 4.3 의도적 동작 변경

- replay는 초기 visibility와 관계없이 지원하는 모든 exec entry를 한 번
  parse하여 메모리에 보존한다.
- selector와 replay normal mode에 `e` key가 추가된다.
- internal replay에서 바꾼 visibility가 selector로 반환된다.
- selector help에서 `Esc`는 selector를 종료하지 않고 help만 닫는다.
- selector와 replay help overlay가 modal이 되어 일반 key가 뒤의 pane,
  search, fullscreen 또는 visibility를 변경하지 않는다.
- header와 footer에 exec visibility와 `e` key 안내가 추가된다.
- `--include-exec` CLI help는 고정 표시가 아니라 initial visibility임을
  명시한다.

## 5. 확정한 기술·구조 결정

### 5.1 사용할 기술과 dependency

기존 기술만 사용한다.

| 기술 | 현재 version/feature | 사용 목적 |
| --- | --- | --- |
| Rust | 최소 `1.85`, edition `2024` | 상태와 filtering 구현 |
| crossterm | `0.29`, default features | `KeyCode::Char('e')`, `KeyEvent` |
| ratatui | `0.30`, default features | header/footer/help 렌더링 |
| anyhow | `1` | 기존 I/O와 terminal 오류 전달 |

`Cargo.toml`의 dependency와 feature는 변경하지 않는다. 현재
`cargo test --all-targets --all-features`와 clippy가 통과하므로 필요한
`KeyCode`, `KeyEvent`, `ListState` API 지원은 baseline에서 확인됐다.

새 TUI framework, async runtime, database API 또는 caching crate는
도입하지 않는다.

### 5.2 toggle key

toggle key는 소문자 `e`로 확정한다.

| 화면 상태 | `e` 처리 |
| --- | --- |
| selector normal | exec visibility toggle |
| selector search | query에 문자 `e` 추가 |
| selector help | 무시 |
| replay normal | exec visibility toggle |
| replay timeline fullscreen | exec visibility toggle |
| replay detail fullscreen | exec visibility toggle |
| replay help | 무시 |

현재 selector normal과 replay에는 `e` binding이 없어 충돌하지 않는다.
대문자 `E`, `Alt-e`, `Ctrl-e`는 새 binding으로 처리하지 않는다.
`KeyCode::Char('e')`이고 modifier가 비어 있는 key만 toggle한다.

### 5.3 상태 타입과 소유권

bool을 여러 함수에서 직접 전달하지 않고 shared enum을 사용한다.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecVisibility {
    Hidden,
    Shown,
}
```

상태 소유권은 다음과 같다.

| lifecycle | owner | 역할 |
| --- | --- | --- |
| CLI parse 직후 | `SelectOptions`/`ReplayOptions`의 기존 bool | 초기값 입력 |
| selector 실행 중 | `SelectorApp.exec_visibility` | 다음 replay의 초기값 |
| internal replay 실행 중 | `ReplayApp.exec_visibility` | timeline filter 상태 |
| replay 정상 종료 | `replay::run` return value | selector로 최종값 반환 |
| external replay 실행 | selector의 현재값 | child argument에만 사용 |

global/static 상태, `Rc<RefCell<_>>`, channel 또는 mutex는 사용하지 않는다.

### 5.4 CLI `--include-exec`와 toggle 관계

- CLI bool은 TUI가 시작할 때 한 번 `ExecVisibility`로 변환한다.
- root selector의 original CLI bool은 TUI가 열리기 전 refresh에 그대로
  사용한다.
- TUI에서 toggle해도 SQLite를 rebuild하거나 `exec_events` table을
  추가/삭제하지 않는다.
- internal replay는 DB가 아니라 JSONL을 parse하므로 DB에 `exec_events`가
  없어도 toggle로 exec entry를 표시할 수 있다.
- standalone `replay`에서 최종 visibility는 process 종료과 함께
  폐기한다.
- `--include-exec`를 process 중간에 다시 CLI option으로 해석하지 않는다.

### 5.5 selector에서의 의미

selector에는 exec event 목록이 없으므로 toggle은 session row filter가
아니다.

- `e`는 `SelectorApp.exec_visibility`만 변경한다.
- session rows, filtered rows, query, search scope와 selected session을
  변경하지 않는다.
- 다음 internal 또는 external replay가 현재 visibility로 열린다.
- internal replay가 반환한 visibility로 selector header가 갱신된다.

### 5.6 replay에서의 의미

replay는 모든 normalized entry를 `all_entries`에 한 번 저장하고
`visible_indices`로 표시 대상을 정한다.

```text
Hidden → User, Agent entry만 visible
Shown  → User, Agent, Exec entry 모두 visible
```

toggle은 JSONL 재읽기, JSON deserialize, exec output 재결합 또는
`Entry` clone을 하지 않는다. `visible_indices: Vec<usize>`만 O(n)으로
재생성한다.

### 5.7 표시 번호와 entry identity

- internal identity는 `all_entries`의 index다.
- `visible_indices`는 internal identity를 원본 순서대로 보존한다.
- 화면 title의 `#NNNN`과 detail의 `index:`는 visible list position을
  사용한다.
- exec를 표시하면 뒤의 user/agent display index가 증가할 수 있고,
  숨기면 다시 감소한다.
- 이는 현재 `--include-exec`를 켜고 끈 별도 실행에서 보이는 조밀한 번호와
  호환된다.

### 5.8 toggle 후 selection 규칙

toggle 직전 선택 entry의 `all_entries` index를 저장한 뒤 visible list를
재생성한다.

1. 이전 entry가 새 visible list에도 있으면 같은 entry를 선택한다.
2. 선택한 exec entry를 숨겼다면 canonical 순서상 뒤에 있는 첫 visible
   entry를 선택한다.
3. 뒤에 없으면 앞에 있는 마지막 visible entry를 선택한다.
4. visible entry가 하나도 없으면 `ListState` selection을 `None`으로
   만든다.
5. 이전 selection이 `None`이고 toggle 후 visible entry가 생기면 첫
   entry를 선택한다.
6. toggle할 때 `detail_scroll`은 `0`, transient `status`는 `None`으로
   reset한다.
7. focus와 fullscreen은 변경하지 않는다.

### 5.9 help modal 규칙

help가 열린 상태에서는 다음 key만 처리한다.

| key | selector | replay |
| --- | --- | --- |
| `Esc` | help 닫기 | help 닫기 |
| `?` | help 닫기 | help 닫기 |
| `q` | TUI 종료 | TUI 종료 |
| `Ctrl-C` | TUI 종료 | TUI 종료 |
| 그 외 | 무시 | 무시 |

selector search mode에서 `?`는 기존처럼 query 문자로 입력된다. help는
normal mode에서만 열 수 있다.

### 5.10 exact 화면 문구

Ratatui span style을 제외한 selector header의 논리 문자열은 다음 형식이다.

```text
 Codex Sessions {selected}/{visible} of {all}[ | /{query}] | exec: {hidden|shown}
```

replay header는 다음 형식이다.

```text
 Codex Replay {selected}/{visible} of {all} | exec: {hidden|shown}
```

selection이 없으면 `{selected}/{visible}`은 `0/0`이다. `{all}`은
`all_entries.len()`이며 exec가 hidden이어도 변하지 않는다.

selector normal footer에 다음 token을 추가한다.

```text
 e exec
```

replay footer에도 다음 token을 추가한다.

```text
 e exec
```

selector help의 Other section에 다음 exact line을 추가한다.

```text
  e               toggle exec entries for the next replay
```

replay help의 Other section에 다음 exact line을 추가한다.

```text
  e               toggle command execution entries
```

CLI root help의 option 설명은 다음으로 변경한다.

```text
      --include-exec             Index exec records and initially show them
```

CLI root Keys section에는 다음을 추가한다.

```text
  e                              toggle exec entries for replay
```

CLI replay help의 option 설명은 다음으로 변경한다.

```text
      --include-exec     Initially show command execution records
                         default: hidden; press e to toggle
```

CLI replay Keys section에는 다음을 추가한다.

```text
  e                  toggle command execution entries
```

### 5.11 선택 이유, 배제한 대안과 성능 계약

확정 구조를 선택한 이유는 다음과 같다.

- toggle 때 JSONL을 다시 parse하는 대안은 file I/O·parse 오류가 사용자 key
  입력 시점에 다시 발생하고 active session 내용이 중간에 바뀔 수 있으므로
  배제한다.
- hidden/shown용 `Vec<Entry>` 두 개를 유지하는 대안은 detail 문자열을
  clone하고 두 list 사이 selection identity를 별도로 맞춰야 하므로
  배제한다.
- SQLite `exec_events`를 replay source로 바꾸는 대안은 현재 raw JSONL
  replay와 stdin replay를 깨고 schema/index 기능을 2번에 결합하므로
  배제한다.
- `all_entries` 하나와 `Vec<usize>` view는 원본 순서를 보존하고 entry
  detail을 복제하지 않으므로 선택 복원이 단순하다.

현재 replay는 visibility가 hidden이어도 전체 입력 문자열을 읽고 모든 JSON
value를 deserialize/normalize한 뒤 exec entry만 결과에서 제외한다. 새
구조는 file read와 JSON deserialize 횟수를 늘리지 않고 exec `Entry`와
call-id mapping을 메모리에 남기는 비용만 추가한다.

성능 계약은 다음과 같다.

- 한 번의 replay 실행에서 input read와 JSON parse는 각각 한 번이다.
- 한 번의 toggle은 `all_entries.len()`에 선형이고 file/DB I/O가 없다.
- 추가 view memory는 visible entry마다 `usize` 하나다.
- 10,000 entry를 100회 toggle하는 non-timing unit test로 구조를 고정한다.
- wall-clock threshold는 공유 CI hardware 편차 때문에 자동 test에 넣지
  않는다.

## 6. 목표 파일 구조와 파일별 책임

최종 관련 파일 구조는 다음과 같다.

```text
Cargo.toml
Cargo.lock
README.md
docs/
  tui-exec-toggle-plan.md
src/
  application.rs
  cli.rs
  lib.rs
  replay/
    mod.rs
  selector/
    mod.rs
  ui_state.rs
tests/
  cli.rs
```

| 파일 | 책임 |
| --- | --- |
| `src/ui_state.rs` | `ExecVisibility`와 bool 변환, toggle, label |
| `src/lib.rs` | private `ui_state` module 선언 |
| `src/selector/mod.rs` | selector 상태, `e` 처리, 상태 렌더링과 modal help |
| `src/replay/mod.rs` | 모든 entry parse, visible index, selection 복원, 최종 상태 반환 |
| `src/application.rs` | CLI 초기값 전달, replay 결과를 selector에 반영, external contract |
| `src/cli.rs` | root/replay help의 initial/toggle 의미와 key 안내 |
| `tests/cli.rs` | 실제 binary help surface 검증 |
| `README.md` | 사용자 사용법, control, SQLite와 toggle 관계 |
| `Cargo.toml` | version `0.3.0` 유지, dependency/target 무변경 확인 |
| `Cargo.lock` | local package version `0.3.0` 유지 확인 |

`src/lib.rs`의 session parser, SQLite 함수, `src/indexer.rs`,
`src/test_support.rs`, `src/terminal.rs`와 CI/hook script는 production
변경 대상이 아니다.

## 7. 타입과 함수 시그니처

### 7.1 `src/ui_state.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecVisibility {
    Hidden,
    Shown,
}

impl ExecVisibility {
    pub(crate) fn from_include_exec(include_exec: bool) -> Self;
    pub(crate) fn is_shown(self) -> bool;
    pub(crate) fn toggle(&mut self);
    pub(crate) fn label(self) -> &'static str;
}
```

동작은 다음과 같이 고정한다.

```rust
fn from_include_exec(include_exec: bool) -> ExecVisibility {
    if include_exec {
        ExecVisibility::Shown
    } else {
        ExecVisibility::Hidden
    }
}

fn is_shown(self) -> bool {
    self == ExecVisibility::Shown
}

fn toggle(&mut self) {
    *self = match *self {
        ExecVisibility::Hidden => ExecVisibility::Shown,
        ExecVisibility::Shown => ExecVisibility::Hidden,
    };
}

fn label(self) -> &'static str {
    match self {
        ExecVisibility::Hidden => "hidden",
        ExecVisibility::Shown => "shown",
    }
}
```

`Default`는 구현하지 않는다. 초기값은 반드시 기존 CLI bool에서 명시적으로
만든다.

### 7.2 selector 타입과 API

```rust
pub(crate) struct SelectorApp {
    rows: Vec<SessionRow>,
    filtered: Vec<SessionRow>,
    list_state: ListState,
    query: String,
    search_scope: SearchScope,
    mode: Mode,
    focus: PaneFocus,
    metadata_scroll: usize,
    message_scroll: u16,
    show_help: bool,
    exec_visibility: ExecVisibility,
    status: Option<String>,
}

impl SelectorApp {
    pub(crate) fn new(
        rows: Vec<SessionRow>,
        exec_visibility: ExecVisibility,
    ) -> Self;

    pub(crate) fn exec_visibility(&self) -> ExecVisibility;
    pub(crate) fn set_exec_visibility(&mut self, visibility: ExecVisibility);

    fn toggle_exec_visibility(&mut self);
    fn handle_key(&mut self, key: KeyEvent) -> Option<SelectorAction>;
}
```

`SelectorAction`은 변경하지 않는다.

```rust
pub(crate) enum SelectorAction {
    Quit,
    OpenReplay(PathBuf),
}
```

`set_exec_visibility`는 visibility field만 변경한다. session filter,
selection, query, focus와 scroll은 변경하지 않는다.

### 7.3 replay entry와 app

```rust
#[derive(Debug, Clone)]
struct Entry {
    kind: EntryKind,
    summary: String,
    detail: String,
}

struct ReplayApp {
    all_entries: Vec<Entry>,
    visible_indices: Vec<usize>,
    list_state: ListState,
    detail_scroll: u16,
    focus: PaneFocus,
    fullscreen: Fullscreen,
    show_help: bool,
    exec_visibility: ExecVisibility,
    status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayControl {
    Continue,
    Quit,
}
```

필수 method와 helper signature는 다음과 같다.

```rust
impl ReplayApp {
    fn new(
        all_entries: Vec<Entry>,
        exec_visibility: ExecVisibility,
    ) -> Self;

    fn selected_visible_index(&self) -> Option<usize>;
    fn selected_all_index(&self) -> Option<usize>;
    fn selected_entry(&self) -> Option<(usize, &Entry)>;
    fn rebuild_visible_indices(&mut self, previous_all_index: Option<usize>);
    fn toggle_exec_visibility(&mut self);
    fn handle_key(&mut self, key: KeyEvent) -> ReplayControl;
}

fn visible_entry_indices(
    entries: &[Entry],
    visibility: ExecVisibility,
) -> Vec<usize>;

fn display_title(visible_index: usize, entry: &Entry) -> String;

fn load_entries_from_str(input: &str) -> Result<Vec<Entry>>;

fn to_entry(event: PayloadEvent) -> Entry;

fn to_exec_tool_entry(
    call_id: Option<String>,
    kind: &str,
    name: &str,
    input: &str,
) -> Entry;

fn detail_text(entry: &Entry, visible_index: usize) -> Text<'static>;

pub(crate) fn run(options: &ReplayOptions) -> Result<ExecVisibility>;

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: ReplayApp,
) -> Result<ExecVisibility>;
```

`Entry.index`는 제거하고 visible index를 render 시 전달한다. 기존
`Entry.title`은 leading number를 제외한 `summary`로 바꾼다.

summary exact 형식은 다음과 같다.

```text
USER
USER [phase]
AGENT
AGENT [phase]
EXEC {command_summary}
```

`display_title`은 다음을 반환한다.

```rust
format!("#{visible_index:04} {}", entry.summary)
```

### 7.4 application API

```rust
fn replay_selected(
    select_options: &SelectOptions,
    path: PathBuf,
    exec_visibility: ExecVisibility,
) -> Result<ExecVisibility>;

fn external_replay_args(
    path: &Path,
    exec_visibility: ExecVisibility,
) -> Vec<OsString>;

fn run_external_replay(
    program: &str,
    path: &Path,
    exec_visibility: ExecVisibility,
) -> Result<ExecVisibility>;
```

external replay가 성공하면 input visibility를 그대로 반환한다. child
process가 내부에서 바꾼 상태를 추측하지 않는다.

## 8. 주요 흐름과 알고리즘 pseudocode

### 8.1 selector 시작과 refresh

refresh는 TUI toggle 전에 실행되므로 original CLI bool을 그대로 사용한다.

```rust
fn run_select(options: SelectOptions) -> Result<()> {
    if options.refresh {
        // 기존 계약: options.include_exec를 internal/external indexer에 전달한다.
        refresh_database(&options)?;
    }

    let rows = load_sessions(&options.db)?;
    if rows.is_empty() {
        bail!("no sessions found in {}", options.db.display());
    }

    let initial_visibility =
        ExecVisibility::from_include_exec(options.include_exec);
    let mut app = SelectorApp::new(rows, initial_visibility);

    if options.print_path {
        if let SelectorAction::OpenReplay(path) = selector::run(&mut app)? {
            println!("{}", path.display());
        }
        return Ok(());
    }

    loop {
        match selector::run(&mut app)? {
            SelectorAction::Quit => return Ok(()),
            SelectorAction::OpenReplay(path) => {
                let before_replay = app.exec_visibility();
                match replay_selected(&options, path, before_replay) {
                    Ok(after_replay) => {
                        app.set_exec_visibility(after_replay);
                        app.clear_status();
                    }
                    Err(error) => {
                        // 실패한 replay의 partial state는 반환되지 않는다.
                        // selector는 before_replay를 유지한다.
                        app.set_status(error.to_string());
                    }
                }
            }
        }
    }
}
```

### 8.2 selector key 처리

event loop는 현재처럼 `KeyEventKind::Press`만 `handle_key`에 넘긴다.

```rust
fn handle_key(&mut self, key: KeyEvent) -> Option<SelectorAction> {
    if key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return Some(SelectorAction::Quit);
    }

    if self.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => self.show_help = false,
            KeyCode::Char('q') => return Some(SelectorAction::Quit),
            _ => {}
        }
        return None;
    }

    match self.mode {
        Mode::Normal => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                Some(SelectorAction::Quit)
            }
            KeyCode::Enter => self
                .selected_path()
                .map(SelectorAction::OpenReplay),
            KeyCode::Char('e') if key.modifiers.is_empty() => {
                self.toggle_exec_visibility();
                None
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                None
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                None
            }
            // 기존 focus/movement/scroll/clipboard key를 내용 변경 없이 이동한다.
            _ => {
                handle_existing_normal_key(self, key);
                None
            }
        },
        Mode::Search => {
            // 기존 search 동작을 유지한다.
            // KeyCode::Char('e')도 이 branch에서 query에 append한다.
            handle_existing_search_key(self, key);
            None
        }
    }
}
```

위 pseudocode의 `handle_existing_normal_key`와
`handle_existing_search_key`는 설명용 이름이다. production helper로
추가하지 않고 현재 match arm을 `handle_key` 안으로 이동한다.

### 8.3 replay parse

initial visibility를 parse filter에 전달하지 않는다.

```rust
fn load_entries_from_str(input: &str) -> Result<Vec<Entry>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let values = parse_json_values(trimmed)?;
    let mut entries = Vec::new();
    let mut exec_by_call_id: HashMap<String, usize> = HashMap::new();

    for value in values {
        let Some(event) = normalize_record(value)? else {
            continue;
        };

        match event {
            NormalizedEvent::Payload(event) => {
                // ExecCommandEnd도 항상 Entry로 만든다.
                entries.push(to_entry(event));
            }
            NormalizedEvent::ExecToolCall {
                call_id,
                kind,
                name,
                input,
            } => {
                let all_index = entries.len();
                entries.push(to_exec_tool_entry(
                    call_id.clone(),
                    &kind,
                    &name,
                    &input,
                ));
                if let Some(call_id) = call_id {
                    exec_by_call_id.insert(call_id, all_index);
                }
            }
            NormalizedEvent::ExecToolOutput { call_id, output } => {
                if let Some(all_index) = call_id
                    .as_deref()
                    .and_then(|id| exec_by_call_id.get(id))
                    .copied()
                {
                    append_exec_output(&mut entries[all_index], &output);
                }
            }
        }
    }

    Ok(entries)
}
```

unmatched output과 non-exec tool output은 현재처럼 timeline entry를 만들지
않는다.

### 8.4 initial visible list

```rust
fn visible_entry_indices(
    entries: &[Entry],
    visibility: ExecVisibility,
) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(all_index, entry)| {
            let visible =
                visibility.is_shown() || !matches!(entry.kind, EntryKind::Exec);
            visible.then_some(all_index)
        })
        .collect()
}

fn new(
    all_entries: Vec<Entry>,
    exec_visibility: ExecVisibility,
) -> ReplayApp {
    let visible_indices =
        visible_entry_indices(&all_entries, exec_visibility);
    let mut list_state = ListState::default();
    if !visible_indices.is_empty() {
        list_state.select(Some(0));
    }

    ReplayApp {
        all_entries,
        visible_indices,
        list_state,
        detail_scroll: 0,
        focus: PaneFocus::Timeline,
        fullscreen: Fullscreen::None,
        show_help: false,
        exec_visibility,
        status: None,
    }
}
```

### 8.5 toggle과 selection 복원

```rust
fn toggle_exec_visibility(&mut self) {
    let previous_all_index = self.selected_all_index();
    self.exec_visibility.toggle();
    self.rebuild_visible_indices(previous_all_index);
    self.detail_scroll = 0;
    self.status = None;
}

fn rebuild_visible_indices(
    &mut self,
    previous_all_index: Option<usize>,
) {
    self.visible_indices =
        visible_entry_indices(&self.all_entries, self.exec_visibility);

    if self.visible_indices.is_empty() {
        self.list_state.select(None);
        return;
    }

    let selected_visible_index = previous_all_index
        .and_then(|all_index| {
            self.visible_indices
                .iter()
                .position(|candidate| *candidate == all_index)
        })
        .or_else(|| {
            previous_all_index.and_then(|all_index| {
                self.visible_indices
                    .iter()
                    .position(|candidate| *candidate > all_index)
            })
        })
        .or_else(|| previous_all_index.map(|_| self.visible_indices.len() - 1))
        .unwrap_or(0);

    self.list_state.select(Some(selected_visible_index));
}
```

`visible_indices`는 항상 ascending이므로 다음/이전 선택 규칙이 결정적이다.

### 8.6 replay key 처리와 종료

```rust
fn handle_key(&mut self, key: KeyEvent) -> ReplayControl {
    if key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return ReplayControl::Quit;
    }

    if self.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => self.show_help = false,
            KeyCode::Char('q') => return ReplayControl::Quit,
            _ => {}
        }
        return ReplayControl::Continue;
    }

    match key.code {
        KeyCode::Char('q') => ReplayControl::Quit,
        KeyCode::Esc => {
            if self.fullscreen != Fullscreen::None {
                self.exit_fullscreen();
                ReplayControl::Continue
            } else {
                ReplayControl::Quit
            }
        }
        KeyCode::Char('e') if key.modifiers.is_empty() => {
            self.toggle_exec_visibility();
            ReplayControl::Continue
        }
        KeyCode::Char('?') => {
            self.show_help = true;
            ReplayControl::Continue
        }
        // 기존 focus/fullscreen/movement/copy key를 내용 변경 없이 처리한다.
        _ => {
            handle_existing_replay_key(self, key);
            ReplayControl::Continue
        }
    }
}
```

`handle_existing_replay_key`는 설명용 이름이며 production helper로 추가하지
않는다.

event loop는 quit 시 현재 visibility를 반환한다.

```rust
fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: ReplayApp,
) -> Result<ExecVisibility> {
    loop {
        terminal.draw(|frame| render(frame, &mut app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if app.handle_key(key) == ReplayControl::Quit {
                    return Ok(app.exec_visibility);
                }
            }
        }
    }
}
```

### 8.7 internal/external replay orchestration

```rust
fn replay_selected(
    select_options: &SelectOptions,
    path: PathBuf,
    exec_visibility: ExecVisibility,
) -> Result<ExecVisibility> {
    if let Some(program) = select_options.replay_command.as_deref() {
        return run_external_replay(program, &path, exec_visibility);
    }

    replay::run(&ReplayOptions {
        input: Some(path),
        include_exec: exec_visibility.is_shown(),
    })
}

fn external_replay_args(
    path: &Path,
    exec_visibility: ExecVisibility,
) -> Vec<OsString> {
    let mut args = Vec::new();
    if exec_visibility.is_shown() {
        args.push(OsString::from("--include-exec"));
    }
    args.push(path.as_os_str().to_owned());
    args
}

fn run_external_replay(
    program: &str,
    path: &Path,
    exec_visibility: ExecVisibility,
) -> Result<ExecVisibility> {
    let status = std::process::Command::new(program)
        .args(external_replay_args(path, exec_visibility))
        .status()?;

    if !status.success() {
        bail!(
            "{} exited with status {}",
            program,
            status.code().unwrap_or(1)
        );
    }

    Ok(exec_visibility)
}
```

standalone replay command는 반환 상태를 의도적으로 버린다.

```rust
Command::Replay(options) => {
    replay::run(&options)?;
    Ok(())
}
```

## 9. 오류 처리, 상태 소유권과 lifecycle

### 9.1 입력과 parse 오류

- file open/read 오류는 terminal 진입 전에 기존 `anyhow::Context`와 함께
  반환한다.
- invalid JSON array/stream 오류도 terminal 진입 전에 반환한다.
- unsupported record는 현재처럼 무시한다.
- unmatched exec output은 현재처럼 무시한다.
- visibility toggle 자체는 I/O를 하지 않으므로 새 runtime 오류를 만들지
  않는다.

### 9.2 terminal 오류

- `terminal::with_terminal`의 초기화, draw, poll 또는 read 오류는 기존처럼
  `Result`로 호출자에게 전달한다.
- internal replay에서 오류가 발생하면 final visibility가 반환되지 않는다.
  selector는 replay를 열기 전 visibility를 유지하고 error text를 status에
  표시한다.
- terminal 복구 책임은 기존 `terminal::with_terminal`에 유지한다.

### 9.3 external process 오류

- external recorder 동작과 오류는 변경하지 않는다.
- external replay program spawn 오류는 기존처럼 반환한다.
- non-zero exit status는 기존 exact 형식의 error로 반환한다.
- external replay 성공 시 selector visibility는 실행 전과 동일하다.
- child process의 TUI 내부 상태를 추측하거나 stdout protocol을 추가하지
  않는다.

### 9.4 empty state

- input에 supported entry가 없으면 `all_entries`와 `visible_indices`가 모두
  비고 selection은 `None`이다.
- exec entry만 있는 input을 hidden으로 열면 `all_entries`는 비어 있지
  않지만 visible selection은 `None`이다.
- 이 상태에서 `e`를 누르면 첫 exec entry를 선택한다.
- shown 상태에서 유일한 exec entry를 숨기면 selection은 `None`이 된다.
- empty selection에서 movement, copy와 toggle은 panic하지 않는다.
- detail pane은 기존 `"No entries"` 문구를 유지한다.

### 9.5 상태 생명주기

- selector process 시작: CLI bool로 초기화
- selector `e`: selector state 변경
- internal replay 진입: selector state를 copy
- replay `e`: replay state 변경
- 정상 replay 종료: final state 반환 및 selector 갱신
- replay 오류: selector의 진입 전 state 유지
- external replay 정상/오류: selector state 변경 없음
- selector 종료: state 폐기

visibility는 disk, environment variable, SQLite 또는 config에 기록하지
않는다.

## 10. 단계별 구현 순서

각 phase는 독립적으로 green 상태가 되어야 하며 phase 사이에 깨진 commit을
남기지 않는다.

### Phase 1. Shared visibility type

변경 파일:

```text
src/lib.rs
src/ui_state.rs
```

순서:

1. `src/ui_state.rs` unit test를 먼저 작성한다.
2. 대상 test가 module/type 부재로 compile 실패하는 red를 확인한다.
3. `ExecVisibility`를 구현하고 `src/lib.rs`에 module을 선언한다.
4. `ui_state` 대상 test와 전체 quality gate를 통과시킨다.

커밋 가능 조건:

- bool 변환, toggle과 exact label test가 green이다.
- selector, replay와 SQLite test가 변함없이 green이다.

권장 commit message:

```text
refactor: add shared exec visibility state
```

### Phase 2. Standalone replay dynamic filter와 selection

변경 파일:

```text
src/replay/mod.rs
src/application.rs
```

순서:

1. 모든 entry parse, initial filtering, toggle과 selection test를 먼저
   작성한다.
2. 기존 `load_entries_from_str(input, include_exec)` signature 때문에
   예상한 compile/assertion red가 발생하는지 확인한다.
3. `Entry.index`/`title`을 `summary` 기반 모델로 변경한다.
4. parser에서 visibility 조건을 제거하고 exec call/output을 항상 결합한다.
5. `ReplayApp.all_entries`, `visible_indices`, `exec_visibility`를 구현한다.
6. render/list/detail/copy가 visible index를 사용하도록 변경한다.
7. replay key handler, fullscreen toggle과 modal help를 구현한다.
8. `replay::run`이 final `ExecVisibility`를 반환하도록 변경한다.
9. `Command::Replay` branch는 반환된 visibility를 버리고 `Ok(())`를
   반환하도록 변경한다.
10. selector에서 호출하는 기존 `replay_selected`는 이 phase에서
    `replay::run(...).map(|_| ())`로 final visibility를 명시적으로 버린다.
    selector 왕복은 Phase 3에서 추가한다.
11. 대상 test와 전체 quality gate를 통과시킨다.

커밋 가능 조건:

- hidden/shown initial list가 정확하다.
- 반복 toggle이 JSON parse 없이 같은 `all_entries`를 사용한다.
- selection 복원과 empty selection test가 green이다.
- 기존 application external argument contract test가 green이다.
- SQLite schema test가 green이다.

권장 commit message:

```text
feat: toggle exec entries in replay
```

### Phase 3. Selector toggle과 application 왕복

변경 파일:

```text
src/selector/mod.rs
src/application.rs
```

순서:

1. selector initial state, normal/search/help key와 상태 보존 test를 먼저
   작성한다.
2. 기존 constructor와 application 고정 bool 전달 때문에 예상한
   compile/assertion red가 발생하는지 확인한다.
3. `SelectorApp`에 visibility field, constructor argument, getter/setter와
   toggle method를 추가한다.
4. selector key handling을 pure method로 분리하고 modal help를 구현한다.
5. selector header/footer/help를 변경한다.
6. application이 current selector state로 internal/external replay를 열고
   internal replay 결과만 selector에 반영하도록 변경한다.
7. 대상 test와 전체 quality gate를 통과시킨다.

커밋 가능 조건:

- selector normal/search/help key test가 green이다.
- replay에서 바꾼 state의 selector 반영 경로가 green이다.
- external replay는 실행 전 state를 그대로 반환한다.
- 기존 selector search/focus/clipboard helper test가 green이다.
- SQLite test가 변함없이 green이다.

권장 commit message:

```text
feat: preserve exec visibility across selector replay
```

### Phase 4. CLI, 문서와 package 검증

변경 파일:

```text
README.md
src/cli.rs
tests/cli.rs
```

순서:

1. CLI help integration assertion을 먼저 추가해 red를 확인한다.
2. root/replay help의 option 의미와 `e` key를 갱신한다.
3. README Quick Start, Selector controls, Replay controls와 SQLite 설명을
   갱신한다.
4. `Cargo.toml`과 `Cargo.lock`의 local package version이 모두
   `0.3.0`인지 확인하고 변경하지 않는다.
5. full quality gate, release build, 임시 install과 manual TUI smoke를
   실행한다.

커밋 가능 조건:

- README와 실제 `--help`가 일치한다.
- release와 임시 install 결과가 단일 binary다.
- `--version`은 계속 `select-codex-session 0.3.0`을 출력한다.
- publish, release tag와 GitHub release를 생성하지 않았다.
- manual smoke의 모든 상태 전이가 기대대로 동작한다.
- 비범위 기능이 추가되지 않았다.

권장 commit message:

```text
docs: document runtime exec visibility toggle
```

## 11. Red/green 및 커밋 정책

모든 phase는 green-only commit 정책을 따른다.

```text
새 test 작성
  ↓
대상 test가 예상한 이유로 실패하는지 확인
  ↓
같은 phase에서 production code 구현
  ↓
대상 test green
  ↓
scripts/check-before-commit.sh green
  ↓
git diff --check green
  ↓
test와 구현을 함께 commit
```

- compile error를 포함한 red 상태를 commit하지 않는다.
- 기존 test를 삭제하거나 assertion을 약화해 green으로 만들지 않는다.
- 기존 test 이름을 새 모델에 맞춰 바꿀 수 있지만 같은 입력 의미와 출력
  계약은 새 test에서 유지한다.
- 처음부터 green인 characterization test만 구현과 분리해 commit할 수
  있다.
- phase 중단 시 실패 상태를 임시 commit하지 않는다.
- commit 생성 자체는 구현 세션에서 사용자가 요청한 경우에만 실행한다.

대상 test 명령은 다음 형식을 사용한다.

```bash
cargo test ui_state::tests::
cargo test selector::tests::
cargo test replay::tests::
cargo test application::tests::
cargo test --test cli
```

## 12. Pre-commit과 CI 자동화

현재 automation을 그대로 사용한다.

```text
scripts/check-before-commit.sh
.githooks/pre-commit
scripts/install-git-hooks.sh
.github/workflows/ci.yml
```

공통 script는 다음을 실행한다.

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

이번 기능에서 hook, installer, CI workflow와 공통 script를 수정하지 않는다.
검사 명령을 다른 script나 workflow에 복제하지 않는다.

각 phase 완료 시 다음을 실행한다.

```bash
scripts/check-before-commit.sh
git diff --check
```

최종 완료 시 추가로 실행한다.

```bash
cargo build --release
```

## 13. Unit, integration과 manual test

### 13.1 `src/ui_state.rs` unit tests

#### `visibility_maps_cli_bool_and_toggles`

입력과 assertion:

```rust
let mut hidden = ExecVisibility::from_include_exec(false);
assert_eq!(hidden, ExecVisibility::Hidden);
assert!(!hidden.is_shown());
assert_eq!(hidden.label(), "hidden");

hidden.toggle();
assert_eq!(hidden, ExecVisibility::Shown);
assert!(hidden.is_shown());
assert_eq!(hidden.label(), "shown");

hidden.toggle();
assert_eq!(hidden, ExecVisibility::Hidden);
```

### 13.2 selector unit tests

기존 `sample_rows()` 두 row fixture를 사용한다.

#### `selector_initial_visibility_matches_cli_state`

- `SelectorApp::new(rows.clone(), Hidden)`과 `Shown`을 각각 생성한다.
- getter가 입력 enum을 exact하게 반환하는지 assert한다.
- 두 app의 `filtered.len()`이 모두 `rows.len()`인지 assert한다.

#### `selector_e_toggles_only_in_normal_mode`

- normal mode app에 modifier 없는 `e`를 전달한다.
- `Hidden → Shown`을 assert한다.
- search mode로 바꾸고 `e`를 전달한다.
- visibility가 `Shown`으로 유지되고 query가 `"e"`인지 assert한다.
- `Ctrl-e`를 normal mode에 전달했을 때 visibility가 바뀌지 않는지
  assert한다.

#### `selector_help_is_modal_for_exec_toggle`

- `?`로 help를 연다.
- `e`, `j`, `Tab`을 순서대로 전달한다.
- visibility, selected index와 focus가 바뀌지 않는지 assert한다.
- `Esc`를 전달하면 action이 없고 help만 닫히는지 assert한다.

#### `selector_toggle_preserves_existing_state`

- 두 번째 row를 선택하고 query, `SearchScope::Branch`,
  `PaneFocus::Message`, metadata/message scroll을 non-zero로 설정한다.
- normal mode에서 `e`를 전달한다.
- visibility 외 모든 field가 exact하게 유지되는지 assert한다.
- transient error status만 `None`으로 clear되는지 assert한다.

#### `selector_toggle_is_safe_with_empty_search_result`

- match하지 않는 query로 `filtered`와 selection을 비운다.
- normal mode에서 `e`를 전달한다.
- selection이 `None`, filtered가 empty인 채 visibility만 바뀌는지
  assert한다.

### 13.3 replay parser와 state unit tests

공통 fixture 순서는 다음과 같다.

```text
USER "hello"
EXEC legacy "pwd"
AGENT "done"
EXEC tool call "git status" + matching output
```

#### `loads_all_supported_entries_regardless_of_initial_visibility`

- `load_entries_from_str(input)`을 한 번 호출한다.
- 결과 길이가 `4`인지 assert한다.
- kind 순서가 `User, Exec, Agent, Exec`인지 assert한다.
- tool output이 마지막 Exec detail에 결합됐는지 assert한다.
- unmatched output이 어떤 detail에도 들어가지 않았는지 assert한다.

#### `initial_visibility_filters_without_dropping_entries`

- 동일 `Vec<Entry>`로 Hidden과 Shown app을 각각 만든다.
- Hidden의 `all_entries.len() == 4`와
  `visible_indices == vec![0, 2]`를 assert한다.
- Shown의 `all_entries.len() == 4`와
  `visible_indices == vec![0, 1, 2, 3]`을 assert한다.

#### `toggle_preserves_selected_non_exec_entry`

- Hidden app에서 Agent를 선택한다.
- Shown으로 toggle한다.
- selected all index가 계속 `2`인지 assert한다.
- visible display index가 `1`에서 `2`로 변경됐는지 assert한다.
- 다시 Hidden으로 toggle하고 selected all index `2`, display index `1`을
  assert한다.

#### `hiding_selected_exec_prefers_next_then_previous`

- Shown app에서 첫 Exec all index `1`을 선택하고 Hidden으로 toggle한다.
- 다음 visible Agent all index `2`가 선택되는지 assert한다.
- 별도 fixture `User, Exec`에서 마지막 Exec를 숨기면 이전 User가
  선택되는지 assert한다.

#### `only_exec_entries_transition_through_empty_selection`

- Exec entry 하나로 Hidden app을 만든다.
- `all_entries.len() == 1`, visible empty, selection `None`을 assert한다.
- Shown으로 toggle하면 visible index `[0]`, selection `Some(0)`을
  assert한다.
- 다시 Hidden이면 visible empty, selection `None`을 assert한다.

#### `display_titles_remain_dense_for_each_visibility`

- 공통 fixture에서 Hidden title이 exact하게
  `#0000 USER`, `#0001 AGENT`인지 assert한다.
- Shown title이 `#0000 USER`, `#0001 EXEC pwd`, `#0002 AGENT`,
  `#0003 EXEC git status`인지 assert한다.
- detail의 `index:` 값도 각각 visible position인지 assert한다.

#### `replay_help_is_modal_and_fullscreen_allows_toggle`

- Shown app에서 help를 열고 `e`, `1`, `Tab`을 전달한다.
- visibility, fullscreen과 focus가 바뀌지 않는지 assert한다.
- `Esc`로 help만 닫는다.
- timeline fullscreen을 켠 후 `e`를 전달한다.
- fullscreen은 Timeline으로 유지되고 visibility만 Hidden인지 assert한다.

#### `repeated_toggle_reuses_all_entries`

- 10,000개의 alternating User/Exec in-memory `Entry`를 만든다.
- app을 만든 뒤 100회 toggle한다.
- `all_entries.len()`이 항상 `10_000`이고 최종 visible count가 초기 상태와
  일치하는지 assert한다.
- wall-clock assertion은 CI 변동 때문에 두지 않는다.
- 이 test는 toggle이 file read/JSON parse API를 호출하지 않고 O(n)
  in-memory filter만 수행하는 구조를 고정한다.

#### `invalid_json_error_is_independent_of_visibility`

- `load_entries_from_str("{invalid")`가 error를 반환하는지 assert한다.
- visibility argument가 parser에서 제거되어 오류가 한 경로에서만
  발생한다는 것을 고정한다.

### 13.4 application unit tests

#### `external_replay_args_use_current_visibility`

- Hidden이면 exact args가 `[path]`인지 assert한다.
- Shown이면 exact args가 `["--include-exec", path]`인지 assert한다.
- 기존 argument order를 유지한다.

#### `external_record_args_still_use_original_cli_bool`

- 기존 `external_record_args_match_legacy_contract`를 유지한다.
- false/true exact vector assertion을 변경하지 않는다.

### 13.5 CLI unit와 integration tests

#### `help_texts_describe_initial_visibility_and_toggle_key`

- root help가
  `"Index exec records and initially show them"`을 포함하는지 assert한다.
- root help가 `"e                              toggle exec entries for replay"`를
  포함하는지 assert한다.
- replay help가 `"default: hidden; press e to toggle"`을 포함하는지
  assert한다.
- replay help가 `"e                  toggle command execution entries"`를
  포함하는지 assert한다.
- index help의 `"Also create and populate exec_events"`가 유지되는지
  assert한다.

#### `root_help_lists_exec_toggle_key`

`tests/cli.rs`에서 실제 binary `--help` stdout에 다음 두 substring이 있는지
assert한다.

```text
--include-exec
toggle exec entries for replay
```

#### `replay_help_lists_exec_toggle_key`

실제 binary `replay --help` stdout에 다음을 assert한다.

```text
default: hidden; press e to toggle
toggle command execution entries
```

### 13.6 SQLite와 비범위 negative tests

기존 test를 삭제하거나 변경하지 않고 전체 suite에서 다음을 확인한다.

- `build_index_without_exec_preserves_default_schema`
  - `sessions` row `1`
  - `exec_events` 없음
  - `sessions_fts` 없음
- `build_index_with_exec_preserves_current_exec_schema`
  - `sessions` row `1`
  - `exec_events` row `3`
  - `sessions_fts` 없음
- `recreate_database_creates_exec_table_only_when_requested`
  - 기존 optional table 생성/삭제 계약 유지
- `exec_text_is_not_a_selector_search_scope`
  - selector search scope에 exec가 추가되지 않음

새 schema, migration metadata, fingerprint column 또는 FTS table이
생성되지 않아야 한다.

### 13.7 Manual TUI smoke test

먼저 debug binary를 build한다.

```bash
cargo build
```

#### Direct replay

```bash
target/debug/select-codex-session replay tests/fixtures/session.jsonl
```

확인 순서:

1. header가 `exec: hidden`, visible `2`, all `3`을 표시한다.
2. timeline에는 USER와 AGENT만 보이고 EXEC는 보이지 않는다.
3. `e`를 누르면 header가 `exec: shown`, visible `3`, all `3`이 된다.
4. timeline 순서가 USER, AGENT, EXEC가 아니라 fixture 원본 정규화 순서인
   USER, AGENT, EXEC임을 확인한다.
5. `e`를 다시 누르면 EXEC가 사라지고 선택 가능한 non-exec가 유지된다.
6. `1`과 `2`로 각각 fullscreen에 들어가 `e`가 동작하며 fullscreen은
   유지되는지 확인한다.
7. `?`로 help를 연 뒤 `e`, `j`, `Tab`, `1`이 뒤 상태를 바꾸지 않는지
   확인한다.
8. help에서 `Esc`는 help만 닫고, fullscreen이 아닐 때 다음 `Esc`가 replay를
   종료하는지 확인한다.

CLI 초기값도 확인한다.

```bash
target/debug/select-codex-session replay \
  --include-exec \
  tests/fixtures/session.jsonl
```

최초 header가 `exec: shown`이고 EXEC가 처음부터 보이는지 확인한다.

#### Selector/replay round trip

명시적으로 제한된 임시 directory를 만든다.

```bash
smoke_root="$(mktemp -d)"
mkdir -p "$smoke_root/home/.codex/sessions/2026/07/26"
cp tests/fixtures/session.jsonl \
  "$smoke_root/home/.codex/sessions/2026/07/26/session.jsonl"

HOME="$smoke_root/home" target/debug/select-codex-session
```

확인 순서:

1. selector header가 `exec: hidden`이다.
2. normal mode에서 `e`를 누르면 `exec: shown`이 되고 session selection은
   유지된다.
3. `/`로 search에 들어가 `e`를 입력하면 query에 `e`가 추가되고 visibility는
   바뀌지 않는다.
4. search에서 `Esc`, `Backspace` 또는 새 query로 fixture session이 다시
   보이게 한다.
5. `Enter`로 replay를 열면 EXEC가 보이는 상태로 시작한다.
6. replay에서 `e`로 hidden으로 바꾸고 `q`로 돌아온다.
7. selector header가 `exec: hidden`이고 기존 query, search scope, pane
   focus와 session selection이 유지되는지 확인한다.
8. selector help에서 `e`, movement와 `Tab`이 뒤 상태를 바꾸지 않고
   `Esc`가 help만 닫는지 확인한다.
9. `q`로 종료한다.

임시 directory를 정확한 resolved path로 제거한다.

```bash
rm -rf "$smoke_root"
```

### 13.8 최종 자동 검증

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
git diff --check
```

## 14. Package와 문서 변경

### 14.1 Version

`Cargo.toml` package version과 `Cargo.lock`의 local package version은
모두 `0.3.0`을 유지한다. 2번 구현에서는 version, dependency, feature,
binary target, edition과 `rust-version`을 변경하지 않는다.

`0.3.0`은 2번만 완료한 시점에 공개하지 않는다. 별도 canonical/증분 index
기능과 3번 FTS5까지 모두 구현하고 각 계획의 완료 조건과 통합 회귀 검증을
통과한 뒤에만 crate publish, release tag와 GitHub release 대상으로 삼는다.

### 14.2 README

다음을 갱신한다.

- Quick Start에서 `--include-exec`가 initial state임을 설명
- selector header의 visibility와 `e` control
- replay에서 parse 후 dynamic filter한다는 사용자 수준 설명
- replay controls에 `e`
- internal replay 변경이 selector에 유지됨
- external replay 변경은 selector로 반환되지 않음
- SQLite section에서 TUI toggle이 schema를 변경하지 않으며 기존
  `index --include-exec` 계약이 유지됨
- canonical/증분 index는 아직 구현되지 않았음을 명시

one-binary refactor 계획과 handoff는 완료된 historical document이므로
수정하지 않는다.

### 14.3 CLI help

root와 replay help만 변경한다. index help와 option parser는 변경하지
않는다. help와 README의 exact key와 initial-state 의미를 일치시킨다.

### 14.4 설치 산출물 검증

명시적으로 제한된 임시 root에 install한다.

```bash
install_root="$(mktemp -d)"
cargo install --path . --root "$install_root"

test -x "$install_root/bin/select-codex-session"
test "$(find "$install_root/bin" -maxdepth 1 -type f | wc -l)" -eq 1
"$install_root/bin/select-codex-session" --version

rm -rf "$install_root"
```

version stdout는 다음 exact 형식이어야 한다.

```text
select-codex-session 0.3.0
```

이 install은 local artifact 검증일 뿐 공개 작업이 아니다.

## 15. 완료 조건

다음 조건을 모두 만족해야 기능 완료다.

- `e`가 selector normal mode와 replay에서 exec visibility를 toggle한다.
- selector search mode의 `e`는 query 입력이다.
- help overlay에서 `e`와 일반 key가 뒤 상태를 변경하지 않는다.
- replay fullscreen에서 `e`가 동작하고 fullscreen 상태가 유지된다.
- CLI option이 initial visibility를 결정한다.
- replay는 입력을 한 번 parse하고 모든 normalized entry를 보존한다.
- toggle은 `visible_indices`만 rebuild한다.
- hidden/shown 상태에서 display index가 각각 `#0000`부터 조밀하다.
- non-exec selection 보존과 selected-exec fallback 규칙이 test로 고정됐다.
- empty/only-exec input에서 panic하지 않는다.
- internal replay의 final visibility가 selector에 유지된다.
- external replay의 기존 argument contract가 유지된다.
- replay 오류 시 selector가 진입 전 visibility를 유지한다.
- selector query, scope, focus, scroll과 selection이 replay 복귀 후 유지된다.
- header에 exact `exec: hidden` 또는 `exec: shown`이 표시된다.
- footer, TUI help, CLI help와 README에 `e`가 설명된다.
- `index --include-exec`와 SQLite schema/rebuild 동작이 변경되지 않았다.
- subsession/empty-message toggle, incremental index와 FTS가 추가되지 않았다.
- 새 dependency와 Cargo feature가 없다.
- package version은 계속 `0.3.0`이고 설치 binary는 하나다.
- crate publish, release tag와 GitHub release를 생성하지 않았다.
- 모든 unit/integration test와 manual smoke가 통과한다.
- fmt, clippy, full test, release build와 `git diff --check`가 통과한다.

## 16. 후속 작업 경계

2번 완료 후 별도
`Canonical and Incremental SQLite Index Implementation Plan`을 먼저 작성하고
구현한다. 그 계획은 다음을 소유한다.

- canonical/superset index 계약
- stable session/exec row identity
- schema version과 legacy DB migration
- file fingerprint
- 신규/변경 file upsert와 삭제 file 정리
- transaction, rollback과 forced rebuild
- 기존 include 계열 index option migration
- index 시간과 DB 크기 benchmark

그 뒤 3번 FTS 검색 강화 계획이 incremental index가 제공하는 stable identity
및 changed/deleted row API를 사용한다.

2번, canonical/증분 index와 3번 FTS5 구현을 모두 완료하고 통합 검증을
통과할 때까지 package version은 `0.3.0`으로 유지하되 외부에 공개하지
않는다. 최종 공개 계획은 세 기능의 완료 commit과 release artifact를 함께
검증해야 한다.

이번 2번 구현은 후속 작업을 위한 schema, metadata table, fingerprint,
row key, FTS table 또는 search API를 미리 추가하지 않는다. 두 후속 기능의
계획이 작성되기 전까지 현재 SQLite 전체 rebuild와 메모리 기반 session
검색을 그대로 유지한다.
