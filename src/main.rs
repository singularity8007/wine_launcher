mod config;
mod downloader;
mod launch_chain;
mod prefix;
mod process;

use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    // First thing at startup: check for updates and (re)install if needed.
    downloader::update_all(&cfg)?;

    prefix::ensure_prefix(&cfg)?;

    // "waitforexitandrun" blocks until the program exits.
    let status = launch_chain::build(&cfg, "waitforexitandrun").status()?;
    println!("exited with: {status}");

    Ok(())
}