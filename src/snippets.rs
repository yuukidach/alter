//! User-defined text snippets.
//!
//! Snippets are intentionally a data-only backend.  The catalog validates and
//! searches user content, while the UI decides whether selecting a result
//! copies it to the Wayland clipboard (or asks an input method to expand it).
//! No shell command or key injection is performed here.

use crate::paths;
use serde::Deserialize;
use serde::de::Deserializer;
use std::cmp::Reverse;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
const MAX_KEYWORDS: usize = 32;
const MAX_KEYWORD_BYTES: usize = 128;
const MAX_QUERY_BYTES: usize = 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 8 * 1024 * 1024;
const QUERY_PLACEHOLDER: &str = "{query}";

/// JSON representation accepted in `snippets.json`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SnippetManifest {
    /// If omitted, Alter derives an id from the name or source filename.
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
    #[serde(alias = "text", alias = "value")]
    pub content: String,
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

/// A validated snippet ready to display or copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub enabled: bool,
    /// The source file is useful for settings UIs and diagnostics.
    pub source: PathBuf,
}

impl Snippet {
    /// Validate and normalize one manifest.
    pub fn from_manifest(
        manifest: SnippetManifest,
        source: impl Into<PathBuf>,
    ) -> Result<Self, SnippetError> {
        let source = source.into();
        let source_for_error = source.clone();

        let mut id = manifest
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_default();
        let mut name = manifest.name.trim().to_owned();
        if id.is_empty() {
            let fallback = if name.is_empty() {
                source
                    .file_stem()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                name.clone()
            };
            id = slugify(&fallback);
        }
        if id.is_empty() {
            return Err(SnippetError::invalid(
                source_for_error,
                "snippet id and name cannot both be empty",
            ));
        }
        if !valid_id(&id) {
            return Err(SnippetError::invalid(
                source_for_error.clone(),
                "snippet id must contain only letters, numbers, '-' or '_' and be at most 128 bytes",
            ));
        }

        if name.is_empty() {
            name = id.clone();
        }
        if !valid_name(&name) {
            return Err(SnippetError::invalid(
                source_for_error.clone(),
                "snippet name is empty, contains a control character, or is too long",
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
            keywords.push(id.clone());
        }
        keywords.sort_by_key(|keyword| Reverse(keyword.chars().count()));

        if !valid_content(&manifest.content) {
            return Err(SnippetError::invalid(
                source_for_error,
                "snippet content must be non-empty, contain no NUL, and be at most 64 KiB",
            ));
        }

        Ok(Self {
            id: id.to_ascii_lowercase(),
            name,
            keywords,
            content: manifest.content,
            enabled: manifest.enabled,
            source,
        })
    }

    /// Parse and validate one JSON manifest.
    pub fn from_json(content: &str, source: impl Into<PathBuf>) -> Result<Self, SnippetError> {
        let source = source.into();
        let manifest: SnippetManifest = serde_json::from_str(content)
            .map_err(|error| SnippetError::parse(source.clone(), error.to_string()))?;
        Self::from_manifest(manifest, source)
    }

    /// Text used for metadata search.
    pub fn search_text(&self) -> String {
        let mut text = format!("{} {}", self.name, self.id);
        for keyword in &self.keywords {
            text.push(' ');
            text.push_str(keyword);
        }
        text
    }

    /// Match an Alfred-style trigger (`sig` or `sig optional words`).
    pub fn match_query(&self, raw_query: &str) -> Option<SnippetMatch> {
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

        let mut keywords: Vec<&str> = self.keywords.iter().map(String::as_str).collect();
        keywords.sort_by_key(|keyword| Reverse(keyword.chars().count()));
        for keyword in keywords {
            let Some(suffix) = strip_prefix_case_insensitive(trimmed, keyword) else {
                continue;
            };
            if !suffix.is_empty() && !suffix.chars().next().is_some_and(char::is_whitespace) {
                continue;
            }
            let query = suffix.trim().to_owned();
            return Some(SnippetMatch {
                snippet: self.clone(),
                keyword: keyword.to_owned(),
                query,
                score: 1_000 + keyword.chars().count() as i64 * 4,
            });
        }
        None
    }

    /// Expand the optional `{query}` placeholder without invoking a shell.
    pub fn expand(&self, query: &str) -> Option<String> {
        if query.len() > MAX_QUERY_BYTES || query.contains('\0') {
            return None;
        }
        substitute_query_bounded(&self.content, query, MAX_EXPANDED_BYTES)
    }
}

/// A snippet selected by a trigger keyword.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetMatch {
    pub snippet: Snippet,
    pub keyword: String,
    /// Text after the trigger keyword.
    pub query: String,
    pub score: i64,
}

impl SnippetMatch {
    pub fn expanded_content(&self) -> Option<String> {
        self.snippet.expand(&self.query)
    }
}

/// Loaded snippet collection.
#[derive(Clone, Debug, Default)]
pub struct SnippetCatalog {
    snippets: Vec<Snippet>,
}

impl SnippetCatalog {
    pub fn new(mut snippets: Vec<Snippet>) -> Self {
        snippets.sort_by_cached_key(|snippet| snippet.name.to_lowercase());
        Self { snippets }
    }

    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    pub fn is_empty(&self) -> bool {
        self.snippets.is_empty()
    }

