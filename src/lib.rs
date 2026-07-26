use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::Value;

mod application;
mod cli;
mod indexer;
mod replay;
mod selector;
mod terminal;
mod ui_state;

#[cfg(test)]
mod test_support;

pub fn run_from_args<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    match cli::parse_args(args.into_iter())? {
        cli::CliAction::Run(command) => application::run(command),
        cli::CliAction::PrintHelp(topic) => {
            println!("{}", cli::help_text(topic));
            Ok(())
        }
        cli::CliAction::PrintVersion => {
            println!("select-codex-session {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub path: PathBuf,
    pub id: Option<String>,
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
    pub repository_url: Option<String>,
    pub branch: Option<String>,
    pub first_message: String,
    pub is_subsession: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecEvent {
    pub session_path: PathBuf,
    pub session_id: Option<String>,
    pub event_index: usize,
    pub call_id: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub command: String,
    pub output: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CollectOptions {
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
    pub include_exec: bool,
}

#[derive(Debug, Clone)]
pub struct CollectedSessionData {
    pub rows: Vec<SessionRow>,
    pub exec_events: Vec<ExecEvent>,
    pub total_files: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone)]
struct ParsedSessionFile {
    row: SessionRow,
    exec_events: Vec<ExecEvent>,
}

pub fn is_subsession_meta(payload: &Value) -> bool {
    let thread_source = payload
        .get("thread_source")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let source_is_subagent = payload
        .get("source")
        .and_then(Value::as_object)
        .is_some_and(|source| source.contains_key("subagent"));

    let has_agent_role = payload
        .get("agent_role")
        .and_then(Value::as_str)
        .is_some_and(|role| !role.trim().is_empty());

    thread_source == "subagent" || source_is_subagent || has_agent_role
}

pub fn parse_session_file(path: &Path) -> Result<Option<SessionRow>> {
    Ok(parse_session_file_data(path, false)?.map(|data| data.row))
}

fn parse_session_file_data(path: &Path, include_exec: bool) -> Result<Option<ParsedSessionFile>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut meta: Option<SessionRow> = None;
    let mut first_message: Option<String> = None;
    let mut exec_events: Vec<ExecEvent> = Vec::new();
    let mut exec_by_call_id: HashMap<String, usize> = HashMap::new();

    for (line_number, line) in reader.lines().enumerate() {
        let line =
            line.with_context(|| format!("failed to read {}:{}", path.display(), line_number + 1))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "warning: {}:{}: invalid json: {err}",
                    path.display(),
                    line_number + 1
                );
                continue;
            }
        };

        let record_type = value.get("type").and_then(Value::as_str);
        let payload = value.get("payload").unwrap_or(&Value::Null);

        if meta.is_none() && record_type == Some("session_meta") {
            let git = payload.get("git").unwrap_or(&Value::Null);
            meta = Some(SessionRow {
                path: path.to_path_buf(),
                id: string_field(payload, "id"),
                timestamp: string_field(payload, "timestamp"),
                cwd: string_field(payload, "cwd"),
                repository_url: string_field(git, "repository_url"),
                branch: string_field(git, "branch"),
                first_message: String::new(),
                is_subsession: is_subsession_meta(payload),
            });
        }

        if first_message.is_none()
            && record_type == Some("event_msg")
            && payload.get("type").and_then(Value::as_str) == Some("user_message")
        {
            first_message = Some(
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
        }

        if include_exec {
            collect_exec_event(
                path,
                line_number,
                record_type,
                payload,
                &mut exec_events,
                &mut exec_by_call_id,
            );
        }

        if !include_exec && meta.is_some() && first_message.is_some() {
            break;
        }
    }

    let Some(mut row) = meta else {
        return Ok(None);
    };
    row.first_message = first_message.unwrap_or_default();
    for event in &mut exec_events {
        event.session_id = row.id.clone();
    }
    Ok(Some(ParsedSessionFile { row, exec_events }))
}

pub fn should_include_row(row: &SessionRow, options: CollectOptions) -> bool {
    if !options.include_empty_messages && row.first_message.trim().is_empty() {
        return false;
    }
    if !options.include_subsessions && row.is_subsession {
        return false;
    }
    true
}

