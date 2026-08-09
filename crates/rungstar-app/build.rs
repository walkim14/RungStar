//! Stamp the build with which commit it came from.
//!
//! Written because "did you rebuild after that fix?" is a question that cannot be answered by
//! looking at a binary, and answering it by reasoning about what happened in what order is
//! exactly the sort of thing that turns out to be wrong. `--check` prints this, so the machine
//! running the build says what it is rather than somebody remembering.
//!
//! A packaged build has no `.git` — the Flatpak copies the tree without it, on purpose — so
//! `packaging/linux/build-flatpak.sh` writes a `BUILD_ID` file into the copy and this reads
//! that instead. Neither being available is fine: the stamp says `unknown` and nothing else
//! changes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let root = workspace_root();
    let stamp = root.join("BUILD_ID");
    println!("cargo:rerun-if-changed={}", stamp.display());
    // A new commit, or a checkout, has to change the stamp. `.git/HEAD` covers switching
    // branches and `.git/index` covers committing on the one you are on.
    for path in [root.join(".git/HEAD"), root.join(".git/index")] {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    println!("cargo:rustc-env=RUNGSTAR_BUILD={}", describe(&root, &stamp));
}

fn describe(root: &Path, stamp: &Path) -> String {
    if let Ok(text) = std::fs::read_to_string(stamp) {
        let text = text.trim();
        if !text.is_empty() {
            return text.to_owned();
        }
    }

    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };

    let Some(commit) = git(&["rev-parse", "--short", "HEAD"]).filter(|s| !s.is_empty()) else {
        return "unknown".to_owned();
    };
    // Uncommitted work is worth saying: a stamp naming a commit that does not contain what is
    // in the binary is worse than no stamp at all.
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty());
    if dirty {
        format!("{commit}-dirty")
    } else {
        commit
    }
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    manifest
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").is_file())
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}
