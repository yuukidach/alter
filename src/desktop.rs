use crate::paths;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};
use walkdir::WalkDir;

/// Avoid re-statting application directories on every keystroke while still
/// picking up newly installed or removed applications even if the launcher
/// window remains open.
const REFRESH_CHECK_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct AppItem {
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    /// The value from the desktop entry's `Icon=` key. It may be a themed
    /// icon name or an absolute/relative image path.
    pub icon: Option<String>,
    pub desktop_file: PathBuf,
}

impl AppItem {
    pub fn search_text(&self) -> String {
        let mut text = self.name.clone();
        if let Some(generic) = &self.generic_name {
            text.push(' ');
            text.push_str(generic);
        }
        for keyword in &self.keywords {
            text.push(' ');
            text.push_str(keyword);
        }
        text.push(' ');
        text.push_str(&self.id);
        text
    }
}

#[derive(Default)]
struct DesktopEntry {
    name: Option<String>,
    generic_name: Option<String>,
    keywords: Vec<String>,
    icon: Option<String>,
    entry_type: Option<String>,
    hidden: bool,
    no_display: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogFormat {
    DesktopEntries,
    #[cfg_attr(not(windows), allow(dead_code))]
    WindowsShortcuts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryStamp {
    root_index: usize,
    path: PathBuf,
    exists: bool,
    is_dir: bool,
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64, i64, i64),
}

impl EntryStamp {
    fn missing(root_index: usize, path: PathBuf) -> Self {
        Self {
            root_index,
            path,
            exists: false,
            is_dir: false,
            len: 0,
            modified: None,
            #[cfg(unix)]
            inode: (0, 0, 0, 0),
        }
    }

    fn from_metadata(
        root_index: usize,
        path: PathBuf,
        is_dir: bool,
        metadata: &fs::Metadata,
    ) -> Self {
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
        };
        Self {
            root_index,
            path,
            exists: true,
            is_dir,
            len: if is_dir { 0 } else { metadata.len() },
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            inode,
        }
    }
}

struct CatalogState {
    apps: Arc<Vec<AppItem>>,
    fingerprint: Vec<EntryStamp>,
    last_checked: Option<Instant>,
}

/// Cached catalog of installed applications that reloads automatically when
/// the underlying desktop-entry or Start Menu directories change.
pub struct AppCatalog {
    roots: Option<Vec<PathBuf>>,
    format: CatalogFormat,
    state: RwLock<CatalogState>,
}

impl AppCatalog {
    #[cfg(not(windows))]
    pub fn load() -> Self {
        Self::from_desktop_roots(application_roots())
    }

    #[cfg(windows)]
    pub fn load() -> Self {
        Self::from_windows_roots(windows_application_roots())
    }

    pub fn from_static(apps: Vec<AppItem>) -> Self {
        Self {
            roots: None,
            format: CatalogFormat::DesktopEntries,
            state: RwLock::new(CatalogState {
                apps: Arc::new(apps),
                fingerprint: Vec::new(),
                last_checked: None,
            }),
        }
    }

    #[cfg_attr(windows, allow(dead_code))]
    pub fn from_desktop_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::from_roots_with_format(roots.into_iter().collect(), CatalogFormat::DesktopEntries)
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn from_windows_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::from_roots_with_format(roots.into_iter().collect(), CatalogFormat::WindowsShortcuts)
    }

    fn from_roots_with_format(roots: Vec<PathBuf>, format: CatalogFormat) -> Self {
        let fingerprint = fingerprint_roots(&roots, format);
        let apps = Arc::new(load_for_format(&roots, format));
        Self {
            roots: Some(roots),
            format,
            state: RwLock::new(CatalogState {
                apps,
                fingerprint,
                last_checked: Some(Instant::now()),
            }),
        }
    }

    /// Force the next [`Self::snapshot`] call on a worker thread to re-check
    /// directory metadata immediately, without doing disk I/O on the UI thread.
    pub fn schedule_check(&self) {
        if self.roots.is_none() {
            return;
        }
        if let Ok(mut state) = self.state.write() {
            state.last_checked = None;
        }
    }

    pub fn snapshot(&self) -> Arc<Vec<AppItem>> {
        let Some(roots) = self.roots.as_deref() else {
            return self
                .state
                .read()
                .map(|state| Arc::clone(&state.apps))
                .unwrap_or_default();
        };

        if let Ok(state) = self.state.read()
            && state
                .last_checked
                .is_some_and(|checked| checked.elapsed() < REFRESH_CHECK_INTERVAL)
        {
            return Arc::clone(&state.apps);
        }

        // Capture the filesystem fingerprint before reading desktop files so
        // any concurrent package-manager writes that land mid-scan still cause
        // a mismatch on the following check.
        let new_fingerprint = fingerprint_roots(roots, self.format);
        let unchanged_apps = self.state.read().ok().and_then(|state| {
            (state.fingerprint == new_fingerprint).then(|| Arc::clone(&state.apps))
        });
        if let Some(apps) = unchanged_apps {
            if let Ok(mut state) = self.state.write()
                && state.fingerprint == new_fingerprint
            {
                state.last_checked = Some(Instant::now());
            }
            return apps;
        }

        let reloaded = Arc::new(load_for_format(roots, self.format));
        if let Ok(mut state) = self.state.write() {
            state.apps = Arc::clone(&reloaded);
            state.fingerprint = new_fingerprint;
            state.last_checked = Some(Instant::now());
        }
        reloaded
    }
}

