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
    /// Organizes files for an episode into a directory.
    OrganizeEpisodes(subcommands::OrganizeEpisodesArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::OrganizeEpisodes(args) => args.exec(),
    }
}
