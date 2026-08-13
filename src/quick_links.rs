//! Alfred-style keyword URL shortcuts.
//!
//! A quick link is a small, user-owned URL template.  Typing a keyword and a
//! value (for example `job j-056rekk80h`) creates a browser action with the
//! value safely percent-encoded into `{query}`.  Quick links are deliberately
//! matched only by an explicit keyword; they never appear in ordinary fuzzy
//! search results.

use crate::paths;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const QUERY_PLACEHOLDER: &str = "{query}";
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
const MAX_KEYWORDS: usize = 32;
const MAX_KEYWORD_BYTES: usize = 128;
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;
const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_EXPANDED_URL_BYTES: usize = 64 * 1024;

/// JSON representation accepted in `quick-links.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct QuickLinkManifest {
    /// If omitted, Alter derives an id from the name or first keyword.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, alias = "title")]
    pub name: String,
    #[serde(
        default,
        alias = "keyword",
        deserialize_with = "deserialize_string_or_strings"
    )]
    pub keywords: Vec<String>,
    #[serde(alias = "url", alias = "template", alias = "urlTemplate")]
    pub url_template: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

fn deserialize_string_or_strings<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(value) => Ok(vec![value]),
        OneOrMany::Many(values) => Ok(values),
    }
}

/// A validated quick link ready to match and execute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLink {
    pub id: String,
    pub name: String,
    pub keywords: Vec<String>,
    pub url_template: String,
    pub enabled: bool,
}

impl QuickLink {
    /// Validate and normalize one manifest.
    pub fn from_manifest(manifest: QuickLinkManifest) -> Result<Self, QuickLinkError> {
        let mut id = manifest
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_default();
        let mut name = manifest.name.trim().to_owned();

        let fallback = if !name.is_empty() {
            name.clone()
        } else {
            manifest.keywords.first().cloned().unwrap_or_default()
        };
        if id.is_empty() {
            id = slugify(&fallback);
            if id.is_empty() {
                id = manifest
                    .keywords
                    .first()
                    .map(|keyword| slugify(keyword))
                    .unwrap_or_default();
            }
        }
        if id.is_empty() {
            return Err(QuickLinkError::Invalid(
                "quick link needs a name, keyword, or id".to_owned(),
            ));
        }
        if !valid_id(&id) {
            return Err(QuickLinkError::Invalid(
                "id must contain only letters, numbers, '-' or '_' and be at most 128 bytes"
                    .to_owned(),
            ));
        }

        if name.is_empty() {
            name = id.clone();
        }
        if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
            return Err(QuickLinkError::Invalid(
                "name contains control characters or is too long".to_owned(),
            ));
        }