    /// Return trigger matches, sorted by the most specific keyword first.
    pub fn matching(&self, query: &str) -> Vec<SnippetMatch> {
        let mut matches: Vec<_> = self
            .snippets
            .iter()
            .filter_map(|snippet| snippet.match_query(query))
            .collect();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.snippet.name.cmp(&right.snippet.name))
        });
        matches
    }

    /// Search names, ids and trigger words.  Trigger invocations are given
    /// priority so typing `sig hello` immediately selects the expansion.
    pub fn search(&self, query: &str) -> Vec<SnippetMatch> {
        let invocation = self.matching(query);
        if !invocation.is_empty() {
            return invocation;
        }
        let query = query.trim();
        if query.is_empty()
            || query.len() > MAX_QUERY_BYTES
            || query.contains('\0')
            || query.chars().any(char::is_control)
        {
            return Vec::new();
        }
        let query = query.to_lowercase();
        let mut matches: Vec<_> = self
            .snippets
            .iter()
            .filter(|snippet| snippet.enabled)
            .filter_map(|snippet| {
                let text = snippet.search_text().to_lowercase();
                let position = text.find(&query)?;
                Some(SnippetMatch {
                    snippet: snippet.clone(),
                    keyword: snippet.keywords.first().cloned().unwrap_or_default(),
                    query: String::new(),
                    score: 400 - position.min(300) as i64,
                })
            })
            .collect();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.snippet.name.cmp(&right.snippet.name))
        });
        matches
    }

    /// Alias convenient for launcher integrations.
    pub fn candidates(&self, query: &str) -> Vec<SnippetMatch> {
        self.search(query)
    }
}

/// Diagnostics returned while loading snippets.  Valid entries continue to
/// load even if a sibling entry is malformed.
#[derive(Clone, Debug, Default)]
pub struct SnippetLoadReport {
    pub snippets: Vec<Snippet>,
    pub errors: Vec<SnippetError>,
}

impl SnippetLoadReport {
    pub fn catalog(self) -> SnippetCatalog {
        SnippetCatalog::new(self.snippets)
    }
}

/// Return the preferred JSON snippet path.
pub fn snippets_path() -> PathBuf {
    paths::config_dir().join("snippets.json")
}

/// Return the optional compact line-format path.
pub fn snippets_conf_path() -> PathBuf {
    paths::config_dir().join("snippets.conf")
}

/// Load both the JSON file and optional compact config file.
pub fn load_snippets() -> SnippetLoadReport {
    let mut report = SnippetLoadReport::default();
    for path in [snippets_path(), snippets_conf_path()] {
        load_file(&path, &mut report);
    }
    report
}

/// Load snippets from one explicit file.
pub fn load_snippets_from(path: &Path) -> SnippetLoadReport {
    let mut report = SnippetLoadReport::default();
    load_file(path, &mut report);
    report
}

fn load_file(path: &Path, report: &mut SnippetLoadReport) {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            report
                .errors
                .push(SnippetError::io(path.to_owned(), error.to_string()));
            return;
        }
    };
    if metadata.len() > MAX_FILE_BYTES {
        report.errors.push(SnippetError::invalid(
            path.to_owned(),
            format!("snippet file exceeds {MAX_FILE_BYTES} bytes"),
        ));
        return;
    }
    let content = match read_bounded_text(path, MAX_FILE_BYTES) {
        Ok(content) => content,
        Err(error) => {
            report
                .errors
                .push(SnippetError::io(path.to_owned(), error.to_string()));
            return;
        }
    };
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("conf"))
    {
        load_line_file(path, &content, report);
    } else {
        load_json_file(path, &content, report);
    }
}

