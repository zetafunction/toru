use anyhow::anyhow;
use clap::Args;
use dialoguer::Confirm;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Args)]
pub struct MakeEpisodeLinksArgs {
    /// Files to create episode links for.
    files: Vec<PathBuf>,

    /// IMDB API for the title, e.g. tt0245429.
    #[arg(long)]
    imdb_id: String,

    #[arg(long)]
    season: u32,
}

#[derive(Debug, Deserialize)]
struct OMDbResult {
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "Year")]
    year: String,
}

impl OMDbResult {
    fn get_name(&self, season: u32, episode: u32, extension: &str) -> String {
        // If there are multiple years, – delimits the first year. But for now, it's not used at
        // all.
        format!(
            "{}.S{season:02}E{episode:02}.{extension}",
            self.title.replace(' ', ".")
        )
    }
}

impl MakeEpisodeLinksArgs {
    pub fn exec(self) -> anyhow::Result<()> {
        let (first_path, paths) = self
            .files
            .split_first()
            .ok_or_else(|| anyhow!("no files provided"))?;
        let expected_parent = first_path
            .parent()
            .ok_or_else(|| anyhow!("{} has no parent", first_path.display()))?;
        let expected_extension = first_path
            .extension()
            .ok_or_else(|| anyhow!("{} has no extension", first_path.display()))?
            .to_str()
            .ok_or_else(|| anyhow!("{} has a non-UTF8 extension", first_path.display()))?;
        check_matching_parent_and_extension(paths, expected_parent, expected_extension)?;

        let body = ureq::get("http://www.omdbapi.com/")
            .query("i", &self.imdb_id)
            .query(
                "apikey",
                crate::config::config()
                    .api_keys
                    .omdb
                    .as_ref()
                    .ok_or_else(|| anyhow!("No OMDb API key!"))?,
            )
            .call()?
            .body_mut()
            .read_to_string()?;
        let result: OMDbResult = serde_json::from_str(&body)?;

        let parent = expected_parent.canonicalize()?;

        let mut sorted_files: Vec<_> = self
            .files
            .iter()
            .map(|file| {
                file.file_name()
                    .map(Path::new)
                    .ok_or_else(|| anyhow!("{} has no file name", file.display()))
            })
            .collect::<Result<_, _>>()?;
        sorted_files.sort();
        let sorted_files = sorted_files;

        eprintln!("Creating the following links in {}:", parent.display());
        for (episode, file) in (1..).zip(&sorted_files) {
            eprintln!(
                "  {} => {}",
                result.get_name(self.season, episode, expected_extension),
                file.display()
            );
        }

        if !Confirm::new()
            .with_prompt("Continue?")
            .default(false)
            .interact()?
        {
            return Ok(());
        }

        std::env::set_current_dir(parent)?;
        for (episode, file) in (1..).zip(&sorted_files) {
            std::os::unix::fs::symlink(
                file,
                result.get_name(self.season, episode, expected_extension),
            )?;
        }

        Ok(())
    }
}

#[derive(Debug, Error, PartialEq)]
enum CheckMatchingParentAndExtensionError {
    #[error("mismatched parents: {actual} does not have expected parent {expected:?}")]
    MismatchedParents { actual: PathBuf, expected: PathBuf },
    #[error("mismatched extensions: {actual} does not have expected extension {expected}")]
    MismatchedExtensions { actual: PathBuf, expected: String },
}

fn check_matching_parent_and_extension(
    paths: &[PathBuf],
    expected_parent: &Path,
    expected_extension: &str,
) -> Result<(), CheckMatchingParentAndExtensionError> {
    type Error = CheckMatchingParentAndExtensionError;

    for path in paths {
        if path.parent() != Some(expected_parent) {
            return Err(Error::MismatchedParents {
                actual: path.clone(),
                expected: expected_parent.to_owned(),
            });
        }
        match path.extension() {
            Some(extension) if extension == expected_extension => Ok(()),
            _ => Err(Error::MismatchedExtensions {
                actual: path.clone(),
                expected: expected_extension.to_owned(),
            }),
        }?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_matching_parent_and_extension_no_paths() {
        assert_eq!(
            check_matching_parent_and_extension(&[], Path::new(""), "mkv".into()),
            Ok(())
        );
        assert_eq!(
            check_matching_parent_and_extension(&[], Path::new("test"), "mkv".into()),
            Ok(())
        );
    }

    #[test]
    fn check_matching_parent_and_extension_ok() {
        assert_eq!(
            check_matching_parent_and_extension(&["test.mkv".into()], Path::new(""), "mkv".into()),
            Ok(())
        );
        assert_eq!(
            check_matching_parent_and_extension(
                &["test/test.mkv".into()],
                Path::new("test"),
                "mkv".into()
            ),
            Ok(())
        );
    }

    #[test]
    fn check_matching_parent_and_extension_mismatched_parent() {
        assert_eq!(
            check_matching_parent_and_extension(
                &["test.mkv".into()],
                Path::new("test"),
                "mkv".into()
            ),
            Err(CheckMatchingParentAndExtensionError::MismatchedParents {
                actual: "test.mkv".into(),
                expected: "test".into(),
            })
        );
        assert_eq!(
            check_matching_parent_and_extension(
                &["test/test.mkv".into()],
                Path::new(""),
                "mkv".into()
            ),
            Err(CheckMatchingParentAndExtensionError::MismatchedParents {
                actual: "test/test.mkv".into(),
                expected: "".into(),
            })
        );
    }

    #[test]
    fn check_matching_parent_and_extension_mismatched_extensions() {
        assert_eq!(
            check_matching_parent_and_extension(&["test.mp4".into()], Path::new(""), "mkv".into()),
            Err(CheckMatchingParentAndExtensionError::MismatchedExtensions {
                actual: "test.mp4".into(),
                expected: "mkv".into(),
            })
        );
        assert_eq!(
            check_matching_parent_and_extension(
                &["test/test.mp4".into()],
                Path::new("test"),
                "mkv".into()
            ),
            Err(CheckMatchingParentAndExtensionError::MismatchedExtensions {
                actual: "test/test.mp4".into(),
                expected: "mkv".into(),
            })
        );
    }
}
