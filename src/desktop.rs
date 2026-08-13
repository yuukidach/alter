use crate::paths;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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

pub fn load_applications() -> Vec<AppItem> {
    load_applications_from_roots(application_roots())
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

fn desktop_id(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('/', "-"))
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
}
