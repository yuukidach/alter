use crate::calculator;
use crate::clipboard::{self, ClipboardItem};
use crate::desktop::AppItem;
use crate::quick_links::{QuickLinkAction, QuickLinkCatalog};
use crate::settings::{self, Settings, SharedSettings};
use crate::snippets::{SnippetCatalog, SnippetMatch};
use crate::web::{WebSearchAction, WebSearchEngine};
use crate::workflow::{WorkflowCatalog, WorkflowMatch, WorkflowResultItem};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResultKind {
    App,
    File,
    Clipboard,
    Calculation,
    Settings,
    Web,
    Workflow,
    Snippet,
    QuickLink,
}

#[derive(Clone, Debug)]
pub enum ResultPayload {
    Web(WebSearchAction),
    Workflow(WorkflowMatch),
    WorkflowItem {
        matched: WorkflowMatch,
        item: WorkflowResultItem,
    },
    Snippet {
        content: String,
    },
    QuickLink(QuickLinkAction),
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub kind: ResultKind,
    pub title: String,
    pub subtitle: String,
    pub target: PathBuf,
    pub icon: Option<String>,
    pub clipboard_id: Option<i64>,
    pub clipboard_content: Option<String>,
    pub clipboard_path: Option<PathBuf>,
    pub clipboard_pinned: bool,
    pub payload: Option<ResultPayload>,
}