        let mut keywords = Vec::new();
        for keyword in manifest.keywords.into_iter().take(MAX_KEYWORDS) {
            let keyword = keyword.trim();
            if !valid_keyword(keyword) {
                continue;
            }
            if !keywords
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(keyword))
            {
                keywords.push(keyword.to_owned());
            }
        }
        if keywords.is_empty() {
            return Err(QuickLinkError::Invalid(
                "at least one keyword is required".to_owned(),
            ));
        }
        keywords.sort_by_key(|keyword| std::cmp::Reverse(keyword.chars().count()));

        let url_template = manifest.url_template.trim().to_owned();
        validate_template(&url_template)
            .map_err(|message| QuickLinkError::Invalid(message.into()))?;

        Ok(Self {
            id: id.to_ascii_lowercase(),
            name,
            keywords,
            url_template,
            enabled: manifest.enabled,
        })
    }

    /// Construct a link from the compact settings form.  `keywords` may be
    /// separated by commas, spaces, or newlines.
    pub fn from_form(
        id: Option<&str>,
        name: &str,
        keywords: &str,
        url_template: &str,
    ) -> Result<Self, QuickLinkError> {
        let keywords = keywords
            .split(|character: char| character == ',' || character.is_whitespace())
            .filter(|value| !value.trim().is_empty())
            .map(str::trim)
            .map(str::to_owned)
            .collect();
        Self::from_manifest(QuickLinkManifest {
            id: id.map(str::to_owned),
            name: name.to_owned(),
            keywords,
            url_template: url_template.to_owned(),
            enabled: true,
        })
    }

    pub fn manifest(&self) -> QuickLinkManifest {
        QuickLinkManifest {
            id: Some(self.id.clone()),
            name: self.name.clone(),
            keywords: self.keywords.clone(),
            url_template: self.url_template.clone(),
            enabled: self.enabled,
        }
    }

    /// Match an explicit Alfred-style invocation such as `cj j-123`.
    pub fn match_query(&self, raw_query: &str) -> Option<QuickLinkMatch> {
        if !self.enabled {
            return None;
        }
        let trimmed = raw_query.trim();
        if trimmed.is_empty()
            || trimmed.len() > MAX_QUERY_BYTES
            || trimmed.contains('\0')
            || trimmed.chars().any(char::is_control)
        {
            return None;
        }

        for keyword in &self.keywords {
            let Some(suffix) = strip_prefix_case_insensitive(trimmed, keyword) else {
                continue;
            };
            if !suffix.is_empty() && !suffix.chars().next().is_some_and(char::is_whitespace) {
                continue;
            }
            let query = suffix.trim();
            if query.is_empty() {
                continue;
            }
            let action = self.action(query)?;
            return Some(QuickLinkMatch {
                link: self.clone(),
                keyword: keyword.clone(),
                query: query.to_owned(),
                action,
                score: 2_200 + keyword.chars().count() as i64 * 8,
            });
        }
        None
    }

    pub fn action(&self, query: &str) -> Option<QuickLinkAction> {
        let query = query.trim();
        if query.is_empty()
            || query.len() > MAX_QUERY_BYTES
            || query.contains('\0')
            || query.chars().any(char::is_control)
        {
            return None;
        }
        Some(QuickLinkAction {
            link_id: self.id.clone(),
            name: self.name.clone(),
            query: query.to_owned(),
            url: expand_template(&self.url_template, query)?,
        })
    }
}

/// A parsed explicit invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLinkMatch {
    pub link: QuickLink,
    pub keyword: String,
    pub query: String,
    pub action: QuickLinkAction,
    pub score: i64,
}

/// A fully expanded browser action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLinkAction {
    pub link_id: String,
    pub name: String,
    pub query: String,
    pub url: String,
}

impl QuickLinkAction {
    pub fn open(&self) -> io::Result<()> {
        Command::new("xdg-open")
            .arg(&self.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
}

/// A collection of configured quick links.
#[derive(Clone, Debug, Default)]
pub struct QuickLinkCatalog {
    links: Vec<QuickLink>,
}

impl QuickLinkCatalog {
    pub fn new(mut links: Vec<QuickLink>) -> Self {
        links.sort_by_cached_key(|link| link.name.to_lowercase());
        Self { links }
    }

    pub fn links(&self) -> &[QuickLink] {
        &self.links
    }

    pub fn matching(&self, raw_query: &str) -> Vec<QuickLinkMatch> {
        let mut matches: Vec<_> = self
            .links
            .iter()
            .filter_map(|link| link.match_query(raw_query))
            .collect();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.link.name.cmp(&right.link.name))
        });
        matches
    }

    pub fn has_keyword(&self, raw_query: &str) -> bool {
        let trimmed = raw_query.trim();
        self.links.iter().any(|link| {
            link.enabled
                && link.keywords.iter().any(|keyword| {
                    strip_prefix_case_insensitive(trimmed, keyword).is_some_and(|suffix| {
                        suffix.is_empty() || suffix.chars().next().is_some_and(char::is_whitespace)
                    })
                })
        })
    }

    pub fn prompt_for_keyword(&self, raw_query: &str) -> Option<QuickLinkPrompt> {
        let trimmed = raw_query.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut candidates: Vec<_> = self
            .links
            .iter()
            .filter(|link| link.enabled)
            .flat_map(|link| {
                link.keywords.iter().filter_map(move |keyword| {
                    strip_prefix_case_insensitive(trimmed, keyword).and_then(|suffix| {
                        suffix.is_empty().then(|| QuickLinkPrompt {
                            link_id: link.id.clone(),
                            name: link.name.clone(),
                            keyword: keyword.clone(),
                        })
                    })
                })
            })
            .collect();
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.keyword.chars().count()));
        candidates.into_iter().next()
    }

    pub fn load() -> Self {
        load_report().catalog()
    }
}

