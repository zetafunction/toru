use anyhow::{anyhow, bail};
use clap::{Args, ValueEnum};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::fs;
use crate::sycli;
use crate::util;

#[derive(Args)]
pub struct MoveArgs {
    /// Source file or directory to move.
    source: PathBuf,

    /// Destination directory.
    target: PathBuf,

    /// How to move the files.
    #[arg(default_value = "copy-and-unlink", long, value_enum)]
    strategy: Strategy,

    /// A directory with symlinks to update. May be specified multiple times.
    #[arg(long = "symlink_dir")]
    symlink_dir: Vec<PathBuf>,

    #[arg(long)]
    dry_run: bool,
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
        let unfiltered_torrents = sycli::get_torrents()?;
        let torrents = sycli::filter_torrents(&unfiltered_torrents, &source_files)?;
        if torrents.is_empty() {
            bail!("could not find torrents that matched {}", source.display());
        }

        if let Some(torrent) = torrents.iter().find(|torrent| torrent.progress != 1.0) {
            bail!("{} is incomplete; cannot move!", torrent.id);
        }

        let mut symlinks_to_update = HashMap::new();
        for symlink_dir in self.symlink_dir {
            symlinks_to_update.extend(
                fs::collect_symlinks(&symlink_dir)?
                    .into_iter()
                    .filter(|(_, target_path)| source_files.contains_key(target_path)),
            );
        }
        let symlinks_to_update = symlinks_to_update;
        // TODO: filter_torrents() should probably take a HashSet since the length value isn't
        // actually used.
        let symlinks_for_filter = symlinks_to_update
            .keys()
            .map(|path| (path.clone(), 0))
            .collect();

        // These need to be paused while files are shuffled around to prevent broken links.
        let symlinked_torrents =
            sycli::filter_torrents(&unfiltered_torrents, &symlinks_for_filter)?;
        if let Some(torrent) = symlinked_torrents
            .iter()
            .find(|torrent| torrent.progress != 1.0)
        {
            bail!("{} (symlinked) is incomplete; cannot move!", torrent.id);
        }

        for torrent in &torrents {
            eprintln!("pausing {}", torrent.id);
            if !self.dry_run {
                sycli::pause_torrent(&torrent.id)?;
            }
        }
        for torrent in &symlinked_torrents {
            eprintln!("pausing {} (symlinked)", torrent.id);
            if !self.dry_run {
                sycli::pause_torrent(&torrent.id)?;
            }
        }

        let move_torrents = || -> anyhow::Result<()> {
            for torrent in &torrents {
                let new_path = calculate_new_base_path(&source, source_is_file, &target, torrent)?;
                eprintln!(
                    "updating {} to directory {}",
                    torrent.id,
                    new_path.display()
                );
                if !self.dry_run {
                    sycli::move_torrent(&torrent.id, &new_path)?;
                }
            }
            Ok(())
        };

        eprintln!(
            "moving files from {} to {}",
            source.display(),
            target.display()
        );
        match self.strategy {
            Strategy::Rename => {
                move_files_with_rename(self.dry_run, &source, &target, move_torrents)
            }
            Strategy::CopyAndUnlink => {
                move_files_with_copy(self.dry_run, &source, &target, move_torrents)
            }
        }?;

        update_symlinks(self.dry_run, &source, &target, &symlinks_to_update)?;

        for torrent in &torrents {
            eprintln!("resuming {}", torrent.id);
            if !self.dry_run {
                sycli::resume_torrent(&torrent.id)?;
            }
        }

        for torrent in &symlinked_torrents {
            eprintln!("resuming {} (symlinked)", torrent.id);
            if !self.dry_run {
                sycli::resume_torrent(&torrent.id)?;
            }
        }

        Ok(())
    }
}

// TODO: Add some tests, especially for the cross-device case.
fn move_files_with_rename<M>(
    dry_run: bool,
    source: &Path,
    target: &Path,
    move_torrents: M,
) -> anyhow::Result<()>
where
    M: FnOnce() -> anyhow::Result<()>,
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

    if !dry_run {
        std::fs::rename(source, &target_with_file_name)?;
    }
    move_torrents()
}

fn move_files_with_copy<M>(
    dry_run: bool,
    source: &Path,
    target: &Path,
    move_torrents: M,
) -> anyhow::Result<()>
where
    M: FnOnce() -> anyhow::Result<()>,
{
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

    let progress = util::new_progress_bar();
    if source.is_dir() {
        if !dry_run {
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
        }
        progress.finish();
        move_torrents()?;
        if !dry_run {
            std::fs::remove_dir_all(source)?;
        }
    } else {
        if !dry_run {
            fs_extra::file::copy_with_progress(
                source,
                target_with_file_name,
                &fs_extra::file::CopyOptions::new(),
                |process| {
                    progress.set_length(process.total_bytes);
                    progress.set_position(process.copied_bytes);
                },
            )?;
        }
        progress.finish();
        move_torrents()?;
        if !dry_run {
            std::fs::remove_file(source)?;
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
enum UpdateSymlinksError {
    #[error("not a prefix: {0}")]
    NotAPrefix(#[from] std::path::StripPrefixError),
    #[error("path {0} has no parent")]
    NoParent(PathBuf),
    #[error("IO error")]
    Io(#[from] std::io::Error),
}

fn update_symlinks(
    dry_run: bool,
    source: &Path,
    target: &Path,
    symlinks: &HashMap<PathBuf, PathBuf>,
) -> Result<(), UpdateSymlinksError> {
    let source_dir = source
        .parent()
        .ok_or_else(|| UpdateSymlinksError::NoParent(source.to_path_buf()))?;
    for (symlink, symlink_target) in symlinks {
        let new_symlink_target = target.join(symlink_target.strip_prefix(source_dir)?);
        eprintln!(
            "updating symlink {} from {} to {}",
            symlink.display(),
            symlink_target.display(),
            new_symlink_target.display()
        );
        if !dry_run {
            fs::create_or_update_symlink(symlink, &new_symlink_target)?;
        }
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
    use std::collections::HashMap;

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
