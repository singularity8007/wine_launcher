//! Total play time and the last-played date, kept in a small JSON file.
//!
//! Time only counts while ZZZ is running *and* this launcher is open, because
//! the launcher is what notices the game starting and stopping.

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How often a running session is written to disk, so a crash or a closed
/// launcher loses at most about this much play time.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(60);

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    total_seconds: u64,
    /// `YYYY-MM-DD`, in the player's local time. Never includes a time of day.
    last_played: Option<String>,
}

pub struct PlayStats {
    path: PathBuf,
    saved: Mutex<Saved>,
    /// When the part of the current session that isn't in `saved` yet began.
    /// `None` while the game isn't running.
    session: Mutex<Option<Instant>>,
}

/// Ends the play session when dropped, even if the job around it panics.
pub struct Session<'a> {
    stats: &'a PlayStats,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        self.stats.end_session();
    }
}

impl PlayStats {
    /// Read the saved numbers (starts from zero if there's no file yet).
    pub fn load(path: PathBuf) -> Self {
        let saved = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            saved: Mutex::new(saved),
            session: Mutex::new(None),
        }
    }

    /// ZZZ was just launched: stamp today's date and start counting.
    pub fn start_session(&self) -> Session<'_> {
        self.saved.lock().unwrap().last_played =
            Some(Local::now().format("%Y-%m-%d").to_string());
        *self.session.lock().unwrap() = Some(Instant::now());
        self.save();
        Session { stats: self }
    }

    fn end_session(&self) {
        self.bank();
        *self.session.lock().unwrap() = None;
        self.save();
    }

    /// Called often by the window; writes a running session to disk about
    /// once a minute.
    pub fn checkpoint(&self) {
        let due = self
            .session
            .lock()
            .unwrap()
            .is_some_and(|start| start.elapsed() >= CHECKPOINT_EVERY);
        if due {
            self.bank();
            self.save();
        }
    }

    /// Total play time, including the session that's running right now.
    pub fn total(&self) -> Duration {
        let live = self
            .session
            .lock()
            .unwrap()
            .map_or(Duration::ZERO, |start| start.elapsed());
        Duration::from_secs(self.saved.lock().unwrap().total_seconds) + live
    }

    /// The last launch date as "Oct 1, 2026", or None if ZZZ was never launched.
    pub fn last_played(&self) -> Option<String> {
        let saved = self.saved.lock().unwrap();
        let date =
            NaiveDate::parse_from_str(saved.last_played.as_deref()?, "%Y-%m-%d").ok()?;
        Some(date.format("%b %-d, %Y").to_string())
    }

    /// Move the running session's whole seconds into the saved total.
    fn bank(&self) {
        let mut session = self.session.lock().unwrap();
        if let Some(start) = session.as_mut() {
            let seconds = start.elapsed().as_secs();
            if seconds > 0 {
                self.saved.lock().unwrap().total_seconds += seconds;
                // Keep the leftover fraction of a second for next time.
                *start += Duration::from_secs(seconds);
            }
        }
    }

    fn save(&self) {
        let text = serde_json::to_string_pretty(&*self.saved.lock().unwrap());
        // Play time is nice to have; never worth failing anything over.
        if let Ok(text) = text {
            let _ = fs::write(&self.path, text);
        }
    }
}

/// "94 hours 37 minutes", "1 hour 1 minute", "12 minutes".
pub fn describe(total: Duration) -> String {
    let minutes = total.as_secs() / 60;
    let (hours, minutes) = (minutes / 60, minutes % 60);
    let unit = |n: u64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    if hours == 0 {
        unit(minutes, "minute")
    } else {
        format!("{} {}", unit(hours, "hour"), unit(minutes, "minute"))
    }
}