pub fn session_date(row: &SessionRow) -> String {
    row.timestamp
        .as_deref()
        .map(|timestamp| timestamp.chars().take(10).collect())
        .unwrap_or_default()
}

pub fn searchable_text(row: &SessionRow) -> String {
    [
        row.first_message.as_str(),
        row.cwd.as_deref().unwrap_or_default(),
        row.repository_url.as_deref().unwrap_or_default(),
        row.branch.as_deref().unwrap_or_default(),
        row.timestamp.as_deref().unwrap_or_default(),
        &session_date(row),
    ]
    .join("\n")
    .to_lowercase()
}

pub fn filter_sessions(rows: &[SessionRow], query: &str) -> Vec<SessionRow> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();

    if terms.is_empty() {
        return rows.to_vec();
    }

    rows.iter()
        .filter(|row| {
            let haystack = searchable_text(row);
            terms.iter().all(|term| haystack.contains(term))
        })
        .cloned()
        .collect()
}

pub fn collect_rows(
    sessions_root: &Path,
    options: CollectOptions,
) -> Result<(Vec<SessionRow>, usize, usize)> {
    let data = collect_session_data(sessions_root, options)?;

    Ok((data.rows, data.total_files, data.skipped))
}

pub fn collect_session_data(
    sessions_root: &Path,
    options: CollectOptions,
) -> Result<CollectedSessionData> {
    let mut paths = session_jsonl_paths(sessions_root)?;
    paths.sort();

    let mut rows = Vec::new();
    let mut exec_events = Vec::new();
    let mut skipped = 0;
    let total = paths.len();

    for path in paths {
        match parse_session_file_data(&path, options.include_exec)? {
            Some(data) if should_include_row(&data.row, options) => {
                rows.push(data.row);
                if options.include_exec {
                    exec_events.extend(data.exec_events);
                }
            }
            _ => skipped += 1,
        }
    }

    Ok(CollectedSessionData {
        rows,
        exec_events,
        total_files: total,
        skipped,
    })
}

pub fn session_jsonl_paths(sessions_root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();

    if !sessions_root.exists() {
        return Ok(out);
    }

    for year in read_dirs(sessions_root)? {
        for month in read_dirs(&year)? {
            for day in read_dirs(&month)? {
                for entry in fs::read_dir(&day)
                    .with_context(|| format!("failed to read {}", day.display()))?
                {
                    let path = entry?.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                        out.push(path);
                    }
                }
            }
        }
    }

    Ok(out)
}

pub fn recreate_database(db_path: &Path, rows: &[SessionRow]) -> Result<()> {
    recreate_database_with_exec(db_path, rows, None)
}

pub fn recreate_database_with_exec(
    db_path: &Path,
    rows: &[SessionRow],
    exec_events: Option<&[ExecEvent]>,
) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut conn = Connection::open(db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    conn.execute("DROP TABLE IF EXISTS sessions", [])?;
    conn.execute(
        r#"
        CREATE TABLE sessions (
            path TEXT,
            id TEXT,
            timestamp TEXT,
            cwd TEXT,
            repository_url TEXT,
            branch TEXT,
            first_message TEXT
        )
        "#,
        [],
    )?;

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            r#"
            INSERT INTO sessions (
                path, id, timestamp, cwd, repository_url, branch, first_message
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )?;

        for row in rows {
            stmt.execute(params![
                row.path.to_string_lossy().as_ref(),
                row.id.as_deref(),
                row.timestamp.as_deref(),
                row.cwd.as_deref(),
                row.repository_url.as_deref(),
                row.branch.as_deref(),
                row.first_message.as_str(),
            ])?;
        }
    }
    tx.commit()?;

    conn.execute("CREATE INDEX sessions_path_idx ON sessions(path)", [])?;
    conn.execute(
        "CREATE INDEX sessions_timestamp_idx ON sessions(timestamp)",
        [],
    )?;
    conn.execute("CREATE INDEX sessions_cwd_idx ON sessions(cwd)", [])?;

    if let Some(exec_events) = exec_events {
        conn.execute("DROP TABLE IF EXISTS exec_events", [])?;
        conn.execute(
            r#"
            CREATE TABLE exec_events (
                session_path TEXT NOT NULL,
                session_id TEXT,
                event_index INTEGER NOT NULL,
                call_id TEXT,
                kind TEXT NOT NULL,
                name TEXT,
                command TEXT,
                output TEXT
            )
            "#,
            [],
        )?;

        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO exec_events (
                    session_path, session_id, event_index, call_id, kind, name, command, output
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
            )?;

            for event in exec_events {
                stmt.execute(params![
                    event.session_path.to_string_lossy().as_ref(),
                    event.session_id.as_deref(),
                    event.event_index as i64,
                    event.call_id.as_deref(),
                    event.kind.as_str(),
                    event.name.as_deref(),
                    event.command.as_str(),
                    event.output.as_str(),
                ])?;
            }
        }
        tx.commit()?;

        conn.execute(
            "CREATE INDEX exec_events_session_path_idx ON exec_events(session_path)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX exec_events_session_id_idx ON exec_events(session_id)",
            [],
        )?;
    } else {
        conn.execute("DROP TABLE IF EXISTS exec_events", [])?;
    }
    Ok(())
}