#[derive(Clone, Debug, Default)]
pub struct QuickLinkLoadReport {
    pub links: Vec<QuickLink>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLinkPrompt {
    pub link_id: String,
    pub name: String,
    pub keyword: String,
}

impl QuickLinkLoadReport {
    pub fn catalog(self) -> QuickLinkCatalog {
        QuickLinkCatalog::new(self.links)
    }
}

pub fn config_path() -> PathBuf {
    paths::config_dir().join("quick-links.json")
}

pub fn load_report() -> QuickLinkLoadReport {
    let path = config_path();
    let content = match read_bounded_text(&path, MAX_FILE_BYTES) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return QuickLinkLoadReport::default();
        }
        Err(error) => {
            return QuickLinkLoadReport {
                links: Vec::new(),
                errors: vec![format!("{}: {error}", path.display())],
            };
        }
    };
    match parse_content(&content) {
        Ok(mut report) => {
            for error in &mut report.errors {
                *error = format!("{}: {error}", path.display());
            }
            report
        }
        Err(error) => QuickLinkLoadReport {
            links: Vec::new(),
            errors: vec![format!("{}: {error}", path.display())],
        },
    }
}

pub fn save(links: &[QuickLink]) -> io::Result<()> {
    save_to(&config_path(), links)
}

fn save_to(path: &Path, links: &[QuickLink]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let manifests: Vec<_> = links.iter().map(QuickLink::manifest).collect();
    let content = serde_json::to_string_pretty(&manifests).map_err(io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, format!("{content}\n"))?;
    fs::rename(temporary, path)
}

fn parse_content(content: &str) -> Result<QuickLinkLoadReport, String> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|error| format!("invalid quick-links JSON: {error}"))?;
    let values = if let Some(array) = value.as_array() {
        array.clone()
    } else if let Some(array) = value
        .get("quick_links")
        .and_then(serde_json::Value::as_array)
    {
        array.clone()
    } else {
        return Err("quick-links config must be a JSON array".to_owned());
    };

    let mut report = QuickLinkLoadReport::default();
    for (index, value) in values.into_iter().enumerate() {
        match serde_json::from_value::<QuickLinkManifest>(value)
            .map_err(|error| QuickLinkError::Invalid(error.to_string()))
            .and_then(QuickLink::from_manifest)
        {
            Ok(link) => report.links.push(link),
            Err(error) => report.errors.push(format!("entry {}: {error}", index + 1)),
        }
    }
    Ok(report)
}

fn read_bounded_text(path: &Path, max_bytes: u64) -> io::Result<String> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("file exceeds {max_bytes} bytes"),
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn encode_query_component(query: &str) -> String {
    let mut encoded = String::with_capacity(query.len());
    for byte in query.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn expand_template(template: &str, query: &str) -> Option<String> {
    validate_template(template).ok()?;
    let occurrences = template.match_indices(QUERY_PLACEHOLDER).count();
    let removed = occurrences.checked_mul(QUERY_PLACEHOLDER.len())?;
    let base = template.len().checked_sub(removed)?;
    let encoded = encode_query_component(query);
    let added = occurrences.checked_mul(encoded.len())?;
    let total = base.checked_add(added)?;
    (total <= MAX_EXPANDED_URL_BYTES).then(|| template.replace(QUERY_PLACEHOLDER, &encoded))
}

fn validate_template(template: &str) -> Result<(), &'static str> {
    if template.len() > MAX_TEMPLATE_BYTES {
        return Err("URL template is too long");
    }
    let Some((scheme, authority_and_path)) = template.split_once("://") else {
        return Err("URL template must use http or https");
    };
    if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
        return Err("URL template must use http or https");
    }
    if !template.contains(QUERY_PLACEHOLDER) {
        return Err("URL template must contain {query}");
    }
    if template
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("URL template cannot contain whitespace or control characters");
    }
    let authority_end = authority_and_path
        .find(['/', '?', '#'])
        .unwrap_or(authority_and_path.len());
    let authority = &authority_and_path[..authority_end];
    if authority.is_empty() || authority.contains(QUERY_PLACEHOLDER) {
        return Err("URL template must contain a static host");
    }
    if authority
        .chars()
        .any(|character| matches!(character, '\\' | '"' | '\''))
    {
        return Err("URL template host contains an unsafe character");
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn valid_keyword(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEYWORD_BYTES
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
        && !value.contains('|')
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if matches!(character, '-' | '_') {
            if !slug.ends_with('-') {
                slug.push(character);
            }
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= MAX_ID_BYTES {
            break;
        }
    }
    slug.trim_matches('-').to_owned()
}

fn strip_prefix_case_insensitive<'a>(value: &'a str, keyword: &str) -> Option<&'a str> {
    let character_count = keyword.chars().count();
    let end = value
        .char_indices()
        .nth(character_count)
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    let prefix = value.get(..end)?;
    if prefix.eq_ignore_ascii_case(keyword) || prefix.to_lowercase() == keyword.to_lowercase() {
        value.get(end..)
    } else {
        None
    }
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'A' + (value - 10)),
        _ => unreachable!(),
    }
}

