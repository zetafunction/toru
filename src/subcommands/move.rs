use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct MoveArgs {
    /// Source file or directory to move.
    source: PathBuf,

    /// Destination directory.
    target: PathBuf,
}

impl MoveArgs {
    pub fn exec(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
