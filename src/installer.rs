use crate::progress::Progress;
use crate::sophon::{self, Plan};
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use nix::sys::statvfs::statvfs;
use prost::Message;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

// DECISION (yours): parallel chunk downloads. Chunks are ~1 MB, so more
// workers hide network latency; raise it if your connection isn't saturated.
const WORKERS: usize = 8;
const MB: u64 = 1024 * 1024;

/// Written into the game folder once an install or update has fully finished.
/// Holds the version it finished at.
const RECORD_NAME: &str = ".installed_version";

/// Also written at the end: every file the manifests listed, with its md5
/// (`<md5>\t<name>` per line). The next update uses it to skip files that
/// didn't change and to delete files the new version dropped.
const LIST_NAME: &str = ".installed_files";

// ---- Manifest format (protobuf). Field numbers come from HoYoverse's format. ----

#[derive(Clone, PartialEq, Message)]
struct Manifest {
    #[prost(message, repeated, tag = "1")]
    files: Vec<ManifestFile>,
}

#[derive(Clone, PartialEq, Message)]
struct ManifestFile {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, repeated, tag = "2")]
    chunks: Vec<Chunk>,
    #[prost(bool, tag = "3")]
    is_folder: bool,
    #[prost(uint64, tag = "4")]
    size: u64,
    #[prost(string, tag = "5")]
    md5: String,
}

#[derive(Clone, PartialEq, Message)]
struct Chunk {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(string, tag = "2")]
    uncompressed_md5: String,
    #[prost(uint64, tag = "3")]
    offset: u64,
    #[prost(uint64, tag = "4")]
    compressed_size: u64,
    #[prost(uint64, tag = "5")]
    uncompressed_size: u64,
}

/// One chunk to fetch and where it goes.
struct Task {
    url: String,
    file: String, // the game file's name from the manifest, used in error messages
    path: PathBuf,
    offset: u64,
    size: u64,   // uncompressed
    md5: String, // of the uncompressed data
    /// The file already existed (an update or an interrupted install), so
    /// this chunk may already be right on disk. Check before downloading.
    verify: bool,
}

/// file name -> lowercase md5 (empty if the manifest gave none).
type FileList = HashMap<String, String>;

/// How the installed game compares with the latest published version.
pub enum GameStatus {
    NotInstalled,
    UpToDate,
    UpdateAvailable,
}

/// True if an install or update has fully finished in `dest`. An update that
/// was interrupted removes the record, so this is false until it completes.
pub fn is_complete(dest: &Path) -> bool {
    dest.join(RECORD_NAME).exists()
}

/// Ask HoYoverse for the latest version and compare. Downloads and changes nothing.
pub fn check(dest: &Path, progress: &Progress) -> Result<GameStatus> {
    progress.set_label("Checking the ZZZ version...");
    let id = version_id(&sophon::plan()?);
    Ok(match fs::read_to_string(dest.join(RECORD_NAME)) {
        Ok(found) if found == id => GameStatus::UpToDate,
        Ok(_) => GameStatus::UpdateAvailable,
        Err(_) => GameStatus::NotInstalled,
    })
}

