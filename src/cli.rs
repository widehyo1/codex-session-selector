use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayOptions {
    pub input: Option<PathBuf>,
    pub include_exec: bool,
}

impl SelectOptions {
    pub(crate) fn defaults() -> Result<Self> {
        Ok(Self {
            db: home_dir()?.join("codex-session-info.sqlite3"),
            refresh: true,
            print_path: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
            record_command: None,
            replay_command: None,
        })
    }
}

impl IndexOptions {
    pub(crate) fn defaults() -> Result<Self> {
        let home = home_dir()?;
        Ok(Self {
            output: home.join("codex-session-info.sqlite3"),
            sessions_root: home.join(".codex").join("sessions"),
            rebuild: false,
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

pub(crate) fn parse_args(mut args: impl Iterator<Item = String>) -> Result<CliAction> {
    let Some(first) = args.next() else {
        return Ok(CliAction::Run(Command::Select(SelectOptions::defaults()?)));
    };

    match first.as_str() {
        "index" => parse_index_args(args),
        "replay" => parse_replay_args(args),
        "-h" | "--help" => Ok(CliAction::PrintHelp(HelpTopic::Root)),
        "-V" | "--version" => Ok(CliAction::PrintVersion),
        _ => parse_select_args(std::iter::once(first).chain(args)),
    }
}

fn parse_select_args(args: impl Iterator<Item = String>) -> Result<CliAction> {
    let mut options = SelectOptions::defaults()?;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => {
                options.db = expand_home(&required_value(&mut args, &arg, "path")?)?;
            }
            "--record-command" => {
                options.record_command = Some(required_value(&mut args, &arg, "command")?);
            }
            "--replay-command" => {
                options.replay_command = Some(required_value(&mut args, &arg, "command")?);
            }
            "--no-refresh" => options.refresh = false,
            "--print-path" => options.print_path = true,
            "--include-subsessions" => options.include_subsessions = true,
            "--include-empty-messages" => options.include_empty_messages = true,
            "--include-exec" => options.include_exec = true,
            "-h" | "--help" => return Ok(CliAction::PrintHelp(HelpTopic::Root)),
            "-V" | "--version" => return Ok(CliAction::PrintVersion),
            _ => bail!("unknown argument: {arg}"),
        }
    }

    Ok(CliAction::Run(Command::Select(options)))
}

fn parse_index_args(mut args: impl Iterator<Item = String>) -> Result<CliAction> {
    let mut options = IndexOptions::defaults()?;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                options.output = expand_home(&required_value(&mut args, &arg, "path")?)?;
            }
            "--sessions-root" => {
                options.sessions_root = expand_home(&required_value(&mut args, &arg, "path")?)?;
            }
            "--rebuild" => options.rebuild = true,
            "--include-subsessions" => options.include_subsessions = true,
            "--include-empty-messages" => options.include_empty_messages = true,
            "--include-exec" => options.include_exec = true,
            "-h" | "--help" => return Ok(CliAction::PrintHelp(HelpTopic::Index)),
            "-V" | "--version" => return Ok(CliAction::PrintVersion),
            _ => bail!("unknown index argument: {arg}"),
        }
    }

    Ok(CliAction::Run(Command::Index(options)))
}

fn parse_replay_args(args: impl Iterator<Item = String>) -> Result<CliAction> {
    let mut options = ReplayOptions::defaults();

    for arg in args {
        match arg.as_str() {
            "--include-exec" => options.include_exec = true,
            "-h" | "--help" => return Ok(CliAction::PrintHelp(HelpTopic::Replay)),
            "-V" | "--version" => return Ok(CliAction::PrintVersion),
            "-" => set_replay_input(&mut options, PathBuf::from("-"))?,
            _ if arg.starts_with('-') => bail!("unknown replay argument: {arg}"),
            _ => set_replay_input(&mut options, PathBuf::from(arg))?,
        }
    }

    Ok(CliAction::Run(Command::Replay(options)))
}

fn set_replay_input(options: &mut ReplayOptions, input: PathBuf) -> Result<()> {
    if options.input.replace(input).is_some() {
        bail!("only one input path may be provided");
    }
    Ok(())
}

