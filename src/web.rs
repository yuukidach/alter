//! Web-search providers and lightweight query suggestions.
//!
//! The UI deliberately does not live in this module.  Callers can turn a
//! keyword-prefixed input into a [`WebSearchAction`], render that action like
//! any other result, and invoke [`WebSearchAction::open`] when it is selected.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

const QUERY_PLACEHOLDER: &str = "{query}";
const MAX_SUGGESTION_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_PROVIDER_ID_BYTES: usize = 128;
const MAX_PROVIDER_NAME_BYTES: usize = 256;
const MAX_PROVIDER_KEYWORDS: usize = 32;
const MAX_PROVIDER_KEYWORD_BYTES: usize = 128;
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;
const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_EXPANDED_URL_BYTES: usize = 64 * 1024;

/// A named web-search URL template.
///
/// `url_template` and, when present, `suggestion_template` must use an HTTP(S)
/// URL containing `{query}`.  Alter replaces the placeholder with an RFC 3986
/// percent-encoded UTF-8 query, never with raw user input.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct WebSearchProvider {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_keywords")]
    pub keywords: Vec<String>,
    #[serde(alias = "url", alias = "search_url")]
    pub url_template: String,
    #[serde(default, alias = "suggest_url", alias = "suggestions_url")]
    pub suggestion_template: Option<String>,
}

impl WebSearchProvider {
    /// Build a validated provider.  Invalid custom templates are rejected
    /// before they can become launcher actions.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        keywords: Vec<String>,
        url_template: impl Into<String>,
        suggestion_template: Option<String>,
    ) -> Result<Self, &'static str> {
        let provider = Self {
            id: id.into(),
            name: name.into(),
            keywords,
            url_template: url_template.into(),
            suggestion_template,
        };
        provider.validate()?;
        Ok(provider.normalized())
    }

    /// Create a browser action for this provider.
    pub fn action(&self, query: &str) -> Option<WebSearchAction> {
        let query = query.trim();
        if query.is_empty()
            || query.len() > MAX_QUERY_BYTES
            || query.contains('\0')
            || query.chars().any(char::is_control)
            || self.validate().is_err()
        {
            return None;
        }
        Some(WebSearchAction {
            provider_id: self.id.clone(),
            provider_name: self.name.clone(),
            query: query.to_owned(),
            url: expand_query_template(&self.url_template, query)?,
        })
    }

    fn validate(&self) -> Result<(), &'static str> {
        if !valid_identifier(&self.id) || self.id.len() > MAX_PROVIDER_ID_BYTES {
            return Err("provider id must contain only letters, numbers, '-' or '_'");
        }
        if self.name.trim().is_empty()
            || self.name.len() > MAX_PROVIDER_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("provider name cannot be empty");
        }
        if self.keywords.is_empty()
            || self.keywords.len() > MAX_PROVIDER_KEYWORDS
            || self.keywords.iter().any(|keyword| {
                keyword.len() > MAX_PROVIDER_KEYWORD_BYTES || !valid_keyword(keyword)
            })
        {
            return Err("provider must have at least one valid keyword");
        }
        validate_template(&self.url_template)?;
        if let Some(template) = self.suggestion_template.as_deref() {
            validate_template(template)?;
        }
        Ok(())
    }

    fn normalized(mut self) -> Self {
        self.id = self.id.trim().to_ascii_lowercase();
        self.name = self.name.trim().to_owned();
        self.keywords = self
            .keywords
            .into_iter()
            .map(|keyword| keyword.trim().to_lowercase())
            .collect();
        self.keywords.sort();
        self.keywords.dedup();
        self
    }

    fn matches(&self, value: &str) -> bool {
        self.id.eq_ignore_ascii_case(value)
            || self.keywords.iter().any(|keyword| {
                keyword.eq_ignore_ascii_case(value) || keyword == &value.to_lowercase()
            })
    }
}