/// Make `dest` match the latest ZZZ: a fresh install, an update, or the rest
/// of an interrupted one. Only files that changed are downloaded; the only
/// files deleted are ones the previous version listed and the new one dropped.
pub fn ensure_game(dest: &Path, progress: &Progress) -> Result<()> {
    progress.set_label("Checking the ZZZ version...");
    let plan = match sophon::plan() {
        Ok(plan) => plan,
        // Offline, but a finished install exists: warn and carry on.
        Err(_) if is_complete(dest) => {
            progress.note(
                "Couldn't reach HoYo's servers to look for a new ZZZ version, \
                 so your installed copy was left as it is.",
            );
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    sync(&plan, dest, progress)
}

/// What the record file holds. Manifest ids embed a checksum, so this changes
/// whenever any piece of the game changes.
fn version_id(plan: &Plan) -> String {
    let ids: Vec<&str> = plan
        .items
        .iter()
        .filter_map(|i| i.manifest_url.rsplit('/').next())
        .collect();
    format!("{} {}", plan.tag, ids.join(","))
}

fn sync(plan: &Plan, dest: &Path, progress: &Progress) -> Result<()> {
    let id = version_id(plan);
    let had_record = is_complete(dest);
    let up_to_date = fs::read_to_string(dest.join(RECORD_NAME)).is_ok_and(|found| found == id);
    if up_to_date && dest.join(LIST_NAME).exists() {
        return Ok(());
    }

    let agent = crate::downloader::build_agent();

    // Step 1: read every manifest before touching a single file, so a network
    // failure here leaves the install exactly as it was.
    let mut manifests = Vec::new();
    for item in &plan.items {
        progress.set_label(format!("Reading manifest: {}", item.name));
        manifests.push((item, fetch_manifest(&agent, &item.manifest_url)?));
    }

    // The manifest is remote data: never let it write outside `dest`.
    // Also build the new file list while we're going through it.
    let mut new_list = FileList::new();
    for (_, manifest) in &manifests {
        for file in &manifest.files {
            check_name(&file.name)?;
            if !file.is_folder {
                new_list.insert(file.name.clone(), file.md5.to_lowercase());
            }
        }
    }

    // Installed by an earlier version of the launcher, which kept no file
    // list: the game is already complete, so just record the list now.
    if up_to_date {
        write_list(dest, &new_list)?;
        return Ok(());
    }

    // What the previous finished sync left behind (empty for a fresh install).
    let old = read_list(dest);

    // From here on files change. Remove the record so an interruption leaves
    // the game marked unfinished (and the launcher won't start a half-updated game).
    fs::create_dir_all(dest)?;
    let _ = fs::remove_file(dest.join(RECORD_NAME));

    // Step 2: create folders and size every file that needs work; list the
    // chunks that might need fetching.
    let mut tasks: Vec<Task> = Vec::new();
    let mut needed = 0u64; // extra bytes of disk this will need
    for (item, manifest) in &manifests {
        for file in &manifest.files {
            let path = dest.join(&file.name);
            if file.is_folder {
                fs::create_dir_all(&path)?;
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            let md5 = file.md5.to_lowercase();
            let meta = fs::metadata(&path).ok();
            let existing_len = meta.as_ref().map(|m| m.len());
            // blocks() counts the 512-byte units really allocated, so the
            // holes in a half-finished file don't count as used space.
            let allocated = meta.as_ref().map_or(0, |m| m.blocks() * 512);

            // Same file as the last finished sync: leave it alone, don't
            // even read it. This is what keeps an update small.
            let unchanged = existing_len == Some(file.size)
                && !md5.is_empty()
                && old.get(&file.name).is_some_and(|previous| *previous == md5);
            if unchanged {
                continue;
            }

            // Create the file if it's missing, but never truncate an existing
            // one: it may hold chunks we can keep. Giving it its final size
            // lets chunks be written anywhere, in any order.
            let f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .with_context(|| format!("opening {}", path.display()))?;
            f.set_len(file.size)?;
            needed += file.size.saturating_sub(allocated);

            for chunk in &file.chunks {
                tasks.push(Task {
                    url: format!("{}/{}", item.chunk_url_prefix, chunk.id),
                    file: file.name.clone(),
                    path: path.clone(),
                    offset: chunk.offset,
                    size: chunk.uncompressed_size,
                    md5: chunk.uncompressed_md5.clone(),
                    verify: existing_len.is_some(),
                });
            }
        }
    }

    // Step 3: make sure what's still missing fits, before downloading anything.
    let gb = |bytes: u64| bytes as f64 / 1e9;
    let free = free_space(dest)?;
    if free < needed {
        bail!(
            "not enough disk space in {}: need {:.1} GB more, only {:.1} GB free",
            dest.display(),
            gb(needed),
            gb(free)
        );
    }

    // Step 4: download on a pool of threads. Each worker repeatedly claims the
    // next unclaimed task by bumping a shared counter. A chunk that's already
    // on disk and correct is skipped; it still counts as done so the bar is accurate.
    if had_record || !old.is_empty() {
        progress.set_label(format!("Updating ZZZ to {}...", plan.tag));
    } else {
        progress.set_label(format!(
            "Installing ZZZ {} ({:.1} GB to download, {:.1} GB on disk)",
            plan.tag,
            gb(plan.items.iter().map(|i| i.compressed).sum()),
            gb(plan.items.iter().map(|i| i.uncompressed).sum()),
        ));
    }
    progress.set_total(tasks.iter().map(|t| t.size).sum());
    let next = AtomicUsize::new(0);
    let failed: Mutex<Option<anyhow::Error>> = Mutex::new(None);

    // thread::scope waits for every worker before it returns.
    thread::scope(|s| {
        let work = || loop {
            // Stop as soon as any worker has hit a fatal error.
            if failed.lock().unwrap().is_some() {
                break;
            }
            let Some(task) = tasks.get(next.fetch_add(1, Ordering::Relaxed)) else {
                break; // no tasks left
            };
            if task.verify && chunk_present(task) {
                progress.add(task.size);
                continue;
            }
            // One attempt, no retries: a failure stops the whole job. Running
            // it again picks up where it stopped, because finished chunks are skipped.
            match fetch_chunk(&agent, task) {
                Ok(()) => progress.add(task.size),
                Err(e) => {
                    failed.lock().unwrap().get_or_insert(e.context(format!(
                        "chunk failed in {} ({})",
                        task.file, task.url
                    )));
                    break;
                }
            }
        };
        for _ in 0..WORKERS {
            s.spawn(work);
        }
    });

    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e.context("stopped; press Install game again to continue where it left off"));
    }

    // Step 5: delete files the old version had and the new one dropped. Only
    // names from our own list are ever touched: never the game's saves, logs
    // or anything else it wrote itself.
    let mut removed = 0;
    for name in old.keys() {
        if new_list.contains_key(name) || check_name(name).is_err() {
            continue;
        }
        let path = dest.join(name);
        if fs::remove_file(&path).is_ok() {
            removed += 1;
            remove_empty_parents(&path, dest);
        }
    }
    if removed > 0 {
        progress.note(format!("Removed {removed} files the new version no longer uses."));
    }

    // Last, on purpose: only a finished sync gets a record.
    write_list(dest, &new_list)?;
    fs::write(dest.join(RECORD_NAME), &id)?;
    Ok(())
}

/// Reject a manifest path that could escape the game folder.
fn check_name(name: &str) -> Result<()> {
    if name.starts_with('/') || name.split('/').any(|part| part == "..") {
        bail!("unsafe path in manifest: {name}");
    }
    Ok(())
}

/// Read the list the last finished sync wrote (empty if there is none).
fn read_list(dest: &Path) -> FileList {
    let Ok(text) = fs::read_to_string(dest.join(LIST_NAME)) else {
        return FileList::new();
    };
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(md5, name)| (name.to_string(), md5.to_string()))
        .collect()
}