fn load_for_format(roots: &[PathBuf], format: CatalogFormat) -> Vec<AppItem> {
    match format {
        CatalogFormat::DesktopEntries => load_applications_from_roots(roots.iter().cloned()),
        CatalogFormat::WindowsShortcuts => load_windows_applications(roots.iter().cloned()),
    }
}

fn is_supported_entry_path(path: &Path, format: CatalogFormat) -> bool {
    match format {
        CatalogFormat::DesktopEntries => {
            path.extension().and_then(|value| value.to_str()) == Some("desktop")
        }
        CatalogFormat::WindowsShortcuts => is_windows_shortcut_path(path),
    }
}

fn is_windows_shortcut_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            ["lnk", "url", "appref-ms"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn fingerprint_roots(roots: &[PathBuf], format: CatalogFormat) -> Vec<EntryStamp> {
    let max_depth = match format {
        CatalogFormat::DesktopEntries => 3,
        CatalogFormat::WindowsShortcuts => 8,
    };
    let mut stamps = Vec::new();

    for (root_index, root) in roots.iter().enumerate() {
        if !root.is_dir() {
            stamps.push(EntryStamp::missing(root_index, root.clone()));
            continue;
        }

        let mut root_stamps = Vec::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(max_depth)
            .into_iter()
            .filter_map(Result::ok)
        {
            let file_type = entry.file_type();
            let path = entry.path();
            if !file_type.is_dir()
                && !(file_type.is_file() && is_supported_entry_path(path, format))
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            root_stamps.push(EntryStamp::from_metadata(
                root_index,
                path.to_path_buf(),
                file_type.is_dir(),
                &metadata,
            ));
        }
        root_stamps.sort_by(|left, right| left.path.cmp(&right.path));
        stamps.extend(root_stamps);
    }

    stamps
}

fn load_applications_from_roots(roots: impl IntoIterator<Item = PathBuf>) -> Vec<AppItem> {
    // Local entries are visited first.  A local Hidden=true entry must mask a
    // system entry with the same desktop id, so we mark ids as seen before
    // parsing each file.
    let mut seen = HashSet::new();
    let mut apps = HashMap::<String, AppItem>::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .max_depth(3)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("desktop")
            {
                continue;
            }

            let id = desktop_id(&root, path);
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(app) = parse_desktop_file(path, id) {
                apps.insert(app.id.clone(), app);
            }
        }
    }

    let mut values: Vec<_> = apps.into_values().collect();
    values.sort_by_cached_key(|app| app.name.to_lowercase());
    values
}

#[cfg(not(windows))]
fn application_roots() -> Vec<PathBuf> {
    let mut data_roots = Vec::new();
    data_roots.push(paths::home_dir().join(".local/share"));

    if let Some(value) = std::env::var_os("XDG_DATA_DIRS") {
        data_roots.extend(std::env::split_paths(&value));
    } else {
        data_roots.push(PathBuf::from("/usr/local/share"));
        data_roots.push(PathBuf::from("/usr/share"));
    }

    // Flatpak exports are not always included in XDG_DATA_DIRS on a minimal
    // Hyprland session.
    data_roots.push(paths::home_dir().join(".local/share/flatpak/exports/share"));
    data_roots.push(PathBuf::from("/var/lib/flatpak/exports/share"));

    data_roots
        .into_iter()
        .map(|root| root.join("applications"))
        .collect()
}

