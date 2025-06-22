use anyhow::{anyhow, bail};
use clap::{Args, ValueEnum};
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::sycli;

#[derive(Args)]
pub struct MoveArgs {
    /// Source file or directory to move.
    source: PathBuf,

    /// Destination directory.
    target: PathBuf,

    /// How to move the files.
    #[arg(default_value = "copy-and-unlink", long, value_enum)]
    strategy: Strategy,
}

#[derive(Copy, Clone, Default, ValueEnum)]
enum Strategy {
    /// Copy-based approach; works across devices at the cost of requiring double the space
    /// temporarily.
    #[default]
    CopyAndUnlink,
    /// Rename-based approach; does not work across devices.
    Rename,
}

impl MoveArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        if !self.target.is_dir() {
            bail!("target {} is not a directory", self.target.display());
        }

        let source = std::path::absolute(self.source)?;
        let target = std::path::absolute(self.target)?;

        // TODO: Make the paths absolute here?

        let source_files = collect_files(&source)?;

        // TODO: Abstract this out so multiple torrent client backends can be used.
        let torrents = filter_torrents(sycli::get_torrents()?, &source_files)?;

        let pause_torrents = || -> anyhow::Result<()> {
            for torrent in &torrents {
                eprintln!("pausing {}", torrent.id);
                sycli::pause_torrent(&torrent.id)?;
            }
            Ok(())
        };

        let move_torrents = || -> anyhow::Result<()> {
            let source_is_file = source.is_file();
            for torrent in &torrents {
                let new_path = calculate_new_base_path(&source, source_is_file, &target, torrent)?;
                eprintln!(
                    "updating {} to directory {}",
                    torrent.id,
                    new_path.display()
                );
                sycli::move_torrent(&torrent.id, &new_path)?;
            }
            Ok(())
        };

        let resume_torrents = || -> anyhow::Result<()> {
            for torrent in &torrents {
                eprintln!("resuming {}", torrent.id);
                sycli::resume_torrent(&torrent.id)?;
            }
            Ok(())
        };

        match self.strategy {
            Strategy::Rename => move_files_with_rename(
                &source,
                &target,
                pause_torrents,
                move_torrents,
                resume_torrents,
            ),
            Strategy::CopyAndUnlink => move_files_with_copy(&source, &target, move_torrents),
        }
    }
}

#[derive(Debug, Error)]
enum CollectFilesError {
    #[error("WalkDir failed: {0:?}")]
    WalkDir(#[from] walkdir::Error),
    #[error("non-file path {1} encountered in {0}")]
    NonFilePath(PathBuf, PathBuf),
    #[error("duplicate entry for {0}")]
    DuplicateEntry(PathBuf),
    #[error("no files")]
    NoFiles,
}

/// Walks `path` and returns a `HashMap` of file paths to file sizes in that directory tree. Any
/// subdirectories are not included in the returned map.
///
/// If `path` is a file, returns a map with a single entry of `path` and its size.
/// If `path` contains any non-directory and non-file entries, returns an error.
fn collect_files(path: &Path) -> Result<HashMap<PathBuf, u64>, CollectFilesError> {
    type Error = CollectFilesError;

    let mut files = HashMap::new();
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;

        if entry.file_type().is_dir() {
            // Do not include directories in the result, as torrents only contain files.
            continue;
        }

        // Give up if anything other than a file or directory is encountered. Directories are
        // normal and expected (though completely ignored by the torrent format), while
        // anything else is unexpected and probably needs the user to decide what to do.
        if !entry.file_type().is_file() {
            return Err(Error::NonFilePath(
                path.to_path_buf(),
                entry.path().to_path_buf(),
            ));
        }

        if let Some(_old_value) = files.insert(entry.path().into(), entry.metadata()?.len()) {
            return Err(Error::DuplicateEntry(entry.path().to_path_buf()));
        }
    }
    if files.is_empty() {
        Err(Error::NoFiles)
    } else {
        Ok(files)
    }
}

#[derive(Debug, Error, PartialEq)]
enum FilterTorrentsError {
    #[error("{0:?} includes source and non-source files")]
    TorrentIncludesSourceAndNonSourceFiles(sycli::Torrent),
    #[error("no torrents matched source files")]
    NoMatchingTorrents,
    #[error("no torrent matched all source files: matched {matched} out of {total} source files")]
    DidNotMatchAllSourceFiles { matched: usize, total: usize },
}

