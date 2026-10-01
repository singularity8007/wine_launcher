use crate::config::Config;
use crate::prefix;
use anyhow::Result;
use std::process::{Command, ExitStatus};

/// True if everything a launch needs is on disk. Local files only, no
/// network, so it's instant and works offline.
pub fn is_ready(cfg: &Config) -> bool {
    [
        cfg.exe.clone(),
        cfg.entry_point(),
        cfg.reaper_path.clone(),
        cfg.runner_dir.join("proton"),
    ]
    .iter()
    .all(|path| path.exists())
}

/// Launch the game and block until it exits.
pub fn run(cfg: &Config) -> Result<ExitStatus> {
    // Create the Wine prefix on the first run; a no-op afterwards.
    prefix::ensure_prefix(cfg)?;
    // "waitforexitandrun" blocks until the program exits.
    Ok(build(cfg, "waitforexitandrun").status()?)
}

/// Build (but don't run) the command that launches `cfg.exe` inside the
/// Steam Linux Runtime, the same way Steam and TwintailLauncher do:
///
///   _v2-entry-point --verb=V -- reaper SteamLaunch AppId=0 -- proton V exe
///
/// `verb` is "run" (fire and forget) or "waitforexitandrun" (block until
/// the game exits).
pub fn build(cfg: &Config, verb: &str) -> Command {
    // Proton keeps its Wine prefix in <data path>/pfx.
    let pfx = cfg.prefix_dir.join("pfx");

    // STEAM_COMPAT_INSTALL_PATH is the game's install folder.
    let install_dir = &cfg.game_dir;

    // The entry script that sets up the container. Everything about
    // bwrap/pressure-vessel stays hidden inside it.
    let mut cmd = Command::new(cfg.steamrt_dir.join("_v2-entry-point"));

    cmd.arg(format!("--verb={verb}"))
        // Everything after this `--` is the program to run inside the container.
        .arg("--")
        // reaper cleans up leftover processes when the game exits. AppId=0
        // because we're not a real Steam game.
        .arg(&cfg.reaper_path)
        .args(["SteamLaunch", "AppId=0", "--"])
        // The runner (the `proton` script) followed by its verb and the exe.
        .arg(cfg.runner_dir.join("proton"))
        .arg(verb)
        .arg(&cfg.exe);

    // Environment that Proton expects Steam to have set up. Without these it
    // refuses to start, since we're not running under a real Steam client.
    cmd.env("WINEARCH", "win64")
        .env("WINEPREFIX", &pfx)
        .env("STEAM_COMPAT_APP_ID", "0")
        .env("STEAM_COMPAT_DATA_PATH", &cfg.prefix_dir)
        .env("STEAM_COMPAT_INSTALL_PATH", install_dir)
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", "")
        .env("STEAM_COMPAT_TOOL_PATHS", &cfg.runner_dir)
        .env(
            "STEAM_COMPAT_LIBRARY_PATHS",
            format!("{}:{}", install_dir.display(), pfx.display()),
        )
        .env("STEAM_COMPAT_SHADER_PATH", cfg.prefix_dir.join("shadercache"))
        // `=d` disables a DLL. Both are Steam-only pieces we don't have.
        .env("WINEDLLOVERRIDES", "lsteamclient=d;KRSDKExternal.exe=d")
        .env("SteamGameId", "0")
        .env("DXVK_HUD", "devinfo,fps,frametimes,gpuload,compiler")
        .env("PROTON_ENABLE_WAYLAND", "1");

    cmd
}