impl SearchResult {
    pub fn app(app: &AppItem, score: i64) -> ScoredResult {
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::App,
                title: app.name.clone(),
                subtitle: app
                    .generic_name
                    .clone()
                    .unwrap_or_else(|| "应用程序".to_owned()),
                target: app.desktop_file.clone(),
                icon: app.icon.clone(),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: None,
            },
        }
    }

    pub fn file(path: PathBuf, score: i64) -> ScoredResult {
        let title = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::File,
                title,
                subtitle: path.to_string_lossy().into_owned(),
                target: path,
                icon: None,
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: None,
            },
        }
    }

    pub fn clipboard(item: &ClipboardItem, score: i64) -> ScoredResult {
        let origin = if item.external {
            "Clipse 历史".to_owned()
        } else {
            age(item.created_at)
        };
        let (title, subtitle) = if let Some(path) = item.file_path.as_deref() {
            let title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let kind = if is_image_path(path) {
                "图片剪贴板"
            } else {
                "文件剪贴板"
            };
            (title, format!("{kind} · {origin}"))
        } else {
            (preview(&item.content, 96), format!("剪贴板 · {origin}"))
        };
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::Clipboard,
                title,
                subtitle,
                target: PathBuf::new(),
                icon: None,
                clipboard_id: Some(item.id),
                clipboard_content: Some(item.content.clone()),
                clipboard_path: item.file_path.clone(),
                clipboard_pinned: item.pinned,
                payload: None,
            },
        }
    }

    pub fn calculation(expression: &str, value: String, score: i64) -> ScoredResult {
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::Calculation,
                title: format!("{expression} = {value}"),
                subtitle: "计算器 · Enter 复制结果".to_owned(),
                target: PathBuf::new(),
                icon: None,
                clipboard_id: None,
                clipboard_content: Some(value),
                clipboard_path: None,
                clipboard_pinned: false,
                payload: None,
            },
        }
    }

    pub fn settings(score: i64) -> ScoredResult {
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::Settings,
                title: "Alter 设置".to_owned(),
                subtitle: "打开偏好设置 · Ctrl+,".to_owned(),
                target: PathBuf::new(),
                icon: None,
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: None,
            },
        }
    }

    pub fn web(action: WebSearchAction, score: i64) -> ScoredResult {
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::Web,
                title: format!("搜索：{}", action.query),
                subtitle: format!("{} · 在默认浏览器中打开", action.provider_name),
                target: PathBuf::new(),
                icon: Some("web-browser".to_owned()),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: Some(ResultPayload::Web(action)),
            },
        }
    }

    pub fn workflow(matched: WorkflowMatch) -> ScoredResult {
        let title = matched.workflow.name.clone();
        let subtitle = if matched.query.is_empty() {
            format!("Workflow · 关键词 {}", matched.keyword)
        } else {
            format!("Workflow · {} {}", matched.keyword, matched.query)
        };
        ScoredResult {
            score: matched.score,
            result: Self {
                kind: ResultKind::Workflow,
                title,
                subtitle,
                target: matched.workflow.source.clone(),
                icon: matched.workflow.icon.clone(),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: Some(ResultPayload::Workflow(matched)),
            },
        }
    }

    pub fn workflow_item(
        matched: WorkflowMatch,
        item: WorkflowResultItem,
        score: i64,
    ) -> ScoredResult {
        let subtitle = if item.subtitle.is_empty() {
            format!("Workflow · 关键词 {} · Enter 执行", matched.keyword)
        } else {
            item.subtitle.clone()
        };
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::Workflow,
                title: item.title.clone(),
                subtitle,
                target: matched.workflow.source.clone(),
                icon: item.icon.clone().or_else(|| matched.workflow.icon.clone()),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: Some(ResultPayload::WorkflowItem { matched, item }),
            },
        }
    }

    pub fn snippet(
        id: String,
        title: String,
        subtitle: String,
        content: String,
        score: i64,
    ) -> ScoredResult {
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::Snippet,
                title,
                subtitle,
                target: PathBuf::from(id),
                icon: Some("text-x-generic".to_owned()),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: Some(ResultPayload::Snippet { content }),
            },
        }
    }

    pub fn snippet_match(matched: SnippetMatch) -> Option<ScoredResult> {
        let content = matched.expanded_content()?;
        Some(Self::snippet(
            matched.snippet.id.clone(),
            matched.snippet.name.clone(),
            format!("Snippet · 关键词 {} · Enter 复制", matched.keyword),
            content,
            matched.score,
        ))
    }

    pub fn quick_link(action: QuickLinkAction, score: i64) -> ScoredResult {
        let title = format!("{} · {}", action.name, action.query);
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::QuickLink,
                title,
                subtitle: "快速链接 · 在默认浏览器中打开".to_owned(),
                target: PathBuf::new(),
                icon: Some("insert-link".to_owned()),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: Some(ResultPayload::QuickLink(action)),
            },
        }
    }

    pub fn quick_link_prompt(
        prompt: crate::quick_links::QuickLinkPrompt,
        score: i64,
    ) -> ScoredResult {
        ScoredResult {
            score,
            result: Self {
                kind: ResultKind::QuickLink,
                title: format!("{} · 输入参数", prompt.name),
                subtitle: format!("快速链接 · {} <参数>", prompt.keyword),
                target: PathBuf::from(prompt.link_id),
                icon: Some("insert-link".to_owned()),
                clipboard_id: None,
                clipboard_content: None,
                clipboard_path: None,
                clipboard_pinned: false,
                payload: None,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScoredResult {
    pub score: i64,
    pub result: SearchResult,
}

#[derive(Clone)]
pub struct SearchEngine {
    apps: Arc<Vec<AppItem>>,
    database: PathBuf,
    settings: SharedSettings,
    pub web: Arc<WebSearchEngine>,
    pub workflows: Arc<WorkflowCatalog>,
    pub snippets: Arc<SnippetCatalog>,
}

// Keep the explicit web action ahead of suggestions and local metadata even
// when a suggestion has accumulated a usage bonus.  The gap is intentionally
// larger than the bounded bonus returned by usage::score_bonus.
const WEB_PRIMARY_SCORE: i64 = 2_000;
const WEB_SUGGESTION_SCORE: i64 = 1_500;
const MAX_WEB_SUGGESTIONS: usize = 5;

impl SearchEngine {
    pub fn new(apps: Vec<AppItem>, database: PathBuf, settings: SharedSettings) -> Self {
        Self {
            apps: Arc::new(apps),
            database,
            settings,
            web: Arc::new(WebSearchEngine::load()),
            workflows: Arc::new(crate::workflow::load_workflows().catalog()),
            snippets: Arc::new(crate::snippets::load_snippets().catalog()),
        }
    }

    pub fn search(&self, raw_query: &str) -> Vec<SearchResult> {
        let (scope, query) = parse_scope(raw_query);
        let query = query.trim();
        let preferences = settings::snapshot(&self.settings);
        let ranking_bonuses = ranking_bonuses(preferences.learning_ranking, &self.database);
        let mut scored = Vec::new();

        // Quick links intentionally take precedence over built-in provider
        // keywords. Resolve them before web search so an overridden `g`/`b`
        // keyword never starts an unnecessary network request.
        if matches!(scope, Scope::All)
            && let Some(results) = quick_link_results(
                raw_query,
                &QuickLinkCatalog::load(),
                &preferences,
                &ranking_bonuses,
            )
        {
            return results;
        }

        // Alfred-style keyword searches are represented as explicit actions.
        // Web search is an opt-out feature: when disabled, even an explicit
        // `? query`/`g query` input remains a normal local search. Online
        // suggestions are deliberately fetched later by the UI so the primary
        // browser action is never held up by DNS or a slow provider.
        if preferences.web_search
            && let Some(action) = self.web.action_for_input(raw_query)
        {
            let key = format!("web:{}:{}", action.provider_id, action.query);
            let score = WEB_PRIMARY_SCORE + ranking_bonus(&ranking_bonuses, &key);
            scored.push(SearchResult::web(action, score));
            return finish_results(scored, preferences.max_results);
        }
        if preferences.workflow_search && !query.is_empty() {
            for matched in self.workflows.search(raw_query).into_iter().take(8) {
                let key = format!("workflow:{}", matched.workflow.id);
                let bonus = ranking_bonus(&ranking_bonuses, &key);
                if matched.workflow.script_filter && matched.invocation {
                    match matched.workflow.script_filter_results(&matched.query) {
                        Ok(items) if !items.is_empty() => {
                            for (index, item) in items.into_iter().enumerate() {
                                let score = matched.score + 120 - index.min(50) as i64 + bonus;
                                scored.push(SearchResult::workflow_item(
                                    matched.clone(),
                                    item,
                                    score,
                                ));
                            }
                            continue;
                        }
                        Ok(_) | Err(_) => {
                            // Keep the parent workflow visible when a script
                            // returns no usable items or fails. Enter can
                            // still run it directly for troubleshooting.
                        }
                    }
                }
                let mut result = SearchResult::workflow(matched);
                result.score += bonus;
                scored.push(result);
            }
        }
        if preferences.snippet_search && !query.is_empty() {
            for matched in self.snippets.search(raw_query).into_iter().take(8) {
                if let Some(result) = SearchResult::snippet_match(matched) {
                    let key = format!("snippet:{}", result.result.target.display());
                    let mut result = result;
                    result.score += ranking_bonus(&ranking_bonuses, &key);
                    scored.push(result);
                }
            }
        }

        if !query.is_empty() && matches!(scope, Scope::All | Scope::Apps) {
            if let Some(value) = calculator::evaluate(query) {
                scored.push(SearchResult::calculation(query, value, 1_000));
            }
            let setting_score = ["settings", "setting", "preferences", "设置", "偏好设置"]
                .iter()
                .filter_map(|alias| fuzzy_score(query, alias))
                .max();
            if let Some(score) = setting_score {
                scored.push(SearchResult::settings(score + 180));
            }
        }

        if matches!(scope, Scope::All | Scope::Apps) {
            for (rank, app) in self.apps.iter().enumerate() {
                let score = if query.is_empty() {
                    // Consider every installed application here and let the
                    // final max-results limit keep the view compact.  This
                    // matters for learning: an app that starts below the
                    // initial viewport can still be promoted after the user
                    // selects it repeatedly.
                    Some(
                        100 - rank.min(100) as i64
                            + ranking_bonus(
                                &ranking_bonuses,
                                &format!("app:{}", app.desktop_file.display()),
                            ),
                    )
                } else {
                    fuzzy_score(query, &app.search_text()).map(|value| {
                        value
                            + 100
                            + ranking_bonus(
                                &ranking_bonuses,
                                &format!("app:{}", app.desktop_file.display()),
                            )
                    })
                };
                if let Some(score) = score {
                    scored.push(SearchResult::app(app, score));
                }
            }
        }

        if preferences.clipboard_search
            && matches!(scope, Scope::All | Scope::Clipboard)
            && let Ok(items) = clipboard::recent_with_retention(
                &self.database,
                200,
                preferences.clipboard_retention_days,
            )
        {
            for (rank, item) in items.iter().enumerate() {
                let score = if query.is_empty() {
                    Some(
                        94 - rank.min(30) as i64
                            + ranking_bonus(&ranking_bonuses, &format!("clip:{}", item.id)),
                    )
                } else {
                    clipboard_match_score(query, item).map(|value| {
                        value
                            + 75
                            + (30 - rank.min(30) as i64)
                            + item.use_count.min(10)
                            + ranking_bonus(&ranking_bonuses, &format!("clip:{}", item.id))
                    })
                };
                if let Some(score) = score {
                    scored.push(SearchResult::clipboard(item, score));
                }
            }
        }

        if preferences.file_search
            && matches!(scope, Scope::All | Scope::Files)
            && query.chars().count() >= 2
        {
            for path in find_files(query) {
                let text = path.to_string_lossy();
                let basename = path
                    .file_name()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_else(|| text.clone());
                let score = fuzzy_score(query, &basename)
                    .or_else(|| fuzzy_score(query, &text))
                    .unwrap_or(0)
                    + 82
                    + ranking_bonus(&ranking_bonuses, &format!("file:{text}"));
                scored.push(SearchResult::file(path, score));
            }
        }

        if query.is_empty() && !preferences.show_recent {
            return Vec::new();
        }
        finish_results(scored, preferences.max_results)
    }

    /// Fetch and rank online suggestions for an explicit web query.
    ///
    /// This is intentionally separate from [`Self::search`]: callers should
    /// first render the immediate primary action, then invoke this blocking
    /// enrichment on a debounced worker thread and replace the result list if
    /// suggestions arrive.
    pub fn search_web_suggestions(&self, raw_query: &str) -> Option<Vec<SearchResult>> {
        let preferences = settings::snapshot(&self.settings);
        if !preferences.web_search || !preferences.web_suggestions {
            return None;
        }
        let action = self.web.action_for_input(raw_query)?;
        if preferences.quick_links && QuickLinkCatalog::load().has_keyword(raw_query) {
            return None;
        }
        let suggestions =
            self.web
                .suggestions_for(&action.provider_id, &action.query, MAX_WEB_SUGGESTIONS);
        if suggestions.is_empty() {
            return None;
        }

        let ranking_bonuses = ranking_bonuses(preferences.learning_ranking, &self.database);
        let key = format!("web:{}:{}", action.provider_id, action.query);
        let score = WEB_PRIMARY_SCORE + ranking_bonus(&ranking_bonuses, &key);
        let mut scored = vec![SearchResult::web(action.clone(), score)];
        append_web_suggestions(
            &mut scored,
            &self.web,
            &action,
            suggestions,
            &ranking_bonuses,
        );
        (scored.len() > 1).then(|| finish_results(scored, preferences.max_results))
    }
}

fn finish_results(mut scored: Vec<ScoredResult>, limit: usize) -> Vec<SearchResult> {
    scored.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| kind_priority(&right.result.kind).cmp(&kind_priority(&left.result.kind)))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|item| item.result)
        .collect()
}

fn quick_link_results(
    raw_query: &str,
    catalog: &QuickLinkCatalog,
    preferences: &Settings,
    ranking_bonuses: &HashMap<String, i64>,
) -> Option<Vec<SearchResult>> {
    if !preferences.quick_links || raw_query.trim().is_empty() || !catalog.has_keyword(raw_query) {
        return None;
    }

    let matches = catalog.matching(raw_query);
    let mut scored = Vec::new();
    for matched in matches.iter().take(8) {
        let key = format!("quick-link:{}", matched.link.id);
        let score = matched.score + ranking_bonus(ranking_bonuses, &key);
        scored.push(SearchResult::quick_link(matched.action.clone(), score));
    }
    if matches.is_empty()
        && let Some(prompt) = catalog.prompt_for_keyword(raw_query)
    {
        let key = format!("quick-link:{}", prompt.link_id);
        let score = 2_200 + ranking_bonus(ranking_bonuses, &key);
        scored.push(SearchResult::quick_link_prompt(prompt, score));
    }
    Some(finish_results(scored, preferences.max_results))
}

fn append_web_suggestions(
    scored: &mut Vec<ScoredResult>,
    web: &WebSearchEngine,
    primary: &WebSearchAction,
    suggestions: impl IntoIterator<Item = String>,
    ranking_bonuses: &HashMap<String, i64>,
) {
    let mut seen = HashSet::new();
    seen.insert(primary.query.trim().to_lowercase());
    let mut added = 0usize;

    for suggestion in suggestions {
        let suggestion = suggestion.trim();
        if suggestion.is_empty() || !seen.insert(suggestion.to_lowercase()) {
            continue;
        }
        let Some(action) = web.action_for(&primary.provider_id, suggestion) else {
            continue;
        };
        let key = format!("web:{}:{}", action.provider_id, action.query);
        let score = WEB_SUGGESTION_SCORE - added as i64 + ranking_bonus(ranking_bonuses, &key);
        let mut result = SearchResult::web(action, score);
        result.result.subtitle = format!("{} · 搜索建议", primary.provider_name);
        scored.push(result);
        added += 1;
        if added == MAX_WEB_SUGGESTIONS {
            break;
        }
    }
}

#[derive(Clone, Copy)]
enum Scope {
    All,
    Apps,
    Files,
    Clipboard,
}

fn parse_scope(query: &str) -> (Scope, &str) {
    let Some((prefix, rest)) = query.split_once(char::is_whitespace) else {
        return (Scope::All, query);
    };
    let scope = match prefix.to_ascii_lowercase().as_str() {
        "a" | "app" | "apps" => Scope::Apps,
        "f" | "file" | "files" => Scope::Files,
        "c" | "clip" | "clipboard" => Scope::Clipboard,
        _ => return (Scope::All, query),
    };
    (scope, rest)
}

fn kind_priority(kind: &ResultKind) -> u8 {
    match kind {
        ResultKind::Web => 8,
        ResultKind::Workflow => 7,
        ResultKind::Calculation => 5,
        ResultKind::Settings => 4,
        ResultKind::Snippet => 4,
        ResultKind::QuickLink => 9,
        ResultKind::App => 3,
        ResultKind::File => 2,
        ResultKind::Clipboard => 1,
    }
}

/// A small allocation-free-enough fuzzy matcher.  It rewards word starts and
/// contiguous matches, while allowing users to type an abbreviated name.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    if query.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = text.to_lowercase().chars().collect();
    if haystack.is_empty() {
        return None;
    }

    let mut query_index = 0;
    let mut last_match = None;
    let mut score = 0i64;
    for (index, character) in haystack.iter().enumerate() {
        if query_index >= query.len() || *character != query[query_index] {
            continue;
        }

        score += 10;
        if index == 0 || is_word_boundary(haystack.get(index.wrapping_sub(1))) {
            score += 22;
        }
        if let Some(previous) = last_match {
            let gap = index.saturating_sub(previous + 1);
            score -= gap.min(12) as i64;
            if gap == 0 {
                score += 9;
            }
        }
        last_match = Some(index);
        query_index += 1;
    }

    if query_index != query.len() {
        return None;
    }

    let lower_text = text.to_lowercase();
    if lower_text.contains(&query.iter().collect::<String>()) {
        score += 28;
    }
    Some(score - haystack.len().min(100) as i64 / 10)
}