/// A fully expanded, safe-to-open browser action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSearchAction {
    pub provider_id: String,
    pub provider_name: String,
    pub query: String,
    pub url: String,
}

impl WebSearchAction {
    /// Open the URL using the desktop's default browser.
    ///
    /// The URL is passed as one argument to `xdg-open`; it is not interpreted
    /// by a shell.  Returning after spawn keeps the launcher responsive.
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

/// Built-in and user-provided web-search providers.
#[derive(Clone, Debug)]
pub struct WebSearchEngine {
    providers: Vec<WebSearchProvider>,
    default_provider: String,
}

impl Default for WebSearchEngine {
    fn default() -> Self {
        Self {
            providers: builtin_providers(),
            // DuckDuckGo is a conservative default for a Linux launcher.  A
            // user can still address Google or Bing by keyword.
            default_provider: "duckduckgo".to_owned(),
        }
    }
}

impl WebSearchEngine {
    /// Load the standard providers and merge
    /// `~/.config/alter/web-searches.json` when it exists.
    ///
    /// A missing or malformed file never disables web search.  This is useful
    /// during launcher startup, where a hand-edited config should not prevent
    /// the UI from appearing.
    pub fn load() -> Self {
        let mut engine = Self::default();
        // JSON is the preferred format.  The compact `.conf` format is kept
        // as a convenient option for users who prefer editing one provider
        // per line.
        for path in [config_path(), line_config_path()] {
            if let Ok(content) = read_bounded_text(&path, MAX_CONFIG_BYTES) {
                let _ = engine.merge_custom_content(&content);
            }
        }
        engine
    }

    /// Load custom providers from a caller-selected file.  Mainly useful for
    /// settings previews and tests.  Missing files are reported to the caller.
    pub fn load_from(path: &Path) -> io::Result<Self> {
        let content = read_bounded_text(path, MAX_CONFIG_BYTES)?;
        let mut engine = Self::default();
        engine
            .merge_custom_content(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(engine)
    }

    pub fn providers(&self) -> &[WebSearchProvider] {
        &self.providers
    }

    pub fn default_provider(&self) -> &WebSearchProvider {
        // `Default` always installs DuckDuckGo, and merge operations only
        // replace providers rather than removing them.
        self.provider(&self.default_provider)
            .unwrap_or(&self.providers[0])
    }

    pub fn provider(&self, id_or_keyword: &str) -> Option<&WebSearchProvider> {
        let needle = id_or_keyword
            .trim()
            .strip_prefix('!')
            .unwrap_or(id_or_keyword.trim());
        self.providers
            .iter()
            .find(|provider| provider.matches(needle))
    }

    /// Parse launcher input into a web action.
    ///
    /// Supported forms are `web query`, `? query`, `google query`, `g query`
    /// and their `!google` / `!g` variants.  Unknown prefixes return `None` so
    /// they do not interfere with application and file search.
    pub fn action_for_input(&self, input: &str) -> Option<WebSearchAction> {
        let input = input.trim();
        if let Some(query) = input.strip_prefix('?') {
            return self.default_provider().action(query);
        }

        let (prefix, query) = input.split_once(char::is_whitespace)?;
        let prefix = prefix.strip_prefix('!').unwrap_or(prefix);
        if prefix.eq_ignore_ascii_case("web") || prefix.eq_ignore_ascii_case("网页") {
            return self.default_provider().action(query);
        }
        self.provider(prefix)?.action(query)
    }

    /// Create an action without parsing a launcher prefix.
    pub fn action_for(&self, provider: &str, query: &str) -> Option<WebSearchAction> {
        self.provider(provider)?.action(query)
    }

    /// Merge a JSON array of custom templates.  An item with the same `id` as
    /// a built-in provider replaces it, which allows users to change locale or
    /// aliases without carrying duplicate search results.
    pub fn merge_custom_json(&mut self, content: &str) -> Result<usize, serde_json::Error> {
        // Decode the array shape first, then validate entries independently.
        // A single malformed provider must not discard otherwise valid
        // siblings in the same user-edited file.
        let values: Vec<Value> = serde_json::from_str(content)?;
        let entries = values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebSearchProvider>(value).ok())
            .collect();
        Ok(self.merge_entries(entries))
    }

