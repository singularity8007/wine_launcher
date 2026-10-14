use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize};
use std::collections::HashSet;

// ZZZ, global server. The game ID and launcher ID identify the game to HoYoverse.
const LAUNCHER_ID: &str = "VYTpXlbWo8";
const GAME_ID: &str = "U5hbdsT9W7";
const BRANCHES_URL: &str = "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameBranches";
const BUILD_URL: &str =
    "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild";

// DECISION (yours): the voice pack to install on top of the base set.
// Options: "en-us", "ja-jp", "ko-kr", "zh-cn". Set to "" to skip voices.
const VOICE: &str = "en-us";

/// One downloadable piece of the game: the base game, a chapter, a voice pack...
#[allow(dead_code)]
pub struct Item {
    pub name: String,
    pub manifest_url: String,
    /// Chunks live at `<chunk_url_prefix>/<chunk name from the manifest>`.
    pub chunk_url_prefix: String,
    pub compressed: u64,
    pub uncompressed: u64,
    pub files: u64,
    pub chunks: u64,
}

/// Everything a fresh install needs.
pub struct Plan {
    pub tag: String,
    pub items: Vec<Item>,
}

// ---- Shapes of the API replies. Only the fields we use; serde ignores the rest. ----

#[derive(Deserialize)]
struct Reply<T> {
    retcode: i64,
    message: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct BranchesData {
    game_branches: Vec<GameBranch>,
}

#[derive(Deserialize)]
struct GameBranch {
    main: Branch,
}

#[derive(Deserialize)]
struct Branch {
    package_id: String,
    branch: String,
    password: String,
    categories: Vec<Category>,
}

#[derive(Deserialize)]
struct Category {
    matching_field: String,
    scenarios: Vec<String>,
}

#[derive(Deserialize)]
struct BuildData {
    tag: String,
    manifests: Vec<BuildManifest>,
}

#[derive(Deserialize)]
struct BuildManifest {
    matching_field: String,
    manifest: ManifestRef,
    manifest_download: Location,
    chunk_download: Location,
    stats: Stats,
}

#[derive(Deserialize)]
struct ManifestRef {
    id: String,
}

#[derive(Deserialize)]
struct Location {
    url_prefix: String,
    encryption: i64,
    compression: i64,
}

// The API sends these numbers as strings.
#[derive(Deserialize)]
struct Stats {
    compressed_size: String,
    uncompressed_size: String,
    file_count: String,
    chunk_count: String,
}

/// Ask HoYoverse for the current ZZZ build and pick what a fresh install needs.
pub fn plan() -> Result<Plan> {
    let agent = crate::downloader::build_agent();

    // Step 1: current branch info: package id, password, and the category list.
    // (`[]` in `game_ids[]` is percent-encoded so the URL parser accepts it.)
    let url = format!("{BRANCHES_URL}?launcher_id={LAUNCHER_ID}&game_ids%5B%5D={GAME_ID}");
    let branches: BranchesData = get_json(&agent, &url)?;
    let main = branches
        .game_branches
        .into_iter()
        .next()
        .context("no game branch in the reply")?
        .main;

    // The API tags the minimal set of categories as CATEGORY_SCENARIO_BASE.
    // That includes the base game itself, which is called "game".
    let mut wanted: HashSet<String> = main
        .categories
        .iter()
        .filter(|c| c.scenarios.iter().any(|s| s == "CATEGORY_SCENARIO_BASE"))
        .map(|c| c.matching_field.clone())
        .collect();
    if !VOICE.is_empty() {
        wanted.insert(VOICE.to_string());
    }
    if !wanted.contains("game") {
        bail!("the base game is missing from the base categories");
    }

    // Step 2: the Sophon build, listing every category's manifest and chunk location.
    let url = format!(
        "{BUILD_URL}?branch={}&package_id={}&password={}",
        main.branch, main.package_id, main.password
    );
    let build: BuildData = get_json(&agent, &url)?;

    let mut items = Vec::new();
    for m in build.manifests {
        if !wanted.contains(&m.matching_field) {
            continue;
        }
        // We only handle plain zstd data. Fail loudly if HoYoverse changes that.
        for loc in [&m.manifest_download, &m.chunk_download] {
            if loc.encryption != 0 || loc.compression != 1 {
                bail!("{}: unsupported encryption/compression", m.matching_field);
            }
        }
        items.push(Item {
            manifest_url: format!("{}/{}", m.manifest_download.url_prefix, m.manifest.id),
            chunk_url_prefix: m.chunk_download.url_prefix,
            compressed: m.stats.compressed_size.parse()?,
            uncompressed: m.stats.uncompressed_size.parse()?,
            files: m.stats.file_count.parse()?,
            chunks: m.stats.chunk_count.parse()?,
            name: m.matching_field,
        });
    }
    if items.len() != wanted.len() {
        bail!("wanted {} pieces but the build lists {}", wanted.len(), items.len());
    }

    Ok(Plan { tag: build.tag, items })
}

/// GET a URL, parse the reply, and return its `data`, or an error if the API refused.
fn get_json<T: DeserializeOwned>(agent: &ureq::Agent, url: &str) -> Result<T> {
    let text = agent.get(url).call()?.body_mut().read_to_string()?;
    let reply: Reply<T> = serde_json::from_str(&text).context("unexpected reply shape")?;
    if reply.retcode != 0 {
        bail!("API error {}: {}", reply.retcode, reply.message);
    }
    reply.data.context("the reply had no data")
}