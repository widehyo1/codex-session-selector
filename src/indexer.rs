use std::{
    collections::BTreeMap,
    env,
    path::{Component, Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::cli::IndexOptions;

pub(crate) mod fts;
pub(crate) mod scan;
pub(crate) mod schema;
pub(crate) mod search;
pub(crate) mod store;

use scan::{ScanPlan, StoredSource, scan_sources};
use schema::{SchemaState, detect_schema};

pub(crate) type SessionKey = i64;
pub(crate) type ExecKey = i64;
pub(crate) type MessageKey = i64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IndexDelta {
    pub inserted_session_keys: Vec<SessionKey>,
    pub updated_session_keys: Vec<SessionKey>,
    pub deleted_session_keys: Vec<SessionKey>,
    pub inserted_exec_keys: Vec<ExecKey>,
    pub updated_exec_keys: Vec<ExecKey>,
    pub deleted_exec_keys: Vec<ExecKey>,
    pub inserted_message_keys: Vec<MessageKey>,
    pub updated_message_keys: Vec<MessageKey>,
    pub deleted_message_keys: Vec<MessageKey>,
    pub touched_session_keys: Vec<SessionKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexMode {
    Incremental,
    Rebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexSummary {
    pub mode: IndexMode,
    pub scanned_files: usize,
    pub parsed_files: usize,
    pub new_files: usize,
    pub changed_files: usize,
    pub unchanged_files: usize,
    pub deleted_files: usize,
    pub unstable_files: usize,
    pub skipped_files: usize,
    pub session_rows: usize,
    pub exec_rows: usize,
    pub message_rows: usize,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexOutcome {
    pub summary: IndexSummary,
    pub delta: IndexDelta,
}

pub(crate) fn build_index(options: &IndexOptions) -> Result<IndexOutcome> {
    let root = absolute_lexical_path(&options.sessions_root)?;
    let mut conn = store::open_configured_connection(&options.output)?;
    fts::verify_runtime(&conn)?;
    let state = detect_schema(&conn)?;
    let mode = choose_mode(&state, &root, options.rebuild)?;
    let stored = match mode {
        IndexMode::Incremental => store::load_fingerprints(&conn)?,
        IndexMode::Rebuild => BTreeMap::<PathBuf, StoredSource>::new(),
    };
    let force_all = matches!(
        state,
        SchemaState::CanonicalV1 { .. } | SchemaState::FtsV2 { .. }
    );
    let plan = scan_sources(&root, &stored, mode == IndexMode::Rebuild || force_all)?;
    if force_all && !plan.unstable_paths.is_empty() {
        bail!(
            "cannot migrate index schema to 3 while {} source files are changing; retry the index refresh",
            plan.unstable_paths.len()
        );
    }

    let tx = store::begin_immediate(&mut conn)?;
    let fts_mode = match (&state, mode) {
        (SchemaState::CanonicalV1 { .. } | SchemaState::FtsV2 { .. }, IndexMode::Incremental) => {
            if matches!(state, SchemaState::FtsV2 { .. }) {
                fts::drop_schema(&tx)?;
            }
            tx.execute_batch("CREATE TABLE message_events (event_index INTEGER NOT NULL CHECK (event_index >= 0), role TEXT NOT NULL CHECK (role IN ('user', 'agent')), content TEXT NOT NULL, message_key INTEGER PRIMARY KEY, session_key INTEGER NOT NULL REFERENCES sessions(session_key) ON DELETE CASCADE, UNIQUE (session_key, event_index)) STRICT;")?;
            fts::create_schema(&tx)?;
            fts::FtsSyncMode::Populate
        }
        (SchemaState::Current { .. }, IndexMode::Incremental) => fts::preflight(&tx)?,
        _ => fts::FtsSyncMode::Populate,
    };
    let mut delta = match mode {
        IndexMode::Incremental => store::apply_incremental(&tx, &root, &plan)?,
        IndexMode::Rebuild => store::apply_rebuild(&tx, &root, &plan)?,
    };
    match fts_mode {
        fts::FtsSyncMode::Delta => fts::apply_delta(&tx, &delta)?,
        fts::FtsSyncMode::Populate => fts::populate_all(&tx)?,
        fts::FtsSyncMode::Rebuild => fts::rebuild(&tx)?,
    }
    fts::verify_invariants(&tx, fts_mode != fts::FtsSyncMode::Delta)?;
    fts::mark_clean(&tx)?;
    store::verify_invariants(&tx)?;
    store::set_schema_version(&tx)?;
    let counts = store::query_counts(&tx)?;
    tx.commit()?;
    store::normalize_delta(&mut delta);

    Ok(IndexOutcome {
        summary: summary_from(mode, &plan, counts, options.output.clone()),
        delta,
    })
}

fn choose_mode(state: &SchemaState, root: &Path, rebuild: bool) -> Result<IndexMode> {
    match state {
        SchemaState::Empty | SchemaState::Legacy => Ok(IndexMode::Rebuild),
        SchemaState::CanonicalV1 { sessions_root }
        | SchemaState::FtsV2 { sessions_root }
        | SchemaState::Current { sessions_root }
            if rebuild || sessions_root.to_string_lossy() != root.to_string_lossy() =>
        {
            Ok(IndexMode::Rebuild)
        }
        SchemaState::CanonicalV1 { .. }
        | SchemaState::FtsV2 { .. }
        | SchemaState::Current { .. } => Ok(IndexMode::Incremental),
        SchemaState::Future { version } => bail!(
            "index schema version {version} is newer than supported version {}; refusing to overwrite it",
            schema::SCHEMA_VERSION
        ),
        SchemaState::Unknown { version, reason } if !rebuild => bail!(
            "unrecognized index schema version {version}: {reason}; run index --rebuild to replace it"
        ),
        SchemaState::Unknown { .. } => Ok(IndexMode::Rebuild),
    }
}

fn summary_from(
    mode: IndexMode,
    plan: &ScanPlan,
    counts: store::StoredCounts,
    output: PathBuf,
) -> IndexSummary {
    IndexSummary {
        mode,
        scanned_files: plan.scanned_files,
        parsed_files: plan.new_sources.len() + plan.changed_sources.len(),
        new_files: plan.new_sources.len(),
        changed_files: plan.changed_sources.len(),
        unchanged_files: plan.unchanged_paths.len(),
        deleted_files: plan.deleted_paths.len(),
        unstable_files: plan.unstable_paths.len(),
        skipped_files: counts.skipped_files,
        session_rows: counts.session_rows,
        exec_rows: counts.exec_rows,
        message_rows: counts.message_rows,
        output,
    }
}

pub(crate) fn format_summary(summary: &IndexSummary) -> String {
    let action = match summary.mode {
        IndexMode::Incremental => "updated",
        IndexMode::Rebuild => "rebuilt",
    };
    format!(
        "{action} canonical index at {}: scanned {} jsonl files; parsed {} ({} new, {} changed), kept {} unchanged, removed {} deleted, deferred {} unstable, skipped {}; stored {} sessions, {} messages and {} exec events",
        summary.output.display(),
        summary.scanned_files,
        summary.parsed_files,
        summary.new_files,
        summary.changed_files,
        summary.unchanged_files,
        summary.deleted_files,
        summary.unstable_files,
        summary.skipped_files,
        summary.session_rows,
        summary.message_rows,
        summary.exec_rows,
    )
}

pub(crate) fn absolute_lexical_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.parent().is_some() {
                    normalized.pop();
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs,
        io::Write,
        time::{Duration, Instant},
    };

    use rusqlite::Connection;
    use serde_json::json;

    use super::*;
    use crate::test_support::{SessionFixture, table_count, table_exists};

    fn options(fixture: &SessionFixture, db: PathBuf) -> IndexOptions {
        IndexOptions {
            output: db,
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        }
    }

    #[test]
    fn canonical_rebuild_stores_exec_even_without_compatibility_option() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");

        let outcome = build_index(&options(&fixture, db.clone())).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Rebuild);
        assert_eq!(outcome.summary.session_rows, 1);
        assert_eq!(outcome.summary.exec_rows, 3);

        let connection = Connection::open(db).unwrap();
        assert_eq!(table_count(&connection, "sessions"), 1);
        assert_eq!(table_count(&connection, "exec_events"), 3);
        assert!(table_exists(&connection, "source_files"));
        assert!(table_exists(&connection, "sessions_fts"));
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn canonical_rebuild_stores_all_classes_and_view_filters_at_read_time() {
        let fixture = SessionFixture::new();
        let normal = fixture.write_session_with_exec();
        let subsession = fixture.write_named_session("subsession.jsonl", "worker task", true);
        let empty = fixture.write_named_session("empty.jsonl", "   ", false);
        fixture.write_no_meta("no-meta.jsonl");
        let db = fixture.path("index.sqlite3");

        let outcome = build_index(&options(&fixture, db.clone())).unwrap();
        assert_eq!(outcome.summary.scanned_files, 4);
        assert_eq!(outcome.summary.parsed_files, 4);
        assert_eq!(outcome.summary.skipped_files, 1);
        assert_eq!(outcome.summary.session_rows, 3);
        assert_eq!(outcome.summary.exec_rows, 3);

        let paths = |view| {
            store::load_sessions_with_view(&db, view)
                .unwrap()
                .into_iter()
                .map(|row| row.path)
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            paths(store::SessionView::default()),
            BTreeSet::from([normal.clone()])
        );
        assert_eq!(
            paths(store::SessionView {
                include_subsessions: true,
                include_empty_messages: false,
            }),
            BTreeSet::from([normal.clone(), subsession])
        );
        assert_eq!(
            paths(store::SessionView {
                include_subsessions: false,
                include_empty_messages: true,
            }),
            BTreeSet::from([empty, normal])
        );
    }

    #[test]
    fn compatibility_include_options_do_not_change_canonical_storage() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        fixture.write_named_session("subsession.jsonl", "worker", true);
        fixture.write_named_session("empty.jsonl", "", false);

        let first_db = fixture.path("first.sqlite3");
        let first = build_index(&options(&fixture, first_db.clone())).unwrap();
        let mut all_options = options(&fixture, fixture.path("all.sqlite3"));
        all_options.include_subsessions = true;
        all_options.include_empty_messages = true;
        all_options.include_exec = true;
        let second = build_index(&all_options).unwrap();
        assert_eq!(first.summary.session_rows, second.summary.session_rows);
        assert_eq!(first.summary.exec_rows, second.summary.exec_rows);

        let counts = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            (
                conn.query_row("SELECT count(*) FROM source_files", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
                conn.query_row("SELECT count(*) FROM sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
                conn.query_row("SELECT count(*) FROM exec_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            )
        };
        assert_eq!(counts(&first_db), counts(&all_options.output));
    }

    #[test]
    fn unchanged_refresh_preserves_keys_and_reports_no_delta() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let connection = Connection::open(&db).unwrap();
        let session_key = connection
            .query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let exec_keys = {
            let mut stmt = connection
                .prepare("SELECT exec_key FROM exec_events ORDER BY exec_key")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        drop(connection);

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Incremental);
        assert_eq!(outcome.summary.parsed_files, 0);
        assert_eq!(outcome.summary.unchanged_files, 1);
        assert_eq!(outcome.delta, IndexDelta::default());

        let connection = Connection::open(db).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT session_key FROM sessions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            session_key
        );
        let after_exec_keys = {
            let mut stmt = connection
                .prepare("SELECT exec_key FROM exec_events ORDER BY exec_key")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(after_exec_keys, exec_keys);
    }

    #[test]
    fn missing_ordinary_index_is_repaired_without_parsing_jsonl() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute("DROP INDEX sessions_cwd_idx", []).unwrap();
        drop(conn);

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Incremental);
        assert_eq!(outcome.summary.parsed_files, 0);
        let conn = Connection::open(&db).unwrap();
        assert!(
            conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema
                    WHERE type = 'index' AND name = 'sessions_cwd_idx'
                 )",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        );
    }

    #[test]
    fn session_and_exec_updates_preserve_keys_and_report_exact_delta() {
        let fixture = SessionFixture::new();
        let source = fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        let source_key = conn
            .query_row("SELECT source_key FROM source_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let session_key = conn
            .query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let exec_key = conn
            .query_row(
                "SELECT exec_key FROM exec_events WHERE call_id = 'call-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        drop(conn);

        let contents = fs::read_to_string(&source)
            .unwrap()
            .replace("real user request", "updated user request with more detail")
            .replace("Script completed", "Execution refreshed");
        fs::write(&source, contents).unwrap();

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.changed_files, 1);
        assert_eq!(outcome.delta.updated_session_keys, vec![session_key]);
        assert_eq!(outcome.delta.updated_exec_keys, vec![exec_key]);
        assert_eq!(outcome.delta.touched_session_keys, vec![session_key]);

        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row("SELECT source_key FROM source_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            source_key
        );
        assert_eq!(
            conn.query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            session_key
        );
        assert_eq!(
            conn.query_row(
                "SELECT exec_key FROM exec_events WHERE call_id = 'call-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            exec_key
        );
        drop(conn);
        let index = search::SearchIndex::open(&db, store::SessionView::default()).unwrap();
        assert_eq!(
            index
                .search("updated", search::SearchScope::FirstMessage)
                .unwrap()
                .len(),
            1
        );
        assert!(
            index
                .search("real", search::SearchScope::FirstMessage)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            index
                .search("execution", search::SearchScope::Exec)
                .unwrap()
                .len(),
            1
        );
        assert!(
            index
                .search("script", search::SearchScope::Exec)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn stale_exec_rows_are_deleted_without_replacing_the_session_key() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        let session_key = conn
            .query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let exec_keys = {
            let mut stmt = conn
                .prepare("SELECT exec_key FROM exec_events ORDER BY exec_key")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        drop(conn);

        fixture.write_named_session("session.jsonl", "replacement", false);
        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.delta.updated_session_keys, vec![session_key]);
        assert_eq!(outcome.delta.deleted_exec_keys, exec_keys);
        assert_eq!(outcome.delta.touched_session_keys, vec![session_key]);
        let conn = Connection::open(db).unwrap();
        assert_eq!(table_count(&conn, "exec_events"), 0);
        assert_eq!(
            conn.query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            session_key
        );
    }

    #[test]
    fn deleted_source_cascades_and_reports_deleted_keys() {
        let fixture = SessionFixture::new();
        let source = fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        let session_key = conn
            .query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let exec_keys = {
            let mut stmt = conn
                .prepare("SELECT exec_key FROM exec_events ORDER BY exec_key")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        drop(conn);
        fs::remove_file(source).unwrap();

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.deleted_files, 1);
        assert_eq!(outcome.summary.parsed_files, 0);
        assert_eq!(outcome.delta.deleted_session_keys, vec![session_key]);
        assert_eq!(outcome.delta.deleted_exec_keys, exec_keys);
        assert_eq!(outcome.delta.touched_session_keys, vec![session_key]);
        let conn = Connection::open(db).unwrap();
        assert_eq!(table_count(&conn, "sessions"), 0);
        assert_eq!(table_count(&conn, "exec_events"), 0);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM source_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
    }

    #[test]
    fn skipped_source_removes_previous_session_and_is_cached() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("session.jsonl", "present", false);
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        fixture.write_no_meta("session.jsonl");

        let changed = build_index(&options).unwrap();
        assert_eq!(changed.summary.changed_files, 1);
        assert_eq!(changed.summary.skipped_files, 1);
        assert_eq!(changed.summary.session_rows, 0);
        let unchanged = build_index(&options).unwrap();
        assert_eq!(unchanged.summary.parsed_files, 0);
        assert_eq!(unchanged.summary.unchanged_files, 1);
        assert_eq!(unchanged.summary.skipped_files, 1);
    }

    #[test]
    fn legacy_database_is_rebuilt_from_jsonl() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                path TEXT, id TEXT, timestamp TEXT, cwd TEXT,
                repository_url TEXT, branch TEXT, first_message TEXT
             );
             INSERT INTO sessions VALUES (
                '/legacy.jsonl', NULL, NULL, NULL, NULL, NULL, 'legacy'
             );",
        )
        .unwrap();
        drop(conn);

        let outcome = build_index(&options(&fixture, db.clone())).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Rebuild);
        assert_eq!(outcome.summary.session_rows, 1);
        let conn = Connection::open(db).unwrap();
        assert_eq!(
            conn.query_row("SELECT first_message FROM sessions", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "real user request"
        );
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn schema_v1_migrates_without_reparsing_unchanged_sources() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();

        let mut conn = store::open_configured_connection(&db).unwrap();
        let tx = store::begin_immediate(&mut conn).unwrap();
        fts::drop_schema(&tx).unwrap();
        tx.execute("DROP TABLE message_events", []).unwrap();
        tx.execute_batch("PRAGMA user_version = 1;").unwrap();
        tx.commit().unwrap();
        assert!(matches!(
            schema::detect_schema(&conn).unwrap(),
            SchemaState::CanonicalV1 { .. }
        ));
        drop(conn);

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Incremental);
        assert_eq!(outcome.summary.parsed_files, 1);
        let index = search::SearchIndex::open(&db, store::SessionView::default()).unwrap();
        assert_eq!(
            index
                .search("read", search::SearchScope::Exec)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn dirty_fts_is_rebuilt_without_changing_session_key() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("session.jsonl", "original token", false);
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();

        let conn = Connection::open(&db).unwrap();
        let key = conn
            .query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        conn.execute(
            "UPDATE sessions SET first_message = 'external replacement'
             WHERE session_key = ?1",
            rusqlite::params![key],
        )
        .unwrap();
        assert_eq!(
            conn.query_row("SELECT dirty FROM fts_sync_state", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        drop(conn);

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.parsed_files, 0);
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            key
        );
        drop(conn);
        let index = search::SearchIndex::open(&db, store::SessionView::default()).unwrap();
        assert_eq!(
            index
                .search("external", search::SearchScope::FirstMessage)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            index
                .search("original", search::SearchScope::FirstMessage)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn missing_fts_row_is_repaired_without_canonical_rebuild() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("session.jsonl", "repair target", false);
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();

        let conn = Connection::open(&db).unwrap();
        let key = conn
            .query_row("SELECT session_key FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        conn.execute(
            "DELETE FROM sessions_fts WHERE rowid = ?1",
            rusqlite::params![key],
        )
        .unwrap();
        drop(conn);

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Incremental);
        assert_eq!(outcome.summary.parsed_files, 0);
        let index = search::SearchIndex::open(&db, store::SessionView::default()).unwrap();
        assert_eq!(
            index
                .search("repair", search::SearchScope::FirstMessage)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn extra_fts_row_is_repaired_without_canonical_rebuild() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("session.jsonl", "repair target", false);
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO sessions_fts(
                 rowid, user_content, agent_content, exec_command, exec_output
             ) VALUES (99999, 'extra', '', '', '')",
            [],
        )
        .unwrap();
        drop(conn);

        let outcome = build_index(&options).unwrap();
        assert_eq!(outcome.summary.mode, IndexMode::Incremental);
        assert_eq!(outcome.summary.parsed_files, 0);
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM sessions_fts WHERE rowid = 99999",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn forced_rebuild_repairs_corrupt_fts() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("session.jsonl", "corruption recovery", false);
        let db = fixture.path("index.sqlite3");
        let mut options = options(&fixture, db.clone());
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute("DELETE FROM sessions_fts_data WHERE id > 0", [])
            .unwrap();
        drop(conn);

        let error = search::SearchIndex::open(&db, store::SessionView::default())
            .err()
            .expect("corrupt FTS must not open");
        assert!(
            search::is_corruption(&error)
                || error.to_string().contains("malformed")
                || error.to_string().contains("corrupt")
        );

        options.rebuild = true;
        assert_eq!(
            build_index(&options).unwrap().summary.mode,
            IndexMode::Rebuild
        );
        let index = search::SearchIndex::open(&db, store::SessionView::default()).unwrap();
        assert_eq!(
            index
                .search("corrupt", search::SearchScope::FirstMessage)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn root_change_and_forced_rebuild_choose_rebuild_mode() {
        let first = SessionFixture::new();
        first.write_named_session("first.jsonl", "first", false);
        let second = SessionFixture::new();
        second.write_named_session("second.jsonl", "second", false);
        let db = first.path("index.sqlite3");
        let mut options = options(&first, db.clone());
        build_index(&options).unwrap();

        options.sessions_root = second.sessions_root();
        let root_changed = build_index(&options).unwrap();
        assert_eq!(root_changed.summary.mode, IndexMode::Rebuild);
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row("SELECT first_message FROM sessions", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "second"
        );
        drop(conn);

        options.rebuild = true;
        assert_eq!(
            build_index(&options).unwrap().summary.mode,
            IndexMode::Rebuild
        );
    }

    #[test]
    fn unknown_requires_rebuild_and_future_is_always_rejected() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("session.jsonl", "message", false);
        let unknown_db = fixture.path("unknown.sqlite3");
        let conn = Connection::open(&unknown_db).unwrap();
        conn.execute_batch("CREATE TABLE unrelated(value TEXT);")
            .unwrap();
        drop(conn);
        let mut unknown_options = options(&fixture, unknown_db.clone());
        assert!(
            build_index(&unknown_options)
                .unwrap_err()
                .to_string()
                .contains("--rebuild")
        );
        unknown_options.rebuild = true;
        assert_eq!(
            build_index(&unknown_options).unwrap().summary.session_rows,
            1
        );

        let future_db = fixture.path("future.sqlite3");
        let conn = Connection::open(&future_db).unwrap();
        conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        drop(conn);
        let mut future_options = options(&fixture, future_db);
        future_options.rebuild = true;
        assert!(
            build_index(&future_options)
                .unwrap_err()
                .to_string()
                .contains("newer")
        );
    }

    #[test]
    fn format_summary_matches_canonical_contract() {
        let summary = IndexSummary {
            mode: IndexMode::Incremental,
            scanned_files: 8,
            parsed_files: 3,
            new_files: 2,
            changed_files: 1,
            unchanged_files: 4,
            deleted_files: 1,
            unstable_files: 1,
            skipped_files: 2,
            session_rows: 6,
            exec_rows: 9,
            message_rows: 12,
            output: PathBuf::from("/tmp/index.sqlite3"),
        };
        assert_eq!(
            format_summary(&summary),
            "updated canonical index at /tmp/index.sqlite3: scanned 8 jsonl files; parsed 3 (2 new, 1 changed), kept 4 unchanged, removed 1 deleted, deferred 1 unstable, skipped 2; stored 6 sessions, 12 messages and 9 exec events"
        );
    }

    #[test]
    fn lexical_paths_remove_dot_and_parent_components() {
        let path = absolute_lexical_path(Path::new("a/./b/../c")).unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("a/c"));
        assert_eq!(
            absolute_lexical_path(Path::new("/../../tmp/sessions")).unwrap(),
            PathBuf::from("/tmp/sessions")
        );
    }

    #[test]
    #[ignore = "release-only synthetic benchmark"]
    fn canonical_incremental_benchmark() {
        fn median(mut values: Vec<Duration>) -> Duration {
            values.sort_unstable();
            values[values.len() / 2]
        }

        for file_count in [100_usize, 1_000, 10_000] {
            let fixture = SessionFixture::new();
            let day = fixture.sessions_root().join("2026/07/27");
            fs::create_dir_all(&day).unwrap();
            let output = "x".repeat(1024);
            let mut paths = Vec::with_capacity(file_count);
            for index in 0..file_count {
                let path = day.join(format!("session-{index:05}.jsonl"));
                let mut records = vec![
                    json!({
                        "type": "session_meta",
                        "payload": {
                            "id": format!("benchmark-{index}"),
                            "timestamp": "2026-07-27T00:00:00Z",
                            "cwd": "/tmp/benchmark path",
                            "source": "cli",
                            "thread_source": "user",
                            "git": {
                                "repository_url": "https://git.example/benchmark.git",
                                "branch": "main"
                            }
                        }
                    }),
                    json!({
                        "type": "event_msg",
                        "payload": {
                            "type": "user_message",
                            "message": "증분 인덱스 확인"
                        }
                    }),
                ];
                for event in 0..10 {
                    records.push(json!({
                        "type": "event_msg",
                        "payload": {
                            "type": "exec_command_end",
                            "parsed_cmd": [{
                                "type": "read",
                                "cmd": format!("printf '\\\"/{index}/{event}\\\"'"),
                                "name": "benchmark"
                            }],
                            "aggregated_output": output
                        }
                    }));
                }
                let contents = records
                    .into_iter()
                    .map(|record| record.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                fs::write(&path, format!("{contents}\n")).unwrap();
                paths.push(path);
            }

            let db = fixture.path("canonical.sqlite3");
            let mut options = options(&fixture, db.clone());
            options.rebuild = true;
            build_index(&options).unwrap();
            let fresh = median(
                (0..5)
                    .map(|_| {
                        let started = Instant::now();
                        let outcome = build_index(&options).unwrap();
                        assert_eq!(outcome.summary.parsed_files, file_count);
                        started.elapsed()
                    })
                    .collect(),
            );
            let canonical_bytes = fs::metadata(&db).unwrap().len();

            let legacy_db = fixture.path("legacy.sqlite3");
            let data = crate::collect_session_data(
                &fixture.sessions_root(),
                crate::CollectOptions {
                    include_subsessions: true,
                    include_empty_messages: true,
                    include_exec: true,
                },
            )
            .unwrap();
            crate::recreate_database_with_exec(&legacy_db, &data.rows, Some(&data.exec_events))
                .unwrap();
            let legacy_bytes = fs::metadata(legacy_db).unwrap().len();

            options.rebuild = false;
            build_index(&options).unwrap();
            let no_op = median(
                (0..5)
                    .map(|_| {
                        let started = Instant::now();
                        let outcome = build_index(&options).unwrap();
                        assert_eq!(outcome.summary.parsed_files, 0);
                        started.elapsed()
                    })
                    .collect(),
            );

            let mut append_index = 0;
            let mut append_once = || {
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .open(&paths[append_index])
                    .unwrap();
                writeln!(file).unwrap();
                append_index += 1;
                let started = Instant::now();
                let outcome = build_index(&options).unwrap();
                assert_eq!(outcome.summary.parsed_files, 1);
                started.elapsed()
            };
            append_once();
            let one_change = median((0..5).map(|_| append_once()).collect());

            let mut delete_index = 0;
            let mut delete_once = || {
                fs::remove_file(&paths[delete_index]).unwrap();
                delete_index += 1;
                let started = Instant::now();
                let outcome = build_index(&options).unwrap();
                assert_eq!(outcome.summary.parsed_files, 0);
                assert_eq!(outcome.summary.deleted_files, 1);
                started.elapsed()
            };
            delete_once();
            let delete = median((0..5).map(|_| delete_once()).collect());

            if file_count >= 1_000 {
                assert!(no_op < fresh);
            }
            if file_count == 10_000 {
                assert!(canonical_bytes * 2 <= legacy_bytes * 3);
            }
            println!(
                "{file_count}: fresh={fresh:?}, no-op={no_op:?}, change={one_change:?}, delete={delete:?}, canonical={canonical_bytes}, legacy={legacy_bytes}, os={}",
                std::env::consts::OS
            );
        }
    }
}
