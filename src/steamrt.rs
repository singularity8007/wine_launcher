use crate::marker;
use anyhow::Result;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tar::Archive;
use xz2::read::XzDecoder;

/// What the marker says is installed. DECISION (yours): a hardcoded label
/// for now. Once the downloader verifies the tarball, this becomes its
/// SHA-256, so a new steamrt4 release triggers a clean reinstall by itself.
const STEAMRT_ID: &str = "steamrt4-4.0.20260914.260627";

/// True if `dest` holds a finished install of exactly STEAMRT_ID.
/// (Checking the marker fixes the old weak check, where one file existing
/// didn't prove a killed extraction had finished.)
pub fn is_extracted(dest: &Path) -> bool {
    marker::is_valid(dest, STEAMRT_ID)
}

/// Extract SteamLinuxRuntime_4.tar.xz into `dest`, dropping the top-level
/// `SteamLinuxRuntime_4/` folder so `_v2-entry-point` lands at
/// `dest/_v2-entry-point`. Any existing `dest` is wiped first.
pub fn extract_steamrt(archive_path: &Path, dest: &Path) -> Result<()> {
    // Start clean: leftovers from a half-finished or tampered install must
    // not survive. Safe because `dest` is a folder the launcher owns, and
    // the tarball sits next to it, not inside it.
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }

    // File -> XzDecoder -> Archive. The decoder only decompresses the bytes
    // tar asks for, so the full 157 MB never sits on disk uncompressed.
    let decoder = XzDecoder::new(File::open(archive_path)?);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;

        // Drop the first path component (the archive's top-level folder).
        let stripped: PathBuf = entry.path()?.components().skip(1).collect();

        // The top folder itself strips down to nothing, so skip it.
        if stripped.as_os_str().is_empty() {
            continue;
        }

        let out = dest.join(&stripped);

        // entry.unpack() doesn't create missing parent folders.
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }

        entry.unpack(&out)?;
    }

    // Last step, on purpose: only a fully extracted install gets a marker.
    marker::write(dest, STEAMRT_ID)
}

/*use anyhow::Result;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tar::Archive;
use xz2::read::XzDecoder;

/// Extract SteamLinuxRuntime_4.tar.xz into `dest`, dropping the top-level
/// `SteamLinuxRuntime_4/` folder so `_v2-entry-point` lands at `dest/_v2-entry-point`.
pub fn extract_steamrt(archive_path: &Path, dest: &Path) -> Result<()> {
    // Three wrappers, each reading from the one inside it:
    // File -> XzDecoder -> Archive.
    // The decoder only decompresses the bytes tar asks for, so the full
    // 157 MB never has to sit on disk uncompressed.
    let file = File::open(archive_path)?;
    let decoder = XzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;

        // Path of this entry inside the archive,
        // e.g. SteamLinuxRuntime_4/_v2-entry-point
        let path = entry.path()?.into_owned();

        // Drop the first component (the top-level folder).
        let stripped: PathBuf = path.components().skip(1).collect();

        // The top folder itself strips down to nothing, so skip it.
        if stripped.as_os_str().is_empty() {
            continue;
        }

        let out = dest.join(&stripped);

        // entry.unpack() doesn't create missing parent folders. Without this,
        // a file whose folder hasn't been made yet fails with "No such file".
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the entry to its final path (file, folder, or symlink).
        entry.unpack(&out)?;
    }

    Ok(())
}

/// True if `dest/_v2-entry-point` exists.
pub fn is_extracted(dest: &Path) -> bool {
    // Weak check: a run killed halfway could leave this file behind with the
    // rest missing. Fine for the rough draft.
    dest.join("_v2-entry-point").exists()
}*/