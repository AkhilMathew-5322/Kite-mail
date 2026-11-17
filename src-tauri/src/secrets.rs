//! Stores passwords / OAuth refresh tokens in the OS credential store
//! (Windows Credential Manager, Linux Secret Service / GNOME Keyring / KWallet).
//! If the OS store is unavailable or rejects the value (e.g. too long), falls back
//! to the local database so the app keeps working.

use crate::db::Db;

const SERVICE: &str = "kitemail";

pub fn set(db: &Db, key: &str, value: &str) -> Result<(), String> {
    let res = keyring::Entry::new(SERVICE, key).and_then(|en| en.set_password(value));
    match res {
        Ok(()) => {
            // Make sure the value round-trips before trusting the OS store.
            if get_os(key).as_deref() == Some(value) {
                db.del_secret(key);
                return Ok(());
            }
            db.set_secret(key, value)
        }
        Err(err) => {
            eprintln!("[kitemail] OS keyring unavailable for {key} ({err}); using local store");
            db.set_secret(key, value)
        }
    }
}

fn get_os(key: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()
}

pub fn get(db: &Db, key: &str) -> Option<String> {
    db.get_secret(key).or_else(|| get_os(key))
}

pub fn delete(db: &Db, key: &str) {
    if let Ok(en) = keyring::Entry::new(SERVICE, key) {
        let _ = en.delete_credential();
    }
    db.del_secret(key);
}

pub fn password_key(account_id: i64) -> String {
    format!("account-{account_id}-password")
}

pub fn refresh_key(account_id: i64) -> String {
    format!("account-{account_id}-refresh-token")
}
