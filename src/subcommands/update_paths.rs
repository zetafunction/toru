use clap::Args;
use std::path::PathBuf;

use crate::fs;
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
            if let Ok(remainder) = torrent.base_path.strip_prefix(&source) {
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
            for symlink in fs::collect_symlinks(&symlink_dir)? {
                let original_target_path = std::fs::read_link(&symlink)?;
                if let Ok(remainder) = original_target_path.strip_prefix(&source) {
                    let new_target_path = target.join(remainder);
                    eprintln!(
                        "Updating symlink {} from {} to {}",
                        symlink.display(),
                        original_target_path.display(),
                        new_target_path.display()
                    );
                    fs::create_or_update_symlink(&symlink, &new_target_path)?;
                }
            }
        }

        Ok(())
    }
}
