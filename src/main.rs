mod config;
mod downloader;
mod launch_chain;
mod marker;
mod prefix;
mod process;
mod steamrt;

use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    // Fetch and install the Steam Linux Runtime if there's no valid marker.
    downloader::ensure_steamrt(&cfg.steamrt_dir)?;

    prefix::ensure_prefix(&cfg)?;

    // "waitforexitandrun" blocks until the program exits.
    let status = launch_chain::build(&cfg, "waitforexitandrun").status()?;
    println!("exited with: {status}");

    Ok(())
}