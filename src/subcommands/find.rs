use clap::Args;
use std::path::PathBuf;

use crate::fs;
use crate::sycli;

#[derive(Args)]
pub struct FindArgs {
    /// Destination directory.
    path: PathBuf,
}

impl FindArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        let path = std::path::absolute(self.path)?;

        let files = fs::collect_files(&path)?;

        // TODO: Abstract this out so multiple torrent client backends can be used.
        let torrents = sycli::filter_torrents(&sycli::get_torrents()?, &files)?;

        println!(
            "Found {} torrent(s) seeded from {}",
            torrents.len(),
            path.display()
        );
        for torrent in torrents {
            println!("  {}", torrent.id);
        }
        Ok(())
    }
}
