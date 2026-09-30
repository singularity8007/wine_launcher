use std::env::{self, VarError};
use std::path::PathBuf;

/// Everything the launcher needs to know. Hardcoded for now (D-006).
#[derive(Debug)]
#[allow(dead_code)]
pub struct Config {
    pub runner_dir: PathBuf,  // GE-Proton folder
    pub steamrt_dir: PathBuf, // extracted SteamLinuxRuntime_4
    pub reaper_path: PathBuf, // vendored reaper binary
    pub prefix_dir: PathBuf,  // folder that holds the pfx/ prefix
    pub exe: String,  // Windows path of the exe to launch
    pub game_dir: PathBuf, // fresh ZZZ install  
        
        
}

impl Config {
    pub fn hardcoded() -> Result<Self, VarError> {
        let root = PathBuf::from(env::var("HOME")?).join("wine_launcher");
        Ok(Self {
            runner_dir: root.join("proton").join("ge-proton"),
            steamrt_dir: root.join("steamrt").join("steamrt4"),
            reaper_path: root.join("bin").join("reaper"),
            prefix_dir: root.join("test").join("prefix"),
            exe: String::from(r"c:\windows\system32\notepad.exe"),
            game_dir: root.join("game"), 
        })
    }

    /// The Wine binary bundled inside GE-Proton.
    pub fn wine_bin(&self) -> PathBuf {
        self.runner_dir.join("files/bin/wine")
    }

    /// SLR's container entry point script.
    #[allow(dead_code)]
    pub fn entry_point(&self) -> PathBuf {
        self.steamrt_dir.join("_v2-entry-point")
    }

    /// The value for the WINEPREFIX environment variable.
    pub fn wine_prefix(&self) -> PathBuf {
        self.prefix_dir.join("pfx")
    }
}
