use std::env::{self, VarError};
use std::path::PathBuf;

/// The game's exe, relative to the install folder.
const GAME_EXE: &str = "ZenlessZoneZero.exe";

/// Everything the launcher needs to know. Hardcoded for now (D-006).
#[derive(Debug)]
#[allow(dead_code)]
pub struct Config {
    pub runner_dir: PathBuf,  // GE-Proton folder
    pub steamrt_dir: PathBuf, // extracted SteamLinuxRuntime_4
    pub reaper_path: PathBuf, // vendored reaper binary
    pub prefix_dir: PathBuf,  // folder that holds the pfx/ prefix
    pub game_dir: PathBuf,    // fresh ZZZ install
    pub exe: PathBuf,         // the exe to launch (Wine accepts a Linux path here)
}

impl Config {
    pub fn hardcoded() -> Result<Self, VarError> {
        let root = PathBuf::from(env::var("HOME")?).join("wine_launcher");
        let game_dir = root.join("game");
        Ok(Self {
            runner_dir: root.join("proton").join("ge-proton"),
            steamrt_dir: root.join("steamrt").join("steamrt4"),
            reaper_path: root.join("bin").join("reaper"),
            prefix_dir: root.join("prefix"),
            exe: game_dir.join(GAME_EXE),
            game_dir,
        })
    }

    /// The Wine binary bundled inside GE-Proton.
    pub fn wine_bin(&self) -> PathBuf {
        self.runner_dir.join("files/bin/wine")
    }

    /// SLR's container entry point script.
    pub fn entry_point(&self) -> PathBuf {
        self.steamrt_dir.join("_v2-entry-point")
    }

    /// The value for the WINEPREFIX environment variable.
    pub fn wine_prefix(&self) -> PathBuf {
        self.prefix_dir.join("pfx")
    }
}