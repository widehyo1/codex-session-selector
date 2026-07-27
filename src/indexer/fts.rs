use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{IndexDelta, SessionKey};
use crate::session_event::MessageRole;

pub(crate) const FTS_MIN_SQLITE: (u32, u32, u32) = (3, 43, 0);

pub(crate) const FTS_DDL: &str = r#"
CREATE TABLE fts_sync_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    dirty INTEGER NOT NULL CHECK (dirty IN (0, 1))
) STRICT;

INSERT INTO fts_sync_state(singleton, dirty) VALUES (1, 1);

CREATE VIRTUAL TABLE sessions_fts USING fts5(
    user_content,
    agent_content,
    exec_command,
    exec_output,
    content='',
    contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2',
    prefix='2 3',
    detail=full,
    columnsize=1
);
"#;

pub(crate) const FTS_TRIGGER_DDL: &str = r#"
CREATE TRIGGER sessions_fts_dirty_ai
AFTER INSERT ON sessions
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER sessions_fts_dirty_au
AFTER UPDATE ON sessions
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER sessions_fts_dirty_ad
AFTER DELETE ON sessions
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER exec_events_fts_dirty_ai
AFTER INSERT ON exec_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER exec_events_fts_dirty_au
AFTER UPDATE ON exec_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER exec_events_fts_dirty_ad
AFTER DELETE ON exec_events
BEGIN
    UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1;
END;

CREATE TRIGGER message_events_fts_dirty_ai AFTER INSERT ON message_events BEGIN UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1; END;
CREATE TRIGGER message_events_fts_dirty_au AFTER UPDATE ON message_events BEGIN UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1; END;
CREATE TRIGGER message_events_fts_dirty_ad AFTER DELETE ON message_events BEGIN UPDATE fts_sync_state SET dirty = 1 WHERE singleton = 1; END;
"#;

const INSERT_DOCUMENT: &str = "
    INSERT OR REPLACE INTO sessions_fts(
        rowid, user_content, agent_content, exec_command, exec_output
    ) VALUES (?1, ?2, ?3, ?4, ?5)";

const DIRTY_TRIGGERS: [&str; 9] = [
    "sessions_fts_dirty_ai",
    "sessions_fts_dirty_au",
    "sessions_fts_dirty_ad",
    "exec_events_fts_dirty_ai",
    "exec_events_fts_dirty_au",
    "exec_events_fts_dirty_ad",
    "message_events_fts_dirty_ai",
    "message_events_fts_dirty_au",
    "message_events_fts_dirty_ad",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchDocument {
    pub session_key: SessionKey,
    pub user_content: String,
    pub agent_content: String,
    pub exec_command: String,
    pub exec_output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FtsSyncMode {
    Delta,
    Populate,
    Rebuild,
}

pub(crate) fn verify_runtime(conn: &Connection) -> Result<()> {
    let version = conn.query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))?;
    let parsed = parse_sqlite_version(&version);
    let fts5 = conn.query_row(
        "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if parsed.is_none_or(|parsed| parsed < FTS_MIN_SQLITE) || fts5 != 1 {
        bail!("SQLite 3.43.0 or newer with FTS5 is required; found {version}");
    }
    Ok(())
}

fn parse_sqlite_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut components = version.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    let patch = components.next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub(crate) fn create_schema(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(FTS_DDL)?;
    tx.execute_batch(FTS_TRIGGER_DDL)?;
    Ok(())
}

pub(crate) fn drop_schema(tx: &Transaction<'_>) -> Result<()> {
    for trigger in DIRTY_TRIGGERS {
        tx.execute_batch(&format!("DROP TRIGGER IF EXISTS {trigger};"))?;
    }
    tx.execute_batch(
        "DROP TABLE IF EXISTS sessions_fts;
         DROP TABLE IF EXISTS fts_sync_state;",
    )?;
    Ok(())
}

pub(crate) fn preflight(tx: &Transaction<'_>) -> Result<FtsSyncMode> {
    let state = tx
        .query_row(
            "SELECT dirty FROM fts_sync_state WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if state != Some(0) {
        return Ok(FtsSyncMode::Rebuild);
    }
    if row_identity_mismatch(tx)? {
        return Ok(FtsSyncMode::Rebuild);
    }
    Ok(FtsSyncMode::Delta)
}

