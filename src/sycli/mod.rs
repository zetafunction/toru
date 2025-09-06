use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Deserialize)]
struct RawTorrent {
    id: String,
    name: String,
    path: PathBuf,
    progress: f64,
    tracker_urls: Vec<String>,
    size: usize,
    files: usize,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    id: String,
    torrent_id: String,
    path: PathBuf,
    size: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Torrent {
    // TODO: Consider representing this as the bytes of the infohash instead.
    pub id: String,
    pub name: String,
    pub base_path: PathBuf,
    pub progress: f64,
    pub tracker_urls: Vec<String>,
    pub size: usize,
    pub files: HashMap<PathBuf, usize>,
}

fn get_raw_torrents() -> Result<Vec<RawTorrent>> {
    let output = Command::new("sycli")
        .args(["list", "-k", "torrent", "-o", "json"])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(serde_json::from_str(&String::from_utf8(output.stdout)?)?)
}

fn get_raw_files() -> Result<Vec<RawFile>> {
    let output = Command::new("sycli")
        .args(["list", "-k", "file", "-o", "json"])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(serde_json::from_str(&String::from_utf8(output.stdout)?)?)
}

pub fn get_torrents() -> Result<Vec<Torrent>> {
    let raw_torrents = get_raw_torrents()?;
    let raw_files = get_raw_files()?;

    let mut torrents = raw_torrents
        .into_iter()
        .map(|t| {
            (
                t.id.clone(),
                (
                    Torrent {
                        id: t.id,
                        name: t.name,
                        base_path: t.path,
                        progress: t.progress,
                        tracker_urls: t.tracker_urls,
                        size: t.size,
                        files: HashMap::new(),
                    },
                    t.files,
                ),
            )
        })
        .collect::<HashMap<_, _>>();

    // TODO: Consider implementing detection of single-file torrents here by checking if the
    // torrent name and the file path are equal.
    for f in raw_files {
        let (torrent, _files_count) = torrents
            .get_mut(&f.torrent_id)
            .ok_or_else(|| anyhow!("{:?} has no matching torrent", f))?;
        if let Some(_old_value) = torrent.files.insert(f.path.clone(), f.size) {
            bail!(
                "{:?} has multiple entries for the same path: {}",
                torrent,
                f.path.display()
            );
        }
    }

    torrents
        .into_iter()
        .map(|(_, (torrent, files_count))| {
            if torrent.files.len() != files_count {
                bail!(
                    "torrent {:?}: got {} files but expected {}",
                    torrent,
                    torrent.files.len(),
                    files_count
                );
            }
            let file_sizes: usize = torrent.files.values().sum();
            if file_sizes != torrent.size {
                bail!(
                    "torrent {:?}: files total {} bytes but expected {}",
                    torrent,
                    file_sizes,
                    torrent.size
                );
            }
            Ok(torrent)
        })
        .collect()
}

pub fn pause_torrent(torrent_id: &str) -> Result<()> {
    let output = Command::new("sycli").args(["pause", torrent_id]).output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub fn resume_torrent(torrent_id: &str) -> Result<()> {
    let output = Command::new("sycli")
        .args(["resume", torrent_id])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub fn move_torrent(torrent_id: &str, dir_path: &Path) -> Result<()> {
    let output = Command::new("sycli")
        .args([
            "torrent",
            torrent_id,
            "move",
            "--skip-files",
            dir_path
                .to_str()
                .ok_or_else(|| anyhow!("move_torrent cannot handle non-UTF8 paths"))?,
        ])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

// TODO: This isn't really the right place for this helper, but since Torrent is also defined
// here for now...
#[derive(Debug, Error, PartialEq)]
pub enum FilterTorrentsError {
    #[error("{0} includes non-source files {1:?}")]
    TorrentIncludesSourceAndNonSourceFiles(String, Vec<PathBuf>),
    #[error("no torrent matched all source files: matched {matched} out of {total} source files")]
    DidNotMatchAllSourceFiles { matched: usize, total: usize },
}

/// Filters `torrents` and return a `Vec` with all torrents that contain files in `source_files`.
///
/// Returns an error if:
/// - a torrent contains some files in `source_files` and some files not in `source_files`.
/// - or if not all `source_files` were matched by `torrents`.
pub fn filter_torrents(
    torrents: &[Torrent],
    source_files: &HashMap<PathBuf, u64>,
) -> Result<Vec<Torrent>, FilterTorrentsError> {
    type Error = FilterTorrentsError;

    let mut filtered_torrents = vec![];
    let mut included_paths = HashSet::new();
    for torrent in torrents {
        let (included, missing) =
            torrent
                .files
                .iter()
                .fold((0, vec![]), |(included, mut missing), (path, _size)| {
                    let path = torrent.base_path.join(path);
                    if source_files.contains_key(&path) {
                        included_paths.insert(path);
                        (included + 1, missing)
                    } else {
                        missing.push(path.clone());
                        (included, missing)
                    }
                });
        if missing.len() == torrent.files.len() {
            // Torrent has no files specified in source files, so it is not interesting.
            continue;
        }
        if included > 0 && missing.len() > 0 {
            return Err(Error::TorrentIncludesSourceAndNonSourceFiles(
                torrent.id.clone(),
                missing,
            ));
        }
        filtered_torrents.push(torrent.clone());
    }

    match (included_paths.len(), source_files.len()) {
        (matched, total) if matched == total => Ok(filtered_torrents),
        (matched, total) => Err(Error::DidNotMatchAllSourceFiles { matched, total }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_deserialize() {
        let json = r#"
          {
            "type": "torrent",
            "id": "1234567890123456789012345678901234567890",
            "name": "data.txt",
            "path": "/tmp",
            "progress": 0.25,
            "tracker_urls": [
              "example.com"
            ],
            "size": 88888888,
            "files": 1,
            "unknown_field": "is_ignored"
          }
        "#;
        let t: RawTorrent = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "1234567890123456789012345678901234567890");
        assert_eq!(t.name, "data.txt",);
        assert_eq!(t.path, Path::new("/tmp"));
        assert_eq!(t.progress, 0.25);
        assert_eq!(t.tracker_urls, &["example.com"]);
        assert_eq!(t.size, 88888888);
        assert_eq!(t.files, 1);
    }

    #[test]
    fn file_deserialize() {
        let json = r#"
          {
            "type": "file",
            "id": "0123456789012345678901234567890123456789",
            "torrent_id": "1234567890123456789012345678901234567890",
            "path": "data.txt",
            "size": 88888888,
            "unknown_field": "is_ignored"
          }
        "#;
        let f: RawFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.id, "0123456789012345678901234567890123456789");
        assert_eq!(f.torrent_id, "1234567890123456789012345678901234567890");
        assert_eq!(f.path, Path::new("data.txt"));
        assert_eq!(f.size, 88888888);
    }

    #[test]
    fn filter_torrents_with_no_source_files() {
        let torrent = Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        assert_eq!(filter_torrents(&[torrent], &HashMap::new()), Ok(vec![]));
    }

    #[test]
    fn filter_torrents_with_no_torrents() {
        let source_files = HashMap::from([("/tmp/test.txt".into(), 123)]);
        assert_eq!(
            filter_torrents(&[], &source_files),
            Err(FilterTorrentsError::DidNotMatchAllSourceFiles {
                matched: 0,
                total: 1
            })
        );
    }

    #[test]
    fn filter_torrents_with_no_torrents_or_source_files() {
        assert_eq!(filter_torrents(&[], &HashMap::new()), Ok(vec![]));
    }

    #[test]
    fn filter_torrents_normal() {
        let torrent = Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        let torrent2 = Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test2.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test2.txt".into(), 123)]),
        };
        let source_files = HashMap::from([("/tmp/test.txt".into(), 123)]);
        assert_eq!(
            filter_torrents(&[torrent.clone(), torrent2], &source_files),
            Ok(vec![torrent])
        );
    }

    #[test]
    fn filter_torrents_torrent_with_included_and_non_included_files() {
        let torrent = Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123), ("test2.txt".into(), 123)]),
        };
        let source_files = HashMap::from([("/tmp/test.txt".into(), 123)]);
        assert_eq!(
            filter_torrents(&[torrent.clone()], &source_files),
            Err(FilterTorrentsError::TorrentIncludesSourceAndNonSourceFiles(
                torrent.id.clone(),
                vec![PathBuf::from("/tmp/test2.txt")],
            ))
        );
    }

    #[test]
    fn filter_torrent_not_all_source_files_matched() {
        let torrent = Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        let source_files = HashMap::from([
            ("/tmp/test.txt".into(), 123),
            ("/tmp/test2.txt".into(), 123),
        ]);
        assert_eq!(
            filter_torrents(&[torrent], &source_files),
            Err(FilterTorrentsError::DidNotMatchAllSourceFiles {
                matched: 1,
                total: 2
            }),
        );
    }
}
