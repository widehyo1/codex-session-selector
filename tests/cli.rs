use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use rusqlite::Connection;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_select-codex-session")
}

struct Fixture {
    root: PathBuf,
    sessions: PathBuf,
    db: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "codex-session-selector-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let sessions = root.join("sessions");
        let db = root.join("index.sqlite3");
        fs::create_dir_all(&sessions).unwrap();
        Self { root, sessions, db }
    }

    fn write_session(&self, name: &str, message: &str) -> PathBuf {
        let day = self.sessions.join("2026/07/27");
        fs::create_dir_all(&day).unwrap();
        let path = day.join(name);
        fs::write(
            &path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{name}\",\"timestamp\":\"2026-07-27T00:00:00Z\",\"cwd\":\"/tmp/demo\",\"source\":\"cli\",\"thread_source\":\"user\"}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{message}\"}}}}\n"
            ),
        )
        .unwrap();
        path
    }

    fn index(&self) -> Command {
        let mut command = Command::new(binary());
        command.args([
            "index",
            "--sessions-root",
            self.sessions.to_str().unwrap(),
            "--output",
            self.db.to_str().unwrap(),
        ]);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stdout(output: std::process::Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn expected_summary(action: &str, db: &Path, counts: [usize; 6]) -> String {
    let [scanned, parsed, new_files, changed, unchanged, deleted] = counts;
    format!(
        "{action} canonical index at {}: scanned {scanned} jsonl files; parsed {parsed} ({new_files} new, {changed} changed), kept {unchanged} unchanged, removed {deleted} deleted, deferred 0 unstable, skipped 0; stored {scanned} sessions and 0 exec events\n",
        db.display()
    )
}

#[test]
fn root_help_lists_exec_toggle_key() {
    let output = Command::new(binary()).arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("index"));
    assert!(stdout.contains("replay"));
    assert!(stdout.contains("--include-exec"));
    assert!(stdout.contains("toggle exec entries for replay"));
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
    assert!(stdout.contains("--rebuild"));
    assert!(stdout.contains("Compatibility option; canonical index always stores exec events"));
}

#[test]
fn index_creates_schema_v2_with_fts() {
    let fixture = Fixture::new();
    fixture.write_session("normal.jsonl", "증분 인덱스 확인");
    let output = stdout(fixture.index().output().unwrap());
    assert_eq!(
        output,
        expected_summary("rebuilt", &fixture.db, [1, 1, 1, 0, 0, 0])
    );

    let conn = Connection::open(&fixture.db).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM sessions_fts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM exec_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn selector_no_refresh_rejects_schema_v1_with_action() {
    let fixture = Fixture::new();
    fixture.write_session("normal.jsonl", "message");
    stdout(fixture.index().output().unwrap());
    let conn = Connection::open(&fixture.db).unwrap();
    conn.execute_batch(
        "DROP TRIGGER sessions_fts_dirty_ai;
         DROP TRIGGER sessions_fts_dirty_au;
         DROP TRIGGER sessions_fts_dirty_ad;
         DROP TRIGGER exec_events_fts_dirty_ai;
         DROP TRIGGER exec_events_fts_dirty_au;
         DROP TRIGGER exec_events_fts_dirty_ad;
         DROP TABLE sessions_fts;
         DROP TABLE fts_sync_state;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    drop(conn);

    let output = Command::new(binary())
        .args([
            "--no-refresh",
            "--db",
            fixture.db.to_str().unwrap(),
            "--print-path",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "search index schema 2 is required; refresh the index or run `select-codex-session index`"
    ));
}

#[test]
fn index_second_run_reports_zero_parsed_files() {
    let fixture = Fixture::new();
    fixture.write_session("normal.jsonl", "message");
    stdout(fixture.index().output().unwrap());
    let output = stdout(fixture.index().output().unwrap());
    assert_eq!(
        output,
        expected_summary("updated", &fixture.db, [1, 0, 0, 0, 1, 0])
    );
}

#[test]
fn index_changed_and_deleted_files_report_incremental_counts() {
    let fixture = Fixture::new();
    let changed = fixture.write_session("changed.jsonl", "before");
    let deleted = fixture.write_session("deleted.jsonl", "delete me");
    stdout(fixture.index().output().unwrap());
    fs::write(
        &changed,
        fs::read_to_string(&changed)
            .unwrap()
            .replace("before", "after with more bytes"),
    )
    .unwrap();
    fs::remove_file(deleted).unwrap();

    let output = stdout(fixture.index().output().unwrap());
    assert_eq!(
        output,
        expected_summary("updated", &fixture.db, [1, 1, 0, 1, 0, 1])
    );
}

#[test]
fn index_rebuild_forces_full_parse() {
    let fixture = Fixture::new();
    fixture.write_session("normal.jsonl", "message");
    stdout(fixture.index().output().unwrap());
    let output = stdout(fixture.index().arg("--rebuild").output().unwrap());
    assert_eq!(
        output,
        expected_summary("rebuilt", &fixture.db, [1, 1, 1, 0, 0, 0])
    );
}

#[test]
fn selector_view_options_are_listed_in_root_help() {
    let output = stdout(Command::new(binary()).arg("--help").output().unwrap());
    assert!(output.contains("--include-subsessions"));
    assert!(output.contains("Show subsessions from the canonical index"));
    assert!(output.contains("--include-empty-messages"));
}

#[test]
fn replay_help_lists_exec_toggle_key() {
    let output = Command::new(binary())
        .args(["replay", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--include-exec"));
    assert!(stdout.contains("[PATH|-]"));
    assert!(stdout.contains("default: hidden; press e to toggle"));
    assert!(stdout.contains("toggle command execution entries"));
}

#[test]
fn version_uses_the_integrated_binary_name() {
    let output = Command::new(binary()).arg("--version").output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("select-codex-session {}\n", env!("CARGO_PKG_VERSION"))
    );
}
