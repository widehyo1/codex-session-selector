use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{ExecEvent, MessageEvent, SessionRow};

use super::{
    ExecKey, IndexDelta, MessageKey, SessionKey,
    scan::{FileFingerprint, ParseStatus, ParsedSource, ScanPlan, StoredSource},
    schema,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SessionView {
    pub include_subsessions: bool,
    pub include_empty_messages: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredCounts {
    pub skipped_files: usize,
    pub session_rows: usize,
    pub exec_rows: usize,
    pub message_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredSession {
    key: SessionKey,
    row: SessionRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredExec {
    key: ExecKey,
    event: ExecEvent,
}

pub(crate) fn open_configured_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

pub(crate) fn load_fingerprints(conn: &Connection) -> Result<BTreeMap<PathBuf, StoredSource>> {
    let mut stmt = conn.prepare(
        "SELECT path, file_size, modified_secs, modified_nanos, parse_status
         FROM source_files
         ORDER BY path",
    )?;
    let rows = stmt.query_map([], |row| {
        let size = row.get::<_, i64>(1)?;
        let nanos = row.get::<_, i64>(3)?;
        match row.get::<_, String>(4)?.as_str() {
            "indexed" | "skipped" => {}
            value => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    format!("invalid parse status {value}").into(),
                ));
            }
        }
        Ok((
            PathBuf::from(row.get::<_, String>(0)?),
            StoredSource {
                fingerprint: FileFingerprint {
                    size: u64::try_from(size).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                    modified_secs: row.get(2)?,
                    modified_nanos: u32::try_from(nanos).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                },
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn begin_immediate(conn: &mut Connection) -> Result<Transaction<'_>> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

pub(crate) fn apply_rebuild(
    tx: &Transaction<'_>,
    root: &Path,
    plan: &ScanPlan,
) -> Result<IndexDelta> {
    schema::drop_user_schema(tx)?;
    schema::create_schema(tx)?;
    tx.execute(
        "INSERT INTO index_metadata(singleton, sessions_root) VALUES (1, ?1)",
        params![root.to_string_lossy().as_ref()],
    )?;

    let mut delta = IndexDelta::default();
    for source in plan.new_sources.iter().chain(plan.changed_sources.iter()) {
        apply_source(tx, source, &mut delta)?;
    }
    Ok(delta)
}

pub(crate) fn apply_incremental(
    tx: &Transaction<'_>,
    _root: &Path,
    plan: &ScanPlan,
) -> Result<IndexDelta> {
    schema::ensure_indexes(tx)?;
    let mut delta = IndexDelta::default();

    for path in &plan.deleted_paths {
        delete_source(tx, path, &mut delta)?;
    }
    for source in plan.new_sources.iter().chain(plan.changed_sources.iter()) {
        apply_source(tx, source, &mut delta)?;
    }
    Ok(delta)
}

fn apply_source(tx: &Transaction<'_>, source: &ParsedSource, delta: &mut IndexDelta) -> Result<()> {
    let size = i64::try_from(source.fingerprint.size)
        .with_context(|| format!("file is too large to index: {}", source.path.display()))?;
    let status = if source.session.is_some() {
        ParseStatus::Indexed
    } else {
        ParseStatus::Skipped
    };
    let source_key = tx.query_row(
        "INSERT INTO source_files(
             path, file_size, modified_secs, modified_nanos, parse_status
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET
             file_size = excluded.file_size,
             modified_secs = excluded.modified_secs,
             modified_nanos = excluded.modified_nanos,
             parse_status = excluded.parse_status
         RETURNING source_key",
        params![
            source.path.to_string_lossy().as_ref(),
            size,
            source.fingerprint.modified_secs,
            i64::from(source.fingerprint.modified_nanos),
            status.as_str(),
        ],
        |row| row.get::<_, i64>(0),
    )?;

    let existing = load_session_for_source(tx, source_key)?;
    let Some(parsed) = source.session.as_ref() else {
        if let Some(existing) = existing {
            record_session_deletion(tx, existing.key, delta)?;
            tx.execute(
                "DELETE FROM sessions WHERE session_key = ?1",
                params![existing.key],
            )?;
        }
        return Ok(());
    };

    let session_key = match existing {
        None => {
            let key = insert_session(tx, source_key, &parsed.row)?;
            delta.inserted_session_keys.push(key);
            delta.touched_session_keys.push(key);
            key
        }
        Some(existing) => {
            if existing.row != parsed.row {
                update_session(tx, existing.key, &parsed.row)?;
                delta.updated_session_keys.push(existing.key);
                delta.touched_session_keys.push(existing.key);
            }
            existing.key
        }
    };
    apply_exec_diff(tx, session_key, &parsed.exec_events, delta)?;
    apply_message_diff(tx, session_key, &parsed.message_events, delta)?;
    Ok(())
}

fn load_session_for_source(tx: &Transaction<'_>, source_key: i64) -> Result<Option<StoredSession>> {
    Ok(tx
        .query_row(
            "SELECT session_key, path, id, timestamp, cwd, repository_url, branch,
                    first_message, is_subsession
             FROM sessions
             WHERE source_key = ?1",
            params![source_key],
            |row| {
                Ok(StoredSession {
                    key: row.get(0)?,
                    row: SessionRow {
                        path: PathBuf::from(row.get::<_, String>(1)?),
                        id: row.get(2)?,
                        timestamp: row.get(3)?,
                        cwd: row.get(4)?,
                        repository_url: row.get(5)?,
                        branch: row.get(6)?,
                        first_message: row.get(7)?,
                        is_subsession: row.get::<_, i64>(8)? != 0,
                    },
                })
            },
        )
        .optional()?)
}

fn insert_session(tx: &Transaction<'_>, source_key: i64, row: &SessionRow) -> Result<i64> {
    Ok(tx.query_row(
        "INSERT INTO sessions(
             path, id, timestamp, cwd, repository_url, branch, first_message,
             source_key, is_subsession, has_nonempty_first_message
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         RETURNING session_key",
        params![
            row.path.to_string_lossy().as_ref(),
            row.id.as_deref(),
            row.timestamp.as_deref(),
            row.cwd.as_deref(),
            row.repository_url.as_deref(),
            row.branch.as_deref(),
            row.first_message.as_str(),
            source_key,
            i64::from(row.is_subsession),
            i64::from(!row.first_message.trim().is_empty()),
        ],
        |result| result.get(0),
    )?)
}

fn update_session(tx: &Transaction<'_>, session_key: i64, row: &SessionRow) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET
             path = ?1,
             id = ?2,
             timestamp = ?3,
             cwd = ?4,
             repository_url = ?5,
             branch = ?6,
             first_message = ?7,
             is_subsession = ?8,
             has_nonempty_first_message = ?9
         WHERE session_key = ?10",
        params![
            row.path.to_string_lossy().as_ref(),
            row.id.as_deref(),
            row.timestamp.as_deref(),
            row.cwd.as_deref(),
            row.repository_url.as_deref(),
            row.branch.as_deref(),
            row.first_message.as_str(),
            i64::from(row.is_subsession),
            i64::from(!row.first_message.trim().is_empty()),
            session_key,
        ],
    )?;
    Ok(())
}

fn load_execs(tx: &Transaction<'_>, session_key: i64) -> Result<BTreeMap<usize, StoredExec>> {
    let mut stmt = tx.prepare(
        "SELECT exec_key, session_path, session_id, event_index, call_id,
                kind, name, command, output
         FROM exec_events
         WHERE session_key = ?1
         ORDER BY event_index",
    )?;
    let rows = stmt.query_map(params![session_key], |row| {
        let event_index = row.get::<_, i64>(3)?;
        let event_index = usize::try_from(event_index).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?;
        Ok((
            event_index,
            StoredExec {
                key: row.get(0)?,
                event: ExecEvent {
                    session_path: PathBuf::from(row.get::<_, String>(1)?),
                    session_id: row.get(2)?,
                    event_index,
                    call_id: row.get(4)?,
                    kind: row.get(5)?,
                    name: row.get(6)?,
                    command: row.get(7)?,
                    output: row.get(8)?,
                },
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn apply_exec_diff(
    tx: &Transaction<'_>,
    session_key: i64,
    parsed: &[ExecEvent],
    delta: &mut IndexDelta,
) -> Result<()> {
    let mut existing = load_execs(tx, session_key)?;
    let mut parsed_by_index = BTreeMap::new();
    for event in parsed {
        if parsed_by_index.insert(event.event_index, event).is_some() {
            bail!(
                "duplicate exec event index {} in {}",
                event.event_index,
                event.session_path.display()
            );
        }
    }

    for (event_index, event) in parsed_by_index {
        match existing.remove(&event_index) {
            None => {
                let key = insert_exec(tx, session_key, event)?;
                delta.inserted_exec_keys.push(key);
                delta.touched_session_keys.push(session_key);
            }
            Some(stored) if stored.event != *event => {
                update_exec(tx, stored.key, event)?;
                delta.updated_exec_keys.push(stored.key);
                delta.touched_session_keys.push(session_key);
            }
            Some(_) => {}
        }
    }

    for stored in existing.into_values() {
        tx.execute(
            "DELETE FROM exec_events WHERE exec_key = ?1",
            params![stored.key],
        )?;
        delta.deleted_exec_keys.push(stored.key);
        delta.touched_session_keys.push(session_key);
    }
    Ok(())
}

fn apply_message_diff(
    tx: &Transaction<'_>,
    session_key: SessionKey,
    parsed: &[MessageEvent],
    delta: &mut IndexDelta,
) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT message_key, event_index, role, content FROM message_events WHERE session_key = ?1",
    )?;
    let mut existing = BTreeMap::new();
    for row in stmt.query_map(params![session_key], |row| {
        Ok((
            usize::try_from(row.get::<_, i64>(1)?).unwrap(),
            row.get::<_, MessageKey>(0)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })? {
        let (index, key, role, content) = row?;
        existing.insert(index, (key, role, content));
    }
    for event in parsed {
        match existing.remove(&event.event_index) {
            None => {
                let key = tx.query_row("INSERT INTO message_events(event_index, role, content, session_key) VALUES (?1, ?2, ?3, ?4) RETURNING message_key", params![i64::try_from(event.event_index)?, event.role.as_str(), event.content, session_key], |row| row.get(0))?;
                delta.inserted_message_keys.push(key);
                delta.touched_session_keys.push(session_key);
            }
            Some((key, role, content))
                if role != event.role.as_str() || content != event.content =>
            {
                tx.execute(
                    "UPDATE message_events SET role=?1, content=?2 WHERE message_key=?3",
                    params![event.role.as_str(), event.content, key],
                )?;
                delta.updated_message_keys.push(key);
                delta.touched_session_keys.push(session_key);
            }
            _ => {}
        }
    }
    for (_, (key, _, _)) in existing {
        tx.execute(
            "DELETE FROM message_events WHERE message_key = ?1",
            params![key],
        )?;
        delta.deleted_message_keys.push(key);
        delta.touched_session_keys.push(session_key);
    }
    Ok(())
}

fn insert_exec(tx: &Transaction<'_>, session_key: i64, event: &ExecEvent) -> Result<i64> {
    Ok(tx.query_row(
        "INSERT INTO exec_events(
             session_path, session_id, event_index, call_id,
             kind, name, command, output, session_key
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         RETURNING exec_key",
        params![
            event.session_path.to_string_lossy().as_ref(),
            event.session_id.as_deref(),
            i64::try_from(event.event_index)
                .with_context(|| "exec event index does not fit in SQLite INTEGER")?,
            event.call_id.as_deref(),
            event.kind.as_str(),
            event.name.as_deref(),
            event.command.as_str(),
            event.output.as_str(),
            session_key,
        ],
        |row| row.get(0),
    )?)
}

fn update_exec(tx: &Transaction<'_>, exec_key: i64, event: &ExecEvent) -> Result<()> {
    tx.execute(
        "UPDATE exec_events SET
             session_path = ?1,
             session_id = ?2,
             call_id = ?3,
             kind = ?4,
             name = ?5,
             command = ?6,
             output = ?7
         WHERE exec_key = ?8",
        params![
            event.session_path.to_string_lossy().as_ref(),
            event.session_id.as_deref(),
            event.call_id.as_deref(),
            event.kind.as_str(),
            event.name.as_deref(),
            event.command.as_str(),
            event.output.as_str(),
            exec_key,
        ],
    )?;
    Ok(())
}

fn delete_source(tx: &Transaction<'_>, path: &Path, delta: &mut IndexDelta) -> Result<()> {
    let session_key = tx
        .query_row(
            "SELECT sessions.session_key
             FROM sessions
             JOIN source_files USING(source_key)
             WHERE source_files.path = ?1",
            params![path.to_string_lossy().as_ref()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(session_key) = session_key {
        record_session_deletion(tx, session_key, delta)?;
    }
    tx.execute(
        "DELETE FROM source_files WHERE path = ?1",
        params![path.to_string_lossy().as_ref()],
    )?;
    Ok(())
}

fn record_session_deletion(
    tx: &Transaction<'_>,
    session_key: i64,
    delta: &mut IndexDelta,
) -> Result<()> {
    let mut stmt =
        tx.prepare("SELECT exec_key FROM exec_events WHERE session_key = ?1 ORDER BY exec_key")?;
    let keys = stmt
        .query_map(params![session_key], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    delta.deleted_exec_keys.extend(keys);
    let mut messages = tx.prepare(
        "SELECT message_key FROM message_events WHERE session_key = ?1 ORDER BY message_key",
    )?;
    delta.deleted_message_keys.extend(
        messages
            .query_map(params![session_key], |row| row.get::<_, MessageKey>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    delta.deleted_session_keys.push(session_key);
    delta.touched_session_keys.push(session_key);
    Ok(())
}

pub(crate) fn verify_invariants(tx: &Transaction<'_>) -> Result<()> {
    let mut foreign_keys = tx.prepare("PRAGMA foreign_key_check")?;
    if foreign_keys.query([])?.next()?.is_some() {
        bail!("foreign key check failed");
    }

    let quick_check = tx.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))?;
    if quick_check != "ok" {
        bail!("SQLite quick check failed: {quick_check}");
    }

    let metadata_count = tx.query_row("SELECT count(*) FROM index_metadata", [], |row| {
        row.get::<_, i64>(0)
    })?;
    if metadata_count != 1 {
        bail!("index_metadata must contain exactly one row");
    }

    let indexed_sources = tx.query_row(
        "SELECT count(*) FROM source_files WHERE parse_status = 'indexed'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let sessions = tx.query_row("SELECT count(*) FROM sessions", [], |row| {
        row.get::<_, i64>(0)
    })?;
    if indexed_sources != sessions {
        bail!("indexed source and session counts differ");
    }

    let skipped_with_session = tx.query_row(
        "SELECT count(*)
         FROM source_files
         JOIN sessions USING(source_key)
         WHERE parse_status = 'skipped'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if skipped_with_session != 0 {
        bail!("skipped sources must not have sessions");
    }
    Ok(())
}

pub(crate) fn query_counts(tx: &Transaction<'_>) -> Result<StoredCounts> {
    fn count(tx: &Transaction<'_>, sql: &str) -> Result<usize> {
        let value = tx.query_row(sql, [], |row| row.get::<_, i64>(0))?;
        Ok(usize::try_from(value)?)
    }
    Ok(StoredCounts {
        skipped_files: count(
            tx,
            "SELECT count(*) FROM source_files WHERE parse_status = 'skipped'",
        )?,
        session_rows: count(tx, "SELECT count(*) FROM sessions")?,
        exec_rows: count(tx, "SELECT count(*) FROM exec_events")?,
        message_rows: count(tx, "SELECT count(*) FROM message_events")?,
    })
}

pub(crate) fn load_sessions_with_view(
    db_path: &Path,
    view: SessionView,
) -> Result<Vec<SessionRow>> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    let canonical = conn
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM pragma_table_info('sessions')
                 WHERE name = 'is_subsession'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    let (sql, arguments): (&str, Vec<i64>) = if canonical {
        (
            "SELECT path, id, timestamp, cwd, repository_url, branch,
                    first_message, is_subsession
             FROM sessions
             WHERE (?1 = 1 OR is_subsession = 0)
               AND (?2 = 1 OR has_nonempty_first_message = 1)
             ORDER BY timestamp DESC",
            vec![
                i64::from(view.include_subsessions),
                i64::from(view.include_empty_messages),
            ],
        )
    } else {
        (
            "SELECT path, id, timestamp, cwd, repository_url, branch,
                    first_message, 0
             FROM sessions
             ORDER BY timestamp DESC",
            Vec::new(),
        )
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(arguments), |row| {
            Ok(SessionRow {
                path: PathBuf::from(row.get::<_, String>(0)?),
                id: row.get(1)?,
                timestamp: row.get(2)?,
                cwd: row.get(3)?,
                repository_url: row.get(4)?,
                branch: row.get(5)?,
                first_message: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                is_subsession: row.get::<_, i64>(7)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn normalize_delta(delta: &mut IndexDelta) {
    fn normalize(values: &mut Vec<i64>) {
        values.sort_unstable();
        values.dedup();
    }
    normalize(&mut delta.inserted_session_keys);
    normalize(&mut delta.updated_session_keys);
    normalize(&mut delta.deleted_session_keys);
    normalize(&mut delta.inserted_exec_keys);
    normalize(&mut delta.updated_exec_keys);
    normalize(&mut delta.deleted_exec_keys);
    normalize(&mut delta.touched_session_keys);
}

pub(crate) fn set_schema_version(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(&format!(
        "PRAGMA user_version = {};",
        schema::SCHEMA_VERSION
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::IndexOptions,
        indexer::{self, scan::scan_sources, schema::detect_schema},
        test_support::SessionFixture,
    };

    fn options(fixture: &SessionFixture, output: PathBuf) -> IndexOptions {
        IndexOptions {
            output,
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        }
    }

    #[test]
    fn legacy_load_ignores_view_filters_it_cannot_reconstruct() {
        let fixture = SessionFixture::new();
        let db = fixture.path("legacy.sqlite3");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                path TEXT, id TEXT, timestamp TEXT, cwd TEXT,
                repository_url TEXT, branch TEXT, first_message TEXT
             );
             INSERT INTO sessions VALUES (
                '/legacy.jsonl', 'legacy', NULL, NULL, NULL, NULL, ''
             );",
        )
        .unwrap();
        drop(conn);

        let rows = load_sessions_with_view(&db, SessionView::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, PathBuf::from("/legacy.jsonl"));
    }

    #[test]
    fn failed_incremental_transaction_rolls_back_all_tables() {
        let fixture = SessionFixture::new();
        let source = fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");
        let options = options(&fixture, db.clone());
        indexer::build_index(&options).unwrap();

        let old_contents = fs::read_to_string(&source).unwrap();
        fs::write(
            &source,
            old_contents.replace("real user request", "updated request that is longer"),
        )
        .unwrap();

        let mut conn = open_configured_connection(&db).unwrap();
        assert!(matches!(
            detect_schema(&conn).unwrap(),
            schema::SchemaState::Current { .. }
        ));
        let stored = load_fingerprints(&conn).unwrap();
        let plan = scan_sources(&fixture.sessions_root(), &stored, false).unwrap();
        let before = conn
            .query_row(
                "SELECT source_files.file_size, sessions.first_message
                 FROM source_files JOIN sessions USING(source_key)",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_session_update
             BEFORE UPDATE ON sessions
             BEGIN
                 SELECT RAISE(ABORT, 'injected session update failure');
             END;",
        )
        .unwrap();

        let tx = begin_immediate(&mut conn).unwrap();
        let error = apply_incremental(&tx, &fixture.sessions_root(), &plan).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected session update failure")
        );
        drop(tx);

        let after = conn
            .query_row(
                "SELECT source_files.file_size, sessions.first_message
                 FROM source_files JOIN sessions USING(source_key)",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn failed_rebuild_transaction_preserves_legacy_database() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("legacy.sqlite3");
        let mut conn = open_configured_connection(&db).unwrap();
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
        let mut plan = scan_sources(&fixture.sessions_root(), &BTreeMap::new(), true).unwrap();
        let event = plan.new_sources[0].session.as_ref().unwrap().exec_events[0].clone();
        plan.new_sources[0]
            .session
            .as_mut()
            .unwrap()
            .exec_events
            .push(event);

        let tx = begin_immediate(&mut conn).unwrap();
        assert!(apply_rebuild(&tx, &fixture.sessions_root(), &plan).is_err());
        drop(tx);

        assert_eq!(
            conn.query_row("SELECT first_message FROM sessions", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "legacy"
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM pragma_table_info('sessions')",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
            7
        );
    }
}
