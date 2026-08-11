use std::env;
use std::path::PathBuf;

/// Return the user's home directory without assuming that it is the current
/// working directory.  `$HOME` is set by every normal graphical login, but we
/// keep a safe fallback for unusual launch environments.
pub fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"))
        .join("alter")
}

pub fn config_dir() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("alter")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.conf")
}

pub fn database_path() -> PathBuf {
    data_dir().join("history.sqlite3")
}

pub fn icon_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe()
        && let Some(project_root) = executable
            .parent()
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::parent)
    {
        candidates.push(project_root.join("resources/alter_icon.png"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/alter_icon.png"));
    candidates.push(home_dir().join(".local/share/icons/alter.png"));
    candidates.push(PathBuf::from(
        "/usr/share/icons/hicolor/512x512/apps/alter.png",
    ));
    candidates.into_iter().find(|path| path.is_file())
}
