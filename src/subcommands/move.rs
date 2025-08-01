use anyhow::{anyhow, bail};
use clap::{Args, ValueEnum};
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::fs;
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
        let source_is_file = source.is_file();
        let target = std::path::absolute(self.target)?;

        let source_files = fs::collect_files(&source)?;

        // TODO: Abstract this out so multiple torrent client backends can be used.
        let torrents = sycli::filter_torrents(sycli::get_torrents()?, &source_files)?;

        let pause_torrents = || -> anyhow::Result<()> {
            for torrent in &torrents {
                eprintln!("pausing {}", torrent.id);
                sycli::pause_torrent(&torrent.id)?;
            }
            Ok(())
        };

        let move_torrents = || -> anyhow::Result<()> {
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
                "{bytes} {elapsed_precise} [ {bytes_per_sec} ] [{wide_bar:.cyan/blue}] {percent}% ETA {eta_precise}",
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
        progress.finish();
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
        progress.finish();
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
