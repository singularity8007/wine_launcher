mod backend;
mod config;
mod downloader;
mod gui;
mod installer;
mod launch_chain;
mod prefix;
mod progress;
mod sophon;

use backend::Backend;
use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    // No update checks and no launch here: just open the window. Everything
    // else happens when a button is pressed.
    gui::run(Backend::new(cfg))?;
    Ok(())
}