/// Write the list through a temp file so it is never half-written.
fn write_list(dest: &Path, list: &FileList) -> Result<()> {
    let mut entries: Vec<_> = list.iter().collect();
    entries.sort();
    let text: String = entries
        .iter()
        .map(|(name, md5)| format!("{md5}\t{name}\n"))
        .collect();
    let temp = dest.join(format!("{LIST_NAME}.tmp"));
    fs::write(&temp, text)?;
    fs::rename(&temp, dest.join(LIST_NAME))?;
    Ok(())
}

/// After deleting a file, remove the folders it leaves empty (never `stop` itself).
fn remove_empty_parents(file: &Path, stop: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == stop || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Free bytes on the disk that holds `path`, as a normal (non-root) user sees it.
fn free_space(path: &Path) -> Result<u64> {
    let s = statvfs(path).context("could not read the free disk space")?;
    Ok(s.blocks_available() as u64 * s.fragment_size() as u64)
}

/// Lowercase hex md5, the format the manifest uses.
fn md5_hex(data: &[u8]) -> String {
    format!("{:x}", Md5::digest(data))
}

/// True if the file already holds exactly this chunk. Checking the real bytes,
/// not a progress file, means a crash or a half-written chunk can never be
/// mistaken for a finished one.
fn chunk_present(task: &Task) -> bool {
    let mut buf = vec![0u8; task.size as usize];
    File::open(&task.path)
        .and_then(|f| f.read_exact_at(&mut buf, task.offset))
        .is_ok()
        && md5_hex(&buf) == task.md5.to_lowercase()
}

/// Download and decode one manifest (zstd-compressed protobuf).
fn fetch_manifest(agent: &ureq::Agent, url: &str) -> Result<Manifest> {
    let compressed = agent
        .get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(256 * MB)
        .read_to_vec()?;
    let raw = zstd::stream::decode_all(&compressed[..])?;
    Ok(Manifest::decode(raw.as_slice())?)
}

/// Fetch, decompress, verify and write one chunk.
fn fetch_chunk(agent: &ureq::Agent, task: &Task) -> Result<()> {
    let compressed = agent
        .get(task.url.as_str())
        .call()?
        .body_mut()
        .with_config()
        .limit(64 * MB)
        .read_to_vec()?;

    // The exact decompressed size is known, so it doubles as a sanity limit.
    let data = zstd::bulk::decompress(&compressed, task.size as usize)?;
    if data.len() as u64 != task.size {
        bail!("wrong size: got {}, expected {}", data.len(), task.size);
    }
    let md5 = md5_hex(&data);
    if md5 != task.md5.to_lowercase() {
        bail!("checksum mismatch: got {md5}, expected {}", task.md5);
    }

    // Write at the chunk's offset. The file already exists and is pre-sized,
    // and different chunks touch different byte ranges, so threads don't clash.
    OpenOptions::new()
        .write(true)
        .open(&task.path)
        .with_context(|| format!("opening {}", task.path.display()))?
        .write_all_at(&data, task.offset)?;
    Ok(())
}