fn required_value(
    args: &mut impl Iterator<Item = String>,
    option: &str,
    noun: &str,
) -> Result<String> {
    args.next()
        .ok_or_else(|| anyhow!("{option} requires a {noun}"))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set"))
}

fn expand_home(value: &str) -> Result<PathBuf> {
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    Ok(PathBuf::from(value))
}

pub(crate) fn help_text(topic: HelpTopic) -> String {
    match topic {
        HelpTopic::Root => root_help(),
        HelpTopic::Index => index_help(),
        HelpTopic::Replay => replay_help(),
    }
}

fn root_help() -> String {
    format!(
        "\
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
      --no-refresh               Do not refresh the SQLite index before opening
      --include-subsessions      Show subsessions from the canonical index
      --include-empty-messages   Show sessions without a non-empty first message
      --include-exec             Initially show exec records in replay
      --print-path               Print the selected JSONL path instead of replaying
  -h, --help                     Show this help
  -V, --version                  Show version

Keys:
  Enter                          replay selected session, then return here
  /                              interactive search
  Tab                            switch pane focus; while searching, cycle scope
  j/k or Up/Down                 move selection or scroll the focused message by one line
  d/u or Page keys               move selection or scroll the focused pane by half its height
  g/G or Home/End                first/last selection or message top/bottom (last line visible)
  h/l or Left/Right              horizontal scroll session metadata
  0                              reset horizontal scroll
  e                              toggle exec entries for replay
  y                              copy `codex resume <session-id>` to clipboard
  ?                              show help
  q, Ctrl-C                      quit
  Esc                            quit normally; leave search or help when open",
        version = env!("CARGO_PKG_VERSION")
    )
}

fn index_help() -> String {
    format!(
        "\
select-codex-session {version}

Build a SQLite index from local Codex session JSONL files.

Usage:
  select-codex-session index [INDEX_OPTIONS]

Options:
  -o, --output PATH              SQLite output path
                                 default: ~/codex-session-info.sqlite3
      --sessions-root PATH       Codex sessions root
                                 default: ~/.codex/sessions
      --include-subsessions      Compatibility option; canonical index always stores subsessions
      --include-empty-messages   Compatibility option; canonical index always stores empty messages
      --include-exec             Compatibility option; canonical index always stores exec events
      --rebuild                  Ignore fingerprints and rebuild the canonical index
  -h, --help                     Show this help
  -V, --version                  Show version

The canonical index always stores all valid sessions and exec events. Selector
visibility is controlled by selector options rather than index contents.",
        version = env!("CARGO_PKG_VERSION")
    )
}