/// Filters `torrents` and return a `Vec` with all torrents that contain files in `source_files`.
///
/// Returns an error if:
/// - a torrent contains some files in `source_files` and some files not in `source_files`.
/// - or if not all `source_files` were matched by `torrents`.
fn filter_torrents(
    torrents: Vec<sycli::Torrent>,
    source_files: &HashMap<PathBuf, u64>,
) -> Result<Vec<sycli::Torrent>, FilterTorrentsError> {
    type Error = FilterTorrentsError;

    let mut filtered_torrents = vec![];
    let mut included_paths = HashSet::new();
    for torrent in torrents {
        let (included, not_included) =
            torrent
                .files
                .iter()
                .fold((0, 0), |(included, missing), (path, _size)| {
                    let path = torrent.base_path.join(path);
                    if source_files.contains_key(&path) {
                        included_paths.insert(path);
                        (included + 1, missing)
                    } else {
                        (included, missing + 1)
                    }
                });
        if not_included == torrent.files.len() {
            // Torrent has no files specified in source files, so it is not interesting.
            continue;
        }
        if included > 0 && not_included > 0 {
            return Err(Error::TorrentIncludesSourceAndNonSourceFiles(torrent));
        }
        filtered_torrents.push(torrent);
    }

    match (included_paths.len(), source_files.len()) {
        (0, _) => Err(Error::NoMatchingTorrents),
        (matched, total) if matched == total => Ok(filtered_torrents),
        (matched, total) => Err(Error::DidNotMatchAllSourceFiles { matched, total }),
    }
}

// TODO: Add some tests, especially for the cross-device case.
fn move_files_with_rename<P, M, R>(
    source: &Path,
    target: &Path,
    pause_torrents: P,
    move_torrents: M,
    resume_torrents: R,
) -> anyhow::Result<()>
where
    P: FnOnce() -> anyhow::Result<()>,
    M: FnOnce() -> anyhow::Result<()>,
    R: FnOnce() -> anyhow::Result<()>,
{
    let target_with_file_name = target.join(source.file_name().ok_or_else(|| {
        anyhow!(
            "could not extract file name component from {}",
            source.display()
        )
    })?);

    eprintln!(
        "moving {} to {} using rename",
        source.display(),
        target_with_file_name.display()
    );

    pause_torrents()?;
    std::fs::rename(source, &target_with_file_name)?;
    move_torrents()?;
    resume_torrents()
}

fn move_files_with_copy<M: FnOnce() -> anyhow::Result<()>>(
    source: &Path,
    target: &Path,
    move_torrents: M,
) -> anyhow::Result<()> {
    let target_with_file_name = target.join(source.file_name().ok_or_else(|| {
        anyhow!(
            "could not extract file name component from {}",
            source.display()
        )
    })?);

    eprintln!(
        "moving {} to {} using copy",
        source.display(),
        target_with_file_name.display()
    );

    // If `source` is a prefix of `target`, cleaning up the original source files will lead
    // to data loss!
    assert!(!target.starts_with(source));

    let progress = ProgressBar::no_length()
        .with_style(
            ProgressStyle::with_template(
                "[{elapsed_precise}] [{bar:30.cyan/blue}] {bytes}/{total_bytes} {msg}",
            )
            .unwrap()
            .progress_chars("=> "),
        )
        .with_finish(ProgressFinish::AndLeave);
    if source.is_dir() {
        fs_extra::dir::copy_with_progress(
            source,
            target,
            &fs_extra::dir::CopyOptions::new(),
            |process| {
                progress.set_message(process.file_name);
                progress.set_length(process.total_bytes);
                progress.set_position(process.copied_bytes);
                fs_extra::dir::TransitProcessResult::ContinueOrAbort
            },
        )?;
        progress.finish_and_clear();
        move_torrents()?;
        std::fs::remove_dir_all(source)?;
    } else {
        fs_extra::file::copy_with_progress(
            source,
            target_with_file_name,
            &fs_extra::file::CopyOptions::new(),
            |process| {
                progress.set_length(process.total_bytes);
                progress.set_position(process.copied_bytes);
            },
        )?;
        progress.finish_and_clear();
        move_torrents()?;
        std::fs::remove_file(source)?;
    }

    Ok(())
}

// TODO: These error messages need improvement.
#[derive(Debug, Error, PartialEq)]
enum CalculateNewBasePathError {
    #[error("not a prefix: {0}")]
    NotAPrefix(#[from] std::path::StripPrefixError),
    #[error("path {0} has no parent")]
    NoParent(PathBuf),
    #[error("source path {0} has no file name component")]
    NoFileName(PathBuf),
}

fn calculate_new_base_path(
    source: &Path,
    source_is_file: bool,
    target: &Path,
    torrent: &sycli::Torrent,
) -> Result<PathBuf, CalculateNewBasePathError> {
    type Error = CalculateNewBasePathError;

    #[cfg(not(test))]
    assert!(target.is_dir());

    if source_is_file {
        return Ok(target.to_path_buf());
    }

    let is_single_file = torrent.files.len() == 1
        && torrent
            .files
            .iter()
            .all(|(path, _size)| path.iter().count() == 1);

    if is_single_file {
        let mut new_base_path = target.to_path_buf();
        let remainder = torrent.base_path.strip_prefix(source)?;
        new_base_path.push(
            source
                .file_name()
                .ok_or_else(|| Error::NoFileName(source.to_path_buf()))?,
        );
        new_base_path.extend(remainder.iter());
        Ok(new_base_path)
    } else {
        let original_base_path = torrent.base_path.join(&torrent.name);
        let source = source
            .parent()
            .ok_or_else(|| Error::NoParent(source.to_path_buf()))?;
        let new_base_path_with_torrent_name = target.join(original_base_path.strip_prefix(source)?);
        Ok(new_base_path_with_torrent_name
            .parent()
            .map(Path::to_path_buf)
            .ok_or(Error::NoParent(new_base_path_with_torrent_name))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_files_empty_dir() {
        let tmp_dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            collect_files(&tmp_dir.path()),
            Err(CollectFilesError::NoFiles)
        ));
    }

    #[test]
    fn collect_files_with_file() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("test_file");
        std::fs::write(&test_file, "").expect("failed to create test file");

        assert!(collect_files(&test_file).is_ok_and(|files| { files.contains_key(&test_file) }));
    }

