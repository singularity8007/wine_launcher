mod backend;
mod config;
mod downloader;
mod gui;
mod installer;
mod launch_chain;
mod playtime;
mod prefix;
mod progress;
mod sophon;

use backend::Backend;
use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    // No update checks and no launch here
    gui::run(Backend::new(cfg))?;
    Ok(())
}