fn load_json_file(path: &Path, content: &str, report: &mut SnippetLoadReport) {
    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(value) => value,
        Err(error) => {
            report
                .errors
                .push(SnippetError::parse(path.to_owned(), error.to_string()));
            return;
        }
    };
    let values: Vec<serde_json::Value> = if let Some(array) = value.as_array() {
        array.clone()
    } else if let Some(array) = value.get("snippets").and_then(serde_json::Value::as_array) {
        array.clone()
    } else {
        vec![value]
    };
    for value in values {
        match serde_json::from_value::<SnippetManifest>(value) {
            Ok(manifest) => match Snippet::from_manifest(manifest, path.to_owned()) {
                Ok(snippet) => report.snippets.push(snippet),
                Err(error) => report.errors.push(error),
            },
            Err(error) => report
                .errors
                .push(SnippetError::parse(path.to_owned(), error.to_string())),
        }
    }
}

fn load_line_file(path: &Path, content: &str, report: &mut SnippetLoadReport) {
    for (line_number, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.splitn(4, '|').map(str::trim).collect();
        if fields.len() != 4 {
            report.errors.push(SnippetError::invalid(
                path.to_owned(),
                format!("line {} must be id|name|keywords|content", line_number + 1),
            ));
            continue;
        }
        let content = decode_line_content(fields[3]);
        let manifest = SnippetManifest {
            id: Some(fields[0].to_owned()),
            name: fields[1].to_owned(),
            keywords: fields[2]
                .split(',')
                .map(str::trim)
                .map(str::to_owned)
                .collect(),
            content,
            enabled: true,
        };
        match Snippet::from_manifest(manifest, path.to_owned()) {
            Ok(snippet) => report.snippets.push(snippet),
            Err(error) => report.errors.push(error),
        }
    }
}

fn decode_line_content(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            let decoded = match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                '|' => '|',
                other => {
                    output.push('\\');
                    other
                }
            };
            output.push(decoded);
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

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_'))
}

fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_NAME_BYTES && !value.chars().any(char::is_control)
}

fn valid_keyword(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEYWORD_BYTES
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}

fn valid_content(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONTENT_BYTES
        && !value.contains('\0')
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

/// Expand a snippet query only when the resulting allocation stays bounded.
fn substitute_query_bounded(template: &str, query: &str, max_bytes: usize) -> Option<String> {
    let occurrences = template.match_indices(QUERY_PLACEHOLDER).count();
    let removed = occurrences.checked_mul(QUERY_PLACEHOLDER.len())?;
    let base = template.len().checked_sub(removed)?;
    let added = occurrences.checked_mul(query.len())?;
    let total = base.checked_add(added)?;
    if total > max_bytes {
        return None;
    }
    Some(template.replace(QUERY_PLACEHOLDER, query))
}

/// Read a snippet file with a hard upper bound, including a file that grows
/// after its metadata was inspected.
fn read_bounded_text(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("file exceeds {max_bytes} bytes"),
        ));
    }
    String::from_utf8(bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("file is not UTF-8: {error}"),
        )
    })
}

/// Return the suffix after a Unicode-safe, case-insensitive prefix match.
fn strip_prefix_case_insensitive<'a>(value: &'a str, keyword: &str) -> Option<&'a str> {
    let character_count = keyword.chars().count();
    let end = value
        .char_indices()
        .nth(character_count)
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    let prefix = value.get(..end)?;
    let keyword_lower = keyword.to_lowercase();
    if prefix.eq_ignore_ascii_case(keyword) || prefix.to_lowercase() == keyword_lower {
        value.get(end..)
    } else {
        None
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                if slug.len() + lower.len_utf8() > MAX_ID_BYTES {
                    break;
                }
                slug.push(lower);
            }
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

/// Errors encountered while parsing or validating one snippet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnippetError {
    Io { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
    Invalid { path: PathBuf, message: String },
}

impl SnippetError {
    fn io(path: PathBuf, message: impl Into<String>) -> Self {
        Self::Io {
            path,
            message: message.into(),
        }
    }

    fn parse(path: PathBuf, message: impl Into<String>) -> Self {
        Self::Parse {
            path,
            message: message.into(),
        }
    }

    fn invalid(path: PathBuf, message: impl Into<String>) -> Self {
        Self::Invalid {
            path,
            message: message.into(),
        }
    }
}

impl fmt::Display for SnippetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(formatter, "{}: {message}", path.display()),
            Self::Parse { path, message } => {
                write!(formatter, "{}: invalid JSON ({message})", path.display())
            }
            Self::Invalid { path, message } => write!(formatter, "{}: {message}", path.display()),
        }
    }
}

