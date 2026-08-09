//! Asking somebody where their songs are.
//!
//! A native dialog rather than one drawn in the game. Two reasons: a file browser is a
//! surprisingly large screen to write and an endlessly fiddly one to get right, and somebody
//! looking for a folder already knows what theirs looks like in the system's own browser.
//!
//! **On Linux it goes through the XDG desktop portal**, not GTK. A Steam Deck in Game Mode has
//! no desktop session and a Flatpak has no GTK to link against, so the GTK backend — which is
//! `rfd`'s default — would either fail to build or open nothing at all. The portal is a D-Bus
//! service the sandbox is allowed to talk to, and it is what a Flatpak is supposed to use.
//!
//! Every path in here is a "somebody might not have one" path: no portal, a cancelled dialog,
//! a folder that has since been deleted. All of them mean the same thing to the caller — no
//! folder was chosen — and none of them is an error worth showing.

use std::path::PathBuf;

/// Ask for a folder, starting at `from` when that still exists.
///
/// Blocks until the dialog closes. That freezes the game's window for as long as it is open,
/// which is what a modal dialog is: the alternative is a game that keeps animating behind a
/// picker and accepts input into both.
pub fn choose(title: &str, from: Option<&std::path::Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    // Starting where the last one was saves the walk up from the home directory. Only when it
    // is still there — a dialog that opens at a deleted path opens somewhere arbitrary instead.
    if let Some(start) = from.filter(|path| path.is_dir()) {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_folder()
}

/// Whether a folder is worth adding, and what to say if not.
///
/// Checked before it goes in the settings rather than after the next scan finds nothing:
/// "0 songs" is the same message for a wrong folder, an empty folder and a broken scanner.
pub fn check(path: &std::path::Path, existing: &[String]) -> Result<String, String> {
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    let text = path.to_string_lossy().into_owned();
    // Compared as written. Two spellings of one folder is not worth resolving symlinks over,
    // and the scanner deduplicates by path anyway; this only catches the obvious repeat.
    if existing.iter().any(|held| held == &text) {
        return Err(format!("{} is already a song folder", path.display()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_is_not_there_is_refused() {
        let missing = std::env::temp_dir().join("rungstar-no-such-folder-3f9a");
        assert!(check(&missing, &[]).is_err());
    }

    #[test]
    fn the_same_folder_twice_is_refused() {
        let dir = std::env::temp_dir();
        let text = dir.to_string_lossy().into_owned();
        assert!(check(&dir, &[]).is_ok());
        let error = check(&dir, &[text]).unwrap_err();
        assert!(error.contains("already"), "{error}");
    }
}
