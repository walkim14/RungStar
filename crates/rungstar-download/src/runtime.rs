//! A JavaScript runtime for yt-dlp's YouTube challenge solver.
//!
//! Current yt-dlp can no longer extract every YouTube format without JavaScript. A developer
//! often has Node already; a clean Windows install and a Steam Deck in Game Mode do not. Use a
//! supported runtime from PATH when one exists, otherwise keep an official Deno release beside
//! the managed yt-dlp copy.

use std::io::Cursor;
use std::path::{Path, PathBuf};

pub const RELEASE_URL: &str = "https://github.com/denoland/deno/releases/latest/download";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsRuntime {
    pub name: &'static str,
    pub program: PathBuf,
}

impl JsRuntime {
    pub fn argument(&self) -> String {
        format!("{}:{}", self.name, self.program.display())
    }
}

pub fn file_name() -> &'static str {
    if cfg!(windows) {
        "deno.exe"
    } else {
        "deno"
    }
}

pub fn managed_path(data_dir: &Path) -> PathBuf {
    data_dir.join("tools").join(file_name())
}

pub fn find(data_dir: &Path) -> Option<JsRuntime> {
    for (name, program) in [
        ("deno", "deno"),
        ("node", "node"),
        ("bun", "bun"),
        ("quickjs", "qjs"),
    ] {
        if let Some(program) = on_path(program) {
            return Some(JsRuntime { name, program });
        }
    }
    let managed = managed_path(data_dir);
    if runs(&managed) {
        return Some(JsRuntime {
            name: "deno",
            program: managed,
        });
    }
    let bundled = beside_the_executable()?;
    Some(JsRuntime {
        name: "deno",
        program: bundled,
    })
}

fn beside_the_executable() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_owned();
    [
        folder.join(file_name()),
        folder.join("tools").join(file_name()),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file() && runs(candidate))
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(&path)
        .map(|folder| folder.join(&file))
        .find(|candidate| candidate.is_file() && runs(candidate))
}

pub fn runs(program: &Path) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn version(runtime: &JsRuntime) -> Option<String> {
    let output = std::process::Command::new(&runtime.program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::to_owned)
}

pub fn asset() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "windows") => "deno-x86_64-pc-windows-msvc.zip",
        ("aarch64", "windows") => "deno-aarch64-pc-windows-msvc.zip",
        ("x86_64", "linux") => "deno-x86_64-unknown-linux-gnu.zip",
        ("aarch64", "linux") => "deno-aarch64-unknown-linux-gnu.zip",
        ("x86_64", "macos") => "deno-x86_64-apple-darwin.zip",
        ("aarch64", "macos") => "deno-aarch64-apple-darwin.zip",
        _ => "",
    }
}

pub fn download_url() -> Option<String> {
    let asset = asset();
    (!asset.is_empty()).then(|| format!("{RELEASE_URL}/{asset}"))
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Deno has no release for this platform")]
    Unsupported,
    #[error("the Deno download is not a readable ZIP: {0}")]
    Archive(String),
    #[error("the Deno download contains no {0}")]
    Missing(String),
    #[error("could not write Deno to {0}: {1}")]
    Write(String, String),
    #[error("the installed Deno binary does not start")]
    NotRunnable,
}

pub fn install(data_dir: &Path, archive: &[u8]) -> Result<JsRuntime, RuntimeError> {
    if asset().is_empty() {
        return Err(RuntimeError::Unsupported);
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|error| RuntimeError::Archive(error.to_string()))?;
    let mut entry = zip
        .by_name(file_name())
        .map_err(|_| RuntimeError::Missing(file_name().to_owned()))?;
    if entry.size() < 1_000_000 {
        return Err(RuntimeError::Archive(format!(
            "{} is only {} bytes",
            file_name(),
            entry.size()
        )));
    }
    let path = managed_path(data_dir);
    let folder = path.parent().unwrap_or(data_dir);
    std::fs::create_dir_all(folder)
        .map_err(|error| RuntimeError::Write(folder.display().to_string(), error.to_string()))?;
    let temporary = path.with_extension("part");
    let mut file = std::fs::File::create(&temporary)
        .map_err(|error| RuntimeError::Write(temporary.display().to_string(), error.to_string()))?;
    std::io::copy(&mut entry, &mut file)
        .map_err(|error| RuntimeError::Write(temporary.display().to_string(), error.to_string()))?;
    drop(file);
    make_runnable(&temporary);
    std::fs::rename(&temporary, &path)
        .map_err(|error| RuntimeError::Write(path.display().to_string(), error.to_string()))?;

    let runtime = JsRuntime {
        name: "deno",
        program: path,
    };
    if !runs(&runtime.program) {
        let _ = std::fs::remove_file(&runtime.program);
        return Err(RuntimeError::NotRunnable);
    }
    Ok(runtime)
}

#[cfg(unix)]
fn make_runnable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_runnable(_path: &Path) {}