impl fmt::Display for QuickLinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickLinkError {
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> QuickLink {
        QuickLink::from_form(
            Some("example-jobs"),
            "Example Jobs",
            "job jobs",
            "https://jobs.example.com/detail?job_id={query}",
        )
        .unwrap()
    }

    #[test]
    fn expands_job_id_and_encodes_query() {
        let link = link();
        let matched = link.match_query("job j-056rekk80h").unwrap();
        assert_eq!(
            matched.action.url,
            "https://jobs.example.com/detail?job_id=j-056rekk80h"
        );
        let encoded = link.action("中文 id").unwrap();
        assert_eq!(
            encoded.url,
            "https://jobs.example.com/detail?job_id=%E4%B8%AD%E6%96%87%20id"
        );
    }

    #[test]
    fn only_explicit_keyword_invocations_match() {
        let link = link();
        assert!(link.match_query("job value").is_some());
        assert!(link.match_query("JOBS value").is_some());
        assert!(link.match_query("jobsite value").is_none());
        assert!(link.match_query("jobs").is_none());
        assert!(link.match_query("job").is_none());
        let catalog = QuickLinkCatalog::new(vec![link]);
        assert!(catalog.has_keyword("job"));
        assert!(!catalog.has_keyword("jobsite"));
        let prompt = catalog.prompt_for_keyword("job").unwrap();
        assert_eq!(prompt.name, "Example Jobs");
        assert_eq!(prompt.keyword, "job");
        assert!(catalog.prompt_for_keyword("job value").is_none());
    }

    #[test]
    fn rejects_unsafe_or_incomplete_templates() {
        assert!(QuickLink::from_form(Some("x"), "x", "x", "javascript:alert({query})").is_err());
        assert!(QuickLink::from_form(Some("x"), "x", "x", "https://example.test/path").is_err());
        assert!(
            QuickLink::from_form(Some("x"), "x", "x", "https://{query}.example.test/").is_err()
        );
    }

    #[test]
    fn parses_array_and_object_wrapped_configs() {
        let json = r#"[{"name":"Jobs","keywords":"cj","url":"https://example.test/?id={query}"}]"#;
        assert_eq!(parse_content(json).unwrap().links.len(), 1);
        let wrapped = r#"{"quick_links":[{"name":"Jobs","keywords":["cj"],"url_template":"https://example.test/?id={query}"}]}"#;
        assert_eq!(parse_content(wrapped).unwrap().links.len(), 1);
    }

    #[test]
    fn serializes_and_round_trips_manifests() {
        let original = vec![link()];
        let json =
            serde_json::to_string(&original.iter().map(QuickLink::manifest).collect::<Vec<_>>())
                .unwrap();
        let parsed = parse_content(&json).unwrap().catalog();
        assert_eq!(parsed.links(), original.as_slice());
    }

    #[test]
    fn saves_and_loads_the_settings_format() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("alter-quick-links-{}-{nonce}", std::process::id()));
        let path = directory.join("quick-links.json");
        let original = vec![link()];
        save_to(&path, &original).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let loaded = parse_content(&content).unwrap().catalog();
        assert_eq!(loaded.links(), original.as_slice());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn derives_an_id_from_keyword_for_non_ascii_names() {
        let link = QuickLink::from_form(
            None,
            "工单详情",
            "ticket",
            "https://example.test/tickets/{query}",
        )
        .unwrap();
        assert_eq!(link.id, "ticket");
        assert_eq!(link.name, "工单详情");
    }
}