    /// Merge either the preferred JSON format or the compact line format.
    ///
    /// The line format is:
    ///
    /// ```text
    /// id|Name|keyword,alias|https://example.test/?q={query}|https://example.test/suggest?q={query}
    /// ```
    ///
    /// The fifth field is optional.  Empty lines and lines beginning with `#`
    /// are ignored.  Invalid entries are skipped so one typo cannot disable
    /// the other providers.
    pub fn merge_custom_content(&mut self, content: &str) -> Result<usize, String> {
        if let Ok(value) = serde_json::from_str::<Value>(content) {
            let Some(values) = value.as_array() else {
                return Err("custom web-search JSON must be an array".to_owned());
            };
            let entries = values
                .iter()
                .cloned()
                .filter_map(|value| serde_json::from_value::<WebSearchProvider>(value).ok())
                .collect();
            return Ok(self.merge_entries(entries));
        }
        parse_line_templates(content).map(|entries| self.merge_entries(entries))
    }

    fn merge_entries(&mut self, entries: Vec<WebSearchProvider>) -> usize {
        let mut merged = 0;
        for entry in entries {
            if entry.validate().is_err() {
                continue;
            }
            let entry = entry.normalized();
            if let Some(position) = self
                .providers
                .iter()
                .position(|provider| provider.id == entry.id)
            {
                self.providers[position] = entry;
            } else {
                self.providers.push(entry);
            }
            merged += 1;
        }
        merged
    }

    /// Fetch suggestions for the default provider.
    ///
    /// This call is blocking (with a two-second curl deadline); use
    /// [`Self::spawn_suggestions`] from the GTK thread.  Any network, command,
    /// HTTP or JSON failure intentionally becomes an empty list.
    pub fn suggestions(&self, query: &str, limit: usize) -> Vec<String> {
        self.suggestions_for(&self.default_provider, query, limit)
    }

    /// Fetch suggestions for a selected provider.  Failures are silent.
    pub fn suggestions_for(&self, provider: &str, query: &str, limit: usize) -> Vec<String> {
        let Some(provider) = self.provider(provider) else {
            return Vec::new();
        };
        fetch_provider_suggestions(provider, query, limit)
    }

