use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CollectFilesError {
    #[error("WalkDir failed: {0:?}")]
    WalkDir(#[from] walkdir::Error),
    #[error("non-file path {1} encountered in {0}")]
    NonFilePath(PathBuf, PathBuf),
    #[error("duplicate entry for {0}")]
    DuplicateEntry(PathBuf),
    #[error("no files")]
    NoFiles,
}

/// Walks `path` and returns a `HashMap` of file paths to file sizes in that directory tree. Any
/// subdirectories are not included in the returned map.
///
/// If `path` is a file, returns a map with a single entry of `path` and its size.
/// If `path` contains any non-directory and non-file entries, returns an error.
pub fn collect_files(path: &Path) -> Result<HashMap<PathBuf, u64>, CollectFilesError> {
    type Error = CollectFilesError;

    let mut files = HashMap::new();
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;

        if entry.file_type().is_dir() {
            // Do not include directories in the result, as torrents only contain files.
            continue;
        }

        // Give up if anything other than a file or directory is encountered. Directories are
        // normal and expected (though completely ignored by the torrent format), while
        // anything else is unexpected and probably needs the user to decide what to do.
        if !entry.file_type().is_file() {
            return Err(Error::NonFilePath(
                path.to_path_buf(),
                entry.path().to_path_buf(),
            ));
        }

        // TODO: Perhaps this should just panic?
        if let Some(_old_value) = files.insert(entry.path().into(), entry.metadata()?.len()) {
            return Err(Error::DuplicateEntry(entry.path().to_path_buf()));
        }
    }
    if files.is_empty() {
        Err(Error::NoFiles)
    } else {
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_files_empty_dir() {
        let tmp_dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            collect_files(&tmp_dir.path()),
            Err(CollectFilesError::NoFiles)
        ));
    }

    #[test]
    fn collect_files_with_file() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("test_file");
        std::fs::write(&test_file, "").expect("failed to create test file");

        assert!(collect_files(&test_file).is_ok_and(|files| { files.contains_key(&test_file) }));
    }

    #[test]
    fn collect_files_with_dir() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("test_file");
        std::fs::write(&test_file, "").expect("failed to create test file");

        let files = collect_files(tmp_dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains_key(&test_file));
    }

    #[test]
    fn collect_files_with_symlink_fails() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let test_file = tmp_dir.path().join("test_file");
        std::fs::write(&test_file, "").expect("failed to create test file");
        let test_symlink = tmp_dir.path().join("test_symlink");
        std::os::unix::fs::symlink(test_file, test_symlink).expect("failed to create test symlink");

        assert!(matches!(
            collect_files(tmp_dir.path()),
            Err(CollectFilesError::NonFilePath(_, _))
        ));
    }
}
