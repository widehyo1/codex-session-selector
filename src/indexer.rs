use std::path::PathBuf;

use anyhow::Result;

use crate::{CollectOptions, cli::IndexOptions, collect_session_data, recreate_database_with_exec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexSummary {
    pub session_rows: usize,
    pub exec_rows: usize,
    pub total_files: usize,
    pub skipped: usize,
    pub output: PathBuf,
    pub exec_enabled: bool,
}

pub(crate) fn build_index(options: &IndexOptions) -> Result<IndexSummary> {
    let collect_options = CollectOptions {
        include_subsessions: options.include_subsessions,
        include_empty_messages: options.include_empty_messages,
        include_exec: options.include_exec,
    };
    let data = collect_session_data(&options.sessions_root, collect_options)?;

    recreate_database_with_exec(
        &options.output,
        &data.rows,
        options.include_exec.then_some(data.exec_events.as_slice()),
    )?;

    Ok(IndexSummary {
        session_rows: data.rows.len(),
        exec_rows: data.exec_events.len(),
        total_files: data.total_files,
        skipped: data.skipped,
        output: options.output.clone(),
        exec_enabled: options.include_exec,
    })
}

pub(crate) fn format_summary(summary: &IndexSummary) -> String {
    if summary.exec_enabled {
        format!(
            "wrote {} session rows and {} exec rows to {} from {} jsonl files; skipped {} filtered or invalid sessions",
            summary.session_rows,
            summary.exec_rows,
            summary.output.display(),
            summary.total_files,
            summary.skipped,
        )
    } else {
        format!(
            "wrote {} session rows to {} from {} jsonl files; skipped {} filtered or invalid sessions; exec indexing disabled",
            summary.session_rows,
            summary.output.display(),
            summary.total_files,
            summary.skipped,
        )
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::test_support::{SessionFixture, table_count, table_exists};

    #[test]
    fn build_index_without_exec_preserves_default_schema() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");

        let summary = build_index(&IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        })
        .unwrap();

        assert_eq!(summary.session_rows, 1);
        assert_eq!(summary.exec_rows, 0);
        assert!(!summary.exec_enabled);

        let connection = Connection::open(db).unwrap();
        assert_eq!(table_count(&connection, "sessions"), 1);
        assert!(!table_exists(&connection, "exec_events"));
        assert!(!table_exists(&connection, "sessions_fts"));
    }

    #[test]
    fn build_index_with_exec_preserves_current_exec_schema() {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        let db = fixture.path("index.sqlite3");

        let summary = build_index(&IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: true,
        })
        .unwrap();

        assert_eq!(summary.session_rows, 1);
        assert_eq!(summary.exec_rows, 3);
        assert!(summary.exec_enabled);

        let connection = Connection::open(db).unwrap();
        assert_eq!(table_count(&connection, "sessions"), 1);
        assert_eq!(table_count(&connection, "exec_events"), 3);
        assert!(!table_exists(&connection, "sessions_fts"));
    }

    #[test]
    fn format_summary_matches_legacy_output() {
        let summary = IndexSummary {
            session_rows: 2,
            exec_rows: 5,
            total_files: 3,
            skipped: 1,
            output: PathBuf::from("/tmp/index.sqlite3"),
            exec_enabled: true,
        };

        assert_eq!(
            format_summary(&summary),
            "wrote 2 session rows and 5 exec rows to /tmp/index.sqlite3 from 3 jsonl files; skipped 1 filtered or invalid sessions"
        );
    }

    #[test]
    fn disabled_exec_summary_matches_legacy_output() {
        let summary = IndexSummary {
            session_rows: 2,
            exec_rows: 0,
            total_files: 3,
            skipped: 1,
            output: PathBuf::from("/tmp/index.sqlite3"),
            exec_enabled: false,
        };

        assert_eq!(
            format_summary(&summary),
            "wrote 2 session rows to /tmp/index.sqlite3 from 3 jsonl files; skipped 1 filtered or invalid sessions; exec indexing disabled"
        );
    }
}