pub(crate) fn load_document(
    tx: &Transaction<'_>,
    session_key: SessionKey,
) -> Result<Option<SearchDocument>> {
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_key = ?1)",
        params![session_key],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let mut users = Vec::new();
    let mut agents = Vec::new();
    let mut messages = tx.prepare(
        "SELECT role, content FROM message_events WHERE session_key = ?1 ORDER BY event_index",
    )?;
    for row in messages.query_map(params![session_key], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (role, content) = row?;
        match MessageRole::from_str(&role)? {
            MessageRole::User => users.push(content),
            MessageRole::Agent => agents.push(content),
        }
    }

    let mut stmt = tx.prepare(
        "SELECT command, output
         FROM exec_events
         WHERE session_key = ?1
         ORDER BY event_index",
    )?;
    let exec = stmt
        .query_map(params![session_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (commands, outputs): (Vec<_>, Vec<_>) = exec.into_iter().unzip();

    Ok(Some(SearchDocument {
        session_key,
        user_content: users.join("\n"),
        agent_content: agents.join("\n"),
        exec_command: commands.join("\n"),
        exec_output: outputs.join("\n"),
    }))
}

fn insert_document(tx: &Transaction<'_>, document: &SearchDocument) -> Result<()> {
    tx.execute(
        INSERT_DOCUMENT,
        params![
            document.session_key,
            document.user_content,
            document.agent_content,
            document.exec_command,
            document.exec_output,
        ],
    )?;
    Ok(())
}

pub(crate) fn populate_all(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare("SELECT session_key FROM sessions ORDER BY session_key")?;
    let keys = stmt
        .query_map([], |row| row.get::<_, SessionKey>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for session_key in keys {
        let document = load_document(tx, session_key)?
            .with_context(|| format!("session {session_key} disappeared during FTS population"))?;
        insert_document(tx, &document)?;
    }
    Ok(())
}

pub(crate) fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    drop_schema(tx)?;
    create_schema(tx)?;
    populate_all(tx)
}

pub(crate) fn apply_delta(tx: &Transaction<'_>, delta: &IndexDelta) -> Result<()> {
    let deleted = delta
        .deleted_session_keys
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for session_key in &deleted {
        tx.execute(
            "DELETE FROM sessions_fts WHERE rowid = ?1",
            params![session_key],
        )?;
    }

    let touched = delta
        .touched_session_keys
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for session_key in touched {
        if deleted.contains(&session_key) {
            continue;
        }
        let document = load_document(tx, session_key)?
            .with_context(|| format!("touched session {session_key} is missing"))?;
        insert_document(tx, &document)?;
    }
    Ok(())
}

pub(crate) fn verify_invariants(tx: &Transaction<'_>, check_internal_index: bool) -> Result<()> {
    if check_internal_index {
        tx.execute(
            "INSERT INTO sessions_fts(sessions_fts, rank)
             VALUES('integrity-check', 0)",
            [],
        )?;
    }
    ensure!(!row_identity_mismatch(tx)?, "FTS row identity mismatch");
    Ok(())
}

fn row_identity_mismatch(tx: &Transaction<'_>) -> Result<bool> {
    let missing = tx.query_row(
        "SELECT count(*)
         FROM sessions AS s
         LEFT JOIN sessions_fts AS f ON f.rowid = s.session_key
         WHERE f.rowid IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let extra = tx.query_row(
        "SELECT count(*)
         FROM sessions_fts AS f
         LEFT JOIN sessions AS s ON s.session_key = f.rowid
         WHERE s.session_key IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(missing != 0 || extra != 0)
}

pub(crate) fn mark_clean(tx: &Transaction<'_>) -> Result<()> {
    let dirty = tx.query_row(
        "SELECT dirty FROM fts_sync_state WHERE singleton = 1",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if dirty == 0 {
        return Ok(());
    }
    let changed = tx.execute(
        "UPDATE fts_sync_state SET dirty = 0 WHERE singleton = 1",
        [],
    )?;
    ensure!(changed == 1, "FTS sync state singleton is missing");
    let clean = tx.query_row(
        "SELECT dirty FROM fts_sync_state WHERE singleton = 1",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    ensure!(clean == 0, "FTS sync state did not become clean");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_sqlite_supports_contentless_delete_fts5() {
        let conn = Connection::open_in_memory().unwrap();
        verify_runtime(&conn).unwrap();
    }

    #[test]
    fn sqlite_version_parser_is_numeric() {
        assert_eq!(parse_sqlite_version("3.53.2"), Some((3, 53, 2)));
        assert_eq!(parse_sqlite_version("3.9.0"), Some((3, 9, 0)));
        assert_eq!(parse_sqlite_version("invalid"), None);
    }
}
