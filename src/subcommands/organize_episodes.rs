use clap::Args;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::PathBuf;

use crate::sycli;

#[derive(Args)]
pub struct OrganizeEpisodesArgs {
    /// Base directory to work from.
    #[arg(long)]
    base_dir: PathBuf,

    /// If true, only prints out what would be done.
    #[arg(long)]
    dry_run: bool,

    /// The files to process.
    #[arg(num_args(1..))]
    files: Vec<PathBuf>,
}

impl OrganizeEpisodesArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        let to_process = self.files.into_iter().collect::<HashSet<_>>();

        let torrents = sycli::get_torrents()?;

        // TODO: Punting on the harder problem here: instead, only bother processing torrents with a
        // single associated file. Otherwise, this would have to do something a bit more clever to
        // coalesce entries.
        let files = torrents
            .into_iter()
            .filter_map(|torrent| {
                if torrent.files.len() != 1 {
                    return None;
                };
                let (path, size) = torrent.files.into_iter().next().unwrap();
                let path = torrent.base_path.join(&path);
                if to_process.contains(&path) {
                    Some((torrent.id, (path, size)))
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();
        let re =
        Regex::new(r"^(?<header>.+?\.S[0-9][0-9])E[0-9][0-9]\..+?\.(?<trailer>(?:720|1080|2160)p\..+?\.WEB-DL.+)\.mkv").unwrap();

        for (torrent_id, (path, _size)) in files {
            eprintln!(
                "processing file {} for torrent {}...",
                path.display(),
                torrent_id
            );
            let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
                eprintln!(
                    "  warning: missing or invalid filename for {}; skipping",
                    path.display()
                );
                continue;
            };

            let Some(captures) = re.captures(file_name) else {
                eprintln!(
                    "  warning: unable to extract metadata from {}; skipping",
                    file_name
                );
                continue;
            };

            let dir_name = [
                captures.name("header").unwrap().as_str(),
                captures.name("trailer").unwrap().as_str(),
            ]
            .join(".");

            let dir_path = self.base_dir.join(dir_name);

            eprintln!("  making directory {}", dir_path.display());
            if !self.dry_run {
                std::fs::create_dir(&dir_path).or_else(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(e)
                    }
                })?;
            }
            eprintln!(
                "  creating link from {} to original {}",
                dir_path.join(file_name).display(),
                path.display(),
            );
            if !self.dry_run {
                std::fs::hard_link(&path, dir_path.join(file_name))?;
            }
            eprintln!(
                "  updating torrent {} directory to {}",
                torrent_id,
                dir_path.display()
            );
            if !self.dry_run {
                sycli::move_torrent(&torrent_id, &dir_path)?;
            }
            eprintln!("  unlinking original path {}", path.display());
            if !self.dry_run {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }
}
