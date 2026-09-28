mod config;
mod prefix;
mod process;
mod steamrt;


use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    let archive = std::path::PathBuf::from(std::env::var("HOME")?)
        .join("wine_launcher/steamrt/SteamLinuxRuntime_4.tar.xz");
    if !steamrt::is_extracted(&cfg.steamrt_dir) {
        steamrt::extract_steamrt(&archive, &cfg.steamrt_dir)?;
    }
    
    prefix::ensure_prefix(&cfg)?;
    let status = process::launch_direct(&cfg)?;
    println!("exited with: {status}");

    Ok(())
}
