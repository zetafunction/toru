use std::path::PathBuf;

use clap::Args;

#[derive(Args)]
pub struct ImportArgs {
    /// If a torrent is successfully matched against a set of files, but the files do not have
    /// matching file names, the importer will create a new directory here that mirrors the
    /// expected structure, using symlinks to referenced the matched files.
    #[arg(long, required(true))]
    symlink_dir: PathBuf,

    /// Directories to search for file matches.
    #[arg(long, required(true))]
    source: Vec<PathBuf>,

    /// Torrent files to import.
    #[arg(required(true))]
    torrents: Vec<PathBuf>,

    /// Find matching files and create symlinks, but do not actually import the torrent file into a
    /// client.
    #[arg(long)]
    skip_add: bool,
}

impl ImportArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        Ok(())
    }
}
