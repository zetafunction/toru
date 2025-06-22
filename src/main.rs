mod config;
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
    /// Moves a file or directory to a new location.
    Move(subcommands::MoveArgs),
    /// Organizes files for an episode into directories.
    BatchEpisodes(subcommands::BatchEpisodesArgs),
    /// Creates symlinks for a TV scanner to recognize files as episodes.
    MakeEpisodeLinks(subcommands::MakeEpisodeLinksArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Move(args) => args.exec(),
        Commands::BatchEpisodes(args) => args.exec(),
        Commands::MakeEpisodeLinks(args) => args.exec(),
    }
}
