use crate::marker;
use anyhow::Result;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tar::Archive;
use xz2::read::XzDecoder;

/// Extract SteamLinuxRuntime_4.tar.xz into `dest`, dropping the top-level
/// `SteamLinuxRuntime_4/` folder so `_v2-entry-point` lands at
/// `dest/_v2-entry-point`. Any existing `dest` is wiped first, and the
/// marker is written last so only a fully extracted install carries one.
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

    // Last step, on purpose.
    marker::write(dest)
}