fn replay_help() -> String {
    format!(
        "\
select-codex-session {version}

Replay a Codex session JSONL file in a terminal UI.

Usage:
  select-codex-session replay [REPLAY_OPTIONS] [PATH|-]

Options:
      --include-exec     Initially show command execution records
                         default: hidden; press e to toggle
  -h, --help             Show this help
  -V, --version          Show version

Input:
  PATH    Raw Codex JSONL file or a JSON array of preprocessed events
  -       Read JSONL/JSON from stdin
  omitted Read JSONL/JSON from stdin

Keys:
  Tab                switch focus between timeline/detail
  j/k or Up/Down     move or scroll, depending on focus
  d/u or Page keys   half-pane move or scroll
  g/G or Home/End    first/last event or detail top/bottom (last line visible)
  1/2/f              fullscreen controls
  e                  toggle command execution entries
  y                  copy detail pane when detail is focused
  q, Ctrl-C          quit
  Esc                quit normally; leave fullscreen or help when open",
        version = env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| (*value).to_owned())
    }

    #[test]
    fn no_args_defaults_to_selector() {
        let action = parse_args(std::iter::empty()).unwrap();
        let CliAction::Run(Command::Select(options)) = action else {
            panic!("expected selector");
        };
        assert!(options.refresh);
        assert!(!options.print_path);
        assert!(!options.include_subsessions);
        assert!(!options.include_empty_messages);
        assert!(!options.include_exec);
        assert_eq!(options.record_command, None);
        assert_eq!(options.replay_command, None);
    }

    #[test]
    fn selector_parses_current_options() {
        let action = parse_args(strings(&[
            "--db",
            "/tmp/sessions.sqlite3",
            "--no-refresh",
            "--print-path",
            "--include-subsessions",
            "--include-empty-messages",
            "--include-exec",
            "--record-command",
            "/tmp/recorder",
            "--replay-command",
            "/tmp/replay",
        ]))
        .unwrap();
        let CliAction::Run(Command::Select(options)) = action else {
            panic!("expected selector");
        };
        assert_eq!(options.db, PathBuf::from("/tmp/sessions.sqlite3"));
        assert!(!options.refresh);
        assert!(options.print_path);
        assert!(options.include_subsessions);
        assert!(options.include_empty_messages);
        assert!(options.include_exec);
        assert_eq!(options.record_command.as_deref(), Some("/tmp/recorder"));
        assert_eq!(options.replay_command.as_deref(), Some("/tmp/replay"));
    }

    #[test]
    fn index_subcommand_parses_legacy_recorder_options() {
        let action = parse_args(strings(&[
            "index",
            "--output",
            "/tmp/index.sqlite3",
            "--sessions-root",
            "/tmp/sessions",
            "--include-subsessions",
            "--include-empty-messages",
            "--include-exec",
            "--rebuild",
        ]))
        .unwrap();
        let CliAction::Run(Command::Index(options)) = action else {
            panic!("expected index");
        };
        assert_eq!(options.output, PathBuf::from("/tmp/index.sqlite3"));
        assert_eq!(options.sessions_root, PathBuf::from("/tmp/sessions"));
        assert!(options.include_subsessions);
        assert!(options.include_empty_messages);
        assert!(options.include_exec);
        assert!(options.rebuild);
    }

    #[test]
    fn replay_subcommand_accepts_stdin_and_include_exec() {
        assert_eq!(
            parse_args(strings(&["replay", "--include-exec", "-"])).unwrap(),
            CliAction::Run(Command::Replay(ReplayOptions {
                input: Some(PathBuf::from("-")),
                include_exec: true,
            }))
        );
    }

    #[test]
    fn replay_rejects_multiple_inputs() {
        let error = parse_args(strings(&["replay", "a.jsonl", "b.jsonl"])).unwrap_err();
        assert!(error.to_string().contains("only one input path"));
    }

    #[test]
    fn root_option_before_subcommand_is_not_reinterpreted() {
        let error = parse_args(strings(&["--include-exec", "replay", "a.jsonl"])).unwrap_err();
        assert!(error.to_string().contains("unknown argument"));
    }

    #[test]
    fn help_is_returned_without_process_exit() {
        assert_eq!(
            parse_args(strings(&["index", "--help"])).unwrap(),
            CliAction::PrintHelp(HelpTopic::Index)
        );
    }

    #[test]
    fn missing_option_values_and_unknown_options_are_rejected() {
        for args in [
            &["--db"][..],
            &["index", "--output"][..],
            &["index", "--sessions-root"][..],
        ] {
            assert!(parse_args(strings(args)).is_err());
        }
        assert!(parse_args(strings(&["--wat"])).is_err());
        assert!(parse_args(strings(&["index", "--wat"])).is_err());
        assert!(parse_args(strings(&["replay", "--wat"])).is_err());
    }

    #[test]
    fn help_texts_describe_initial_visibility_and_toggle_key() {
        let root = help_text(HelpTopic::Root);
        assert!(root.contains("select-codex-session index"));
        assert!(root.contains("select-codex-session replay"));
        assert!(root.contains("Show subsessions from the canonical index"));
        assert!(root.contains("Initially show exec records in replay"));
        assert!(root.contains("e                              toggle exec entries for replay"));
        assert!(!root.ends_with('\n'));

        let index = help_text(HelpTopic::Index);
        assert!(index.contains("--include-empty-messages"));
        assert!(index.contains("Compatibility option; canonical index always stores exec events"));
        assert!(index.contains("Ignore fingerprints and rebuild the canonical index"));

        let replay = help_text(HelpTopic::Replay);
        assert!(replay.contains("[PATH|-]"));
        assert!(replay.contains("default: hidden; press e to toggle"));
        assert!(replay.contains("e                  toggle command execution entries"));
    }
}
