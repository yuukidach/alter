//! Small operating-system integration points shared by the UI and actions.

use std::io;
use std::path::Path;

pub fn open_url(url: &str) -> io::Result<()> {
    open::that_detached(url)
}

pub fn open_path(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

#[cfg(target_os = "linux")]
pub fn launch_application(path: &Path) -> io::Result<()> {
    let status = std::process::Command::new("gio")
        .arg("launch")
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "gio launch exited with status {status}"
        )))
    }
}

#[cfg(windows)]
pub fn launch_application(path: &Path) -> io::Result<()> {
    // Start-menu entries are .lnk/.url files. ShellExecute (used by `open`)
    // resolves them through the same Windows shell path as Explorer.
    open::that_detached(path)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn launch_application(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

#[cfg(windows)]
pub fn reveal_path(path: &Path) -> io::Result<()> {
    let status = std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "Explorer exited with status {status}"
        )))
    }
}

#[cfg(not(windows))]
pub fn reveal_path(path: &Path) -> io::Result<()> {
    let directory = path.parent().unwrap_or(path);
    open_path(directory)
}
