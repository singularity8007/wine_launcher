//! A small shared "what's happening right now" record.
//!
//! Backend code (installer, downloader) writes to it while it works; the
//! window reads it about ten times a second. It knows nothing about the GUI
//! and the GUI knows nothing about downloads: this is the only thing they share.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    label: Mutex<String>,
    /// Warnings that aren't failures, e.g. "offline, using the installed copy".
    notes: Mutex<Vec<String>>,
}

/// A copy of the numbers at one moment, safe to hold while they keep changing.
pub struct Snapshot {
    pub label: String,
    pub done: u64,
    pub total: u64,
}

impl Progress {
    /// Clear everything. Called at the start of each job.
    pub fn reset(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.label.lock().unwrap().clear();
        self.notes.lock().unwrap().clear();
    }

    /// One line describing the current step ("Reading manifest: game").
    pub fn set_label(&self, text: impl Into<String>) {
        *self.label.lock().unwrap() = text.into();
    }

    /// How many bytes the current step will move in total. 0 means "unknown",
    /// which the window shows as a spinner instead of a bar.
    pub fn set_total(&self, bytes: u64) {
        self.total.store(bytes, Ordering::Relaxed);
    }

    /// Workers call this as they finish pieces of the job.
    pub fn add(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record a warning to show when the job ends.
    pub fn note(&self, text: impl Into<String>) {
        self.notes.lock().unwrap().push(text.into());
    }

    /// Hand over the warnings collected so far and empty the list.
    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.lock().unwrap())
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            label: self.label.lock().unwrap().clone(),
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        }
    }
}