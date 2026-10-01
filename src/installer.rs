use crate::progress::Progress;
use crate::sophon::{self, Plan};
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use nix::sys::statvfs::statvfs;
use prost::Message;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

// DECISION (yours): parallel chunk downloads. Chunks are ~1 MB, so more
// workers hide network latency; raise it if your connection isn't saturated.
const WORKERS: usize = 8;
const MB: u64 = 1024 * 1024;

/// Written into the game folder once an install has fully finished.
const RECORD_NAME: &str = ".installed_version";

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
}

/// Install ZZZ into `dest` if it isn't installed yet. Progress and warnings
/// go to `progress`; the caller decides how to show them.
pub fn ensure_game(dest: &Path, progress: &Progress) -> Result<()> {
    progress.set_label("Checking the ZZZ version...");
    let plan = match sophon::plan() {
        Ok(plan) => plan,
        // Offline, but a finished install exists: warn and carry on.
        Err(e) if dest.join(RECORD_NAME).exists() => {
            progress.note(format!(
                "Could not check ZZZ for updates ({e:#}); using the installed copy."
            ));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    install(&plan, dest, progress)
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

fn install(plan: &Plan, dest: &Path, progress: &Progress) -> Result<()> {
    let id = version_id(plan);
    match fs::read_to_string(dest.join(RECORD_NAME)) {
        Ok(found) if found == id => return Ok(()), // already installed
        // Fresh-install-only: no patching. Never delete 70 GB without being asked.
        Ok(_) => bail!(
            "ZZZ has a new version and updating isn't built yet; delete {} to reinstall",
            dest.display()
        ),
        Err(_) => {}
    }

    // A finished install returned above, so a folder with no record is an
    // install that got interrupted. There's no resume, so it's useless:
    // clear it out and start over from nothing.
    if dest.exists() {
        progress.set_label("Removing an unfinished install...");
        fs::remove_dir_all(dest)
            .with_context(|| format!("could not remove {}", dest.display()))?;
    }
    fs::create_dir_all(dest)?;

    let agent = crate::downloader::build_agent();

    // Step 1: read every manifest, create the folders and pre-sized files, and
    // list all the chunks to fetch.
    let mut tasks: Vec<Task> = Vec::new();
    let mut total_size = 0u64; // bytes the finished game takes up
    for item in &plan.items {
        progress.set_label(format!("Reading manifest: {}", item.name));
        let manifest = fetch_manifest(&agent, &item.manifest_url)?;
        for file in manifest.files {
            // The manifest is remote data: never let it write outside `dest`.
            if file.name.starts_with('/') || file.name.split('/').any(|p| p == "..") {
                bail!("unsafe path in manifest: {}", file.name);
            }
            let path = dest.join(&file.name);
            if file.is_folder {
                fs::create_dir_all(&path)?;
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // Create the file at its final size up front. That lets chunks be
            // written anywhere, in any order; the unwritten parts stay sparse,
            // so they take no disk space yet.
            let f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .with_context(|| format!("creating {}", path.display()))?;
            f.set_len(file.size)?;
            total_size += file.size;
            for chunk in file.chunks {
                tasks.push(Task {
                    url: format!("{}/{}", item.chunk_url_prefix, chunk.id),
                    file: file.name.clone(),
                    path: path.clone(),
                    offset: chunk.offset,
                    size: chunk.uncompressed_size,
                    md5: chunk.uncompressed_md5,
                });
            }
        }
    }

    // Step 2: make sure the whole game fits, before downloading anything.
    let gb = |bytes: u64| bytes as f64 / 1e9;
    let free = free_space(dest)?;
    if free < total_size {
        bail!(
            "not enough disk space in {}: need {:.1} GB, only {:.1} GB free",
            dest.display(),
            gb(total_size),
            gb(free)
        );
    }

    // Step 3: download chunks on a pool of threads. Each worker repeatedly
    // claims the next unclaimed task by bumping a shared counter, and reports
    // each finished chunk straight to `progress` for the window to read.
    progress.set_label(format!(
        "Downloading ZZZ {} ({:.1} GB to download, {:.1} GB on disk)",
        plan.tag,
        gb(plan.items.iter().map(|i| i.compressed).sum()),
        gb(total_size),
    ));
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
            // One attempt, no retries: a failure stops the whole install.
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
        return Err(e.context("download stopped; run Install again to start over"));
    }

    // Last, on purpose: only a finished install gets a record.
    fs::write(dest.join(RECORD_NAME), &id)?;
    Ok(())
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