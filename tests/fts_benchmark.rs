use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use rusqlite::{Connection, params};
use serde_json::json;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_select-codex-session")
}

struct BenchRoot {
    path: PathBuf,
}

impl BenchRoot {
    fn new(size: usize) -> Self {
        let path = std::env::temp_dir().join(format!(
            "codex-session-selector-fts-benchmark-{}-{size}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for BenchRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
struct Measurements {
    size: usize,
    canonical_time: Duration,
    canonical_bytes: u64,
    fts_time: Duration,
    fts_bytes: u64,
    incremental_time: Duration,
    median: Duration,
    p95: Duration,
}

#[test]
#[ignore = "release benchmark"]
fn fts_release_benchmark() {
    let sqlite = Connection::open_in_memory()
        .unwrap()
        .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))
        .unwrap();
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("model name\t: ").map(str::to_owned))
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    println!(
        "environment: commit={sha} rustc={} sqlite={sqlite} cpu={cpu} seed=0x5eed profile=release",
        rustc_version()
    );

    let mut measurements = Vec::new();
    for size in [1_000, 10_000, 50_000] {
        measurements.push(run_corpus(size));
    }

    println!("corpus | canonical time/size | FTS time/size | incremental | median | p95");
    for value in &measurements {
        println!(
            "{} | {:?}/{} | {:?}/{} | {:?} | {:?} | {:?}",
            value.size,
            value.canonical_time,
            value.canonical_bytes,
            value.fts_time,
            value.fts_bytes,
            value.incremental_time,
            value.median,
            value.p95
        );
        assert!(
            value.fts_bytes <= value.canonical_bytes.saturating_mul(3),
            "{} session FTS database exceeded the 3.0x size gate",
            value.size
        );
        assert!(
            value.incremental_time <= value.fts_time / 5,
            "{} session incremental refresh exceeded 20% of full build",
            value.size
        );
    }
    assert!(
        measurements[1].p95 <= Duration::from_millis(100),
        "10k warm query p95 exceeded 100 ms"
    );
    assert!(
        measurements[2].p95 <= Duration::from_millis(250),
        "50k warm query p95 exceeded 250 ms"
    );
}

fn run_corpus(size: usize) -> Measurements {
    let root = BenchRoot::new(size);
    let sessions_root = root.path.join("sessions");
    let day = sessions_root.join("2026/07/27");
    fs::create_dir_all(&day).unwrap();
    for index in 0..size {
        write_session(&day.join(format!("session-{index:05}.jsonl")), index, false);
    }

    let canonical_db = root.path.join("canonical.sqlite3");
    let started = Instant::now();
    build_canonical_only(&canonical_db, size);
    let canonical_time = started.elapsed();
    let canonical_bytes = fs::metadata(&canonical_db).unwrap().len();

    let fts_db = root.path.join("fts.sqlite3");
    let started = Instant::now();
    run_index(&sessions_root, &fts_db, true);
    let fts_time = started.elapsed();
    let fts_bytes = fs::metadata(&fts_db).unwrap().len();

    let mut timings = Vec::new();
    let queries = benchmark_queries();
    let conn = Connection::open(&fts_db).unwrap();
    let mut expected = Vec::new();
    for (query_index, query) in queries.iter().enumerate() {
        for iteration in 0..31 {
            let started = Instant::now();
            let result = run_query(&conn, query);
            let elapsed = started.elapsed();
            if iteration == 0 {
                expected.push(result);
            } else {
                assert_eq!(
                    result, expected[query_index],
                    "query result path/order changed between repetitions"
                );
                timings.push(elapsed);
            }
        }
        println!(
            "corpus={size} query={} results={}",
            query_index + 1,
            expected[query_index].len()
        );
    }
    timings.sort_unstable();
    let median = timings[timings.len() / 2];
    let p95 = timings[(timings.len() * 95).div_ceil(100) - 1];
    drop(conn);

    let changed = size.min(4_242).saturating_sub(1);
    write_session(
        &day.join(format!("session-{changed:05}.jsonl")),
        changed,
        true,
    );
    let started = Instant::now();
    run_index(&sessions_root, &fts_db, false);
    let incremental_time = started.elapsed();

    Measurements {
        size,
        canonical_time,
        canonical_bytes,
        fts_time,
        fts_bytes,
        incremental_time,
        median,
        p95,
    }
}

fn write_session(path: &Path, index: usize, changed: bool) {
    let marker = if changed { " changed" } else { "" };
    let mut records = vec![
        json!({
            "type": "session_meta",
            "payload": {
                "id": format!("benchmark-{index}"),
                "timestamp": format!("2026-07-{:02}T01:02:03Z", index % 28 + 1),
                "cwd": format!("/workspace/team-{}/project-{index}", index % 100),
                "source": "cli",
                "thread_source": "user",
                "git": {
                    "repository_url": format!("https://github.com/bench/repo-{}.git", index % 1000),
                    "branch": format!("feature/batch-{}", index % 10)
                }
            }
        }),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": format!("session benchmark item{index} 검색기능{marker}")
            }
        }),
    ];
    for event in 0..5 {
        records.push(json!({
            "type": "event_msg",
            "payload": {
                "type": "exec_command_end",
                "parsed_cmd": [{
                    "type": "unknown",
                    "cmd": format!("cargo test target{index} event{event} {}", "c".repeat(80))
                }],
                "aggregated_output": format!(
                    "needle{index} result{event}{marker} {}",
                    "x".repeat(1024)
                )
            }
        }));
    }
    let contents = records
        .into_iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, format!("{contents}\n")).unwrap();
}