fn is_word_boundary(previous: Option<&char>) -> bool {
    previous.is_none_or(|character| !character.is_alphanumeric())
}

fn ranking_bonuses(enabled: bool, database: &Path) -> HashMap<String, i64> {
    if !enabled || database.as_os_str().is_empty() {
        return HashMap::new();
    }
    crate::usage::score_bonuses(database).unwrap_or_default()
}

fn ranking_bonus(bonuses: &HashMap<String, i64>, key: &str) -> i64 {
    bonuses.get(key).copied().unwrap_or_default()
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            [
                "png", "jpg", "jpeg", "jpe", "webp", "gif", "bmp", "tif", "tiff", "svg", "svgz",
                "avif",
            ]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

/// Return the strongest fuzzy match for a clipboard item. Clipse usually puts
/// a useful label in `value`, but file-backed entries can contain an opaque
/// value while the actual filename lives only in `filePath`. Search both
/// fields so `.png`, `screenshot`, and directory-name queries find them.
fn clipboard_match_score(query: &str, item: &ClipboardItem) -> Option<i64> {
    let mut best = fuzzy_score(query, &item.content);
    if let Some(path) = item.file_path.as_deref() {
        if let Some(score) = path
            .file_name()
            .and_then(|name| fuzzy_score(query, &name.to_string_lossy()))
        {
            best = Some(best.map_or(score, |current| current.max(score)));
        }
        if let Some(score) = fuzzy_score(query, &path.to_string_lossy()) {
            best = Some(best.map_or(score, |current| current.max(score)));
        }
    }
    best
}

fn find_files(query: &str) -> Vec<PathBuf> {
    let output = Command::new("plocate")
        .args(["--ignore-case", "--limit", "80", "--"])
        .arg(query)
        .output();

    match output {
        Ok(output) if output.status.success() || output.status.code() == Some(1) => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect()
        }
        _ => find_files_with_fd(query),
    }
}

