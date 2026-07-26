use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::{
    cli::{Command, IndexOptions, ReplayOptions, SelectOptions},
    indexer::{self, search::SearchIndex, store::SessionView},
    replay,
    selector::{self, SelectorAction, SelectorApp},
    ui_state::ExecVisibility,
};

pub(crate) fn run(command: Command) -> Result<()> {
    match command {
        Command::Select(options) => run_select(options),
        Command::Index(options) => run_index(options),
        Command::Replay(options) => {
            replay::run(&options)?;
            Ok(())
        }
    }
}

fn run_index(options: IndexOptions) -> Result<()> {
    let outcome = indexer::build_index(&options)?;
    println!("{}", indexer::format_summary(&outcome.summary));
    Ok(())
}

fn run_select(options: SelectOptions) -> Result<()> {
    if options.refresh {
        refresh_database(&options)?;
    }

    let search_index = SearchIndex::open(
        &options.db,
        SessionView {
            include_subsessions: options.include_subsessions,
            include_empty_messages: options.include_empty_messages,
        },
    )?;

    let initial_visibility = ExecVisibility::from_include_exec(options.include_exec);
    let mut app = SelectorApp::new(search_index, initial_visibility)?;
    if app.is_empty() {
        bail!("no sessions found in {}", options.db.display());
    }
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
                    Err(error) => app.set_status(error.to_string()),
                }
            }
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
        rebuild: false,
        include_subsessions: false,
        include_empty_messages: false,
        include_exec: false,
        ..defaults
    };
    let outcome = indexer::build_index(&index_options)?;
    println!("{}", indexer::format_summary(&outcome.summary));
    Ok(())
}

fn external_record_args(
    db: &Path,
    include_subsessions: bool,
    include_empty_messages: bool,
    include_exec: bool,
) -> Vec<String> {
    let mut args = vec!["--output".to_string(), db.to_string_lossy().to_string()];
    if include_subsessions {
        args.push("--include-subsessions".to_string());
    }
    if include_empty_messages {
        args.push("--include-empty-messages".to_string());
    }
    if include_exec {
        args.push("--include-exec".to_string());
    }
    args
}

fn run_external_refresh(program: &str, options: &SelectOptions) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(external_record_args(
            &options.db,
            options.include_subsessions,
            options.include_empty_messages,
            options.include_exec,
        ))
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

fn external_replay_args(path: &Path, exec_visibility: ExecVisibility) -> Vec<OsString> {
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
    if status.success() {
        return Ok(exec_visibility);
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
            external_record_args(Path::new("/tmp/index.sqlite3"), false, false, false),
            vec!["--output".to_string(), "/tmp/index.sqlite3".to_string(),]
        );
        assert_eq!(
            external_record_args(Path::new("/tmp/index.sqlite3"), true, true, true),
            vec![
                "--output".to_string(),
                "/tmp/index.sqlite3".to_string(),
                "--include-subsessions".to_string(),
                "--include-empty-messages".to_string(),
                "--include-exec".to_string(),
            ]
        );
    }

    #[test]
    fn external_replay_args_use_current_visibility() {
        assert_eq!(
            external_replay_args(Path::new("/tmp/session.jsonl"), ExecVisibility::Hidden),
            vec![OsString::from("/tmp/session.jsonl")]
        );
        assert_eq!(
            external_replay_args(Path::new("/tmp/session.jsonl"), ExecVisibility::Shown),
            vec![
                OsString::from("--include-exec"),
                OsString::from("/tmp/session.jsonl"),
            ]
        );
    }
}
