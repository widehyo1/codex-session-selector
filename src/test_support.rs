use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::json;

pub(crate) struct SessionFixture {
    root: PathBuf,
}

impl SessionFixture {
    pub(crate) fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let unique = format!(
            "codex-session-selector-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
        let day = self.sessions_root().join("2026").join("07").join("26");
        std::fs::create_dir_all(&day).unwrap();
        let path = day.join("session.jsonl");
        write_jsonl_with_exec(&path);
        path
    }
}

impl Drop for SessionFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn write_jsonl_with_exec(path: &Path) {
    let records = [
        json!({
            "type": "session_meta",
            "payload": {
                "id": "session-id",
                "timestamp": "2026-05-28T00:00:00Z",
                "cwd": "/repo/demo",
                "source": "cli",
                "thread_source": "user",
                "git": {
                    "repository_url": "https://git.example/demo.git",
                    "branch": "main"
                }
            }
        }),
        json!({
            "type": "event_msg",
            "payload": {"type": "user_message", "message": "real user request"}
        }),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "exec_command_end",
                "parsed_cmd": [{
                    "type": "read",
                    "cmd": "sed -n '1,80p' README.md",
                    "name": "README.md"
                }],
                "aggregated_output": "# Title"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "call_id": "call-1",
                "name": "exec",
                "input": "const r = await tools.exec_command({\"cmd\":\"pwd\"});",
                "status": "completed"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call_output",
                "call_id": "call-1",
                "output": [
                    {"type": "input_text", "text": "Script completed"},
                    {"type": "input_text", "text": "/repo/demo"}
                ]
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "call_id": "call-2",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"git status --short\",\"workdir\":\"/repo/demo\"}"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call-2",
                "output": " M README.md"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "non-exec-call",
                "output": "not an exec result"
            }
        }),
    ];
    let contents = records
        .into_iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{contents}\n")).unwrap();
}

pub(crate) fn table_exists(connection: &Connection, table: &str) -> bool {
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

pub(crate) fn table_count(connection: &Connection, table: &str) -> i64 {
    let sql = match table {
        "sessions" => "SELECT count(*) FROM sessions",
        "exec_events" => "SELECT count(*) FROM exec_events",
        _ => panic!("unsupported fixture table: {table}"),
    };
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}