impl Error for SnippetError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, keywords: &[&str], content: &str) -> SnippetManifest {
        SnippetManifest {
            id: Some(id.to_owned()),
            name: id.to_owned(),
            keywords: keywords.iter().map(|value| (*value).to_owned()).collect(),
            content: content.to_owned(),
            enabled: true,
        }
    }

    #[test]
    fn parses_single_and_multiple_keyword_forms() {
        let one = Snippet::from_json(
            r#"{"id":"sig","name":"Signature","keyword":"sig","content":"Best, {query}"}"#,
            "sig.json",
        )
        .unwrap();
        assert_eq!(one.keywords, vec!["sig"]);
        let many = serde_json::from_str::<SnippetManifest>(
            r#"{"id":"mail","name":"Mail","keywords":["m","mail"],"content":"Hi"}"#,
        )
        .unwrap();
        assert_eq!(many.keywords.len(), 2);
    }

    #[test]
    fn validates_limits_and_control_characters() {
        assert!(Snippet::from_manifest(manifest("ok", &["o"], "text"), "x").is_ok());
        assert!(Snippet::from_manifest(manifest("bad id", &["b"], "text"), "x").is_err());
        assert!(Snippet::from_manifest(manifest("nul", &["n"], "a\0b"), "x").is_err());
        assert!(Snippet::from_manifest(manifest("ctl", &["c"], "a\u{0007}b"), "x").is_err());
        assert!(
            Snippet::from_manifest(
                manifest("huge", &["h"], &"x".repeat(MAX_CONTENT_BYTES + 1)),
                "x"
            )
            .is_err()
        );
    }

    #[test]
    fn matches_longest_keyword_and_expands_query() {
        let snippet =
            Snippet::from_manifest(manifest("mail", &["m", "mail"], "Hello {query}!"), "x")
                .unwrap();
        let matched = snippet.match_query("mail team").unwrap();
        assert_eq!(matched.keyword, "mail");
        assert_eq!(matched.query, "team");
        assert_eq!(matched.expanded_content().as_deref(), Some("Hello team!"));
        assert!(snippet.match_query("mailbox").is_none());
    }

    #[test]
    fn supports_chinese_names_and_keywords_without_panicking_on_utf8_boundaries() {
        let snippet = Snippet::from_manifest(
            SnippetManifest {
                id: None,
                name: "常用签名".to_owned(),
                keywords: vec!["签名".to_owned()],
                content: "谢谢".to_owned(),
                enabled: true,
            },
            "x",
        )
        .unwrap();
        assert_eq!(snippet.id, "常用签名");
        assert!(snippet.match_query("签字 abc").is_none());
        assert!(snippet.match_query("签名").is_some());

        let ascii_keyword = Snippet::from_manifest(manifest("ascii", &["a"], "text"), "x").unwrap();
        // A one-byte keyword must not be used as a byte offset into a
        // multi-byte first character.
        assert!(ascii_keyword.match_query("中 hello").is_none());
    }

    #[test]
    fn bounds_repeated_query_expansion_before_allocating() {
        let snippet =
            Snippet::from_manifest(manifest("bounded", &["b"], &"{query}".repeat(32)), "x")
                .unwrap();
        let query = "x".repeat(MAX_QUERY_BYTES);
        assert!(snippet.expand(&query).is_none());
    }

    #[test]
    fn catalog_searches_invocations_and_metadata() {
        let first = Snippet::from_manifest(manifest("sig", &["s"], "Regards"), "x").unwrap();
        let second = Snippet::from_manifest(
            SnippetManifest {
                name: "Email address".to_owned(),
                ..manifest("email", &["address"], "a@example.test")
            },
            "x",
        )
        .unwrap();
        let catalog = SnippetCatalog::new(vec![first, second]);
        assert_eq!(catalog.candidates("s hello")[0].snippet.id, "sig");
        assert_eq!(catalog.search("address")[0].snippet.id, "email");
        assert!(catalog.search("").is_empty());
    }

    #[test]
    fn parses_compact_conf_with_escaped_newlines() {
        let path = PathBuf::from("snippets.conf");
        let mut report = SnippetLoadReport::default();
        load_line_file(&path, "sig|Signature|s|Hello\\nWorld", &mut report);
        assert_eq!(report.errors.len(), 0);
        assert_eq!(report.snippets[0].content, "Hello\nWorld");
    }

    #[test]
    fn rejects_oversized_file_without_reading_it() {
        // Exercise the same boundary used by load_file without creating a
        // material multi-megabyte fixture on disk.
        assert!(MAX_FILE_BYTES > MAX_CONTENT_BYTES as u64);
    }
}
