use std::{env, path::PathBuf};

use anyhow::{Result, bail};
use codex_session_selector::{CollectOptions, collect_session_data, recreate_database_with_exec};

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;
    let options = CollectOptions {
        include_subsessions: args.include_subsessions,
        include_empty_messages: args.include_empty_messages,
        include_exec: args.include_exec,
    };

    let data = collect_session_data(&args.sessions_root, options)?;
    recreate_database_with_exec(
        &args.output,
        &data.rows,
        args.include_exec.then_some(data.exec_events.as_slice()),
    )?;

    if args.include_exec {
        println!(
            "wrote {} session rows and {} exec rows to {} from {} jsonl files; skipped {} filtered or invalid sessions",
            data.rows.len(),
            data.exec_events.len(),
            args.output.display(),
            data.total_files,
            data.skipped
        );
    } else {
        println!(
            "wrote {} session rows to {} from {} jsonl files; skipped {} filtered or invalid sessions; exec indexing disabled",
            data.rows.len(),
            args.output.display(),
            data.total_files,
            data.skipped
        );
    }

    Ok(())
}

struct Args {
    output: PathBuf,
    sessions_root: PathBuf,
    include_subsessions: bool,
    include_empty_messages: bool,
    include_exec: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let home = home_dir()?;
        let mut parsed = Self {
            output: home.join("codex-session-info.sqlite3"),
            sessions_root: home.join(".codex").join("sessions"),
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        };

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-o" | "--output" => {
                    let Some(value) = args.next() else {
                        bail!("{arg} requires a path");
                    };
                    parsed.output = expand_home(&value)?;
                }
                "--sessions-root" => {
                    let Some(value) = args.next() else {
                        bail!("{arg} requires a path");
                    };
                    parsed.sessions_root = expand_home(&value)?;
                }
                "--include-subsessions" => parsed.include_subsessions = true,
                "--include-empty-messages" => parsed.include_empty_messages = true,
                "--include-exec" => parsed.include_exec = true,
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                "-V" | "--version" => {
                    println!("record-codex-session-info {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(parsed)
    }
}

fn print_help() {
    println!(
        "\
record-codex-session-info {version}

Build a SQLite index from local Codex session JSONL files.

Usage:
  record-codex-session-info [OPTIONS]

Options:
  -o, --output PATH              SQLite output path
                                 default: ~/codex-session-info.sqlite3
      --sessions-root PATH       Codex sessions root
                                 default: ~/.codex/sessions
      --include-subsessions      Include subagent/subsession records
      --include-empty-messages   Include sessions without a first user message
      --include-exec             Also create and populate exec_events.
                                 Off by default to keep the index small.
  -h, --help                     Show this help
  -V, --version                  Show version

Default schema:
  sessions(path, id, timestamp, cwd, repository_url, branch, first_message)

With --include-exec:
  exec_events(session_path, session_id, event_index, call_id, kind, name, command, output)

Exec sources:
  old JSONL: event_msg.payload.type == exec_command_end
  new JSONL: response_item payloads with exec custom_tool_call/function_call records
",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
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
