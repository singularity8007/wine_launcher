use crate::config::Config;
use std::io;
use std::process::Command;

/// Create the Wine prefix if it doesn't exist yet.
pub fn ensure_prefix(cfg: &Config) -> io::Result<()> {
    let pfx = cfg.wine_prefix();

    // wineboot writes system.reg last-ish, so its presence means a finished prefix.
    if pfx.join("system.reg").exists() {
        println!("prefix exists: {}", pfx.display());
        return Ok(());
    }

    println!("creating prefix: {}", pfx.display());
    let status = Command::new(cfg.wine_bin())
        .arg("wineboot")
        .env("WINEPREFIX", &pfx)
        .status()?;

    if !status.success() {
        return Err(io::Error::other(format!("wineboot failed: {status}")));
    }
    Ok(())
}
