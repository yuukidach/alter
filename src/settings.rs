use crate::paths;
use std::fs;
use std::io;
use std::sync::{Arc, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub file_search: bool,
    pub clipboard_search: bool,
    pub web_search: bool,
    pub web_suggestions: bool,
    pub workflow_search: bool,
    pub snippet_search: bool,
    pub learning_ranking: bool,
    pub show_recent: bool,
    pub max_results: usize,
    pub clipboard_retention_days: u32,
    pub theme: Theme,
}

pub type SharedSettings = Arc<RwLock<Settings>>;

impl Default for Settings {
    fn default() -> Self {
        Self {
            file_search: true,
            clipboard_search: true,
            web_search: true,
            web_suggestions: true,
            workflow_search: true,
            snippet_search: true,
            learning_ranking: true,
            show_recent: true,
            max_results: 40,
            clipboard_retention_days: 30,
            theme: Theme::Dark,
        }
    }
}

impl Settings {
    pub fn clamp(&mut self) {
        self.max_results = self.max_results.clamp(10, 100);
        self.clipboard_retention_days = self.clipboard_retention_days.clamp(1, 3650);
    }
}

pub fn load() -> Settings {
    let path = paths::settings_path();
    let Ok(content) = fs::read_to_string(path) else {
        return Settings::default();
    };
    parse(&content)
}

pub fn shared(settings: Settings) -> SharedSettings {
    Arc::new(RwLock::new(settings))
}

pub fn snapshot(settings: &SharedSettings) -> Settings {
    settings
        .read()
        .map(|value| value.clone())
        .unwrap_or_else(|_| Settings::default())
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let path = paths::settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut value = settings.clone();
    value.clamp();
    let content = format!(
        "# Alter preferences\nfile_search={}\nclipboard_search={}\nweb_search={}\nweb_suggestions={}\nworkflow_search={}\nsnippet_search={}\nlearning_ranking={}\nshow_recent={}\nmax_results={}\nclipboard_retention_days={}\ntheme={}\n",
        value.file_search,
        value.clipboard_search,
        value.web_search,
        value.web_suggestions,
        value.workflow_search,
        value.snippet_search,
        value.learning_ranking,
        value.show_recent,
        value.max_results,
        value.clipboard_retention_days,
        value.theme.as_str()
    );
    let temporary = path.with_extension("conf.tmp");
    fs::write(&temporary, content)?;
    fs::rename(temporary, path)
}

fn parse(content: &str) -> Settings {
    let mut settings = Settings::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "file_search" => settings.file_search = parse_bool(value, settings.file_search),
            "clipboard_search" => {
                settings.clipboard_search = parse_bool(value, settings.clipboard_search)
            }
            "web_search" => settings.web_search = parse_bool(value, settings.web_search),
            "web_suggestions" => {
                settings.web_suggestions = parse_bool(value, settings.web_suggestions)
            }
            "workflow_search" => {
                settings.workflow_search = parse_bool(value, settings.workflow_search)
            }
            "snippet_search" => {
                settings.snippet_search = parse_bool(value, settings.snippet_search)
            }
            "learning_ranking" => {
                settings.learning_ranking = parse_bool(value, settings.learning_ranking)
            }
            "show_recent" => settings.show_recent = parse_bool(value, settings.show_recent),
            "max_results" => {
                if let Ok(number) = value.parse() {
                    settings.max_results = number;
                }
            }
            "clipboard_retention_days" => {
                if let Ok(number) = value.parse() {
                    settings.clipboard_retention_days = number;
                }
            }
            "theme" => {
                settings.theme = match value {
                    "light" => Theme::Light,
                    _ => Theme::Dark,
                }
            }
            _ => {}
        }
    }
    settings.clamp();
    settings
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value {
        "true" | "1" | "yes" => true,
        "false" | "0" | "no" => false,
        _ => fallback,
    }
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_clamps_preferences() {
        let settings = parse(
            "file_search=false\nclipboard_search=true\nweb_search=false\nweb_suggestions=0\nworkflow_search=no\nsnippet_search=false\nlearning_ranking=1\nshow_recent=no\nmax_results=1000\nclipboard_retention_days=9999\ntheme=light\n",
        );
        assert!(!settings.file_search);
        assert!(settings.clipboard_search);
        assert!(!settings.web_search);
        assert!(!settings.web_suggestions);
        assert!(!settings.workflow_search);
        assert!(!settings.snippet_search);
        assert!(settings.learning_ranking);
        assert!(!settings.show_recent);
        assert_eq!(settings.max_results, 100);
        assert_eq!(settings.clipboard_retention_days, 3650);
        assert_eq!(settings.theme, Theme::Light);
    }

    #[test]
    fn defaults_to_thirty_days_of_clipboard_history() {
        assert_eq!(Settings::default().clipboard_retention_days, 30);
    }

    #[test]
    fn enables_new_search_features_for_legacy_configuration() {
        let settings =
            parse("file_search=false\nclipboard_search=false\nshow_recent=false\nmax_results=20\n");

        assert!(settings.web_search);
        assert!(settings.web_suggestions);
        assert!(settings.workflow_search);
        assert!(settings.snippet_search);
        assert!(settings.learning_ranking);
    }

    #[test]
    fn invalid_feature_switches_keep_their_defaults() {
        let settings = parse(
            "web_search=maybe\nweb_suggestions=enabled\nworkflow_search=invalid\nsnippet_search=perhaps\nlearning_ranking=unknown\n",
        );

        assert!(settings.web_search);
        assert!(settings.web_suggestions);
        assert!(settings.workflow_search);
        assert!(settings.snippet_search);
        assert!(settings.learning_ranking);
    }
}
