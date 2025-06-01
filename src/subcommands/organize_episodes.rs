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
        let files = sycli::get_files()?;

        eprintln!("got {} torrents and {} files", torrents.len(), files.len());

        // TODO: torrent.id is redundant, but meh?
        let torrents = torrents
            .into_iter()
            .map(|torrent| (torrent.id.clone(), torrent))
            .collect::<HashMap<_, _>>();

        // TODO: Punting on the harder problem here: instead, only bother processing torrents with a
        // single-associated file. Otherwise, this would have to do something a bit more clever to
        // coalesce entries.
        let files = files
            .into_iter()
            .filter_map(|file| if let Some(torrent) = torrents.get(&file.torrent_id) {
                if torrent.files != 1 {
                    None
                } else if torrent.size != file.size {
                    eprintln!(
                        "warning: file {} associated with torrent {} but size ({} vs {}) does not match; skipping",
                        file.path.display(), torrent.id, file.size, torrent.size,
                    );
                    None
                } else {
                    let path = torrent.path.join(file.path);
                    if to_process.contains(&path) {
                        Some((
                                file.torrent_id.clone(),
                                sycli::File {
                                    id: file.id,
                                    torrent_id: file.torrent_id,
                                    path,
                                    size: file.size,
                                },
                        ))
                    } else {
                        None
                    }
                }
            } else  {
                eprintln!("warning: file {} has no matching torrent; skipping", file.path.display());
                None
            })
            .collect::<HashMap<_, _>>();
        let re =
        Regex::new(r"^(?<header>.+?\.S[0-9][0-9])E[0-9][0-9]\..+?\.(?<trailer>(?:720|1080|2160)p\..+?\.WEB-DL.+)\.mkv").unwrap();

        for (torrent_id, file) in files {
            eprintln!(
                "processing file {} for torrent {}...",
                file.path.display(),
                torrent_id
            );
            let Some(file_name) = file.path.file_name().and_then(OsStr::to_str) else {
                eprintln!(
                    "  warning: missing or invalid filename for {:?}; skipping",
                    file
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
                file.path.display(),
            );
            if !self.dry_run {
                std::fs::hard_link(&file.path, dir_path.join(file_name))?;
            }
            eprintln!(
                "  updating torrent {} directory to {}",
                torrent_id,
                dir_path.display()
            );
            if !self.dry_run {
                sycli::move_torrent(&torrent_id, &dir_path)?;
            }
            eprintln!("  unlinking original path {}", file.path.display());
            if !self.dry_run {
                std::fs::remove_file(&file.path)?;
            }
        }
        Ok(())
    }
}