    #[test]
    fn collect_files_with_dir() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("test_file");
        std::fs::write(&test_file, "").expect("failed to create test file");

        let files = collect_files(tmp_dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains_key(&test_file));
    }

    #[test]
    fn collect_files_with_symlink_fails() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("test_file");
        std::fs::write(&test_file, "").expect("failed to create test file");
        let test_symlink = tmp_dir.path().join("test_symlink");
        std::os::unix::fs::symlink(test_file, test_symlink).expect("failed to create test symlink");

        assert!(matches!(
            collect_files(tmp_dir.path()),
            Err(CollectFilesError::NonFilePath(_, _))
        ));
    }

    #[test]
    fn filter_torrents_with_no_source_files() {
        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        assert_eq!(
            filter_torrents(vec![torrent], &HashMap::new()),
            Err(FilterTorrentsError::NoMatchingTorrents)
        );
    }

    #[test]
    fn filter_torrents_with_no_torrents() {
        let source_files = HashMap::from([("/tmp/test.txt".into(), 123)]);
        assert_eq!(
            filter_torrents(vec![], &source_files),
            Err(FilterTorrentsError::NoMatchingTorrents)
        );
    }

    #[test]
    fn filter_torrents_normal() {
        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        let torrent2 = sycli::Torrent {
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
            filter_torrents(vec![torrent.clone(), torrent2], &source_files),
            Ok(vec![torrent])
        );
    }

    #[test]
    fn filter_torrents_torrent_with_included_and_non_included_files() {
        let torrent = sycli::Torrent {
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
            filter_torrents(vec![torrent.clone()], &source_files),
            Err(FilterTorrentsError::TorrentIncludesSourceAndNonSourceFiles(
                torrent
            ))
        );
    }

    #[test]
    fn filter_torrent_not_all_source_files_matched() {
        let torrent = sycli::Torrent {
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
            filter_torrents(vec![torrent], &source_files),
            Err(FilterTorrentsError::DidNotMatchAllSourceFiles {
                matched: 1,
                total: 2
            }),
        );
    }

    #[test]
    fn calculate_new_base_path_with_single_file_torrent() {
        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        assert_eq!(
            calculate_new_base_path(
                Path::new("/tmp/test.txt"),
                true,
                Path::new("/home/test/data"),
                &torrent
            ),
            Ok("/home/test/data".into())
        );

        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp/test torrent".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        assert_eq!(
            calculate_new_base_path(
                Path::new("/tmp/test torrent"),
                false,
                Path::new("/home/test/data"),
                &torrent
            ),
            Ok("/home/test/data/test torrent".into())
        );

        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test.txt".into(),
            base_path: "/tmp/test torrent/disc 1".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test.txt".into(), 123)]),
        };
        assert_eq!(
            calculate_new_base_path(
                Path::new("/tmp/test torrent"),
                false,
                Path::new("/home/test/data"),
                &torrent
            ),
            Ok("/home/test/data/test torrent/disc 1".into())
        );
    }

    #[test]
    fn calculate_new_base_path_with_multi_file_torrent() {
        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "test torrent".into(),
            base_path: "/tmp".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("test data/test.txt".into(), 123)]),
        };
        assert_eq!(
            calculate_new_base_path(
                Path::new("/tmp/test torrent"),
                false,
                Path::new("/home/test/data"),
                &torrent
            ),
            Ok("/home/test/data".into())
        );

        let torrent = sycli::Torrent {
            id: "0123456789012345678901234567890123456789".into(),
            name: "disc 1".into(),
            base_path: "/tmp/test torrent".into(),
            progress: 1.0,
            tracker_urls: vec!["https://example.com:9999".into()],
            size: 123,
            files: HashMap::from([("disc 1/test.txt".into(), 123)]),
        };
        assert_eq!(
            calculate_new_base_path(
                Path::new("/tmp/test torrent"),
                false,
                Path::new("/home/test/data"),
                &torrent
            ),
            Ok("/home/test/data/test torrent".into())
        );
    }
}
