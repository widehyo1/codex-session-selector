use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::Value;

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

#[derive(Debug, Clone, Copy, Default)]
pub struct CollectOptions {
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
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
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut meta: Option<SessionRow> = None;
    let mut first_message: Option<String> = None;

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

        if meta.is_some() && first_message.is_some() {
            break;
        }
    }

    let Some(mut row) = meta else {
        return Ok(None);
    };
    row.first_message = first_message.unwrap_or_default();
    Ok(Some(row))
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
    let mut paths = session_jsonl_paths(sessions_root)?;
    paths.sort();

    let mut rows = Vec::new();
    let mut skipped = 0;
    let total = paths.len();

    for path in paths {
        match parse_session_file(&path)? {
            Some(row) if should_include_row(&row, options) => rows.push(row),
            _ => skipped += 1,
        }
    }

    Ok((rows, total, skipped))
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
}
