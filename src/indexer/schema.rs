use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, Transaction, params};

pub(crate) const SCHEMA_VERSION: i64 = 1;

pub(crate) const TABLE_DDL: &str = r#"
CREATE TABLE index_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    sessions_root TEXT NOT NULL
) STRICT;

CREATE TABLE source_files (
    source_key INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    modified_secs INTEGER NOT NULL CHECK (modified_secs >= 0),
    modified_nanos INTEGER NOT NULL
        CHECK (modified_nanos >= 0 AND modified_nanos < 1000000000),
    parse_status TEXT NOT NULL
        CHECK (parse_status IN ('indexed', 'skipped'))
) STRICT;

CREATE TABLE sessions (
    path TEXT NOT NULL,
    id TEXT,
    timestamp TEXT,
    cwd TEXT,
    repository_url TEXT,
    branch TEXT,
    first_message TEXT NOT NULL,
    session_key INTEGER PRIMARY KEY,
    source_key INTEGER NOT NULL UNIQUE
        REFERENCES source_files(source_key) ON DELETE CASCADE,
    is_subsession INTEGER NOT NULL
        CHECK (is_subsession IN (0, 1)),
    has_nonempty_first_message INTEGER NOT NULL
        CHECK (has_nonempty_first_message IN (0, 1)),
    UNIQUE (path)
) STRICT;

CREATE TABLE exec_events (
    session_path TEXT NOT NULL,
    session_id TEXT,
    event_index INTEGER NOT NULL CHECK (event_index >= 0),
    call_id TEXT,
    kind TEXT NOT NULL,
    name TEXT,
    command TEXT NOT NULL,
    output TEXT NOT NULL,
    exec_key INTEGER PRIMARY KEY,
    session_key INTEGER NOT NULL
        REFERENCES sessions(session_key) ON DELETE CASCADE,
    UNIQUE (session_key, event_index)
) STRICT;
"#;

