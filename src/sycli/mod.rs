use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct RawTorrent {
    id: String,
    name: String,
    path: PathBuf,
    progress: f64,
    tracker_urls: Vec<String>,
    size: usize,
    files: usize,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    id: String,
    torrent_id: String,
    path: PathBuf,
    size: usize,
}

#[derive(Debug)]
pub struct Torrent {
    // TODO: Consider representing this as the bytes of the infohash instead.
    pub id: String,
    pub name: String,
    pub base_path: PathBuf,
    pub progress: f64,
    pub tracker_urls: Vec<String>,
    pub size: usize,
    pub files: HashMap<PathBuf, usize>,
}

fn get_raw_torrents() -> Result<Vec<RawTorrent>> {
    let output = Command::new("sycli")
        .args(["list", "-k", "torrent", "-o", "json"])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(serde_json::from_str(&String::from_utf8(output.stdout)?)?)
}

fn get_raw_files() -> Result<Vec<RawFile>> {
    let output = Command::new("sycli")
        .args(["list", "-k", "file", "-o", "json"])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(serde_json::from_str(&String::from_utf8(output.stdout)?)?)
}

pub fn get_torrents() -> Result<Vec<Torrent>> {
    let raw_torrents = get_raw_torrents()?;
    let raw_files = get_raw_files()?;

    let mut torrents = raw_torrents
        .into_iter()
        .map(|t| {
            (
                t.id.clone(),
                (
                    Torrent {
                        id: t.id,
                        name: t.name,
                        base_path: t.path,
                        progress: t.progress,
                        tracker_urls: t.tracker_urls,
                        size: t.size,
                        files: HashMap::new(),
                    },
                    t.files,
                ),
            )
        })
        .collect::<HashMap<_, _>>();

    // TODO: Consider implementing detection of single-file torrents here by checking if the
    // torrent name and the file path are equal.
    for f in raw_files {
        let (torrent, _files_count) = torrents
            .get_mut(&f.torrent_id)
            .ok_or_else(|| anyhow!("{:?} has no matching torrent", f))?;
        if let Some(_old_value) = torrent.files.insert(f.path.clone(), f.size) {
            bail!(
                "{:?} has multiple entries for the same path: {}",
                torrent,
                f.path.display()
            );
        }
    }

    torrents
        .into_iter()
        .map(|(_, (torrent, files_count))| {
            if torrent.files.len() != files_count {
                bail!(
                    "torrent {:?}: got {} files but expected {}",
                    torrent,
                    torrent.files.len(),
                    files_count
                );
            }
            let file_sizes: usize = torrent.files.values().sum();
            if file_sizes != torrent.size {
                bail!(
                    "torrent {:?}: files total {} bytes but expected {}",
                    torrent,
                    file_sizes,
                    torrent.size
                );
            }
            Ok(torrent)
        })
        .collect()
}

pub fn pause_torrent(torrent_id: &str) -> Result<()> {
    let output = Command::new("sycli").args(["pause", torrent_id]).output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub fn resume_torrent(torrent_id: &str) -> Result<()> {
    let output = Command::new("sycli")
        .args(["resume", torrent_id])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub fn move_torrent(torrent_id: &str, dir_path: &Path) -> Result<()> {
    let output = Command::new("sycli")
        .args([
            "torrent",
            torrent_id,
            "move",
            "--skip-files",
            dir_path
                .to_str()
                .ok_or_else(|| anyhow!("move_torrent cannot handle non-UTF8 paths"))?,
        ])
        .output()?;

    if !output.status.success() {
        bail!(
            "sycli finished with non-zero status: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_deserialize() {
        let json = r#"
          {
            "type": "torrent",
            "id": "1234567890123456789012345678901234567890",
            "name": "data.txt",
            "path": "/tmp",
            "progress": 0.25,
            "tracker_urls": [
              "example.com"
            ],
            "size": 88888888,
            "files": 1,
            "unknown_field": "is_ignored"
          }
        "#;
        let t: RawTorrent = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "1234567890123456789012345678901234567890");
        assert_eq!(t.name, "data.txt",);
        assert_eq!(t.path, Path::new("/tmp"));
        assert_eq!(t.progress, 0.25);
        assert_eq!(t.tracker_urls, &["example.com"]);
        assert_eq!(t.size, 88888888);
        assert_eq!(t.files, 1);
    }

    #[test]
    fn file_deserialize() {
        let json = r#"
          {
            "type": "file",
            "id": "0123456789012345678901234567890123456789",
            "torrent_id": "1234567890123456789012345678901234567890",
            "path": "data.txt",
            "size": 88888888,
            "unknown_field": "is_ignored"
          }
        "#;
        let f: RawFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.id, "0123456789012345678901234567890123456789");
        assert_eq!(f.torrent_id, "1234567890123456789012345678901234567890");
        assert_eq!(f.path, Path::new("data.txt"));
        assert_eq!(f.size, 88888888);
    }
}
