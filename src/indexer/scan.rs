use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, bail};

use crate::{ParsedSessionFile, parse_session_file_data, session_jsonl_paths};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    pub size: u64,
    pub modified_secs: i64,
    pub modified_nanos: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseStatus {
    Indexed,
    Skipped,
}

impl ParseStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Indexed => "indexed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedSource {
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    pub session: Option<ParsedSessionFile>,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredSource {
    pub fingerprint: FileFingerprint,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanPlan {
    pub scanned_files: usize,
    pub new_sources: Vec<ParsedSource>,
    pub changed_sources: Vec<ParsedSource>,
    pub unchanged_paths: Vec<PathBuf>,
    pub deleted_paths: Vec<PathBuf>,
    pub unstable_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum StableParse {
    Stable(ParsedSource),
    Unstable(PathBuf),
}

pub(crate) fn fingerprint(path: &Path) -> Result<FileFingerprint> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read modified time for {}", path.display()))?
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("modified time predates UNIX epoch for {}", path.display()))?;
    Ok(FileFingerprint {
        size: metadata.len(),
        modified_secs: i64::try_from(modified.as_secs())
            .with_context(|| format!("modified time is too large for {}", path.display()))?,
        modified_nanos: modified.subsec_nanos(),
    })
}

pub(crate) fn stable_parse(path: &Path) -> Result<StableParse> {
    stable_parse_with(path, |path| parse_session_file_data(path, true))
}

fn stable_parse_with(
    path: &Path,
    mut parse: impl FnMut(&Path) -> Result<Option<ParsedSessionFile>>,
) -> Result<StableParse> {
    for _ in 0..2 {
        let before = fingerprint(path)?;
        let session = parse(path)?;
        let after = fingerprint(path)?;
        if before == after {
            return Ok(StableParse::Stable(ParsedSource {
                path: path.to_path_buf(),
                fingerprint: after,
                session,
            }));
        }
    }
    Ok(StableParse::Unstable(path.to_path_buf()))
}

pub(crate) fn scan_sources(
    sessions_root: &Path,
    stored: &BTreeMap<PathBuf, StoredSource>,
    force_all: bool,
) -> Result<ScanPlan> {
    let mut paths = session_jsonl_paths(sessions_root)?;
    paths.sort();
    let mut current = BTreeSet::new();
    for path in &paths {
        let stored_path = PathBuf::from(path.to_string_lossy().into_owned());
        if !current.insert(stored_path) {
            bail!(
                "multiple source paths have the same lossy UTF-8 representation: {}",
                path.display()
            );
        }
    }
    let mut plan = ScanPlan {
        scanned_files: paths.len(),
        new_sources: Vec::new(),
        changed_sources: Vec::new(),
        unchanged_paths: Vec::new(),
        deleted_paths: stored
            .keys()
            .filter(|path| !current.contains(*path))
            .cloned()
            .collect(),
        unstable_paths: Vec::new(),
    };

    for path in paths {
        let stored_path = PathBuf::from(path.to_string_lossy().into_owned());
        let current_fingerprint = fingerprint(&path)?;
        if !force_all
            && stored
                .get(&stored_path)
                .is_some_and(|old| old.fingerprint == current_fingerprint)
        {
            plan.unchanged_paths.push(path);
            continue;
        }

        match stable_parse(&path)? {
            StableParse::Stable(parsed) if stored.contains_key(&stored_path) => {
                plan.changed_sources.push(parsed);
            }
            StableParse::Stable(parsed) => plan.new_sources.push(parsed),
            StableParse::Unstable(path) => plan.unstable_paths.push(path),
        }
    }

    plan.new_sources.sort_by(|a, b| a.path.cmp(&b.path));
    plan.changed_sources.sort_by(|a, b| a.path.cmp(&b.path));
    plan.unchanged_paths.sort();
    plan.deleted_paths.sort();
    plan.unstable_paths.sort();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use std::{io::Write, os::unix::fs::symlink};

    use super::*;
    use crate::test_support::SessionFixture;

    #[test]
    fn fingerprint_uses_size_seconds_and_nanoseconds() {
        let fixture = SessionFixture::new();
        let path = fixture.write_session_with_exec();
        let value = fingerprint(&path).unwrap();
        let metadata = fs::metadata(path).unwrap();
        let modified = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        assert_eq!(value.size, metadata.len());
        assert_eq!(value.modified_secs, modified.as_secs() as i64);
        assert_eq!(value.modified_nanos, modified.subsec_nanos());
    }

    #[test]
    fn new_changed_deleted_and_unchanged_are_classified() {
        let fixture = SessionFixture::new();
        let unchanged = fixture.write_session_with_exec();
        let changed = fixture.write_named_session("changed.jsonl", "before", false);
        let new = fixture.write_named_session("new.jsonl", "new", false);
        let deleted = fixture.sessions_root().join("2026/07/26/deleted.jsonl");
        let stored = BTreeMap::from([
            (
                unchanged.clone(),
                StoredSource {
                    fingerprint: fingerprint(&unchanged).unwrap(),
                },
            ),
            (
                changed.clone(),
                StoredSource {
                    fingerprint: fingerprint(&changed).unwrap(),
                },
            ),
            (
                deleted.clone(),
                StoredSource {
                    fingerprint: FileFingerprint {
                        size: 1,
                        modified_secs: 1,
                        modified_nanos: 0,
                    },
                },
            ),
        ]);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut file = fs::OpenOptions::new().append(true).open(&changed).unwrap();
        writeln!(file).unwrap();

        let plan = scan_sources(&fixture.sessions_root(), &stored, false).unwrap();
        assert_eq!(plan.new_sources[0].path, new);
        assert_eq!(plan.changed_sources[0].path, changed);
        assert_eq!(plan.unchanged_paths, vec![unchanged]);
        assert_eq!(plan.deleted_paths, vec![deleted]);
    }

    #[test]
    fn file_changed_during_both_attempts_is_deferred() {
        let fixture = SessionFixture::new();
        let path = fixture.write_session_with_exec();
        let result = stable_parse_with(&path, |path| {
            let parsed = parse_session_file_data(path, true)?;
            let mut file = fs::OpenOptions::new().append(true).open(path)?;
            writeln!(file)?;
            Ok(parsed)
        })
        .unwrap();
        assert!(matches!(result, StableParse::Unstable(value) if value == path));
    }

    #[test]
    fn hard_file_error_aborts_scan() {
        let fixture = SessionFixture::new();
        let day = fixture.sessions_root().join("2026/07/26");
        fs::create_dir_all(&day).unwrap();
        symlink(day.join("missing-target"), day.join("broken.jsonl")).unwrap();
        let error = scan_sources(&fixture.sessions_root(), &BTreeMap::new(), false).unwrap_err();
        assert!(error.to_string().contains("metadata"));
    }
}