#[cfg(windows)]
fn windows_application_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(app_data) = std::env::var_os("APPDATA") {
        roots.push(
            PathBuf::from(app_data)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
        roots.push(
            PathBuf::from(program_data)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    roots
}

#[cfg_attr(not(windows), allow(dead_code))]
fn load_windows_applications(roots: impl IntoIterator<Item = PathBuf>) -> Vec<AppItem> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if !is_windows_shortcut_path(path) {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .map(|value| value.to_string_lossy().trim().to_owned())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let identity = name.to_lowercase();
            if !seen.insert(identity) {
                continue;
            }
            let keywords: Vec<String> = path
                .strip_prefix(&root)
                .ok()
                .and_then(Path::parent)
                .map(|parent| {
                    parent
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .filter(|part| !part.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            apps.push(AppItem {
                id: desktop_id(&root, path),
                name,
                generic_name: Some("应用程序".to_owned()),
                keywords,
                icon: None,
                desktop_file: path.to_path_buf(),
            });
        }
    }

    apps.sort_by_cached_key(|app| app.name.to_lowercase());
    apps
}

fn desktop_id(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|relative| {
            relative
                .to_string_lossy()
                .replace('/', "-")
                .replace('\\', "-")
        })
        .unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

fn parse_desktop_file(path: &Path, id: String) -> Option<AppItem> {
    let content = fs::read_to_string(path).ok()?;
    let mut entry = DesktopEntry::default();
    let mut in_desktop_entry = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unescape(value.trim());
        match key {
            "Name" => entry.name = Some(value),
            key if key.starts_with("Name[") => {
                // Prefer a locale-specific name when it matches the current
                // locale, while retaining the plain Name as a fallback.
                if locale_matches(key) {
                    entry.name = Some(value);
                }
            }
            "GenericName" => entry.generic_name = Some(value),
            key if key.starts_with("GenericName[") => {
                if locale_matches(key) {
                    entry.generic_name = Some(value);
                }
            }
            "Keywords" => {
                entry.keywords = value
                    .split(';')
                    .filter(|keyword| !keyword.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "Icon" => entry.icon = Some(value),
            "Type" => entry.entry_type = Some(value),
            "Hidden" => entry.hidden = is_true(&value),
            "NoDisplay" => entry.no_display = is_true(&value),
            _ => {}
        }
    }

    if entry
        .entry_type
        .as_deref()
        .is_some_and(|value| value != "Application")
        || entry.hidden
        || entry.no_display
    {
        return None;
    }

    let name = entry.name?.trim().to_owned();
    if name.is_empty() {
        return None;
    }

    Some(AppItem {
        id,
        name,
        generic_name: entry.generic_name.filter(|value| !value.is_empty()),
        keywords: entry.keywords,
        icon: entry.icon.filter(|value| !value.is_empty()),
        desktop_file: path.to_path_buf(),
    })
}

fn is_true(value: &str) -> bool {
    value.eq_ignore_ascii_case("true")
}

fn locale_matches(key: &str) -> bool {
    let Some(locale) = key
        .strip_prefix("Name[")
        .or_else(|| key.strip_prefix("GenericName["))
    else {
        return false;
    };
    let locale = locale.trim_end_matches(']');
    let current = std::env::var("LC_MESSAGES")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let current = current.split('.').next().unwrap_or_default();
    locale.eq_ignore_ascii_case(current)
        || locale
            .split('_')
            .next()
            .is_some_and(|language| current.starts_with(language))
}

fn unescape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            output.push(match character {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                _ => character,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            output.push(character);
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_basic_desktop_entry() {
        let path = std::env::temp_dir().join(format!("alter-test-{}.desktop", std::process::id()));
        let mut file = fs::File::create(&path).unwrap();
        writeln!(
            file,
            "[Desktop Entry]\nType=Application\nName=Test\\sApp\nIcon=test-icon\nNoDisplay=false\nExec=test"
        )
        .unwrap();
        let app = parse_desktop_file(&path, "test.desktop".to_owned()).unwrap();
        assert_eq!(app.name, "Test App");
        assert_eq!(app.icon.as_deref(), Some("test-icon"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn user_desktop_entry_overrides_system_entry_with_same_id() {
        let test_root = std::env::temp_dir().join(format!(
            "alter-desktop-priority-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let user_root = test_root.join("user");
        let system_root = test_root.join("system");
        fs::create_dir_all(&user_root).unwrap();
        fs::create_dir_all(&system_root).unwrap();
        fs::write(
            user_root.join("example.desktop"),
            "[Desktop Entry]\nType=Application\nName=User Entry\nExec=user-command\n",
        )
        .unwrap();
        fs::write(
            system_root.join("example.desktop"),
            "[Desktop Entry]\nType=Application\nName=System Entry\nExec=system-command\n",
        )
        .unwrap();

        let apps = load_applications_from_roots([user_root.clone(), system_root]);

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "User Entry");
        assert_eq!(apps[0].desktop_file, user_root.join("example.desktop"));
        let _ = fs::remove_dir_all(test_root);
    }

    #[test]
    fn discovers_windows_start_menu_shortcuts() {
        let test_root = std::env::temp_dir().join(format!(
            "alter-windows-apps-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let development = test_root.join("Development");
        fs::create_dir_all(&development).unwrap();
        fs::write(development.join("Visual Studio Code.lnk"), []).unwrap();
        fs::write(test_root.join("Documentation.txt"), []).unwrap();

        let apps = load_windows_applications([test_root.clone()]);

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Visual Studio Code");
        assert!(apps[0].keywords.iter().any(|value| value == "Development"));
        assert_eq!(apps[0].id, "Development-Visual Studio Code.lnk");
        let _ = fs::remove_dir_all(test_root);
    }

    #[test]
    fn catalog_reloads_when_desktop_entries_are_installed_updated_or_removed() {
        let test_root = std::env::temp_dir().join(format!(
            "alter-desktop-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let user_root = test_root.join("user-applications");
        let system_root = test_root.join("system-applications");
        // Leave `user_root` initially absent to verify that creating `~/.local/share/applications`
        // after the daemon starts is detected too.
        fs::create_dir_all(&system_root).unwrap();

        let catalog = AppCatalog::from_desktop_roots([user_root.clone(), system_root.clone()]);
        assert!(catalog.snapshot().is_empty());

        let t3_path = system_root.join("t3code.desktop");
        fs::write(
            &t3_path,
            "[Desktop Entry]\nType=Application\nName=T3 Code\nExec=t3code %U\n",
        )
        .unwrap();
        catalog.schedule_check();
        let after_install = catalog.snapshot();
        assert_eq!(after_install.len(), 1);
        assert_eq!(after_install[0].name, "T3 Code");

        // Unchanged directories reuse the exact same Arc allocation.
        catalog.schedule_check();
        let unchanged = catalog.snapshot();
        assert!(Arc::ptr_eq(&after_install, &unchanged));

        // Creating the previously missing user directory overrides the system entry.
        fs::create_dir_all(&user_root).unwrap();
        fs::write(
            user_root.join("t3code.desktop"),
            "[Desktop Entry]\nType=Application\nName=T3 Code Custom\nExec=t3code\n",
        )
        .unwrap();
        catalog.schedule_check();
        let after_override = catalog.snapshot();
        assert_eq!(after_override.len(), 1);
        assert_eq!(after_override[0].name, "T3 Code Custom");

        fs::remove_file(user_root.join("t3code.desktop")).unwrap();
        fs::remove_file(&t3_path).unwrap();
        catalog.schedule_check();
        assert!(catalog.snapshot().is_empty());

        let _ = fs::remove_dir_all(test_root);
    }

    #[test]
    fn windows_catalog_reloads_when_shortcuts_change() {
        let test_root = std::env::temp_dir().join(format!(
            "alter-windows-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&test_root).unwrap();

        let catalog = AppCatalog::from_windows_roots([test_root.clone()]);
        assert!(catalog.snapshot().is_empty());

        fs::write(test_root.join("T3 Code.lnk"), []).unwrap();
        catalog.schedule_check();
        let apps = catalog.snapshot();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "T3 Code");

        let _ = fs::remove_dir_all(test_root);
    }
}
