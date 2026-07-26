use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};

use crate::SessionRow;

use super::{
    fts,
    schema::{self, SchemaState},
    store::SessionView,
};

#[cfg(test)]
const BM25_WEIGHTS: &str = "bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchScope {
    All,
    FirstMessage,
    Cwd,
    Branch,
    Repository,
    Date,
    Exec,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchHit {
    pub row: SessionRow,
    pub rank: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueryAtom {
    Prefix(String),
    Phrase(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueryGroup {
    pub atoms: Vec<QueryAtom>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchQuery {
    pub groups: Vec<QueryGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryError {
    NoSearchableToken,
    InvalidOr,
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSearchableToken => formatter.write_str("query contains no searchable token"),
            Self::InvalidOr => formatter.write_str("OR requires an expression on both sides"),
        }
    }
}

impl Error for QueryError {}

pub(crate) struct SearchIndex {
    conn: Connection,
    view: SessionView,
}

pub(crate) fn parse_query(input: &str) -> std::result::Result<SearchQuery, QueryError> {
    enum Token {
        Atom(QueryAtom),
        Or,
    }

    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    while chars.peek().is_some() {
        while chars
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            chars.next();
        }
        let Some(character) = chars.peek().copied() else {
            break;
        };
        if character == '"' {
            chars.next();
            let mut text = String::new();
            while let Some(character) = chars.next() {
                match character {
                    '"' => break,
                    '\\' => match chars.peek().copied() {
                        Some('"' | '\\') => text.push(chars.next().unwrap()),
                        _ => text.push('\\'),
                    },
                    _ => text.push(character),
                }
            }
            ensure_searchable(&text)?;
            tokens.push(Token::Atom(QueryAtom::Phrase(text)));
        } else {
            let mut text = String::new();
            while chars
                .peek()
                .is_some_and(|character| !character.is_whitespace())
            {
                text.push(chars.next().unwrap());
            }
            if text == "|" {
                tokens.push(Token::Or);
            } else {
                ensure_searchable(&text)?;
                tokens.push(Token::Atom(QueryAtom::Prefix(text)));
            }
        }
    }

    if tokens.is_empty() {
        return Ok(SearchQuery { groups: Vec::new() });
    }
    let mut groups = vec![QueryGroup { atoms: Vec::new() }];
    for token in tokens {
        match token {
            Token::Atom(atom) => groups.last_mut().unwrap().atoms.push(atom),
            Token::Or => {
                if groups.last().unwrap().atoms.is_empty() {
                    return Err(QueryError::InvalidOr);
                }
                groups.push(QueryGroup { atoms: Vec::new() });
            }
        }
    }
    if groups.last().unwrap().atoms.is_empty() {
        return Err(QueryError::InvalidOr);
    }
    Ok(SearchQuery { groups })
}

fn ensure_searchable(text: &str) -> std::result::Result<(), QueryError> {
    if text.chars().any(char::is_alphanumeric) {
        Ok(())
    } else {
        Err(QueryError::NoSearchableToken)
    }
}

pub(crate) fn compile_match(
    query: &SearchQuery,
    scope: SearchScope,
) -> std::result::Result<String, QueryError> {
    if query.groups.is_empty() || query.groups.iter().any(|group| group.atoms.is_empty()) {
        return Err(QueryError::NoSearchableToken);
    }
    let compiled = query
        .groups
        .iter()
        .map(|group| {
            let atoms = group
                .atoms
                .iter()
                .map(|atom| match atom {
                    QueryAtom::Prefix(text) => format!("\"{}\"*", escape_fts(text)),
                    QueryAtom::Phrase(text) => format!("\"{}\"", escape_fts(text)),
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            format!("({atoms})")
        })
        .collect::<Vec<_>>()
        .join(" OR ");

    let columns = match scope {
        SearchScope::All => return Ok(compiled),
        SearchScope::FirstMessage => "first_message",
        SearchScope::Cwd => "cwd",
        SearchScope::Branch => "branch",
        SearchScope::Repository => "repository_url",
        SearchScope::Date => "date",
        SearchScope::Exec => "exec_command exec_output",
    };
    Ok(format!("{{{columns}}} : ({compiled})"))
}

fn escape_fts(value: &str) -> String {
    value.replace('"', "\"\"")
}

impl SearchIndex {
    pub(crate) fn open(path: &Path, view: SessionView) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("failed to open {}", path.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        fts::verify_runtime(&conn)?;
        match schema::detect_schema(&conn)? {
            SchemaState::Current { .. } => {}
            SchemaState::Empty
            | SchemaState::Legacy
            | SchemaState::CanonicalV1 { .. }
            | SchemaState::Unknown { version: 0 | 1, .. } => bail!(
                "search index schema 2 is required; refresh the index or run `select-codex-session index`"
            ),
            SchemaState::Future { version } => bail!(
                "index schema version {version} is newer than supported version {}; refusing to open it",
                schema::SCHEMA_VERSION
            ),
            SchemaState::Unknown { version, reason } => {
                bail!("unrecognized search index schema version {version}: {reason}")
            }
        }
        ensure_clean_state(&conn)?;
        ensure_row_identity(&conn)?;
        Ok(Self { conn, view })
    }

    pub(crate) fn search(&self, input: &str, scope: SearchScope) -> Result<Vec<SearchHit>> {
        if input.trim().is_empty() {
            return self.query_unranked();
        }
        ensure_clean_state(&self.conn)?;
        let query = parse_query(input)?;
        let match_query = compile_match(&query, scope)?;
        let mut stmt = self.conn.prepare(
            "SELECT
                 s.path, s.id, s.timestamp, s.cwd, s.repository_url, s.branch,
                 s.first_message, s.is_subsession, sessions_fts.rank
             FROM sessions_fts
             JOIN sessions AS s ON s.session_key = sessions_fts.rowid
             WHERE sessions_fts MATCH ?1
               AND sessions_fts.rank MATCH
                   'bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)'
               AND (?2 = 1 OR s.is_subsession = 0)
               AND (?3 = 1 OR s.has_nonempty_first_message = 1)
             ORDER BY sessions_fts.rank ASC, s.timestamp DESC, s.session_key DESC",
        )?;
        let rows = stmt.query_map(
            params![
                match_query,
                i64::from(self.view.include_subsessions),
                i64::from(self.view.include_empty_messages),
            ],
            map_ranked_hit,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn query_unranked(&self) -> Result<Vec<SearchHit>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, id, timestamp, cwd, repository_url, branch,
                    first_message, is_subsession
             FROM sessions
             WHERE (?1 = 1 OR is_subsession = 0)
               AND (?2 = 1 OR has_nonempty_first_message = 1)
             ORDER BY timestamp DESC, session_key DESC",
        )?;
        let rows = stmt.query_map(
            params![
                i64::from(self.view.include_subsessions),
                i64::from(self.view.include_empty_messages),
            ],
            |row| {
                Ok(SearchHit {
                    row: map_session_row(row)?,
                    rank: 0.0,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn map_ranked_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        row: map_session_row(row)?,
        rank: row.get(8)?,
    })
}

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
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
}

fn ensure_clean_state(conn: &Connection) -> Result<()> {
    let (count, dirty) = conn.query_row(
        "SELECT count(*), coalesce(max(dirty), -1) FROM fts_sync_state",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if count != 1 || dirty != 0 {
        bail!("FTS index is dirty; refresh or rebuild the index");
    }
    Ok(())
}

fn ensure_row_identity(conn: &Connection) -> Result<()> {
    let mismatch = conn.query_row(
        "SELECT
             EXISTS(
                 SELECT 1 FROM sessions AS s
                 LEFT JOIN sessions_fts AS f ON f.rowid = s.session_key
                 WHERE f.rowid IS NULL
             )
             OR EXISTS(
                 SELECT 1 FROM sessions_fts AS f
                 LEFT JOIN sessions AS s ON s.session_key = f.rowid
                 WHERE s.session_key IS NULL
             )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if mismatch {
        bail!("FTS row identity mismatch; refresh or rebuild the index");
    }
    Ok(())
}

pub(crate) fn is_corruption(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<rusqlite::Error>()
            .is_some_and(|error| {
                matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseCorrupt)
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::IndexOptions, indexer::build_index, test_support::SessionFixture};

    fn fixture_index() -> (SessionFixture, SearchIndex) {
        let fixture = SessionFixture::new();
        fixture.write_session_with_exec();
        fixture.write_named_session("korean.jsonl", "검색기능 벤치마크 작성", false);
        let db = fixture.path("index.sqlite3");
        build_index(&IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        })
        .unwrap();
        let index = SearchIndex::open(&db, SessionView::default()).unwrap();
        (fixture, index)
    }

    #[test]
    fn bare_terms_compile_to_and_prefix_phrases() {
        let query = parse_query("fix read").unwrap();
        assert_eq!(
            compile_match(&query, SearchScope::All).unwrap(),
            "(\"fix\"* AND \"read\"*)"
        );
    }

    #[test]
    fn quoted_text_and_spaced_pipe_compile() {
        let query = parse_query("\"readme parser\" | cargo test").unwrap();
        assert_eq!(
            compile_match(&query, SearchScope::All).unwrap(),
            "(\"readme parser\") OR (\"cargo\"* AND \"test\"*)"
        );
    }

    #[test]
    fn unclosed_quote_is_valid_interactive_phrase() {
        assert_eq!(
            parse_query("\"readme parser").unwrap(),
            SearchQuery {
                groups: vec![QueryGroup {
                    atoms: vec![QueryAtom::Phrase("readme parser".to_owned())]
                }]
            }
        );
    }

    #[test]
    fn invalid_or_and_punctuation_only_queries_are_rejected() {
        for input in ["| foo", "foo |", "foo | | bar"] {
            assert_eq!(parse_query(input), Err(QueryError::InvalidOr));
        }
        for input in ["---", "\"***\""] {
            assert_eq!(parse_query(input), Err(QueryError::NoSearchableToken));
        }
    }

    #[test]
    fn scopes_wrap_only_the_expected_columns() {
        let query = parse_query("read").unwrap();
        let cases = [
            (SearchScope::All, "(\"read\"*)"),
            (SearchScope::FirstMessage, "{first_message} : ((\"read\"*))"),
            (SearchScope::Cwd, "{cwd} : ((\"read\"*))"),
            (SearchScope::Branch, "{branch} : ((\"read\"*))"),
            (SearchScope::Repository, "{repository_url} : ((\"read\"*))"),
            (SearchScope::Date, "{date} : ((\"read\"*))"),
            (
                SearchScope::Exec,
                "{exec_command exec_output} : ((\"read\"*))",
            ),
        ];
        for (scope, expected) in cases {
            assert_eq!(compile_match(&query, scope).unwrap(), expected);
        }
    }

    #[test]
    fn raw_fts_operators_are_quoted_as_text() {
        let query = parse_query("NOT repo:main {cwd} a-b foo*").unwrap();
        assert_eq!(
            compile_match(&query, SearchScope::All).unwrap(),
            "(\"NOT\"* AND \"repo:main\"* AND \"{cwd}\"* AND \"a-b\"* AND \"foo*\"*)"
        );
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        let query = parse_query("\"a\\\"b\\\\c\"").unwrap();
        assert_eq!(
            compile_match(&query, SearchScope::All).unwrap(),
            "(\"a\"\"b\\c\")"
        );
    }

    #[test]
    fn bm25_weight_mapping_stays_fixed() {
        assert_eq!(
            BM25_WEIGHTS,
            "bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)"
        );
    }

    #[test]
    fn search_matches_token_prefixes_scopes_and_exec_content() {
        let (_fixture, index) = fixture_index();

        let message = index.search("real req", SearchScope::FirstMessage).unwrap();
        assert_eq!(message.len(), 1);
        assert!(message[0].row.path.ends_with("session.jsonl"));
        assert!(
            index
                .search("eal", SearchScope::FirstMessage)
                .unwrap()
                .is_empty()
        );
        assert_eq!(index.search("repo dem", SearchScope::Cwd).unwrap().len(), 2);
        assert_eq!(
            index
                .search("git exam", SearchScope::Repository)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            index.search("sed read", SearchScope::Exec).unwrap().len(),
            1
        );
        assert_eq!(
            index.search("real read", SearchScope::All).unwrap().len(),
            1
        );
        assert_eq!(
            index.search("2026 05 28", SearchScope::Date).unwrap().len(),
            1
        );
    }

    #[test]
    fn korean_prefix_matches_but_middle_of_token_does_not() {
        let (_fixture, index) = fixture_index();

        let prefix = index.search("검색", SearchScope::FirstMessage).unwrap();
        assert_eq!(prefix.len(), 1);
        assert!(prefix[0].row.path.ends_with("korean.jsonl"));
        assert!(
            index
                .search("기능", SearchScope::FirstMessage)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn exact_phrase_and_or_groups_execute_as_compiled() {
        let (_fixture, index) = fixture_index();
        assert_eq!(
            index
                .search("\"real user\"", SearchScope::FirstMessage)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            index
                .search("\"real user\" | 검색", SearchScope::FirstMessage)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn bm25_prefers_message_over_exec_output() {
        let fixture = SessionFixture::new();
        let first = fixture.write_named_session("a.jsonl", "placeholder", false);
        let second = fixture.write_named_session("b.jsonl", "placeholder", false);
        let db = fixture.path("index.sqlite3");
        let options = IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        };
        build_index(&options).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE sessions SET first_message = 'weightedterm'
             WHERE path = ?1",
            params![first.to_string_lossy().as_ref()],
        )
        .unwrap();
        let second_key = conn
            .query_row(
                "SELECT session_key FROM sessions WHERE path = ?1",
                params![second.to_string_lossy().as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO exec_events(
                 session_path, event_index, kind, command, output, session_key
             ) VALUES (?1, 0, 'test', '', 'weightedterm', ?2)",
            params![second.to_string_lossy().as_ref(), second_key],
        )
        .unwrap();
        drop(conn);
        build_index(&options).unwrap();

        let index = SearchIndex::open(&db, SessionView::default()).unwrap();
        let hits = index.search("weighted", SearchScope::All).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].row.path, first);
        assert!(hits[0].rank < hits[1].rank);
    }

    #[test]
    fn equal_rank_uses_timestamp_then_session_key() {
        let fixture = SessionFixture::new();
        let first = fixture.write_named_session("a.jsonl", "same ranking term", false);
        let second = fixture.write_named_session("b.jsonl", "same ranking term", false);
        let db = fixture.path("index.sqlite3");
        build_index(&IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        })
        .unwrap();

        let index = SearchIndex::open(&db, SessionView::default()).unwrap();
        let hits = index.search("same", SearchScope::FirstMessage).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].row.path, second);
        assert_eq!(hits[1].row.path, first);
        assert_eq!(hits[0].rank, hits[1].rank);
    }

    #[test]
    fn view_filters_apply_before_results_are_returned() {
        let fixture = SessionFixture::new();
        fixture.write_named_session("normal.jsonl", "visible", false);
        fixture.write_named_session("sub.jsonl", "visible", true);
        fixture.write_named_session("empty.jsonl", "   ", false);
        let db = fixture.path("index.sqlite3");
        build_index(&IndexOptions {
            output: db.clone(),
            sessions_root: fixture.sessions_root(),
            rebuild: false,
            include_subsessions: false,
            include_empty_messages: false,
            include_exec: false,
        })
        .unwrap();

        let counts = [
            (SessionView::default(), 1),
            (
                SessionView {
                    include_subsessions: true,
                    include_empty_messages: false,
                },
                2,
            ),
            (
                SessionView {
                    include_subsessions: false,
                    include_empty_messages: true,
                },
                2,
            ),
            (
                SessionView {
                    include_subsessions: true,
                    include_empty_messages: true,
                },
                3,
            ),
        ];
        for (view, expected) in counts {
            let index = SearchIndex::open(&db, view).unwrap();
            assert_eq!(index.search("", SearchScope::All).unwrap().len(), expected);
        }
    }
}
