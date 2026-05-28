use std::{env, path::PathBuf};

use anyhow::{Result, bail};
use codex_session_selector::{CollectOptions, collect_rows, recreate_database};

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;
    let options = CollectOptions {
        include_subsessions: args.include_subsessions,
        include_empty_messages: args.include_empty_messages,
    };

    let (rows, total_files, skipped) = collect_rows(&args.sessions_root, options)?;
    recreate_database(&args.output, &rows)?;

    println!(
        "wrote {} rows to {} from {} jsonl files; skipped {} filtered or invalid sessions",
        rows.len(),
        args.output.display(),
        total_files,
        skipped
    );

    Ok(())
}

struct Args {
    output: PathBuf,
    sessions_root: PathBuf,
    include_subsessions: bool,
    include_empty_messages: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let home = home_dir()?;
        let mut parsed = Self {
            output: home.join("codex-session-info.sqlite3"),
            sessions_root: home.join(".codex").join("sessions"),
            include_subsessions: false,
            include_empty_messages: false,
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
                "-h" | "--help" => {
                    print_help();
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
        "usage: record-codex-session-info [--output PATH] [--sessions-root PATH] [--include-subsessions] [--include-empty-messages]"
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
