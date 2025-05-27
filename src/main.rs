mod sycli;

use clap::Parser;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
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

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let to_process = args.files.into_iter().collect::<HashSet<_>>();

    let torrents = sycli::get_torrents()?;
    let files = sycli::get_files()?;

    eprintln!("Got {} torrents and {} files", torrents.len(), files.len());

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
        .filter_map(|file| match torrents.get(&file.torrent_id) {
            Some(torrent) => {
                if torrent.files == 1 {
                    if torrent.size == file.size {
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
                    } else {
                        eprintln!(
                            "got file {:?} with torrent {:?} but size does not match!",
                            file, torrent
                        );
                        None
                    }
                } else {
                    None
                }
            }
            None => {
                eprintln!("got file {:?} with no matching torrent!", file);
                None
            }
        })
        .collect::<HashMap<_, _>>();

    eprintln!("Got {files:?} to work on");

    let re =
        Regex::new(r"^(?<header>.+?\.S[0-9][0-9])E[0-9][0-9]\..+?\.(?<trailer>(?:720|1080|2160)p\..+?\.WEB-DL.+)\.mkv").unwrap();

    for (torrent_id, file) in files {
        let Some(file_name) = file.path.file_name().and_then(&OsStr::to_str) else {
            eprintln!("missing or invalid filename for {:?}; skipping", file);
            continue;
        };

        let Some(captures) = re.captures(&file_name) else {
            eprintln!("unable to extract anything useful from {}", file_name);
            continue;
        };

        let dir_name = [
            captures.name("header").unwrap().as_str(),
            captures.name("trailer").unwrap().as_str(),
        ]
        .join(".");

        let dir_path = args.base_dir.join(dir_name);

        if args.dry_run {
            eprintln!("Making directory {}", dir_path.display());
            eprintln!(
                "Hardlinking {} to {}",
                file.path.display(),
                dir_path.join(file_name).display()
            );
            eprintln!(
                "Updating torrent {} path to {}",
                torrent_id,
                dir_path.display()
            );
            eprintln!("Unlinking old path {}", file.path.display());
        } else {
            std::fs::create_dir(&dir_path).or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
            std::fs::hard_link(&file.path, dir_path.join(file_name))?;
            sycli::move_torrent(&torrent_id, &dir_path)?;
            std::fs::remove_file(&file.path)?;
        }
    }

    Ok(())
}
