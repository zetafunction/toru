use anyhow::{bail, Result};
use serde::Deserialize;
use std::process::Command;

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct Torrent {
    id: String,
    name: String,
    path: String,
    tracker_urls: Vec<String>,
    size: usize,
    files: usize,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct File {
    id: String,
    torrent_id: String,
    path: String,
    size: usize,
}

pub fn get_torrents() -> Result<Vec<Torrent>> {
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

pub fn get_files() -> Result<Vec<File>> {
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
            "tracker_urls": [
              "example.com"
            ],
            "size": 88888888,
            "files": 1,
            "unknown_field": "is_ignored"
          }
        "#;
        let t: Torrent = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "1234567890123456789012345678901234567890");
        assert_eq!(t.name, "data.txt",);
        assert_eq!(t.path, "/tmp");
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
        let f: File = serde_json::from_str(json).unwrap();
        assert_eq!(f.id, "0123456789012345678901234567890123456789");
        assert_eq!(f.torrent_id, "1234567890123456789012345678901234567890");
        assert_eq!(f.path, "data.txt");
        assert_eq!(f.size, 88888888);
    }
}