fn find_files_with_fd(query: &str) -> Vec<PathBuf> {
    let home = crate::paths::home_dir();
    let output = Command::new("fd")
        .args([
            "--hidden",
            "--no-ignore-vcs",
            "--type",
            "f",
            "--type",
            "d",
            "--max-results",
            "80",
            "--",
            query,
        ])
        .arg(home)
        .output();
    match output {
        Ok(output) if output.status.success() || output.status.code() == Some(1) => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect()
        }
        _ => Vec::new(),
    }
}

fn preview(value: &str, limit: usize) -> String {
    let one_line = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ↵ ");
    let mut characters = one_line.chars();
    let truncated: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        format!("{truncated}…")
    } else if truncated.is_empty() {
        "(空白)".to_owned()
    } else {
        truncated
    }
}

fn age(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(timestamp);
    let seconds = now.saturating_sub(timestamp);
    match seconds {
        0..=59 => "刚刚".to_owned(),
        60..=3599 => format!("{} 分钟前", seconds / 60),
        3600..=86_399 => format!("{} 小时前", seconds / 3600),
        _ => format!("{} 天前", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::AppItem;
    use crate::quick_links::QuickLink;
    use crate::settings::{Settings, shared};
    use crate::snippets::{Snippet, SnippetCatalog};
    use crate::workflow::{Workflow, WorkflowCatalog};
    use std::fs;
    use std::sync::Arc;
    use std::time::Duration;

    fn temporary_database(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        std::env::temp_dir().join(format!(
            "alter-search-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_database(path: &PathBuf) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    fn local_settings(learning_ranking: bool) -> Settings {
        Settings {
            file_search: false,
            clipboard_search: false,
            web_search: false,
            web_suggestions: false,
            workflow_search: false,
            snippet_search: false,
            learning_ranking,
            ..Default::default()
        }
    }

    fn test_engine(apps: Vec<AppItem>, database: PathBuf, preferences: Settings) -> SearchEngine {
        SearchEngine {
            apps: Arc::new(apps),
            database,
            settings: shared(preferences),
            web: Arc::new(WebSearchEngine::default()),
            workflows: Arc::new(WorkflowCatalog::default()),
            snippets: Arc::new(SnippetCatalog::default()),
        }
    }

    fn test_app(name: &str, desktop_file: &str) -> AppItem {
        AppItem {
            id: desktop_file.to_owned(),
            name: name.to_owned(),
            generic_name: None,
            keywords: Vec::new(),
            icon: None,
            desktop_file: PathBuf::from(desktop_file),
        }
    }

    #[test]
    fn fuzzy_match_rewards_contiguous_text() {
        assert!(
            fuzzy_score("calc", "Calculator").unwrap()
                > fuzzy_score("calc", "Call Center").unwrap()
        );
        assert!(fuzzy_score("xyz", "Calculator").is_none());
    }

    #[test]
    fn parses_scopes() {
        assert!(matches!(parse_scope("a calc").0, Scope::Apps));
        assert_eq!(parse_scope("a calc").1, "calc");
        assert!(matches!(parse_scope("anything").0, Scope::All));
    }

    #[test]
    fn exposes_calculator_and_alter_settings_as_search_results() {
        let preferences = crate::settings::Settings {
            file_search: false,
            clipboard_search: false,
            ..Default::default()
        };
        let engine = SearchEngine::new(
            Vec::new(),
            PathBuf::new(),
            crate::settings::shared(preferences),
        );

        let calculator = engine.search("2 + 2");
        assert_eq!(calculator[0].kind, ResultKind::Calculation);
        assert_eq!(calculator[0].clipboard_content.as_deref(), Some("4"));

        let settings = engine.search("settings");
        assert_eq!(settings[0].kind, ResultKind::Settings);
    }

    #[test]
    fn feature_switches_gate_web_workflows_and_snippets_without_network() {
        let workflow = Workflow::from_json(
            r#"{"id":"test-workflow","name":"Test Workflow","keyword":"tw","command":["true"]}"#,
            "test-workflow.json",
        )
        .unwrap();
        let snippet = Snippet::from_json(
            r#"{"id":"test-snippet","name":"Test Snippet","keyword":"ts","content":"hello {query}"}"#,
            "test-snippet.json",
        )
        .unwrap();

        let database = temporary_database("switches-disabled");
        let disabled = test_engine(Vec::new(), database.clone(), local_settings(false));
        assert!(
            disabled
                .search("? alter")
                .iter()
                .all(|result| result.kind != ResultKind::Web)
        );
        assert!(
            !disabled
                .search("tw hello")
                .iter()
                .any(|result| result.kind == ResultKind::Workflow)
        );
        assert!(
            !disabled
                .search("ts hello")
                .iter()
                .any(|result| result.kind == ResultKind::Snippet)
        );

        let mut enabled_preferences = local_settings(false);
        enabled_preferences.web_search = true;
        // This keeps the test entirely offline while proving that the main
        // action remains available when suggestions are disabled.
        enabled_preferences.web_suggestions = false;
        enabled_preferences.workflow_search = true;
        enabled_preferences.snippet_search = true;
        let mut enabled = test_engine(Vec::new(), database.clone(), enabled_preferences);
        enabled.workflows = Arc::new(WorkflowCatalog::new(vec![workflow]));
        enabled.snippets = Arc::new(SnippetCatalog::new(vec![snippet]));

        let web_results = enabled.search("? alter");
        assert_eq!(
            web_results
                .iter()
                .filter(|result| result.kind == ResultKind::Web)
                .count(),
            1
        );
        assert!(
            enabled
                .search("tw hello")
                .iter()
                .any(|result| result.kind == ResultKind::Workflow)
        );
        assert!(
            enabled
                .search("ts hello")
                .iter()
                .any(|result| result.kind == ResultKind::Snippet)
        );
        remove_database(&database);
    }

    #[test]
    fn initial_web_result_does_not_wait_for_online_suggestions() {
        let mut preferences = local_settings(false);
        preferences.web_search = true;
        preferences.web_suggestions = true;
        preferences.quick_links = false;
        let engine = test_engine(Vec::new(), PathBuf::new(), preferences);

        let results = engine.search("? alter launcher");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ResultKind::Web);
        assert!(matches!(
            results[0].payload.as_ref(),
            Some(ResultPayload::Web(action)) if action.query == "alter launcher"
        ));
    }

    #[test]
    fn quick_links_only_claim_explicit_keyword_queries() {
        let link = QuickLink::from_form(
            Some("job-details"),
            "Job details",
            "job",
            "https://example.test/jobs/{query}",
        )
        .unwrap();
        let catalog = QuickLinkCatalog::new(vec![link]);

        let matched = catalog.matching("job j-056rekk80h");
        assert_eq!(matched.len(), 1);
        assert_eq!(
            matched[0].action.url,
            "https://example.test/jobs/j-056rekk80h"
        );
        assert!(catalog.matching("j-056rekk80h").is_empty());
        assert!(catalog.matching("jobs j-056rekk80h").is_empty());

        let preferences = local_settings(false);
        let results =
            quick_link_results("job j-056rekk80h", &catalog, &preferences, &HashMap::new())
                .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ResultKind::QuickLink);
        assert!(matches!(
            results[0].payload,
            Some(ResultPayload::QuickLink(_))
        ));

        let prompt = quick_link_results("job", &catalog, &preferences, &HashMap::new()).unwrap();
        assert_eq!(prompt.len(), 1);
        assert_eq!(prompt[0].kind, ResultKind::QuickLink);
        assert!(prompt[0].payload.is_none());

        let mut disabled = preferences.clone();
        disabled.quick_links = false;
        assert!(quick_link_results("job value", &catalog, &disabled, &HashMap::new()).is_none());
        assert!(
            quick_link_results("ordinary search", &catalog, &preferences, &HashMap::new(),)
                .is_none()
        );
    }

    #[test]
    fn explicit_script_filter_workflow_results_are_searchable() {
        let workflow = Workflow::from_manifest(
            crate::workflow::WorkflowManifest {
                id: Some("script-filter".to_owned()),
                name: "Script Filter".to_owned(),
                description: String::new(),
                keywords: vec!["sf".to_owned()],
                command: vec![
                    "printf".to_owned(),
                    r#"{"items":[{"title":"Choice","subtitle":"Pick one","arg":"selected"}]}"#
                        .to_owned(),
                ],
                icon: None,
                cwd: None,
                env: std::collections::BTreeMap::new(),
                script_filter: true,
                action: None,
                actions: vec![crate::workflow::WorkflowActionManifest {
                    title: "Open".to_owned(),
                    subtitle: "Open selected item".to_owned(),
                    command: vec!["true".to_owned(), "{arg}".to_owned()],
                    icon: Some("document-open".to_owned()),
                }],
                enabled: true,
            },
            "/tmp/script-filter.json",
        )
        .unwrap();
        let database = temporary_database("script-filter-search");
        let mut preferences = local_settings(false);
        preferences.workflow_search = true;
        let mut engine = test_engine(Vec::new(), database.clone(), preferences);
        engine.workflows = Arc::new(WorkflowCatalog::new(vec![workflow]));

        let results = engine.search("sf hello");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ResultKind::Workflow);
        assert_eq!(results[0].title, "Choice");
        match results[0].payload.as_ref() {
            Some(ResultPayload::WorkflowItem { matched, item }) => {
                assert_eq!(matched.query, "hello");
                assert_eq!(item.arg, "selected");
                assert_eq!(matched.workflow.actions[0].title, "Open");
            }
            payload => panic!("unexpected workflow payload: {payload:?}"),
        }
        remove_database(&database);
    }

    #[test]
    fn web_suggestions_are_unique_bounded_and_follow_primary_action_offline() {
        let web = WebSearchEngine::default();
        let primary = web.action_for("ddg", "rust").unwrap();
        let mut scored = vec![SearchResult::web(primary.clone(), WEB_PRIMARY_SCORE)];
        append_web_suggestions(
            &mut scored,
            &web,
            &primary,
            [
                "rust".to_owned(),
                "rust gtk".to_owned(),
                "Rust GTK".to_owned(),
                "rust linux".to_owned(),
                "rust book".to_owned(),
                "rust cargo".to_owned(),
                "rust wayland".to_owned(),
                "rust extra".to_owned(),
            ],
            &HashMap::new(),
        );

        assert_eq!(scored.len(), 6); // primary + at most five suggestions
        assert_eq!(scored[0].score, WEB_PRIMARY_SCORE);
        let queries: Vec<_> = scored
            .iter()
            .map(|item| match item.result.payload.as_ref() {
                Some(ResultPayload::Web(action)) => action.query.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(queries[0], "rust");
        assert_eq!(
            queries[1..],
            [
                "rust gtk",
                "rust linux",
                "rust book",
                "rust cargo",
                "rust wayland"
            ]
        );
    }

    #[test]
    fn learning_ranking_is_optional_and_affects_empty_and_normal_app_searches() {
        let apps = vec![
            test_app("App One", "/tmp/alter-app-one.desktop"),
            test_app("App Two", "/tmp/alter-app-two.desktop"),
        ];

        // With learning disabled, a search against a missing database must
        // not even create the usage schema.
        let disabled_database = temporary_database("learning-disabled");
        let disabled = test_engine(
            apps.clone(),
            disabled_database.clone(),
            local_settings(false),
        );
        let disabled_results = disabled.search("app");
        assert_eq!(disabled_results[0].title, "App One");
        assert!(!disabled_database.exists());

        let enabled_database = temporary_database("learning-enabled");
        crate::usage::record_use(
            &enabled_database,
            "app:/tmp/alter-app-two.desktop",
            "App Two",
            "app",
        )
        .unwrap();
        let enabled = test_engine(apps, enabled_database.clone(), local_settings(true));
        assert_eq!(enabled.search("app")[0].title, "App Two");
        assert_eq!(enabled.search("")[0].title, "App Two");

        remove_database(&disabled_database);
        remove_database(&enabled_database);
    }

    #[test]
    fn empty_search_can_promote_an_app_beyond_the_initial_viewport() {
        let apps: Vec<_> = (0..32)
            .map(|index| {
                test_app(
                    &format!("App {index:02}"),
                    &format!("/tmp/alter-app-{index:02}.desktop"),
                )
            })
            .collect();
        let database = temporary_database("empty-learning-depth");
        let key = "app:/tmp/alter-app-31.desktop";
        for _ in 0..20 {
            crate::usage::record_use(&database, key, "App 31", "app").unwrap();
        }

        let engine = test_engine(apps, database.clone(), local_settings(true));
        let results = engine.search("");
        assert_eq!(
            results.first().map(|result| result.title.as_str()),
            Some("App 31")
        );
        remove_database(&database);
    }

    #[test]
    fn file_backed_clipboard_results_expose_path_and_friendly_labels() {
        let item = ClipboardItem {
            id: -1,
            content: "/tmp/screenshot.PNG".to_owned(),
            created_at: 0,
            use_count: 0,
            external: true,
            pinned: false,
            file_path: Some(PathBuf::from("/tmp/screenshot.PNG")),
        };
        let result = SearchResult::clipboard(&item, 100).result;
        assert_eq!(result.title, "screenshot.PNG");
        assert!(result.subtitle.contains("图片剪贴板"));
        assert_eq!(result.clipboard_path, item.file_path);
    }

    #[test]
    fn file_backed_clipboard_search_checks_file_path_when_value_is_opaque() {
        let item = ClipboardItem {
            id: -7,
            content: "clipboard-internal-token".to_owned(),
            created_at: 0,
            use_count: 0,
            external: true,
            pinned: false,
            file_path: Some(PathBuf::from("/home/example/Pictures/Screen Shot 42.PNG")),
        };

        assert!(clipboard_match_score("screenshot", &item).is_some());
        assert!(clipboard_match_score(".png", &item).is_some());
        assert!(clipboard_match_score("not-present", &item).is_none());
    }
}