    /// Fetch suggestions on a small worker thread so the launcher UI remains
    /// responsive even when DNS or the network is unavailable.
    pub fn spawn_suggestions(
        &self,
        provider: Option<&str>,
        query: impl Into<String>,
        limit: usize,
    ) -> thread::JoinHandle<Vec<String>> {
        let provider = provider
            .and_then(|value| self.provider(value))
            .unwrap_or_else(|| self.default_provider())
            .clone();
        let query = query.into();
        thread::spawn(move || fetch_provider_suggestions(&provider, &query, limit))
    }
}

/// Standard user configuration path.
pub fn config_path() -> PathBuf {
    config_root().join("alter/web-searches.json")
}

/// Optional compact custom-provider configuration path.
pub fn line_config_path() -> PathBuf {
    config_root().join("alter/web-searches.conf")
}

/// RFC 3986 percent-encode a query as one URL component.
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

fn builtin_providers() -> Vec<WebSearchProvider> {
    vec![
        WebSearchProvider::new(
            "google",
            "Google",
            vec!["g".to_owned(), "google".to_owned(), "谷歌".to_owned()],
            "https://www.google.com/search?hl=zh-CN&q={query}",
            Some(
                "https://suggestqueries.google.com/complete/search?client=firefox&hl=zh-CN&q={query}"
                    .to_owned(),
            ),
        )
        .expect("built-in Google provider must be valid"),
        WebSearchProvider::new(
            "bing",
            "Bing",
            vec!["b".to_owned(), "bing".to_owned(), "必应".to_owned()],
            "https://www.bing.com/search?setlang=zh-cn&q={query}",
            Some("https://api.bing.com/osjson.aspx?market=zh-CN&query={query}".to_owned()),
        )
        .expect("built-in Bing provider must be valid"),
        WebSearchProvider::new(
            "duckduckgo",
            "DuckDuckGo",
            vec![
                "d".to_owned(),
                "ddg".to_owned(),
                "duckduckgo".to_owned(),
            ],
            "https://duckduckgo.com/?kl=cn-zh&q={query}",
            Some("https://duckduckgo.com/ac/?kl=cn-zh&q={query}&type=list".to_owned()),
        )
        .expect("built-in DuckDuckGo provider must be valid"),
    ]
}

fn config_root() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

fn parse_line_templates(content: &str) -> Result<Vec<WebSearchProvider>, String> {
    let mut entries = Vec::new();
    let mut saw_entry = false;
    let mut saw_well_formed_line = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        saw_entry = true;
        let fields: Vec<_> = line.splitn(5, '|').map(str::trim).collect();
        if !(4..=5).contains(&fields.len()) {
            continue;
        }
        saw_well_formed_line = true;
        let keywords = fields[2]
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        let suggestion_template = fields
            .get(4)
            .filter(|value| !value.is_empty())
            .map(|value| (*value).to_owned());
        if let Ok(provider) = WebSearchProvider::new(
            fields[0],
            fields[1],
            keywords,
            fields[3],
            suggestion_template,
        ) {
            entries.push(provider);
        }
    }
    if saw_entry && !saw_well_formed_line {
        Err("custom web-search config is neither JSON nor valid line format".to_owned())
    } else {
        Ok(entries)
    }
}

fn expand_query_template(template: &str, query: &str) -> Option<String> {
    if query.len() > MAX_QUERY_BYTES || query.contains('\0') || query.chars().any(char::is_control)
    {
        return None;
    }
    validate_template(template).ok()?;
    let encoded = encode_query_component(query);
    let occurrences = template.match_indices(QUERY_PLACEHOLDER).count();
    let removed = occurrences.checked_mul(QUERY_PLACEHOLDER.len())?;
    let base = template.len().checked_sub(removed)?;
    let added = occurrences.checked_mul(encoded.len())?;
    let total = base.checked_add(added)?;
    (total <= MAX_EXPANDED_URL_BYTES).then(|| template.replace(QUERY_PLACEHOLDER, &encoded))
}

fn validate_template(template: &str) -> Result<(), &'static str> {
    if template.len() > MAX_TEMPLATE_BYTES {
        return Err("template is too long");
    }
    let Some((scheme, authority_and_path)) = template.split_once("://") else {
        return Err("template must use http or https");
    };
    if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
        return Err("template must use http or https");
    }
    if !template.contains(QUERY_PLACEHOLDER) {
        return Err("template must contain {query}");
    }
    if template
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("template cannot contain whitespace or control characters");
    }
    let authority_end = authority_and_path
        .find(['/', '?', '#'])
        .unwrap_or(authority_and_path.len());
    let authority = &authority_and_path[..authority_end];
    if authority.is_empty() || authority.contains(QUERY_PLACEHOLDER) {
        return Err("template must contain a static host");
    }
    if authority
        .chars()
        .any(|character| matches!(character, '\\' | '"' | '\''))
    {
        return Err("template host contains an unsafe character");
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn valid_keyword(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
        && !value.contains('|')
}

/// Read a custom provider file with a hard upper bound, including when the
/// file grows after a metadata check.
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
    String::from_utf8(bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("file is not UTF-8: {error}"),
        )
    })
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        _ => char::from(b'A' + value - 10),
    }
}

