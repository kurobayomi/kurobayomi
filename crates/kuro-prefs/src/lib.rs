//! Typed preferences for Kurobayomi, stored in `config.toml` and `state.toml`.
//!
//! Keys are registered in [`registry`]. A [`store::PrefStore`] holds the values; a typed
//! [`pref::Pref`] handle reads and writes one key; [`toml_backend`] persists them; and
//! [`settings_load`] opens the store for the app's directories.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub mod backup;
pub mod base;
pub mod category;
pub mod checkbox;
pub mod export;
pub mod download;
pub mod dropped;
pub mod helpers;
pub mod import;
pub mod flags;
pub mod keys;
pub mod library;
pub mod legacy;
pub mod memory;
pub mod migrate;
pub mod network;
pub mod pref;
pub mod ranges;
pub mod player;
pub mod prefs;
pub mod reader;
pub mod registry;
pub mod settings_load;
pub mod source;
pub mod source_file;
pub mod store;
#[cfg(test)]
mod testutil;
pub mod toml_backend;
pub mod tracking;
pub mod tristate;
pub mod ui;
pub mod values;

pub use prefs::Prefs;

/// Errors from opening, reading or writing the preference files.
#[derive(Debug, thiserror::Error)]
pub enum PrefError {
    #[error("preferences are locked by another Kurobayomi process")]
    Locked,
    #[error("cannot create directory {}: {source}", path.display())]
    Dir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Locks a mutex, recovering the value if another thread panicked while holding it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn read<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn write<T>(m: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(PoisonError::into_inner)
}
