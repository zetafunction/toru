mod config;
mod fs;
mod subcommands;
mod sycli;

use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Imports torrent files for cross-seeding, creating symlinks if necessary.
    Import(subcommands::ImportArgs),

    /// Finds the torrents that correspond to a given path.
    Find(subcommands::FindArgs),
    /// Moves a file or directory to a new location.
    Move(subcommands::MoveArgs),
    /// Update paths after files or directories are externally moved.
    UpdatePaths(subcommands::UpdatePathsArgs),

    /// Organizes files for an episode into directories.
    BatchEpisodes(subcommands::BatchEpisodesArgs),
    /// Creates symlinks for a TV scanner to recognize files as episodes.
    MakeEpisodeLinks(subcommands::MakeEpisodeLinksArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Import(args) => args.exec(),
        Commands::Find(args) => args.exec(),
        Commands::Move(args) => args.exec(),
        Commands::UpdatePaths(args) => args.exec(),
        Commands::BatchEpisodes(args) => args.exec(),
        Commands::MakeEpisodeLinks(args) => args.exec(),
    }
}