fn fetch_provider_suggestions(
    provider: &WebSearchProvider,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let Some(template) = provider.suggestion_template.as_deref() else {
        return Vec::new();
    };
    let Some(url) = expand_query_template(template, query) else {
        return Vec::new();
    };
    let Ok(output) = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--location",
            "--compressed",
            "--connect-timeout",
            "1",
            "--max-time",
            "2",
            "--max-filesize",
            "131072",
            "--max-redirs",
            "5",
            "--proto",
            "=http,https",
            "--proto-redir",
            "=http,https",
            "--user-agent",
            "Alter/0.1 web-suggestions",
            "--url",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() || output.stdout.len() > MAX_SUGGESTION_RESPONSE_BYTES {
        return Vec::new();
    }
    parse_suggestions(&output.stdout, query, limit)
}

fn parse_suggestions(bytes: &[u8], query: &str, limit: usize) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let candidates = suggestion_candidates(&value);
    let mut unique = HashSet::new();
    let query_key = query.trim().to_lowercase();
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty()
                || candidate.len() > 1024
                || candidate.chars().count() > 256
                || candidate.chars().any(char::is_control)
            {
                return None;
            }
            let key = candidate.to_lowercase();
            if key == query_key || !unique.insert(key) {
                return None;
            }
            Some(candidate.to_owned())
        })
        .take(limit.min(20))
        .collect()
}

fn suggestion_candidates(value: &Value) -> Vec<&str> {
    // Google and Bing use the OpenSearch shape: [query, [suggestions...]].
    if let Some(items) = value
        .as_array()
        .and_then(|array| array.get(1))
        .and_then(Value::as_array)
    {
        return items.iter().filter_map(Value::as_str).collect();
    }

    // DuckDuckGo returns [{"phrase":"..."}, ...].  The final branch also
    // tolerates a few compatible endpoints which use `value` or `text`.
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(|item| {
                item.as_str().or_else(|| {
                    item.as_object().and_then(|object| {
                        ["phrase", "value", "text"]
                            .iter()
                            .find_map(|key| object.get(*key).and_then(Value::as_str))
                    })
                })
            })
            .collect();
    }

    value
        .get("suggestions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Keywords {
    One(String),
    Many(Vec<String>),
}

