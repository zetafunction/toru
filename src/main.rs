mod sycli;

fn main() -> anyhow::Result<()> {
    let torrents = sycli::get_torrents()?;
    let files = sycli::get_files()?;
    println!("Got {} torrents and {} files", torrents.len(), files.len());
    Ok(())
}