pub fn load_sessions(db_path: &Path) -> Result<Vec<SessionRow>> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    let mut stmt = conn.prepare(
        r#"
        SELECT path, id, timestamp, cwd, repository_url, branch, first_message
        FROM sessions
        ORDER BY timestamp DESC
        "#,
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SessionRow {
                path: PathBuf::from(row.get::<_, String>(0)?),
                id: row.get(1)?,
                timestamp: row.get(2)?,
                cwd: row.get(3)?,
                repository_url: row.get(4)?,
                branch: row.get(5)?,
                first_message: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                is_subsession: false,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn collect_exec_event(
    path: &Path,
    line_number: usize,
    record_type: Option<&str>,
    payload: &Value,
    exec_events: &mut Vec<ExecEvent>,
    exec_by_call_id: &mut HashMap<String, usize>,
) {
    if record_type == Some("event_msg")
        && payload.get("type").and_then(Value::as_str) == Some("exec_command_end")
    {
        exec_events.push(legacy_exec_event(path, line_number, payload));
        return;
    }

    if record_type != Some("response_item") {
        return;
    }

    match payload.get("type").and_then(Value::as_str) {
        Some("custom_tool_call") | Some("function_call") if is_exec_tool_call(payload) => {
            let call_id = string_field(payload, "call_id");
            let event = ExecEvent {
                session_path: path.to_path_buf(),
                session_id: None,
                event_index: line_number,
                call_id: call_id.clone(),
                kind: payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call")
                    .to_string(),
                name: string_field(payload, "name"),
                command: exec_tool_command(payload),
                output: String::new(),
            };
            let index = exec_events.len();
            exec_events.push(event);
            if let Some(call_id) = call_id {
                exec_by_call_id.insert(call_id, index);
            }
        }
        Some("custom_tool_call_output") | Some("function_call_output") => {
            let call_id = string_field(payload, "call_id");
            let output = payload_text(payload, "output");
            if let Some(index) = call_id
                .as_deref()
                .and_then(|call_id| exec_by_call_id.get(call_id))
                .copied()
            {
                exec_events[index].output = output;
            }
        }
        _ => {}
    }
}

fn is_exec_tool_call(payload: &Value) -> bool {
    matches!(
        payload.get("name").and_then(Value::as_str),
        Some("exec") | Some("exec_command")
    )
}

fn exec_tool_command(payload: &Value) -> String {
    let input = payload_text(payload, "input");
    let input = if input.trim().is_empty() {
        payload_text(payload, "arguments")
    } else {
        input
    };

    serde_json::from_str::<Value>(&input)
        .ok()
        .and_then(|arguments| {
            arguments
                .get("cmd")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or(input)
}

fn legacy_exec_event(path: &Path, line_number: usize, payload: &Value) -> ExecEvent {
    let parsed_cmd = payload
        .get("parsed_cmd")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let command = parsed_cmd
        .iter()
        .filter_map(|cmd| cmd.get("cmd").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let name = parsed_cmd.iter().find_map(command_name);

    ExecEvent {
        session_path: path.to_path_buf(),
        session_id: None,
        event_index: line_number,
        call_id: None,
        kind: "exec_command_end".to_string(),
        name,
        command,
        output: payload_text(payload, "aggregated_output"),
    }
}

fn command_name(cmd: &Value) -> Option<String> {
    ["name", "path", "type"]
        .iter()
        .filter_map(|field| cmd.get(field).and_then(Value::as_str))
        .find(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn payload_text(payload: &Value, field: &str) -> String {
    let Some(value) = payload.get(field) else {
        return String::new();
    };

    value_to_text(value)
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(value_to_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| object.get("output").and_then(Value::as_str))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn read_dirs(path: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use super::*;

    #[test]
    fn collect_options_default_disables_all_optional_data() {
        let options = CollectOptions::default();

        assert!(!options.include_subsessions);
        assert!(!options.include_empty_messages);
        assert!(!options.include_exec);
    }

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-session-selector-{nonce}-{name}"))
    }

    fn write_jsonl(path: &Path, message: &str, source: Value, thread_source: &str) {
        let meta = json!({
            "type": "session_meta",
            "payload": {
                "id": "session-id",
                "timestamp": "2026-05-28T00:00:00Z",
                "cwd": "/repo/demo",
                "source": source,
                "thread_source": thread_source,
                "git": {
                    "repository_url": "https://git.example/demo.git",
                    "branch": "main"
                }
            }
        });
        let user = json!({
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": message
            }
        });
        fs::write(path, format!("{meta}\n{user}\n")).unwrap();
    }

    #[test]
    fn detects_subsessions_from_session_meta_structure() {
        assert!(is_subsession_meta(&json!({
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}
        })));
        assert!(is_subsession_meta(&json!({"thread_source": "subagent"})));
        assert!(is_subsession_meta(&json!({"agent_role": "worker"})));
        assert!(!is_subsession_meta(
            &json!({"source": "cli", "thread_source": "user"})
        ));
    }

    #[test]
    fn parse_session_file_extracts_metadata_and_first_message() {
        let path = temp_path("human.jsonl");
        write_jsonl(&path, "real user request", json!("cli"), "user");

        let row = parse_session_file(&path).unwrap().unwrap();

        assert_eq!(row.path, path);
        assert_eq!(row.id.as_deref(), Some("session-id"));
        assert_eq!(row.timestamp.as_deref(), Some("2026-05-28T00:00:00Z"));
        assert_eq!(row.cwd.as_deref(), Some("/repo/demo"));
        assert_eq!(
            row.repository_url.as_deref(),
            Some("https://git.example/demo.git")
        );
        assert_eq!(row.branch.as_deref(), Some("main"));
        assert_eq!(row.first_message, "real user request");
        assert!(!row.is_subsession);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn should_include_hides_empty_and_subsession_by_default() {
        let mut row = SessionRow {
            path: PathBuf::from("/tmp/a.jsonl"),
            id: None,
            timestamp: None,
            cwd: None,
            repository_url: None,
            branch: None,
            first_message: "ordinary looking task text".to_string(),
            is_subsession: true,
        };
        assert!(!should_include_row(&row, CollectOptions::default()));

        row.is_subsession = false;
        row.first_message = "   ".to_string();
        assert!(!should_include_row(&row, CollectOptions::default()));

        row.first_message = "human request".to_string();
        assert!(should_include_row(&row, CollectOptions::default()));
    }

    #[test]
    fn filter_sessions_searches_message_metadata_and_date() {
        let rows = vec![
            SessionRow {
                path: PathBuf::from("/tmp/a.jsonl"),
                id: None,
                timestamp: Some("2026-05-27T01:00:00Z".to_string()),
                cwd: Some("/repo/alpha".to_string()),
                repository_url: Some("https://git.example/alpha.git".to_string()),
                branch: Some("main".to_string()),
                first_message: "fix docker compose".to_string(),
                is_subsession: false,
            },
            SessionRow {
                path: PathBuf::from("/tmp/b.jsonl"),
                id: None,
                timestamp: Some("2026-05-28T01:00:00Z".to_string()),
                cwd: Some("/repo/beta".to_string()),
                repository_url: Some("https://git.example/beta.git".to_string()),
                branch: Some("feature/search".to_string()),
                first_message: "add selector".to_string(),
                is_subsession: false,
            },
        ];

        assert_eq!(
            filter_sessions(&rows, "docker")[0].path,
            PathBuf::from("/tmp/a.jsonl")
        );
        assert_eq!(
            filter_sessions(&rows, "alpha.git")[0].path,
            PathBuf::from("/tmp/a.jsonl")
        );
        assert_eq!(
            filter_sessions(&rows, "feature")[0].path,
            PathBuf::from("/tmp/b.jsonl")
        );
        assert_eq!(
            filter_sessions(&rows, "2026-05-28")[0].path,
            PathBuf::from("/tmp/b.jsonl")
        );
    }

    #[test]
    fn collect_session_data_omits_exec_by_default_and_collects_when_enabled() {
        let root = temp_path("sessions");
        let day = root.join("2026").join("05").join("28");
        fs::create_dir_all(&day).unwrap();
        let path = day.join("rollout.jsonl");
        crate::test_support::write_jsonl_with_exec(&path);

        let default_data = collect_session_data(&root, CollectOptions::default()).unwrap();
        assert_eq!(default_data.rows.len(), 1);
        assert_eq!(default_data.exec_events.len(), 0);

        let with_exec = collect_session_data(
            &root,
            CollectOptions {
                include_exec: true,
                ..CollectOptions::default()
            },
        )
        .unwrap();

        assert_eq!(with_exec.rows.len(), 1);
        assert_eq!(with_exec.exec_events.len(), 3);
        assert_eq!(with_exec.exec_events[0].kind, "exec_command_end");
        assert_eq!(with_exec.exec_events[0].name.as_deref(), Some("README.md"));
        assert_eq!(with_exec.exec_events[0].command, "sed -n '1,80p' README.md");
        assert_eq!(with_exec.exec_events[0].output, "# Title");
        assert_eq!(with_exec.exec_events[1].kind, "custom_tool_call");
        assert_eq!(with_exec.exec_events[1].call_id.as_deref(), Some("call-1"));
        assert!(
            with_exec.exec_events[1]
                .command
                .contains("tools.exec_command")
        );
        assert!(with_exec.exec_events[1].output.contains("Script completed"));
        assert!(with_exec.exec_events[1].output.contains("/repo/demo"));
        assert_eq!(with_exec.exec_events[2].kind, "function_call");
        assert_eq!(
            with_exec.exec_events[2].name.as_deref(),
            Some("exec_command")
        );
        assert_eq!(with_exec.exec_events[2].command, "git status --short");
        assert_eq!(with_exec.exec_events[2].output, " M README.md");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recreate_database_creates_exec_table_only_when_requested() {
        let db_path = temp_path("sessions.sqlite3");
        let row = SessionRow {
            path: PathBuf::from("/tmp/session.jsonl"),
            id: Some("session-id".to_string()),
            timestamp: Some("2026-05-28T00:00:00Z".to_string()),
            cwd: Some("/repo/demo".to_string()),
            repository_url: Some("https://git.example/demo.git".to_string()),
            branch: Some("main".to_string()),
            first_message: "real user request".to_string(),
            is_subsession: false,
        };
        let exec = ExecEvent {
            session_path: row.path.clone(),
            session_id: row.id.clone(),
            event_index: 2,
            call_id: Some("call-1".to_string()),
            kind: "custom_tool_call".to_string(),
            name: Some("exec".to_string()),
            command: "pwd".to_string(),
            output: "/repo/demo".to_string(),
        };

        recreate_database(&db_path, std::slice::from_ref(&row)).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert!(conn.prepare("SELECT count(*) FROM exec_events").is_err());
        drop(conn);

        recreate_database_with_exec(&db_path, &[row], Some(&[exec])).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM exec_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let _ = fs::remove_file(db_path);
    }
}
