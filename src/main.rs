mod config;
mod downloader;
mod installer;
mod launch_chain;
mod prefix;
mod sophon;

use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    // First thing at startup: check for updates and (re)install if needed.
    downloader::update_all(&cfg)?;
    installer::ensure_game(&cfg.game_dir)?;

    prefix::ensure_prefix(&cfg)?;

    // "waitforexitandrun" blocks until the program exits.
    let status = launch_chain::build(&cfg, "waitforexitandrun").status()?;
    println!("exited with: {status}");

    Ok(())
}