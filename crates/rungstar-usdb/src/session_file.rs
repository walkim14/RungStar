//! Staying signed in on a machine with no password store.
//!
//! The OS keyring is the right place for a password, and on Windows and macOS it is always
//! there. On Linux it is a D-Bus **Secret Service** — gnome-keyring or KWallet — and that is a
//! desktop session service. A Steam Deck in Game Mode has no desktop session, so there is very
//! likely nothing listening, and the same is true of a kiosk, a container, or anything started
//! from a TTY.
//!
//! Without a fallback that machine asks for a password on an **on-screen keyboard, every
//! launch**, which is exactly the machine where that is worst.
//!
//! So the fallback keeps the **session cookie** rather than the password. That is a real
//! difference, not a smaller version of the same risk:
//!
//! - a cookie expires, a password does not;
//! - a cookie is worth nothing anywhere else, and a forum password is one people reuse;
//! - a cookie cannot be used to change the account's email and take it over.
//!
//! What it costs is that the session eventually ends and has to be signed in again. On a
//! machine with a keyring that never happens, because the password is there to renew it
//! silently. On one without, it is a few weeks of not typing instead of none.
//!
//! Deliberately **not** encrypting the password with a key derived from the machine. That is
//! obfuscation dressed as security: the key is in the binary, so anyone with the file has the
//! password, and the only thing it reliably protects against is the reader understanding what
//! they are looking at.

use std::io::{BufReader, Write};
use std::path::Path;

/// Why a session could not be kept or restored.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("the saved sign-in could not be read: {0}")]
    Read(String),
    #[error("the sign-in could not be saved: {0}")]
    Write(String),
}

/// Where the session is kept, under the data directory.
pub const FILE: &str = "usdb-session.json";

/// Write the agent's cookies so the next launch is already signed in.
///
/// Owner-only on Unix. On Windows the data directory is already under the user's profile,
/// which is the same protection by a different route.
pub fn save(agent: &ureq::Agent, path: &Path) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| SessionError::Write(e.to_string()))?;
    }
    let mut bytes: Vec<u8> = Vec::new();
    agent
        .cookie_jar_lock()
        .save_json(&mut bytes)
        .map_err(|e| SessionError::Write(e.to_string()))?;

    // Through a temporary file, so an interrupted write cannot leave a truncated jar that
    // fails to parse and silently signs somebody out.
    let temporary = path.with_extension("part");
    let mut file =
        std::fs::File::create(&temporary).map_err(|e| SessionError::Write(e.to_string()))?;
    restrict(&file);
    file.write_all(&bytes)
        .map_err(|e| SessionError::Write(e.to_string()))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|e| SessionError::Write(e.to_string()))
}

/// Load a saved session into an agent.
///
/// A missing file is not an error: it is a first run, or a machine that has a keyring and
/// never needed this.
pub fn load(agent: &ureq::Agent, path: &Path) -> Result<bool, SessionError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(SessionError::Read(error.to_string())),
    };
    agent
        .cookie_jar_lock()
        .load_json(BufReader::new(file))
        .map_err(|e| SessionError::Read(e.to_string()))?;
    Ok(true)
}

/// Delete the saved session. Called on signing out.
pub fn forget(path: &Path) -> Result<(), SessionError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SessionError::Write(error.to_string())),
    }
}

#[cfg(unix)]
fn restrict(file: &std::fs::File) {
    use std::os::unix::fs::PermissionsExt;
    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_file: &std::fs::File) {}
