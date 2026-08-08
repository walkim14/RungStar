//! Where the USDB password lives, which is not the config file.
//!
//! The OS keyring: Credential Manager on Windows, the Keychain on macOS, the Secret Service on
//! Linux. A `settings.toml` that quietly contains somebody's password is how it ends up in a
//! backup, in a screenshot, and in a bug report — and this is a password people reuse.
//!
//! Failing to store one is not fatal. A machine with no Secret Service running — a bare
//! SteamOS session, a container — should still be able to log in for that session and be told
//! why it will have to again.

const SERVICE: &str = "rungstar-usdb";

/// Why a secret could not be kept.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("this machine has nowhere to keep a password safely: {0}")]
    NoStore(String),
    #[error("the password store refused: {0}")]
    Refused(String),
}

/// Read the saved password for a USDB username.
pub fn password(user: &str) -> Result<Option<String>, SecretError> {
    let entry = entry(user)?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(SecretError::Refused(error.to_string())),
    }
}

/// Keep a password for next time.
pub fn remember(user: &str, password: &str) -> Result<(), SecretError> {
    entry(user)?
        .set_password(password)
        .map_err(|error| SecretError::Refused(error.to_string()))
}

/// Forget it. Called when logging out, and when a login is refused — a stored password that
/// no longer works is worse than none, because it fails silently on every launch.
pub fn forget(user: &str) -> Result<(), SecretError> {
    match entry(user)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(SecretError::Refused(error.to_string())),
    }
}

fn entry(user: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(SERVICE, user).map_err(|error| SecretError::NoStore(error.to_string()))
}
