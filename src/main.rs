mod config;
mod prefix;
mod process;

use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::hardcoded()?;

    prefix::ensure_prefix(&cfg)?;
    let status = process::launch_direct(&cfg)?;
    println!("exited with: {status}");

    Ok(())
}
