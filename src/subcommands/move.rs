use anyhow::{anyhow, bail};
use clap::Args;
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::sycli;

#[derive(Args)]
pub struct MoveArgs {
    /// Source file or directory to move.
    source: PathBuf,

    /// Destination directory.
    target: PathBuf,
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

        for torrent in &torrents {
            eprintln!("pausing {}", torrent.id);
            sycli::pause_torrent(&torrent.id)?;
        }

        move_files(
            &source,
            &target,
            || -> anyhow::Result<()> {
                let source_is_file = source.is_file();
                for torrent in &torrents {
                    let new_path =
                        calculate_new_base_path(&source, source_is_file, &target, torrent)?;
                    eprintln!(
                        "updating {} to directory {}",
                        torrent.id,
                        new_path.display()
                    );
                    sycli::move_torrent(&torrent.id, &new_path)?
                }
                Ok(())
            },
            || -> anyhow::Result<()> {
                for torrent in &torrents {
                    eprintln!("resuming {}", torrent.id);
                    sycli::resume_torrent(&torrent.id)?;
                }
                Ok(())
            },
        )?;

        // TODO: Make this work with the cross-seed subcommand (to be added), which uses symlinks.

        Ok(())
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
}

fn collect_files(path: &Path) -> Result<HashMap<PathBuf, std::fs::Metadata>, CollectFilesError> {
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

        let metadata = entry.metadata()?;

        if let Some(_old_value) = files.insert(entry.path().into(), metadata) {
            return Err(Error::DuplicateEntry(entry.path().to_path_buf()));
        }
    }
    // TODO: Maybe comment on empty directories?
    Ok(files)
}

#[derive(Debug, Error)]
enum FilterTorrentsError {
    #[error("{0:?} includes source and non-source files")]
    TorrentIncludesSourceAndNonSourceFiles(sycli::Torrent),
    #[error("no torrents matched source files")]
    NoMatchingTorrents(),
    #[error("no torrent matched all source files: matched {matched} out of {total} source files")]
    DidNotMatchAllSourceFiles { matched: usize, total: usize },
}

fn filter_torrents(
    torrents: Vec<sycli::Torrent>,
    source_files: &HashMap<PathBuf, std::fs::Metadata>,
) -> Result<Vec<sycli::Torrent>, FilterTorrentsError> {
    type Error = FilterTorrentsError;

    let mut filtered_torrents = vec![];
    for torrent in torrents {
        let (included, not_included) =
            torrent
                .files
                .iter()
                .fold((0, 0), |(included, missing), (path, _size)| {
                    if source_files.contains_key(&torrent.base_path.join(path)) {
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

    match filtered_torrents
        .iter()
        .map(|torrent| torrent.files.len())
        .max()
    {
        None => Err(Error::NoMatchingTorrents()),
        Some(len) if len == source_files.len() => Ok(filtered_torrents),
        Some(len) => Err(Error::DidNotMatchAllSourceFiles {
            matched: len,
            total: source_files.len(),
        }),
    }
}

// TODO: Add some tests, especially for the cross-device case.
fn move_files<F, G>(
    source: &Path,
    target: &Path,
    move_torrents: F,
    resume_torrents: G,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
    G: FnOnce() -> anyhow::Result<()>,
{
    let target_with_file_name = target.join(source.file_name().ok_or_else(|| {
        anyhow!(
            "could not extract file name component from {}",
            source.display()
        )
    })?);

    eprintln!(
        "moving {} to {}",
        source.display(),
        target_with_file_name.display()
    );
    match std::fs::rename(source, &target_with_file_name) {
        Ok(()) => {
            move_torrents()?;
            resume_torrents()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            // If `source` is a prefix of `target`, cleaning up the original source files will lead
            // to data loss!
            assert!(!target.starts_with(&source));

            eprintln!("rename failed; falling back to manual file copy");
            // In this case, it's fine to resume the torrents pre-emptively since the files will be
            // copied to `target`.
            resume_torrents()?;

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
        }
        e => e?,
    };

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
    #[cfg(not(test))]
    assert!(target.is_dir());

    type Error = CalculateNewBasePathError;

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
