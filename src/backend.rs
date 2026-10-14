//! The only thing the window talks to.
//!
//! The GUI calls `launch()`, `install_game()` and `check_updates()` and draws
//! whatever `view()` returns. Everything else (threads, downloads, play time,
//! the wording of messages, deciding whether a launch "failed") lives here or
//! further back, so no logic is duplicated between the two sides.

use crate::config::Config;
use crate::installer::GameStatus;
use crate::playtime::{self, PlayStats};
use crate::progress::Progress;
use crate::{downloader, installer, launch_chain};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A launch that ends non-zero sooner than this counts as "the game didn't
/// start". Longer than this counts as a normal play session.
const EARLY_EXIT: Duration = Duration::from_secs(30);

// ---- Messages shown to the player. Kept gentle: say what probably happened
// and what to try, and keep the technical text for the "details" line. ----

const NOT_READY: &str = "ZZZ isn't ready to play yet. Press \"Install game\" first. \
If you already did and it got interrupted, press it again and it will finish the job.";

const LAUNCH_FAILED: &str = "ZZZ didn't start this time. HoYo's servers may just be slow to answer. \
Check your internet connection, or press \"Check for updates\", then try again.";

const INSTALL_FAILED: &str = "ZZZ didn't finish downloading. Nothing is lost. \
Check your internet connection, then press \"Install game\" again and it will carry on from where it stopped.";

const NO_DISK_SPACE: &str = "There isn't enough free disk space for this yet. \
Free up some space, then press \"Install game\" again.";

const UPDATE_FAILED: &str = "Couldn't check for updates just now. This usually means the connection dropped. \
Check your internet and try again in a moment.";

const CRASHED: &str = "Something unexpected went wrong inside the launcher. \
Please try again; if it keeps happening, the details below can help.";

/// A job that didn't work out: friendly text for the player, plus the
/// technical reason in case they want to share it.
#[derive(Clone)]
pub struct Failure {
    text: String,
    details: Option<String>,
}

impl Failure {
    fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), details: None }
    }

    fn with_details(text: impl Into<String>, details: impl std::fmt::Display) -> Self {
        Self { text: text.into(), details: Some(details.to_string()) }
    }
}

#[derive(Clone)]
enum State {
    Idle,
    Running,
    Done(String),
    Failed(Failure),
}

/// Something to tell the player. Errors are shown in a popup, good news in
/// the status line.
pub struct Notice {
    pub text: String,
    /// The technical reason, shown small and dim under an error.
    pub details: Option<String>,
    pub is_error: bool,
}

/// Everything the window needs to draw one frame, already decided.
pub struct View {
    /// A job is running: the window disables the buttons.
    pub busy: bool,
    /// What the job is doing right now.
    pub label: String,
    /// 0.0 to 1.0, or None when the job has no known size.
    pub fraction: Option<f32>,
    /// Result of the last job, if it has ended.
    pub notice: Option<Notice>,
    /// Total play time, e.g. "94 hours 37 minutes".
    pub play_time: String,
    /// Date of the last launch, e.g. "Oct 1, 2026", or "Never".
    pub last_played: String,
}

/// Cheap to clone: every field is shared, so all clones are the same backend.
#[derive(Clone)]
pub struct Backend {
    cfg: Arc<Config>,
    progress: Arc<Progress>,
    state: Arc<Mutex<State>>,
    stats: Arc<PlayStats>,
}

impl Backend {
    pub fn new(cfg: Config) -> Self {
        let stats = PlayStats::load(cfg.stats_path.clone());
        Self {
            cfg: Arc::new(cfg),
            progress: Arc::new(Progress::default()),
            state: Arc::new(Mutex::new(State::Idle)),
            stats: Arc::new(stats),
        }
    }

    /// Install ZZZ, or bring an existing install up to the latest version.
    /// Only changed files are downloaded.
    pub fn install_game(&self) {
        self.start(|cfg, progress| {
            installer::ensure_game(&cfg.game_dir, progress).map_err(|e| {
                let details = format!("{e:#}");
                let text = if details.contains("not enough disk space") {
                    NO_DISK_SPACE
                } else {
                    INSTALL_FAILED
                };
                Failure::with_details(text, details)
            })?;
            Ok("ZZZ is installed and ready to play.".into())
        });
    }