pub(crate) const INDEX_DDL: &str = r#"
CREATE INDEX IF NOT EXISTS sessions_timestamp_idx ON sessions(timestamp);
CREATE INDEX IF NOT EXISTS sessions_cwd_idx ON sessions(cwd);
CREATE INDEX IF NOT EXISTS exec_events_session_path_idx ON exec_events(session_path);
CREATE INDEX IF NOT EXISTS exec_events_session_id_idx ON exec_events(session_id);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaState {
    Empty,
    Legacy,
    Current { sessions_root: PathBuf },
    Future { version: i64 },
    Unknown { version: i64, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Column {
    name: String,
    data_type: String,
    not_null: bool,
    primary_key: i64,
}

pub(crate) fn detect_schema(conn: &Connection) -> Result<SchemaState> {
    let version = conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if version > SCHEMA_VERSION {
        return Ok(SchemaState::Future { version });
    }

    let tables = user_tables(conn)?;
    if version == 0 && tables.is_empty() {
        return Ok(SchemaState::Empty);
    }
    if version == 0 && is_legacy(conn, &tables)? {
        return Ok(SchemaState::Legacy);
    }
    if version != SCHEMA_VERSION {
        return Ok(SchemaState::Unknown {
            version,
            reason: format!("expected schema version {SCHEMA_VERSION}, found {version}"),
        });
    }

    match validate_current(conn, &tables).and_then(|()| current_root(conn)) {
        Ok(sessions_root) => Ok(SchemaState::Current { sessions_root }),
        Err(error) => Ok(SchemaState::Unknown {
            version,
            reason: error.to_string(),
        }),
    }
}

fn current_root(conn: &Connection) -> Result<PathBuf> {
    let count = conn.query_row("SELECT count(*) FROM index_metadata", [], |row| {
        row.get::<_, i64>(0)
    })?;
    if count != 1 {
        bail!("canonical index metadata must contain exactly one row");
    }
    let root = conn
        .query_row(
            "SELECT sessions_root FROM index_metadata WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .context("canonical index metadata is missing its singleton row")?;
    Ok(PathBuf::from(root))
}

fn user_tables(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    Ok(stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<Column>> {
    let mut stmt = conn.prepare(
        "SELECT name, type, \"notnull\", pk
         FROM pragma_table_info(?1)
         ORDER BY cid",
    )?;
    Ok(stmt
        .query_map(params![table], |row| {
            Ok(Column {
                name: row.get(0)?,
                data_type: row.get(1)?,
                not_null: row.get::<_, i64>(2)? != 0,
                primary_key: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn expected_columns(values: &[(&str, &str, bool, i64)]) -> Vec<Column> {
    values
        .iter()
        .map(|(name, data_type, not_null, primary_key)| Column {
            name: (*name).to_owned(),
            data_type: (*data_type).to_owned(),
            not_null: *not_null,
            primary_key: *primary_key,
        })
        .collect()
}

fn is_legacy(conn: &Connection, tables: &BTreeSet<String>) -> Result<bool> {
    let expected_tables = [
        BTreeSet::from(["sessions".to_owned()]),
        BTreeSet::from(["exec_events".to_owned(), "sessions".to_owned()]),
    ];
    if !expected_tables.contains(tables) {
        return Ok(false);
    }

    let session_names = columns(conn, "sessions")?
        .into_iter()
        .map(|column| column.name)
        .collect::<Vec<_>>();
    if session_names
        != [
            "path",
            "id",
            "timestamp",
            "cwd",
            "repository_url",
            "branch",
            "first_message",
        ]
    {
        return Ok(false);
    }

    if tables.contains("exec_events") {
        let exec_names = columns(conn, "exec_events")?
            .into_iter()
            .map(|column| column.name)
            .collect::<Vec<_>>();
        if exec_names
            != [
                "session_path",
                "session_id",
                "event_index",
                "call_id",
                "kind",
                "name",
                "command",
                "output",
            ]
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_current(conn: &Connection, tables: &BTreeSet<String>) -> Result<()> {
    let expected_tables = BTreeSet::from([
        "exec_events".to_owned(),
        "index_metadata".to_owned(),
        "sessions".to_owned(),
        "source_files".to_owned(),
    ]);
    if tables != &expected_tables {
        bail!("canonical table set does not match the required schema");
    }
    let unexpected_objects = conn.query_row(
        "SELECT count(*) FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%' AND type IN ('trigger', 'view')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if unexpected_objects != 0 {
        bail!("canonical schema contains an unexpected trigger or view");
    }

    let expected = BTreeMap::from([
        (
            "index_metadata",
            expected_columns(&[
                ("singleton", "INTEGER", false, 1),
                ("sessions_root", "TEXT", true, 0),
            ]),
        ),
        (
            "source_files",
            expected_columns(&[
                ("source_key", "INTEGER", false, 1),
                ("path", "TEXT", true, 0),
                ("file_size", "INTEGER", true, 0),
                ("modified_secs", "INTEGER", true, 0),
                ("modified_nanos", "INTEGER", true, 0),
                ("parse_status", "TEXT", true, 0),
            ]),
        ),
        (
            "sessions",
            expected_columns(&[
                ("path", "TEXT", true, 0),
                ("id", "TEXT", false, 0),
                ("timestamp", "TEXT", false, 0),
                ("cwd", "TEXT", false, 0),
                ("repository_url", "TEXT", false, 0),
                ("branch", "TEXT", false, 0),
                ("first_message", "TEXT", true, 0),
                ("session_key", "INTEGER", false, 1),
                ("source_key", "INTEGER", true, 0),
                ("is_subsession", "INTEGER", true, 0),
                ("has_nonempty_first_message", "INTEGER", true, 0),
            ]),
        ),
        (
            "exec_events",
            expected_columns(&[
                ("session_path", "TEXT", true, 0),
                ("session_id", "TEXT", false, 0),
                ("event_index", "INTEGER", true, 0),
                ("call_id", "TEXT", false, 0),
                ("kind", "TEXT", true, 0),
                ("name", "TEXT", false, 0),
                ("command", "TEXT", true, 0),
                ("output", "TEXT", true, 0),
                ("exec_key", "INTEGER", false, 1),
                ("session_key", "INTEGER", true, 0),
            ]),
        ),
    ]);

    for (table, expected_columns) in expected {
        if columns(conn, table)? != expected_columns {
            bail!("{table} columns do not match the canonical schema");
        }
        let strict = conn.query_row(
            "SELECT strict FROM pragma_table_list WHERE name = ?1",
            params![table],
            |row| row.get::<_, i64>(0),
        )?;
        if strict != 1 {
            bail!("{table} is not STRICT");
        }
    }

    require_unique(conn, "source_files", &["path"])?;
    require_unique(conn, "sessions", &["source_key"])?;
    require_unique(conn, "sessions", &["path"])?;
    require_unique(conn, "exec_events", &["session_key", "event_index"])?;
    require_foreign_key(conn, "sessions", "source_key", "source_files", "source_key")?;
    require_foreign_key(
        conn,
        "exec_events",
        "session_key",
        "sessions",
        "session_key",
    )?;
    Ok(())
}

fn require_unique(conn: &Connection, table: &str, expected: &[&str]) -> Result<()> {
    let mut indexes = conn.prepare(
        "SELECT name FROM pragma_index_list(?1)
         WHERE \"unique\" = 1",
    )?;
    let names = indexes
        .query_map(params![table], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for name in names {
        let mut columns = conn.prepare(
            "SELECT name FROM pragma_index_info(?1)
             ORDER BY seqno",
        )?;
        let actual = columns
            .query_map(params![name], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
        {
            return Ok(());
        }
    }
    bail!(
        "{table} is missing required UNIQUE({})",
        expected.join(", ")
    )
}

fn require_foreign_key(
    conn: &Connection,
    table: &str,
    from: &str,
    target_table: &str,
    target_column: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT \"table\", \"from\", \"to\", on_delete
         FROM pragma_foreign_key_list(?1)",
    )?;
    let rows = stmt
        .query_map(params![table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows
        .iter()
        .any(|(actual_table, actual_from, actual_to, on_delete)| {
            actual_table == target_table
                && actual_from == from
                && actual_to == target_column
                && on_delete == "CASCADE"
        })
    {
        return Ok(());
    }
    bail!("{table}.{from} is missing its required cascading foreign key")
}

pub(crate) fn create_schema(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(TABLE_DDL)?;
    ensure_indexes(tx)?;
    Ok(())
}

pub(crate) fn ensure_indexes(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(INDEX_DDL)?;
    Ok(())
}

pub(crate) fn drop_user_schema(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare(
        "SELECT type, name FROM sqlite_schema
         WHERE name NOT LIKE 'sqlite_%'
           AND type IN ('trigger', 'view', 'table')
         ORDER BY CASE type WHEN 'trigger' THEN 0 WHEN 'view' THEN 1 ELSE 2 END,
                  name",
    )?;
    let objects = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (kind, name) in objects {
        let kind = match kind.as_str() {
            "trigger" => "TRIGGER",
            "view" => "VIEW",
            "table" => "TABLE",
            _ => unreachable!(),
        };
        let escaped = name.replace('"', "\"\"");
        tx.execute_batch(&format!("DROP {kind} IF EXISTS \"{escaped}\";"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_database_is_detected() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(detect_schema(&conn).unwrap(), SchemaState::Empty);
    }

    #[test]
    fn legacy_sessions_only_database_is_detected() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                path TEXT, id TEXT, timestamp TEXT, cwd TEXT,
                repository_url TEXT, branch TEXT, first_message TEXT
            );",
        )
        .unwrap();
        assert_eq!(detect_schema(&conn).unwrap(), SchemaState::Legacy);
    }

    #[test]
    fn legacy_database_with_exec_is_detected() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                path TEXT, id TEXT, timestamp TEXT, cwd TEXT,
                repository_url TEXT, branch TEXT, first_message TEXT
            );
            CREATE TABLE exec_events (
                session_path TEXT NOT NULL, session_id TEXT,
                event_index INTEGER NOT NULL, call_id TEXT,
                kind TEXT NOT NULL, name TEXT, command TEXT, output TEXT
            );",
        )
        .unwrap();
        assert_eq!(detect_schema(&conn).unwrap(), SchemaState::Legacy);
    }

    #[test]
    fn canonical_schema_uses_user_version_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        create_schema(&tx).unwrap();
        tx.execute(
            "INSERT INTO index_metadata(singleton, sessions_root) VALUES (1, '/tmp/sessions')",
            [],
        )
        .unwrap();
        tx.execute_batch("PRAGMA user_version = 1;").unwrap();
        tx.commit().unwrap();

        assert_eq!(
            detect_schema(&conn).unwrap(),
            SchemaState::Current {
                sessions_root: PathBuf::from("/tmp/sessions")
            }
        );
    }

    #[test]
    fn future_schema_is_classified_without_inspection() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        assert_eq!(
            detect_schema(&conn).unwrap(),
            SchemaState::Future { version: 2 }
        );
    }

    #[test]
    fn malformed_version_one_schema_is_unknown() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions(path TEXT);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        assert!(matches!(
            detect_schema(&conn).unwrap(),
            SchemaState::Unknown { version: 1, .. }
        ));
    }

    #[test]
    fn canonical_schema_rejects_unexpected_trigger() {
        let mut conn = Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        create_schema(&tx).unwrap();
        tx.execute(
            "INSERT INTO index_metadata(singleton, sessions_root) VALUES (1, '/tmp/sessions')",
            [],
        )
        .unwrap();
        tx.execute_batch(
            "CREATE TRIGGER unexpected AFTER INSERT ON sessions BEGIN SELECT 1; END;
             PRAGMA user_version = 1;",
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(matches!(
            detect_schema(&conn).unwrap(),
            SchemaState::Unknown { version: 1, .. }
        ));
    }
}
