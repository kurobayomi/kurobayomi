//! The unlock method and the one modal prompt every gate uses (T-47a-063, T-47a-067, T-47a-068).
//!
//! Method (decision A): an app passphrase. Its argon2id hash, as a PHC string, is the secret
//! `kurobayomi-applock` in the app's secret store; the passphrase itself is never stored. Fingerprint is not built.
//! The method exists while the Require unlock switch is on: turning the switch on sets the passphrase, turning it off
//! (after an unlock) removes it. [`confirm`] passes at once when no passphrase is set (T-47a-067), so the Hide
//! content, lock-when-idle and NSFW rows ask only while the lock is on.
use kuro_i18n::{MsgKey, tr};
use std::sync::{Arc, OnceLock};

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{Error as HashError, PasswordVerifier};
use argon2::Argon2;
use gtk::prelude::*;
use kuro_platform::secrets::SecretStore;

use crate::pref_model::widgets::dialog;
use crate::theme::tokens::{SPACE_M};

pub fn confirm_lock_change() -> String {
    tr(MsgKey::confirm_lock_change)
}
pub fn unlock_button() -> String {
    tr(MsgKey::settings_auth_unlock_button)
}
pub fn wrong_passphrase() -> String {
    tr(MsgKey::settings_auth_wrong_passphrase)
}

const SERVICE: &str = "kurobayomi-applock";
const ACCOUNT: &str = "passphrase";

/// The passphrase method over a secret store.
pub struct Passphrase<'a>(&'a dyn SecretStore);

impl<'a> Passphrase<'a> {
    pub fn new(store: &'a dyn SecretStore) -> Self {
        Self(store)
    }

    /// Whether a passphrase is set, which is whether the method exists.
    pub fn is_set(&self) -> Result<bool, String> {
        self.0.get(SERVICE, ACCOUNT).map(|hash| hash.is_some()).map_err(|e| e.to_string())
    }

    /// `Ok(true)` for the right passphrase, `Ok(false)` for a wrong one, `Err` when the hash cannot be read.
    pub fn verify(&self, passphrase: &str) -> Result<bool, String> {
        let stored = self.0.get(SERVICE, ACCOUNT).map_err(|e| e.to_string())?.ok_or(tr(MsgKey::ui_no_passphrase_is_set))?;
        let hash = PasswordHash::new(&stored).map_err(|e| e.to_string())?;
        match Argon2::default().verify_password(passphrase.as_bytes(), &hash) {
            Ok(()) => Ok(true),
            Err(HashError::PasswordInvalid) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

}

/// The app's secret store, opened once. Tests get an empty in-memory one.
fn store() -> Arc<dyn SecretStore> {
    static STORE: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();
    STORE.get_or_init(open).clone()
}

#[cfg(not(test))]
fn open() -> Arc<dyn SecretStore> {
    use kuro_platform::system_paths::SystemPaths;
    match SystemPaths::system() {
        Ok(paths) => Arc::new(kuro_platform::secrets::open(&paths)),
        Err(_) => Arc::new(kuro_prefs::memory::MemorySecrets::default()),
    }
}

#[cfg(test)]
fn open() -> Arc<dyn SecretStore> {
    Arc::new(kuro_prefs::memory::MemorySecrets::default())
}

/// Runs a store call off the GTK thread: it may wait on the secret service, and hashing takes a moment. Both
/// the secret store and hashing are blocking work, so the gio thread pool is used like the rest of the app.
async fn blocking<T: Send + 'static>(work: impl FnOnce(&Passphrase<'_>) -> T + Send + 'static) -> T {
    let run = move || work(&Passphrase::new(store().as_ref()));
    #[cfg(test)]
    {
        run()
    }
    #[cfg(not(test))]
    {
        crate::background::blocking(run).await.expect("the unlock task panicked")
    }
}

fn parent() -> Option<gtk::Window> {
    gtk::Window::list_toplevels().into_iter().find_map(|w| w.downcast::<gtk::Window>().ok())
}

fn entry() -> gtk::PasswordEntry {
    gtk::PasswordEntry::builder().show_peek_icon(true).activates_default(false).build()
}

fn body(text: &str, entries: &[&gtk::PasswordEntry]) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, SPACE_M);
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    column.append(&label);
    for e in entries {
        column.append(*e);
    }
    column
}

/// `auth::confirm(title, subtitle)` (T-47a-068): true when no unlock method is set, otherwise the verdict of the one
/// modal passphrase dialog. A failing secret store refuses (the lock must not open by accident) and shows its error.
pub async fn confirm(title: &str, subtitle: &str) -> bool {
    match blocking(|p| p.is_set()).await {
        Ok(false) => return true,
        Ok(true) => {}
        Err(message) => {
            crate::toast::notify(message);
            return false;
        }
    }
    let Some(parent) = parent() else { return false };
    let field = entry();
    let (tx, rx) = async_channel::bounded::<bool>(1);
    let on_cancel = tx.clone();
    dialog::show(
        &parent,
        title,
        &body(subtitle, &[&field]),
        Some(&unlock_button()),
        {
            let field = field.clone();
            move |dialog| {
                let typed = field.text().to_string();
                dialog.set_confirm_enabled(false);
                let (field, tx) = (field.clone(), tx.clone());
                gtk::glib::spawn_future_local(async move {
                    match blocking(move |p| p.verify(&typed)).await {
                        Ok(true) => {
                            let _ = tx.try_send(true);
                            dialog.finish();
                        }
                        Ok(false) => {
                            dialog.set_error(Some(&wrong_passphrase()));
                            dialog.set_confirm_enabled(true);
                            field.select_region(0, -1);
                        }
                        Err(message) => {
                            dialog.set_error(Some(&message));
                            dialog.set_confirm_enabled(true);
                        }
                    }
                });
            }
        },
        move || {
            let _ = on_cancel.try_send(false);
        },
    );
    field.grab_focus();
    rx.recv().await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    

    use super::*;

    /// The futures here resolve at once (no dialog opens), so one poll is enough.
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(future).as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("a gate that needs no dialog stayed pending"),
        }
    }

    #[test]
    fn no_method_set_means_confirm_passes_without_a_dialog() {
        assert!(block_on(confirm("Title", &confirm_lock_change())));
    }

}
