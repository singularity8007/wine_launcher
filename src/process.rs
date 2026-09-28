use crate::config::Config;
use std::io;
use std::process::{Command, ExitStatus};

/// Launch the exe straight through Wine (no SLR yet), and wait for it to exit.
pub fn launch_direct(cfg: &Config) -> io::Result<ExitStatus> {
    Command::new(cfg.wine_bin())
        .arg(&cfg.exe)
        .env("WINEPREFIX", cfg.wine_prefix())
        .status()
}