    /// Update GE-Proton and the Steam Linux Runtime, and say whether ZZZ has
    /// a newer version (without downloading it).
    pub fn check_updates(&self) {
        self.start(|cfg, progress| {
            downloader::update_all(cfg, progress)
                .map_err(|e| Failure::with_details(UPDATE_FAILED, format!("{e:#}")))?;
            let game = match installer::check(&cfg.game_dir, progress) {
                Ok(GameStatus::UpToDate) => "ZZZ is up to date.".to_string(),
                Ok(GameStatus::UpdateAvailable) => "A new ZZZ version is out! Press \
                    \"Install game\" to get it. Only the parts that changed are downloaded."
                    .to_string(),
                Ok(GameStatus::NotInstalled) => {
                    "ZZZ isn't installed yet. Press \"Install game\" to get started."
                        .to_string()
                }
                // Offline or HoYo down: not a failure of the runtime update.
                Err(_) => "Couldn't check ZZZ just now. Try again in a moment.".to_string(),
            };
            Ok(format!(
                "GE-Proton and the Steam Linux Runtime are up to date.\n{game}"
            ))
        });
    }

    /// Just try to run the game: no network, no update checks.
    pub fn launch(&self) {
        let stats = Arc::clone(&self.stats);
        self.start(move |cfg, progress| {
            // Also false while an install or update is unfinished, so a
            // half-updated game is never started.
            if !installer::is_complete(&cfg.game_dir) || !launch_chain::is_ready(cfg) {
                return Err(Failure::plain(NOT_READY));
            }
            progress.set_label("ZZZ is running. Have fun!");
            let started = Instant::now();
            // Counts play time and stamps today's date; stops counting when
            // this goes out of scope, however the job ends.
            let _session = stats.start_session();
            match launch_chain::run(cfg) {
                // Clean exit, or it ran a normal length of time: a normal session.
                Ok(status) if status.success() || started.elapsed() >= EARLY_EXIT => {
                    Ok("Welcome back! ZZZ has closed.".into())
                }
                Ok(status) => Err(Failure::with_details(
                    LAUNCH_FAILED,
                    format!("exit status: {status}"),
                )),
                Err(e) => Err(Failure::with_details(LAUNCH_FAILED, format!("{e:#}"))),
            }
        });
    }

    /// The player closed the error popup.
    pub fn dismiss(&self) {
        let mut state = self.state.lock().unwrap();
        if matches!(*state, State::Failed(_)) {
            *state = State::Idle;
        }
    }

    /// True while a job (including a play session) is running.
    pub fn is_busy(&self) -> bool {
        matches!(*self.state.lock().unwrap(), State::Running)
    }

    /// Called regularly by the window: saves a running play session.
    pub fn tick(&self) {
        self.stats.checkpoint();
    }

    /// Snapshot for the window to draw. Cheap; called every frame.
    pub fn view(&self) -> View {
        let snap = self.progress.snapshot();
        let state = self.state.lock().unwrap().clone();
        View {
            busy: matches!(state, State::Running),
            label: snap.label,
            fraction: (snap.total > 0).then(|| snap.done as f32 / snap.total as f32),
            notice: match state {
                State::Done(text) => Some(Notice { text, details: None, is_error: false }),
                State::Failed(f) => Some(Notice {
                    text: f.text,
                    details: f.details,
                    is_error: true,
                }),
                State::Idle | State::Running => None,
            },
            play_time: playtime::describe(self.stats.total()),
            last_played: self.stats.last_played().unwrap_or_else(|| "Never".into()),
        }
    }

    /// Run `job` on its own thread so the window never freezes. Ignored if a
    /// job is already running. Warnings the job noted are appended to its result.
    fn start<F>(&self, job: F)
    where
        F: FnOnce(&Config, &Progress) -> Result<String, Failure> + Send + 'static,
    {
        {
            let mut state = self.state.lock().unwrap();
            if matches!(*state, State::Running) {
                return;
            }
            *state = State::Running;
        }
        self.progress.reset();

        let cfg = Arc::clone(&self.cfg);
        let progress = Arc::clone(&self.progress);
        let state = Arc::clone(&self.state);
        thread::spawn(move || {
            // catch_unwind so a panic in a job shows as an error instead of
            // leaving the window stuck on "running" forever.
            let result = catch_unwind(AssertUnwindSafe(|| job(&cfg, &progress)))
                .unwrap_or_else(|_| {
                    Err(Failure::with_details(CRASHED, "the job panicked"))
                });
            let notes = progress.take_notes();
            *state.lock().unwrap() = match result {
                Ok(mut text) => {
                    for note in notes {
                        text.push('\n');
                        text.push_str(&note);
                    }
                    State::Done(text)
                }
                Err(failure) => State::Failed(failure),
            };
        });
    }
}