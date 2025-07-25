use clap::Args;
use std::path::{Path, PathBuf};

use crate::sycli;

#[derive(Args)]
pub struct UpdatePathsArgs {
    /// Source file or directory to move.
    source: PathBuf,

    /// Destination directory.
    target: PathBuf,

    /// A directory with symlinks to update. May be specified multiple times.
    #[arg(long)]
    symlink_dir: Vec<PathBuf>,
}

impl UpdatePathsArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        let source = std::path::absolute(self.source)?;
        let target = std::path::absolute(self.target)?;

        // TODO: Consider changing this logic to handle paths in a similar way to the move
        // subcommand, since it could be used to help pick up the pieces if move fails in the
        // middle for whatever reason.
        for torrent in sycli::get_torrents()? {
            if let Some(remainder) = torrent.base_path.strip_prefix(&source).ok() {
                let new_base_path = target.join(remainder);
                eprintln!(
                    "Updating {} from {} to {}",
                    torrent.id,
                    source.display(),
                    target.display()
                );
                sycli::move_torrent(&torrent.id, &new_base_path)?;
            }
        }

        for symlink_dir in self.symlink_dir {
            for symlink in collect_symlinks(&symlink_dir)? {
                let original_target_path = std::fs::read_link(&symlink)?;
                if let Some(remainder) = original_target_path.strip_prefix(&source).ok() {
                    let new_target_path = target.join(remainder);
                    eprintln!(
                        "Updating symlink {} from {} to {}",
                        symlink.display(),
                        original_target_path.display(),
                        new_target_path.display()
                    );
                    create_or_update_symlink(&symlink, &new_target_path)?;
                }
            }
        }

        Ok(())
    }
}

fn collect_symlinks(path: &Path) -> Result<Vec<PathBuf>, walkdir::Error> {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) => {
                if entry.path_is_symlink() {
                    Some(Ok(entry.into_path()))
                } else {
                    None
                }
            }
            Err(err) => Some(Err(err)),
        })
        .collect()
}

/// Unlike `ln -sfn`, this does not try to be clever and preserve state on failure. The underlying
/// implementation deletes the original and creates a new symlink if `link` already exists.
fn create_or_update_symlink(link: &Path, target: &Path) -> std::io::Result<()> {
    match std::os::unix::fs::symlink(target, link) {
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&link)?;
            std::os::unix::fs::symlink(target, link)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_symlinks_no_symlinks() {
        let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
        assert!(collect_symlinks(tmp_dir.path()).unwrap().is_empty());

        std::fs::write(tmp_dir.path().join("file"), "contents")
            .expect("failed to create normal file");

        assert!(collect_symlinks(tmp_dir.path()).unwrap().is_empty());
    }

    #[test]
    fn collect_symlinks_regular() {
        let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let file_path = tmp_dir.path().join("file");
        let symlink_path = tmp_dir.path().join("symlink");

        std::os::unix::fs::symlink(&file_path, &symlink_path).expect("failed to create symlink");

        assert_eq!(
            collect_symlinks(tmp_dir.path()).unwrap(),
            vec![symlink_path.clone()]
        );
    }

    #[test]
    fn create_or_update_symlink_basic() {
        let tmp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let file_path = tmp_dir.path().join("file");
        let symlink_path = tmp_dir.path().join("symlink");

        create_or_update_symlink(&symlink_path, &file_path).expect("failed to create symlink");
        assert_eq!(std::fs::read_link(&symlink_path).unwrap(), file_path);

        let new_file_path = tmp_dir.path().join("new file");
        create_or_update_symlink(&symlink_path, &new_file_path).expect("failed to update symlink");
        assert_eq!(std::fs::read_link(&symlink_path).unwrap(), new_file_path);
    }
}