fn deserialize_keywords<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Keywords::deserialize(deserializer)? {
        Keywords::One(keyword) => vec![keyword],
        Keywords::Many(keywords) => keywords,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encodes_unicode_and_shell_characters() {
        assert_eq!(
            encode_query_component("Rust GTK+ 中文 & x/y;$(id)"),
            "Rust%20GTK%2B%20%E4%B8%AD%E6%96%87%20%26%20x%2Fy%3B%24%28id%29"
        );
    }

    #[test]
    fn exposes_three_localized_builtin_providers() {
        let engine = WebSearchEngine::default();
        assert_eq!(engine.providers().len(), 3);
        assert!(engine.provider("google").is_some());
        assert!(engine.provider("必应").is_some());
        assert!(engine.provider("ddg").is_some());

        let action = engine.action_for("g", "GTK 4 中文").unwrap();
        assert_eq!(action.provider_name, "Google");
        assert!(action.url.contains("hl=zh-CN"));
        assert!(action.url.ends_with("GTK%204%20%E4%B8%AD%E6%96%87"));
    }

    #[test]
    fn parses_explicit_web_inputs_without_claiming_normal_queries() {
        let engine = WebSearchEngine::default();
        assert_eq!(
            engine
                .action_for_input("? alter launcher")
                .unwrap()
                .provider_id,
            "duckduckgo"
        );
        assert_eq!(
            engine
                .action_for_input("!g alter launcher")
                .unwrap()
                .provider_id,
            "google"
        );
        assert_eq!(
            engine.action_for_input("网页 alter").unwrap().provider_id,
            "duckduckgo"
        );
        assert!(engine.action_for_input("alter launcher").is_none());
        assert!(engine.action_for_input("g ").is_none());
    }

    #[test]
    fn merges_valid_custom_templates_and_overrides_by_id() {
        let mut engine = WebSearchEngine::default();
        let merged = engine
            .merge_custom_json(
                r#"[
                    {
                        "id": "baidu",
                        "name": "百度",
                        "keywords": ["bd", "baidu"],
                        "url_template": "https://www.baidu.com/s?wd={query}"
                    },
                    {
                        "id": "google",
                        "name": "Google 镜像",
                        "keywords": "gg",
                        "url": "https://example.test/search?q={query}"
                    }
                ]"#,
            )
            .unwrap();
        assert_eq!(merged, 2);
        assert_eq!(engine.providers().len(), 4);
        assert_eq!(engine.provider("bd").unwrap().name, "百度");
        assert_eq!(engine.provider("google").unwrap().name, "Google 镜像");
        assert!(engine.provider("g").is_none());
    }

    #[test]
    fn skips_unsafe_or_incomplete_custom_templates() {
        let mut engine = WebSearchEngine::default();
        let merged = engine
            .merge_custom_json(
                r#"[
                    {
                        "id": "unsafe",
                        "name": "Unsafe",
                        "keywords": ["unsafe"],
                        "url_template": "javascript:alert({query})"
                    },
                    {
                        "id": "missing",
                        "name": "Missing placeholder",
                        "keywords": ["missing"],
                        "url_template": "https://example.test/search"
                    }
                ]"#,
            )
            .unwrap();
        assert_eq!(merged, 0);
        assert_eq!(engine.providers().len(), 3);
    }

    #[test]
    fn isolates_malformed_json_provider_entries() {
        let mut engine = WebSearchEngine::default();
        let merged = engine
            .merge_custom_json(
                r#"[
                    {"id":"good","name":"Good","keywords":"ok","url_template":"https://example.test/?q={query}"},
                    {"id":"bad","name":42,"keywords":"bad","url_template":"https://example.test/?q={query}"}
                ]"#,
            )
            .unwrap();
        assert_eq!(merged, 1);
        assert!(engine.provider("good").is_some());
        assert!(engine.provider("bad").is_none());
    }

    #[test]
    fn rejects_invalid_hosts_and_unbounded_queries() {
        assert!(
            WebSearchProvider::new(
                "bad-host",
                "Bad",
                vec!["bad".to_owned()],
                "https://?q={query}",
                None,
            )
            .is_err()
        );
        let engine = WebSearchEngine::default();
        assert!(
            engine
                .action_for("g", &"x".repeat(MAX_QUERY_BYTES + 1))
                .is_none()
        );
        assert!(engine.action_for("g", "a".repeat(1024).as_str()).is_some());
    }

    #[test]
    fn accepts_compact_line_configuration() {
        let mut engine = WebSearchEngine::default();
        let merged = engine
            .merge_custom_content(
                &("# id|name|keywords|search template|suggestion template\n".to_owned()
                    + "wiki|Wikipedia|w,wiki|https://zh.wikipedia.org/w/index.php?search={query}|"),
            )
            .unwrap();
        assert_eq!(merged, 1);
        let action = engine.action_for_input("!w Wayland").unwrap();
        assert_eq!(action.provider_name, "Wikipedia");
        assert!(action.url.ends_with("search=Wayland"));
    }

    #[test]
    fn parses_opensearch_and_duckduckgo_suggestions() {
        let google = br#"["alter", ["alter linux", "Alter Linux", "alter app"]]"#;
        assert_eq!(
            parse_suggestions(google, "alter", 10),
            vec!["alter linux", "alter app"]
        );

        let duckduckgo = br#"[{"phrase":"alter launcher"},{"phrase":"alter linux"},{"nope":"x"}]"#;
        assert_eq!(
            parse_suggestions(duckduckgo, "alter", 1),
            vec!["alter launcher"]
        );
    }

    #[test]
    fn malformed_suggestion_response_falls_back_silently() {
        assert!(parse_suggestions(b"not json", "alter", 10).is_empty());
        assert!(parse_suggestions(br#"["alter", []]"#, "alter", 10).is_empty());
    }
}
