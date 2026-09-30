use crate::sophon::{self, Plan};
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use prost::Message;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

// DECISION (yours): parallel chunk downloads. Chunks are ~1 MB, so more
// workers hide network latency; raise it if your connection isn't saturated.
const WORKERS: usize = 8;
const CHUNK_ATTEMPTS: u32 = 3;
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
    path: PathBuf,
    offset: u64,
    size: u64, // uncompressed
    md5: String, // of the uncompressed data
}

/// Install ZZZ into `dest` if it isn't installed yet. Call at startup.
pub fn ensure_game(dest: &Path) -> Result<()> {
    let plan = match sophon::plan() {
        Ok(plan) => plan,
        // Offline, but a finished install exists: warn and carry on.
        Err(e) if dest.join(RECORD_NAME).exists() => {
            eprintln!("could not check ZZZ for updates ({e:#}); using the installed copy");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    install(&plan, dest)
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

fn install(plan: &Plan, dest: &Path) -> Result<()> {
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

    let gb = |bytes: u64| bytes as f64 / 1e9;
    println!(
        "Installing ZZZ {}: {} files in {} chunks, {:.1} GB to download, {:.1} GB on disk",
        plan.tag,
        plan.items.iter().map(|i| i.files).sum::<u64>(),
        plan.items.iter().map(|i| i.chunks).sum::<u64>(),
        gb(plan.items.iter().map(|i| i.compressed).sum()),
        gb(plan.items.iter().map(|i| i.uncompressed).sum()),
    );

    // No record means nothing finished, so any leftovers are a killed install.
    // The folder belongs to the launcher, so start clean.
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;

    let agent = crate::downloader::build_agent();

    // Step 1: read every manifest, create the folders and empty pre-sized
    // files, and list all the chunks that need fetching.
    let mut tasks: Vec<Task> = Vec::new();
    for item in &plan.items {
        println!("Reading manifest: {}", item.name);
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
            // Pre-size the file so chunks can be written anywhere, in any order.
            File::create(&path)?.set_len(file.size)?;
            for chunk in file.chunks {
                tasks.push(Task {
                    url: format!("{}/{}", item.chunk_url_prefix, chunk.id),
                    path: path.clone(),
                    offset: chunk.offset,
                    size: chunk.uncompressed_size,
                    md5: chunk.uncompressed_md5,
                });
            }
        }
    }

    // Step 2: download chunks on a pool of threads. Each worker repeatedly
    // claims the next unclaimed task by bumping a shared counter.
    let total: u64 = tasks.iter().map(|t| t.size).sum();
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let failed: Mutex<Option<anyhow::Error>> = Mutex::new(None);

    thread::scope(|s| {
        let work = || loop {
            // Stop as soon as any worker has hit a fatal error.
            if failed.lock().unwrap().is_some() {
                break;
            }
            let Some(task) = tasks.get(next.fetch_add(1, Ordering::Relaxed)) else {
                break; // no tasks left
            };
            match fetch_chunk(&agent, task) {
                Ok(()) => {
                    done.fetch_add(task.size, Ordering::Relaxed);
                }
                Err(e) => {
                    failed.lock().unwrap().get_or_insert(e);
                    break;
                }
            }
        };
        let handles: Vec<_> = (0..WORKERS).map(|_| s.spawn(work)).collect();

        // This thread just reports progress until the workers finish.
        while handles.iter().any(|h| !h.is_finished()) {
            let d = done.load(Ordering::Relaxed);
            print!("\r{:.1} / {:.1} GB ({}%)   ", gb(d), gb(total), d * 100 / total.max(1));
            std::io::stdout().flush().ok();
            thread::sleep(Duration::from_secs(1));
        }
        println!();
    });

    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e);
    }

    // Last, on purpose: only a finished install gets a record.
    fs::write(dest.join(RECORD_NAME), &id)?;
    println!("ZZZ {} installed", plan.tag);
    Ok(())
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

/// Fetch one chunk, retrying a few times. With ~58,000 chunks, one flaky
/// request shouldn't sink a 70 GB install.
fn fetch_chunk(agent: &ureq::Agent, task: &Task) -> Result<()> {
    let mut last = None;
    for _ in 0..CHUNK_ATTEMPTS {
        match try_chunk(agent, task) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    let e = last.expect("CHUNK_ATTEMPTS is at least 1");
    Err(e.context(format!("chunk failed after {CHUNK_ATTEMPTS} attempts: {}", task.url)))
}

fn try_chunk(agent: &ureq::Agent, task: &Task) -> Result<()> {
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
    let md5 = format!("{:x}", Md5::digest(&data));
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