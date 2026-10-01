//! The only thing the window talks to.
//!
//! The GUI calls `launch()`, `install_game()` and `check_updates()` and draws
//! whatever `view()` returns. Everything else (threads, downloads, error
//! wording, deciding whether a launch "failed") lives here or further back, so
//! no logic is duplicated between the two sides.

use crate::config::Config;
use crate::progress::Progress;
use crate::{downloader, installer, launch_chain};
use anyhow::{bail, Result};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A launch that ends non-zero sooner than this counts as "the game didn't
/// start". Longer than this counts as a normal play session.
const EARLY_EXIT: Duration = Duration::from_secs(30);

/// Shown when a launch fails. We can't tell *why* from outside the game, so
/// the wording lists the likely causes instead of claiming one.
const LAUNCH_FAILED: &str = "The game didn't start. HoYo's servers may have timed out. \
Check your internet connection, or use \"Check for updates\", then try again.";

#[derive(Clone)]
enum State {
    Idle,
    Running,
    Done(String),
    Failed(String),
}

/// Everything the window needs to draw one frame, already decided.
pub struct View {
    /// A job is running: the window disables the buttons.
    pub busy: bool,
    /// What the job is doing right now.
    pub label: String,
    /// 0.0 to 1.0, or None when the job has no known size (show a spinner).
    pub fraction: Option<f32>,
    /// Result of the last job, if it has ended.
    pub message: Option<Message>,
}

pub struct Message {
    pub text: String,
    pub is_error: bool,
}

pub struct Backend {
    cfg: Arc<Config>,
    progress: Arc<Progress>,
    state: Arc<Mutex<State>>,
}

impl Backend {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg: Arc::new(cfg),
            progress: Arc::new(Progress::default()),
            state: Arc::new(Mutex::new(State::Idle)),
        }
    }

    /// Download and install ZZZ (does nothing if it's already installed).
    pub fn install_game(&self) {
        self.start(|cfg, progress| {
            installer::ensure_game(&cfg.game_dir, progress)?;
            Ok("ZZZ is installed.".into())
        });
    }

    /// Update GE-Proton and the Steam Linux Runtime.
    pub fn check_updates(&self) {
        self.start(|cfg, progress| {
            downloader::update_all(cfg, progress)?;
            Ok("GE-Proton and the Steam Linux Runtime were checked.".into())
        });
    }

    /// Just try to run the game: no network, no update checks.
    pub fn launch(&self) {
        self.start(|cfg, progress| {
            // A local file check only, so a missing install gets a clear message
            // instead of a confusing spawn error.
            if !launch_chain::is_ready(cfg) {
                bail!("ZZZ isn't installed yet. Use \"Install ZZZ\" first.");
            }
            progress.set_label("Game running...");
            let started = Instant::now();
            match launch_chain::run(cfg) {
                // Clean exit, or it ran a normal length of time: a normal session.
                Ok(status) if status.success() || started.elapsed() >= EARLY_EXIT => {
                    Ok("Game closed.".into())
                }
                Ok(status) => bail!("{LAUNCH_FAILED}\n\n(exit status: {status})"),
                Err(e) => bail!("{LAUNCH_FAILED}\n\n(details: {e:#})"),
            }
        });
    }

    /// Snapshot for the window to draw. Cheap; called every frame.
    pub fn view(&self) -> View {
        let snap = self.progress.snapshot();
        let state = self.state.lock().unwrap().clone();
        View {
            busy: matches!(state, State::Running),
            label: snap.label,
            fraction: (snap.total > 0).then(|| snap.done as f32 / snap.total as f32),
            message: match state {
                State::Done(text) => Some(Message { text, is_error: false }),
                State::Failed(text) => Some(Message { text, is_error: true }),
                State::Idle | State::Running => None,
            },
        }
    }

    /// Run `job` on its own thread so the window never freezes. Ignored if a
    /// job is already running. Warnings the job noted are appended to its result.
    fn start<F>(&self, job: F)
    where
        F: FnOnce(&Config, &Progress) -> Result<String> + Send + 'static,
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
                .unwrap_or_else(|_| Err(anyhow::anyhow!("internal error: the job crashed")));
            let notes = progress.take_notes();
            *state.lock().unwrap() = match result {
                Ok(mut text) => {
                    for note in notes {
                        text.push('\n');
                        text.push_str(&note);
                    }
                    State::Done(text)
                }
                Err(e) => State::Failed(format!("{e:#}")),
            };
        });
    }
}