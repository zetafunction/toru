use anyhow::{anyhow, bail};
use clap::{Args, ValueEnum};
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::sycli;

#[derive(Args)]
pub struct UpdatePathsArgs {
    /// Source file or directory to move.
    source: PathBuf,

    /// Destination directory.
    target: PathBuf,

    /// Directories with symlinks to update.
    #[arg(long)]
    symlinks: Vec<PathBuf>,
}

impl UpdatePathsArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        let source = std::path::absolute(self.source)?;
        let target = std::path::absolute(self.target)?;

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

        Ok(())
    }
}
