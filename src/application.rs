use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::{
    cli::{Command, IndexOptions, ReplayOptions, SelectOptions},
    indexer, load_sessions, replay,
    selector::{self, SelectorAction, SelectorApp},
};

pub(crate) fn run(command: Command) -> Result<()> {
    match command {
        Command::Select(options) => run_select(options),
        Command::Index(options) => run_index(options),
        Command::Replay(options) => replay::run(&options),
    }
}

fn run_index(options: IndexOptions) -> Result<()> {
    let summary = indexer::build_index(&options)?;
    println!("{}", indexer::format_summary(&summary));
    Ok(())
}

fn run_select(options: SelectOptions) -> Result<()> {
    if options.refresh {
        refresh_database(&options)?;
    }

    let rows = load_sessions(&options.db)?;
    if rows.is_empty() {
        bail!("no sessions found in {}", options.db.display());
    }

    let mut app = SelectorApp::new(rows);
    if options.print_path {
        if let SelectorAction::OpenReplay(path) = selector::run(&mut app)? {
            println!("{}", path.display());
        }
        return Ok(());
    }

    loop {
        match selector::run(&mut app)? {
            SelectorAction::Quit => return Ok(()),
            SelectorAction::OpenReplay(path) => match replay_selected(&options, path) {
                Ok(()) => app.clear_status(),
                Err(error) => app.set_status(error.to_string()),
            },
        }
    }
}

fn refresh_database(options: &SelectOptions) -> Result<()> {
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

fn external_record_args(db: &Path, include_exec: bool) -> Vec<String> {
    let mut args = vec!["--output".to_string(), db.to_string_lossy().to_string()];
    if include_exec {
        args.push("--include-exec".to_string());
    }
    args
}

fn run_external_refresh(program: &str, options: &SelectOptions) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(external_record_args(&options.db, options.include_exec))
        .status()?;

    if !status.success() {
        bail!(
            "{} failed with status {}",
            program,
            status.code().unwrap_or(1)
        );
    }
    Ok(())
}

fn replay_selected(select_options: &SelectOptions, path: PathBuf) -> Result<()> {
    if let Some(program) = select_options.replay_command.as_deref() {
        return run_external_replay(program, &path, select_options.include_exec);
    }

    replay::run(&ReplayOptions {
        input: Some(path),
        include_exec: select_options.include_exec,
    })
}

fn external_replay_args(path: &Path, include_exec: bool) -> Vec<OsString> {
    let mut args = Vec::new();
    if include_exec {
        args.push(OsString::from("--include-exec"));
    }
    args.push(path.as_os_str().to_owned());
    args
}

fn run_external_replay(program: &str, path: &Path, include_exec: bool) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(external_replay_args(path, include_exec))
        .status()?;
    if status.success() {
        return Ok(());
    }
    bail!(
        "{} exited with status {}",
        program,
        status.code().unwrap_or(1)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_record_args_match_legacy_contract() {
        assert_eq!(
            external_record_args(Path::new("/tmp/index.sqlite3"), false),
            vec!["--output".to_string(), "/tmp/index.sqlite3".to_string(),]
        );
        assert_eq!(
            external_record_args(Path::new("/tmp/index.sqlite3"), true),
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
            external_replay_args(Path::new("/tmp/session.jsonl"), true),
            vec![
                OsString::from("--include-exec"),
                OsString::from("/tmp/session.jsonl"),
            ]
        );
    }
}