fn run_index(sessions_root: &Path, db: &Path, rebuild: bool) {
    let mut command = Command::new(binary());
    command.args([
        "index",
        "--sessions-root",
        sessions_root.to_str().unwrap(),
        "--output",
        db.to_str().unwrap(),
    ]);
    if rebuild {
        command.arg("--rebuild");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn build_canonical_only(path: &Path, size: usize) {
    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions(
             session_key INTEGER PRIMARY KEY,
             first_message TEXT, cwd TEXT, repository_url TEXT,
             branch TEXT, timestamp TEXT
         );
         CREATE TABLE exec_events(
             exec_key INTEGER PRIMARY KEY,
             session_key INTEGER, event_index INTEGER,
             command TEXT, output TEXT
         );",
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    for index in 0..size {
        tx.execute(
            "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                index as i64 + 1,
                format!("session benchmark item{index} 검색기능"),
                format!("/workspace/team-{}/project-{index}", index % 100),
                format!("https://github.com/bench/repo-{}.git", index % 1000),
                format!("feature/batch-{}", index % 10),
                format!("2026-07-{:02}T01:02:03Z", index % 28 + 1),
            ],
        )
        .unwrap();
        for event in 0..5 {
            tx.execute(
                "INSERT INTO exec_events(session_key, event_index, command, output)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    index as i64 + 1,
                    event,
                    format!("cargo test target{index} event{event} {}", "c".repeat(80)),
                    format!("needle{index} result{event} {}", "x".repeat(1024)),
                ],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
}

fn benchmark_queries() -> [&'static str; 8] {
    [
        "(\"item424\"*)",
        "{first_message} : ((\"검색\"* AND \"item4242\"*))",
        "{cwd} : ((\"project-4242\"*))",
        "{repository_url} : ((\"repo-42\"*))",
        "{branch} : ((\"batch-7\"*))",
        "{date} : ((\"2026\"* AND \"07\"* AND \"27\"*))",
        "{exec_command exec_output} : ((\"target4242\"*))",
        "(\"needle4242\"*)",
    ]
}

fn run_query(conn: &Connection, query: &str) -> Vec<i64> {
    let mut stmt = conn
        .prepare(
            "SELECT s.session_key
             FROM sessions_fts
             JOIN sessions AS s ON s.session_key = sessions_fts.rowid
             WHERE sessions_fts MATCH ?1
               AND sessions_fts.rank MATCH
                   'bm25(10.0, 4.0, 4.0, 5.0, 2.0, 2.0, 1.5, 0.25)'
             ORDER BY sessions_fts.rank, s.timestamp DESC, s.session_key DESC",
        )
        .unwrap();
    stmt.query_map(params![query], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}
