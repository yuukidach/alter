//! Small, dependency-free localization helpers for the launcher UI.
//!
//! Alter currently ships two complete UI languages.  User-provided names,
//! workflow labels and clipboard contents are intentionally left untouched;
//! only strings owned by Alter are translated.

use crate::search::{ResultKind, SearchResult};
use std::env;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Chinese,
    English,
}

impl Language {
    pub fn text(self, chinese: &'static str, english: &'static str) -> &'static str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguagePreference {
    System,
    Chinese,
    English,
}

impl Default for LanguagePreference {
    fn default() -> Self {
        Self::System
    }
}

impl LanguagePreference {
    pub fn effective(self) -> Language {
        match self {
            Self::System => detect_system_language(),
            Self::Chinese => Language::Chinese,
            Self::English => Language::English,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_cn" | "chinese" => Self::Chinese,
            "en" | "en-us" | "en_us" | "english" => Self::English,
            _ => Self::System,
        }
    }

    pub fn selected_index(self) -> u32 {
        match self {
            Self::System => 0,
            Self::Chinese => 1,
            Self::English => 2,
        }
    }

    pub fn from_selected_index(index: u32) -> Self {
        match index {
            1 => Self::Chinese,
            2 => Self::English,
            _ => Self::System,
        }
    }
}

pub fn detect_system_language() -> Language {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .to_ascii_lowercase();

    if locale.starts_with("en") {
        Language::English
    } else if locale.starts_with("zh") {
        Language::Chinese
    } else {
        // Keep the historical Chinese UI for an unset/C locale.  Users can
        // choose English explicitly in Settings when desired.
        Language::Chinese
    }
}

pub fn result_kind_label(kind: &ResultKind, language: Language) -> &'static str {
    match kind {
        ResultKind::App => language.text("应用", "App"),
        ResultKind::File => language.text("文件", "File"),
        ResultKind::Clipboard => language.text("剪贴板", "Clipboard"),
        ResultKind::Calculation => language.text("计算", "Calc"),
        ResultKind::Settings => language.text("设置", "Settings"),
        ResultKind::Web => language.text("网页", "Web"),
        ResultKind::Workflow => language.text("工作流", "Workflow"),
        ResultKind::Snippet => language.text("片段", "Snippet"),
    }
}

/// Translate the built-in metadata attached to a search result.  Titles and
/// subtitles supplied by desktop files, workflows and snippets remain as the
/// user authored them.
pub fn localized_result(result: &SearchResult, language: Language) -> SearchResult {
    if language == Language::Chinese {
        return result.clone();
    }

    let mut localized = result.clone();
    match result.kind {
        ResultKind::App => {
            if result.subtitle == "应用程序" {
                localized.subtitle = "Application".to_owned();
            }
        }
        ResultKind::Clipboard => {
            localized.subtitle = clipboard_subtitle_in_english(&result.subtitle);
        }
        ResultKind::Calculation => {
            localized.subtitle = "Calculator · Enter to copy".to_owned();
        }
        ResultKind::Settings => {
            localized.title = "Alter Settings".to_owned();
            localized.subtitle = "Open preferences · Ctrl+,".to_owned();
        }
        ResultKind::Web => {
            localized.title = result
                .title
                .strip_prefix("搜索：")
                .map(|query| format!("Search: {query}"))
                .unwrap_or_else(|| result.title.clone());
            localized.subtitle = result
                .subtitle
                .strip_suffix(" · 在默认浏览器中打开")
                .map(|provider| format!("{provider} · Open in default browser"))
                .unwrap_or_else(|| result.subtitle.clone());
        }
        ResultKind::Workflow => {
            localized.subtitle = workflow_subtitle_in_english(&result.subtitle);
        }
        ResultKind::Snippet => {
            localized.subtitle = snippet_subtitle_in_english(&result.subtitle);
        }
        ResultKind::File => {}
    }
    localized
}

fn clipboard_subtitle_in_english(value: &str) -> String {
    let value = value
        .replace("图片剪贴板", "Image clipboard")
        .replace("文件剪贴板", "File clipboard")
        .replace("剪贴板 ·", "Clipboard ·")
        .replace("Clipse 历史", "Clipse history");
    value
        .strip_suffix("刚刚")
        .map(|prefix| format!("{prefix}just now"))
        .or_else(|| {
            value
                .strip_suffix(" 分钟前")
                .map(|prefix| format!("{prefix} min ago"))
        })
        .or_else(|| {
            value
                .strip_suffix(" 小时前")
                .map(|prefix| format!("{prefix} hr ago"))
        })
        .or_else(|| {
            value
                .strip_suffix(" 天前")
                .map(|prefix| format!("{prefix} days ago"))
        })
        .unwrap_or(value)
}

fn workflow_subtitle_in_english(value: &str) -> String {
    let value = value.replace("关键词", "keyword");
    value
        .strip_suffix(" · Enter 执行")
        .map(|prefix| format!("{prefix} · Enter to run"))
        .unwrap_or(value)
}

fn snippet_subtitle_in_english(value: &str) -> String {
    let value = value.replace("关键词", "keyword");
    value
        .strip_suffix(" · Enter 复制")
        .map(|prefix| format!("{prefix} · Enter to copy"))
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_language_preferences() {
        assert_eq!(
            LanguagePreference::parse("zh-CN"),
            LanguagePreference::Chinese
        );
        assert_eq!(
            LanguagePreference::parse("EN_us"),
            LanguagePreference::English
        );
        assert_eq!(
            LanguagePreference::parse("unknown"),
            LanguagePreference::System
        );
    }

    #[test]
    fn translates_builtin_result_metadata_without_touching_content() {
        let result = SearchResult::settings(1).result;
        let translated = localized_result(&result, Language::English);
        assert_eq!(translated.title, "Alter Settings");
        assert_eq!(translated.subtitle, "Open preferences · Ctrl+,");
    }
}
