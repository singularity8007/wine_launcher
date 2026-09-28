use anyhow::Result;
use std::fs;
use std::path::Path;

/// Hidden file the launcher drops into an install folder once the install
/// has fully finished. Missing or wrong content means "incomplete or
/// tampered with", and the folder gets reinstalled.
const MARKER_NAME: &str = ".wine_launcher_ok";
const MARKER_TEXT: &str = "Divine Intellect";

/// Write the marker. Call this LAST, after everything else succeeded, so an
/// install that gets killed halfway never carries a marker.
pub fn write(dir: &Path) -> Result<()> {
    fs::write(dir.join(MARKER_NAME), MARKER_TEXT)?;
    Ok(())
}

/// True only if the marker exists and holds exactly MARKER_TEXT.
pub fn is_valid(dir: &Path) -> bool {
    fs::read_to_string(dir.join(MARKER_NAME)).is_ok_and(|found| found == MARKER_TEXT)
}