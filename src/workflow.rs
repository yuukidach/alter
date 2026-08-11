//! User-defined workflows for Alter.
//!
//! A workflow is deliberately described as an argv array rather than a shell
//! command.  For example:
//!
//! ```json
//! {
//!   "id": "search-github",
//!   "name": "Search GitHub",
//!   "description": "Open a GitHub search for the supplied words",
//!   "keyword": "gh",
//!   "command": ["xdg-open", "https://github.com/search?q={query}"]
//! }
//! ```
//!
//! Files in `~/.config/alter/workflows/` (or
//! `$XDG_CONFIG_HOME/alter/workflows/`) are loaded as independent JSON
//! manifests.  A malformed file is reported in [`WorkflowLoadReport`] while
//! valid files continue to load.  Nothing in this module executes a workflow
//! while loading or searching; callers explicitly opt in by calling
//! [`Workflow::execute`] or [`PreparedCommand::spawn`].

use crate::paths;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Maximum size accepted for one manifest.  This keeps a malformed or
/// accidentally huge file from consuming an unreasonable amount of memory
/// when the launcher starts.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_ARGUMENTS: usize = 128;
const MAX_ACTIONS: usize = 16;
const MAX_QUERY_BYTES: usize = 1024 * 1024;
// A query may occur more than once in a manifest.  Keep the total expanded
// argv/cwd/environment data bounded so a small manifest cannot trigger an
// unexpectedly gigantic allocation when a large query is supplied.
const MAX_PREPARED_BYTES: usize = 8 * 1024 * 1024;
const QUERY_PLACEHOLDER: &str = "{query}";
/// Script filters are opt-in because they execute a user-owned command while
/// an explicit workflow keyword is being searched. Keep each invocation
/// bounded so a broken script cannot freeze the launcher indefinitely.
const MAX_SCRIPT_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_SCRIPT_ITEMS: usize = 50;
const MAX_SCRIPT_FIELD_BYTES: usize = 16 * 1024;
const SCRIPT_FILTER_TIMEOUT: Duration = Duration::from_millis(800);
const ARG_PLACEHOLDER: &str = "{arg}";

/// JSON representation of a workflow.
///
/// `keyword` may be a single string and `keywords` may be an array.  Both are
/// accepted so small personal workflows stay pleasant to write.  `command`
/// must be an array; accepting a shell string here would make quoting and
/// injection mistakes very easy.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkflowManifest {
    /// Stable identifier.  If omitted, Alter derives one from the filename or
    /// display name.
    #[serde(default)]
    pub id: Option<String>,
    /// Human-readable title.  `title` is accepted as an alias for manifests
    /// written in an Alfred-like style.
    #[serde(default, alias = "title")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Trigger words.  `keyword` is accepted as an alias for one trigger.
    #[serde(
        default,
        alias = "keyword",
        deserialize_with = "deserialize_string_or_strings"
    )]
    pub keywords: Vec<String>,
    /// Executable followed by zero or more arguments.  This is intentionally
    /// an argv array, never a shell command string.
    pub command: Vec<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Optional working directory.  `{query}` is expanded before execution.
    #[serde(default, alias = "working_directory", alias = "workdir")]
    pub cwd: Option<String>,
    /// Extra environment variables.  Existing process variables are retained
    /// unless a key is explicitly replaced here.
    #[serde(default, alias = "environment")]
    pub env: BTreeMap<String, String>,
    /// When enabled, an explicit keyword invocation parses the command's
    /// stdout as Alfred-style result items. Plain workflows keep their old
    /// one-command behaviour.
    #[serde(default, alias = "scriptFilter", alias = "filter")]
    pub script_filter: bool,
    /// Optional argv command to run when a Script Filter item is activated.
    /// `{query}` is the original input and `{arg}` is the selected item.
    #[serde(default, alias = "result_command", alias = "on_select")]
    pub action: Option<Vec<String>>,
    /// Named alternatives exposed through Alter's Tab / Right action page.
    #[serde(default)]
    pub actions: Vec<WorkflowActionManifest>,
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

/// JSON representation of one named Workflow action.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkflowActionManifest {
    #[serde(default, alias = "name")]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// A validated action ready to appear in the launcher action page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowAction {
    pub title: String,
    pub subtitle: String,
    pub command: Vec<String>,
    pub icon: Option<String>,
}

/// A validated workflow ready to be searched or explicitly executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub command: Vec<String>,
    pub icon: Option<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub script_filter: bool,
    pub actions: Vec<WorkflowAction>,
    pub enabled: bool,
    /// Source manifest, useful for diagnostics and settings UIs.
    pub source: PathBuf,
}

/// One item emitted by an opt-in Script Filter workflow.
///
/// Alfred accepts JSON objects with `title`, `subtitle`, `arg`, and `icon`.
/// Alter keeps the same useful subset and also accepts a compact tab-separated
/// fallback for tiny scripts. `arg` is what gets passed to the workflow again
/// when the user activates an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowResultItem {
    pub title: String,
    pub subtitle: String,
    pub arg: String,
    pub icon: Option<String>,
}

impl WorkflowResultItem {
    fn validate(
        title: String,
        subtitle: String,
        arg: String,
        icon: Option<String>,
    ) -> Option<Self> {
        let title = title.trim().to_owned();
        let subtitle = subtitle.trim().to_owned();
        let arg = arg.trim().to_owned();
        let icon = icon
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if title.is_empty()
            || title.len() > MAX_SCRIPT_FIELD_BYTES
            || subtitle.len() > MAX_SCRIPT_FIELD_BYTES
            || arg.len() > MAX_SCRIPT_FIELD_BYTES
            || icon
                .as_deref()
                .is_some_and(|value| value.len() > MAX_SCRIPT_FIELD_BYTES)
            || [title.as_str(), subtitle.as_str(), arg.as_str()]
                .into_iter()
                .any(|value| value.contains('\0') || value.chars().any(char::is_control))
            || icon
                .as_deref()
                .is_some_and(|value| value.contains('\0') || value.chars().any(char::is_control))
        {
            return None;
        }
        Some(Self {
            title,
            subtitle,
            arg,
            icon,
        })
    }
}

fn validate_workflow_action(
    manifest: WorkflowActionManifest,
    source: &Option<PathBuf>,
    index: usize,
) -> Result<WorkflowAction, WorkflowError> {
    let mut command = manifest.command;
    if command.is_empty() {
        return Err(WorkflowError::invalid(
            source.clone(),
            format!("action {index} must contain an executable"),
        ));
    }
    if command.len() > MAX_ARGUMENTS {
        return Err(WorkflowError::invalid(
            source.clone(),
            format!("action {index} contains more than {MAX_ARGUMENTS} arguments"),
        ));
    }
    command[0] = command[0].trim().to_owned();
    if command[0].is_empty()
        || command[0].contains('\0')
        || command[0].chars().any(char::is_control)
        || command[0].contains(QUERY_PLACEHOLDER)
        || command[0].contains(ARG_PLACEHOLDER)
    {
        return Err(WorkflowError::invalid(
            source.clone(),
            format!(
                "action {index} executable cannot be empty, contain control characters, or use a placeholder"
            ),
        ));
    }
    if command
        .iter()
        .any(|argument| argument.contains('\0') || argument.chars().any(char::is_control))
    {
        return Err(WorkflowError::invalid(
            source.clone(),
            format!("action {index} arguments cannot contain NUL or control characters"),
        ));
    }

    let title = manifest.title.trim();
    let title = if title.is_empty() {
        format!("动作 {index}")
    } else {
        title.to_owned()
    };
    let subtitle = manifest.subtitle.trim().to_owned();
    let icon = manifest
        .icon
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if title.len() > 1024
        || subtitle.len() > 8 * 1024
        || title.chars().any(char::is_control)
        || subtitle.chars().any(char::is_control)
        || icon
            .as_deref()
            .is_some_and(|value| value.len() > 4096 || value.chars().any(char::is_control))
    {
        return Err(WorkflowError::invalid(
            source.clone(),
            format!("action {index} metadata contains control characters or is too long"),
        ));
    }
    Ok(WorkflowAction {
        title,
        subtitle,
        command,
        icon,
    })
}

impl Workflow {
    /// Validate and normalize a manifest.  `source` is retained only as an
    /// origin label; this method does not read or execute anything.
    pub fn from_manifest(
        manifest: WorkflowManifest,
        source: impl Into<PathBuf>,
    ) -> Result<Self, WorkflowError> {
        let source = source.into();
        let source_for_error = if source.as_os_str().is_empty() {
            None
        } else {
            Some(source.clone())
        };

        let name = manifest.name.trim().to_owned();
        let id = manifest
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if source.file_stem().is_some_and(|stem| !stem.is_empty()) {
                    source
                        .file_stem()
                        .map(|stem| slugify(&stem.to_string_lossy()))
                        .unwrap_or_default()
                } else {
                    slugify(&name)
                }
            });
        let id = if id.is_empty() { slugify(&name) } else { id };
        if id.is_empty() {
            return Err(WorkflowError::invalid(
                source_for_error,
                "workflow id and name cannot both be empty",
            ));
        }
        if id.contains('\0') || id.len() > 256 || id.chars().any(char::is_whitespace) {
            return Err(WorkflowError::invalid(
                source_for_error,
                "workflow id must not contain whitespace or NUL and must be at most 256 bytes",
            ));
        }

        let name = if name.is_empty() { id.clone() } else { name };
        if name.contains('\0') || name.len() > 1024 {
            return Err(WorkflowError::invalid(
                source_for_error,
                "workflow name is empty, contains NUL, or is too long",
            ));
        }

        if manifest.command.is_empty() {
            return Err(WorkflowError::invalid(
                source_for_error,
                "command must contain an executable",
            ));
        }
        if manifest.command.len() > MAX_ARGUMENTS {
            return Err(WorkflowError::invalid(
                source_for_error,
                format!("command contains more than {MAX_ARGUMENTS} arguments"),
            ));
        }

        let mut command = manifest.command;
        command[0] = command[0].trim().to_owned();
        if command[0].is_empty()
            || command[0].contains('\0')
            || command[0].chars().any(char::is_control)
        {
            return Err(WorkflowError::invalid(
                source_for_error,
                "command executable cannot be empty or contain NUL",
            ));
        }
        if command
            .iter()
            .any(|argument| argument.contains('\0') || argument.chars().any(char::is_control))
        {
            return Err(WorkflowError::invalid(
                source_for_error,
                "command arguments cannot contain NUL",
            ));
        }

        let action_count = manifest.actions.len() + usize::from(manifest.action.is_some());
        if action_count > MAX_ACTIONS {
            return Err(WorkflowError::invalid(
                source_for_error,
                format!("workflow contains more than {MAX_ACTIONS} actions"),
            ));
        }
        let mut actions = Vec::with_capacity(action_count);
        if let Some(command) = manifest.action {
            actions.push(validate_workflow_action(
                WorkflowActionManifest {
                    title: "执行".to_owned(),
                    subtitle: "使用当前候选参数执行".to_owned(),
                    command,
                    icon: Some("system-run".to_owned()),
                },
                &source_for_error,
                1,
            )?);
        }
        let action_offset = actions.len();
        for (index, action) in manifest.actions.into_iter().enumerate() {
            actions.push(validate_workflow_action(
                action,
                &source_for_error,
                action_offset + index + 1,
            )?);
        }

        let mut keywords = Vec::new();
        for keyword in manifest.keywords {
            let keyword = keyword.trim();
            if keyword.is_empty() || keyword.contains('\0') {
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
            // An id is a useful and predictable fallback trigger for a tiny
            // manifest that only specifies a name and command.
            keywords.push(id.clone());
        }

        let description = manifest.description.trim().to_owned();
        if description.contains('\0') || description.len() > 8 * 1024 {
            return Err(WorkflowError::invalid(
                source_for_error,
                "workflow description contains NUL or is too long",
            ));
        }

        let cwd = manifest.cwd.and_then(|value| {
            let value = value.trim().to_owned();
            (!value.is_empty()).then_some(value)
        });
        if cwd.as_deref().is_some_and(|value| {
            value.contains('\0') || value.chars().any(char::is_control) || value.len() > 4096
        }) {
            return Err(WorkflowError::invalid(
                source_for_error,
                "working directory cannot contain control characters or exceed 4096 bytes",
            ));
        }

        for key in manifest.env.keys() {
            if key.is_empty() || key.contains('=') || key.contains('\0') {
                return Err(WorkflowError::invalid(
                    source_for_error,
                    format!("invalid environment variable name: {key:?}"),
                ));
            }
        }
        if manifest.env.values().any(|value| value.contains('\0')) {
            return Err(WorkflowError::invalid(
                source_for_error,
                "environment values cannot contain NUL",
            ));
        }

        Ok(Self {
            id,
            name,
            description,
            keywords,
            command,
            icon: manifest.icon.filter(|value| !value.trim().is_empty()),
            cwd,
            env: manifest.env,
            script_filter: manifest.script_filter,
            actions,
            enabled: manifest.enabled,
            source,
        })
    }

    /// Parse and validate one JSON manifest.
    pub fn from_json(content: &str, source: impl Into<PathBuf>) -> Result<Self, WorkflowError> {
        let source = source.into();
        let manifest: WorkflowManifest =
            serde_json::from_str(content).map_err(|error| WorkflowError::Parse {
                path: source.clone(),
                source: error,
            })?;
        Self::from_manifest(manifest, source)
    }

    /// Return the text used by metadata/fuzzy search.
    pub fn search_text(&self) -> String {
        let mut text = self.name.clone();
        if !self.description.is_empty() {
            text.push(' ');
            text.push_str(&self.description);
        }
        for keyword in &self.keywords {
            text.push(' ');
            text.push_str(keyword);
        }
        text.push(' ');
        text.push_str(&self.id);
        text
    }

    /// Check whether `raw_query` invokes this workflow.  Alfred-like syntax
    /// is used: the query starts with a keyword, followed by optional input.
    /// Matching is case-insensitive and requires a whitespace boundary, so a
    /// keyword `gh` does not unexpectedly match `ghost`.
    pub fn match_query(&self, raw_query: &str) -> Option<WorkflowMatch> {
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

        // Longest keywords win when aliases overlap (`git` and `gitlab`).
        let mut keywords: Vec<&str> = self.keywords.iter().map(String::as_str).collect();
        keywords.sort_by_key(|keyword| std::cmp::Reverse(keyword.chars().count()));
        for keyword in keywords {
            let Some(suffix) = strip_prefix_case_insensitive(trimmed, keyword) else {
                continue;
            };
            if !suffix.is_empty() && !suffix.chars().next().is_some_and(char::is_whitespace) {
                continue;
            }
            let query = suffix.trim().to_owned();
            let score = 1_000 + keyword.chars().count() as i64 * 4;
            return Some(WorkflowMatch {
                workflow: self.clone(),
                keyword: keyword.to_owned(),
                query,
                score,
                invocation: true,
            });
        }
        None
    }

    /// Compute a metadata-search score for a query that does not necessarily
    /// contain a trigger keyword.
    pub fn search_score(&self, query: &str) -> Option<i64> {
        fuzzy_score(query, &self.search_text())
    }

    /// Expand `{query}` in the command, working directory, and environment,
    /// then return a command that can be spawned without invoking a shell.
    /// This method performs no process execution.
    pub fn prepare(&self, query: &str) -> Result<PreparedCommand, WorkflowError> {
        if !self.enabled {
            return Err(WorkflowError::Disabled {
                id: self.id.clone(),
            });
        }
        if query.len() > MAX_QUERY_BYTES
            || query.contains('\0')
            || query.chars().any(char::is_control)
        {
            return Err(WorkflowError::InvalidQuery {
                message: format!(
                    "query must not contain NUL/control characters and must be at most {MAX_QUERY_BYTES} bytes"
                ),
            });
        }
        if self.command.is_empty() {
            return Err(WorkflowError::invalid_for_workflow(
                &self.id,
                "command must contain an executable",
            ));
        }

        // The executable itself stays static.  Allowing user input to choose
        // the executable would turn a harmless search query into an implicit
        // command dispatcher; arguments are where workflows should consume
        // `{query}`.
        if self.command[0].contains("{query}") {
            return Err(WorkflowError::invalid_for_workflow(
                &self.id,
                "{query} is not allowed in the command executable",
            ));
        }

        let program = self.command[0].clone();
        let mut remaining = MAX_PREPARED_BYTES;
        let mut args = Vec::with_capacity(self.command.len().saturating_sub(1));
        for argument in &self.command[1..] {
            let substituted =
                substitute_query_bounded(argument, query, remaining).ok_or_else(|| {
                    WorkflowError::InvalidQuery {
                        message: format!(
                            "expanded workflow arguments exceed {MAX_PREPARED_BYTES} bytes"
                        ),
                    }
                })?;
            remaining = remaining.saturating_sub(substituted.len());
            args.push(substituted);
        }
        let cwd = self.cwd.as_deref().map(|value| {
            substitute_query_bounded(value, query, remaining).map(|substituted| {
                remaining = remaining.saturating_sub(substituted.len());
                expand_home(substituted)
            })
        });
        let cwd = match cwd {
            Some(Some(value)) => Some(value),
            Some(None) => {
                return Err(WorkflowError::InvalidQuery {
                    message: format!(
                        "expanded workflow arguments exceed {MAX_PREPARED_BYTES} bytes"
                    ),
                });
            }
            None => None,
        };
        let mut env = BTreeMap::new();
        for (key, value) in &self.env {
            let substituted =
                substitute_query_bounded(value, query, remaining).ok_or_else(|| {
                    WorkflowError::InvalidQuery {
                        message: format!(
                            "expanded workflow arguments exceed {MAX_PREPARED_BYTES} bytes"
                        ),
                    }
                })?;
            remaining = remaining.saturating_sub(substituted.len());
            env.insert(key.clone(), substituted);
        }

        Ok(PreparedCommand {
            program,
            args,
            cwd,
            env,
            workflow_id: self.id.clone(),
        })
    }

    /// Run an explicitly enabled Script Filter and parse its bounded stdout.
    /// The command is started with argv semantics, stderr is discarded, and
    /// a short timeout prevents a stalled user script from blocking search.
    pub fn script_filter_results(
        &self,
        query: &str,
    ) -> Result<Vec<WorkflowResultItem>, WorkflowError> {
        if !self.script_filter {
            return Ok(Vec::new());
        }
        let prepared = self.prepare(query)?;
        let mut child = prepared
            .spawn_capture()
            .map_err(|source| WorkflowError::Execution {
                workflow_id: self.id.clone(),
                source,
            })?;
        let mut stdout = child.stdout.take().ok_or_else(|| WorkflowError::Script {
            workflow_id: self.id.clone(),
            message: "script stdout was unavailable".to_owned(),
        })?;
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .by_ref()
                .take((MAX_SCRIPT_OUTPUT_BYTES + 1) as u64)
                .read_to_end(&mut bytes);
            (result, bytes)
        });

        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() < SCRIPT_FILTER_TIMEOUT => {
                    thread::sleep(Duration::from_millis(8));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(WorkflowError::Script {
                        workflow_id: self.id.clone(),
                        message: format!(
                            "script filter timed out after {} ms",
                            SCRIPT_FILTER_TIMEOUT.as_millis()
                        ),
                    });
                }
                Err(source) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(WorkflowError::Execution {
                        workflow_id: self.id.clone(),
                        source,
                    });
                }
            }
        };
        let (read_result, bytes) = reader.join().map_err(|_| WorkflowError::Script {
            workflow_id: self.id.clone(),
            message: "script output reader failed".to_owned(),
        })?;
        read_result.map_err(|source| WorkflowError::Execution {
            workflow_id: self.id.clone(),
            source,
        })?;
        let status = status.map_err(|source| WorkflowError::Execution {
            workflow_id: self.id.clone(),
            source,
        })?;
        if !status.success() {
            return Err(WorkflowError::Script {
                workflow_id: self.id.clone(),
                message: format!("script exited with status {status}"),
            });
        }
        parse_script_output(&bytes).map_err(|message| WorkflowError::Script {
            workflow_id: self.id.clone(),
            message,
        })
    }

    /// Execute the optional Script Filter action for a selected item. If no
    /// action command is configured, preserve the simple workflow behaviour
    /// by running the main command with the item's argument.
    pub fn execute_result(&self, query: &str, arg: &str) -> Result<Child, WorkflowError> {
        if self.actions.is_empty() {
            self.execute(arg)
        } else {
            self.execute_action(0, query, arg)
        }
    }

    pub fn execute_action(
        &self,
        index: usize,
        query: &str,
        arg: &str,
    ) -> Result<Child, WorkflowError> {
        let action = self.actions.get(index).ok_or_else(|| {
            WorkflowError::invalid_for_workflow(&self.id, format!("unknown action index {index}"))
        })?;
        let prepared = self.prepare_action(action, query, arg)?;
        prepared.spawn().map_err(|source| WorkflowError::Execution {
            workflow_id: self.id.clone(),
            source,
        })
    }

    fn prepare_action(
        &self,
        action: &WorkflowAction,
        query: &str,
        arg: &str,
    ) -> Result<PreparedCommand, WorkflowError> {
        if !self.enabled {
            return Err(WorkflowError::Disabled {
                id: self.id.clone(),
            });
        }
        for (label, value) in [("query", query), ("arg", arg)] {
            if value.len() > MAX_QUERY_BYTES
                || value.contains('\0')
                || value.chars().any(char::is_control)
            {
                return Err(WorkflowError::InvalidQuery {
                    message: format!(
                        "workflow {label} contains NUL, control characters, or is too long"
                    ),
                });
            }
        }
        let Some(program) = action.command.first() else {
            return Err(WorkflowError::invalid_for_workflow(
                &self.id,
                "action must contain an executable",
            ));
        };
        let mut remaining = MAX_PREPARED_BYTES;
        let mut args = Vec::with_capacity(action.command.len().saturating_sub(1));
        for template in &action.command[1..] {
            let substituted = substitute_action_bounded(template, query, arg, remaining)
                .ok_or_else(|| WorkflowError::InvalidQuery {
                    message: format!("expanded workflow action exceeds {MAX_PREPARED_BYTES} bytes"),
                })?;
            remaining = remaining.saturating_sub(substituted.len());
            args.push(substituted);
        }
        let cwd = self.cwd.as_deref().map(|template| {
            substitute_action_bounded(template, query, arg, remaining).map(|substituted| {
                remaining = remaining.saturating_sub(substituted.len());
                expand_home(substituted)
            })
        });
        let cwd = match cwd {
            Some(Some(value)) => Some(value),
            Some(None) => {
                return Err(WorkflowError::InvalidQuery {
                    message: format!("expanded workflow action exceeds {MAX_PREPARED_BYTES} bytes"),
                });
            }
            None => None,
        };
        let mut env = BTreeMap::new();
        for (key, template) in &self.env {
            let substituted = substitute_action_bounded(template, query, arg, remaining)
                .ok_or_else(|| WorkflowError::InvalidQuery {
                    message: format!("expanded workflow action exceeds {MAX_PREPARED_BYTES} bytes"),
                })?;
            remaining = remaining.saturating_sub(substituted.len());
            env.insert(key.clone(), substituted);
        }
        Ok(PreparedCommand {
            program: program.clone(),
            args,
            cwd,
            env,
            workflow_id: self.id.clone(),
        })
    }

    /// Explicitly spawn this workflow with argv semantics (never through a
    /// shell).  Loading and searching workflows never call this method.
    pub fn execute(&self, query: &str) -> Result<Child, WorkflowError> {
        self.prepare(query)?
            .spawn()
            .map_err(|source| WorkflowError::Execution {
                workflow_id: self.id.clone(),
                source,
            })
    }
}

/// A parsed invocation of a workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowMatch {
    pub workflow: Workflow,
    pub keyword: String,
    /// Text after the trigger keyword, with surrounding whitespace removed.
    pub query: String,
    pub score: i64,
    /// True when the user explicitly typed the workflow keyword. Script
    /// Filters are only executed for explicit invocations, never for a
    /// metadata-only fuzzy search.
    pub invocation: bool,
}

/// A command prepared with substituted arguments but not yet started.
///
/// Fields are read-only to callers; constructing this value through
/// [`Workflow::prepare`] guarantees the executable is static and strings do
/// not contain NUL bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCommand {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: BTreeMap<String, String>,
    workflow_id: String,
}

impl PreparedCommand {
    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    /// Spawn the prepared argv directly.  No shell, `sh -c`, or string
    /// re-parsing is involved.
    pub fn spawn(&self) -> io::Result<Child> {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        if !self.env.is_empty() {
            command.envs(&self.env);
        }
        command.spawn()
    }

    fn spawn_capture(&self) -> io::Result<Child> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        if !self.env.is_empty() {
            command.envs(&self.env);
        }
        command.spawn()
    }
}

/// A collection of workflows loaded from a directory.
#[derive(Clone, Debug, Default)]
pub struct WorkflowCatalog {
    workflows: Vec<Workflow>,
}

/// Alias emphasizing that the catalog is also the plugin registry for now.
pub type WorkflowStore = WorkflowCatalog;

impl WorkflowCatalog {
    pub fn new(mut workflows: Vec<Workflow>) -> Self {
        workflows.sort_by_cached_key(|workflow| workflow.name.to_lowercase());
        Self { workflows }
    }

    pub fn workflows(&self) -> &[Workflow] {
        &self.workflows
    }

    pub fn is_empty(&self) -> bool {
        self.workflows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.workflows.len()
    }

    /// Find workflows invoked by an Alfred-style trigger query (`gh rust`).
    pub fn matching(&self, raw_query: &str) -> Vec<WorkflowMatch> {
        let mut matches: Vec<_> = self
            .workflows
            .iter()
            .filter_map(|workflow| workflow.match_query(raw_query))
            .collect();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.workflow.name.cmp(&right.workflow.name))
        });
        matches
    }

    /// Search workflow metadata as well as trigger keywords.  A query that
    /// starts with a trigger only returns invocation matches; otherwise the
    /// result can be shown in Alter's regular search list and then invoked
    /// with an empty parameter.
    pub fn search(&self, query: &str) -> Vec<WorkflowMatch> {
        let invocation = self.matching(query);
        if !invocation.is_empty() {
            return invocation;
        }

        let mut matches: Vec<_> = self
            .workflows
            .iter()
            .filter(|workflow| workflow.enabled)
            .filter_map(|workflow| {
                workflow.search_score(query).map(|score| WorkflowMatch {
                    workflow: workflow.clone(),
                    keyword: workflow.keywords.first().cloned().unwrap_or_default(),
                    query: String::new(),
                    score,
                    invocation: false,
                })
            })
            .collect();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.workflow.name.cmp(&right.workflow.name))
        });
        matches
    }
}

/// Diagnostics returned while loading a workflow directory.  Valid manifests
/// are retained even when one or more sibling files are malformed.
#[derive(Debug, Default)]
pub struct WorkflowLoadReport {
    pub workflows: Vec<Workflow>,
    pub errors: Vec<WorkflowError>,
}

impl WorkflowLoadReport {
    pub fn catalog(self) -> WorkflowCatalog {
        WorkflowCatalog::new(self.workflows)
    }
}

/// Return the default user workflow directory.
pub fn workflow_dir() -> PathBuf {
    paths::config_dir().join("workflows")
}

/// Load workflows from the user's default config directory.  This is a
/// read-only operation; it does not create the directory and does not execute
/// any command.
pub fn load_workflows() -> WorkflowLoadReport {
    load_workflows_from(&workflow_dir())
}

/// Load direct `*.json` children of `directory`, collecting per-file errors.
/// Symlinks and nested directories are ignored intentionally so a workflow
/// cannot silently pull manifests from an unrelated tree.
pub fn load_workflows_from(directory: &Path) -> WorkflowLoadReport {
    let mut report = WorkflowLoadReport::default();
    let read_dir = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return report,
        Err(error) => {
            report.errors.push(WorkflowError::Io {
                path: directory.to_path_buf(),
                source: error,
            });
            return report;
        }
    };

    let mut paths = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                let is_json = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
                if is_json && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    paths.push(path);
                }
            }
            Err(error) => report.errors.push(WorkflowError::Io {
                path: directory.to_path_buf(),
                source: error,
            }),
        }
    }
    paths.sort();

    let mut seen_ids = BTreeMap::<String, PathBuf>::new();
    for path in paths {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                report.errors.push(WorkflowError::Io {
                    path,
                    source: error,
                });
                continue;
            }
        };
        if metadata.len() > MAX_MANIFEST_BYTES {
            report.errors.push(WorkflowError::invalid(
                Some(path),
                format!("manifest exceeds {MAX_MANIFEST_BYTES} bytes"),
            ));
            continue;
        }
        let content = match read_bounded_text(&path, MAX_MANIFEST_BYTES) {
            Ok(content) => content,
            Err(error) => {
                report.errors.push(WorkflowError::Io {
                    path,
                    source: error,
                });
                continue;
            }
        };
        let workflow = match Workflow::from_json(&content, path.clone()) {
            Ok(workflow) => workflow,
            Err(error) => {
                report.errors.push(error);
                continue;
            }
        };
        let id_key = workflow.id.to_lowercase();
        if let Some(previous) = seen_ids.get(&id_key) {
            report.errors.push(WorkflowError::invalid(
                Some(path),
                format!(
                    "duplicate workflow id (already defined by {})",
                    previous.display()
                ),
            ));
            continue;
        }
        seen_ids.insert(id_key, path.clone());
        report.workflows.push(workflow);
    }

    report
        .workflows
        .sort_by_cached_key(|workflow| workflow.name.to_lowercase());
    report
}

/// Expand every `{query}` placeholder in one template string.
pub fn substitute_query(template: &str, query: &str) -> String {
    template.replace(QUERY_PLACEHOLDER, query)
}

/// Expand `{query}` in each argv element while preserving argument boundaries.
pub fn substitute_arguments(arguments: &[String], query: &str) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| substitute_query(argument, query))
        .collect()
}

/// A workflow loading or execution failure.
#[derive(Debug)]
pub enum WorkflowError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidManifest {
        path: Option<PathBuf>,
        message: String,
    },
    Disabled {
        id: String,
    },
    InvalidQuery {
        message: String,
    },
    Execution {
        workflow_id: String,
        source: io::Error,
    },
    Script {
        workflow_id: String,
        message: String,
    },
}

impl WorkflowError {
    fn invalid(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self::InvalidManifest {
            path,
            message: message.into(),
        }
    }

    fn invalid_for_workflow(id: &str, message: impl Into<String>) -> Self {
        Self::InvalidManifest {
            path: None,
            message: format!("workflow {id:?}: {}", message.into()),
        }
    }
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Parse { path, source } => {
                write!(formatter, "{}: invalid JSON: {source}", path.display())
            }
            Self::InvalidManifest {
                path: Some(path),
                message,
            } => {
                write!(formatter, "{}: {message}", path.display())
            }
            Self::InvalidManifest {
                path: None,
                message,
            } => formatter.write_str(message),
            Self::Disabled { id } => write!(formatter, "workflow {id:?} is disabled"),
            Self::InvalidQuery { message } => formatter.write_str(message),
            Self::Execution {
                workflow_id,
                source,
            } => {
                write!(
                    formatter,
                    "workflow {workflow_id:?} could not be started: {source}"
                )
            }
            Self::Script {
                workflow_id,
                message,
            } => write!(
                formatter,
                "workflow {workflow_id:?} script filter failed: {message}"
            ),
        }
    }
}

impl Error for WorkflowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Execution { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn expand_home(value: String) -> PathBuf {
    if value == "~" {
        paths::home_dir()
    } else if let Some(rest) = value.strip_prefix("~/") {
        paths::home_dir().join(rest)
    } else {
        PathBuf::from(value)
    }
}

/// Parse the two deliberately small Script Filter formats supported by Alter:
/// Alfred-compatible JSON (`[{"title":...,"arg":...}]` or
/// `{"items":[...]}`), and one `title<TAB>arg` item per line. Invalid items
/// are isolated so one malformed line does not discard useful siblings.
fn parse_script_output(bytes: &[u8]) -> Result<Vec<WorkflowResultItem>, String> {
    if bytes.len() > MAX_SCRIPT_OUTPUT_BYTES {
        return Err(format!(
            "script output exceeds {MAX_SCRIPT_OUTPUT_BYTES} bytes"
        ));
    }
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| "script output must be valid UTF-8".to_owned())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if matches!(trimmed.as_bytes().first(), Some(b'{') | Some(b'[')) {
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .map_err(|error| format!("script JSON output is invalid: {error}"))?;
        return parse_script_json_value(value);
    }

    let mut items = Vec::new();
    for line in text.lines().take(MAX_SCRIPT_ITEMS) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (title, arg) = line.split_once('\t').unwrap_or((line, line));
        if let Some(item) =
            WorkflowResultItem::validate(title.to_owned(), String::new(), arg.to_owned(), None)
        {
            items.push(item);
        }
    }
    Ok(items)
}

fn parse_script_json_value(value: serde_json::Value) -> Result<Vec<WorkflowResultItem>, String> {
    let values = match value {
        serde_json::Value::Array(values) => values,
        serde_json::Value::Object(mut object) => match object.remove("items") {
            Some(serde_json::Value::Array(values)) => values,
            Some(_) => return Err("script JSON `items` must be an array".to_owned()),
            None => vec![serde_json::Value::Object(object)],
        },
        _ => return Err("script JSON output must be an object or array".to_owned()),
    };

    let mut items = Vec::new();
    for value in values.into_iter().take(MAX_SCRIPT_ITEMS) {
        let serde_json::Value::Object(object) = value else {
            continue;
        };
        let arg = object
            .get("arg")
            .or_else(|| object.get("value"))
            .and_then(json_scalar_text);
        let title = object
            .get("title")
            .and_then(json_scalar_text)
            .or_else(|| arg.clone());
        let Some(title) = title else {
            continue;
        };
        let arg = arg.unwrap_or_else(|| title.clone());
        let subtitle = object
            .get("subtitle")
            .and_then(json_scalar_text)
            .unwrap_or_default();
        let icon = object.get("icon").and_then(|value| match value {
            serde_json::Value::Object(object) => object.get("path").and_then(json_scalar_text),
            other => json_scalar_text(other),
        });
        if let Some(item) = WorkflowResultItem::validate(title, subtitle, arg, icon) {
            items.push(item);
        }
    }
    Ok(items)
}

fn json_scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Expand a query without allowing an unbounded allocation.  The public
/// `substitute_query` helper retains its historical infallible API; workflow
/// execution uses this checked variant instead.
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

fn substitute_action_bounded(
    template: &str,
    query: &str,
    arg: &str,
    max_bytes: usize,
) -> Option<String> {
    let query_occurrences = template.match_indices(QUERY_PLACEHOLDER).count();
    let arg_occurrences = template.match_indices(ARG_PLACEHOLDER).count();
    let removed = query_occurrences
        .checked_mul(QUERY_PLACEHOLDER.len())?
        .checked_add(arg_occurrences.checked_mul(ARG_PLACEHOLDER.len())?)?;
    let base = template.len().checked_sub(removed)?;
    let added = query_occurrences
        .checked_mul(query.len())?
        .checked_add(arg_occurrences.checked_mul(arg.len())?)?;
    let total = base.checked_add(added)?;
    if total > max_bytes {
        return None;
    }
    Some(
        template
            .replace(QUERY_PLACEHOLDER, query)
            .replace(ARG_PLACEHOLDER, arg),
    )
}

/// Read a configuration file with a hard upper bound, including when the
/// file changes size between metadata inspection and the read.
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

/// Return the suffix after a Unicode-safe, case-insensitive prefix match.
/// Slicing at `keyword.len()` directly can panic when the input starts with a
/// different multi-byte character, so the boundary is derived from character
/// count first.
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
    let mut result = String::new();
    let mut separator = false;
    for character in value.trim().chars() {
        if character.is_alphanumeric() {
            result.extend(character.to_lowercase());
            separator = false;
        } else if !result.is_empty() && !separator {
            result.push('-');
            separator = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    result
}

/// Small subsequence matcher used by the catalog without coupling the
/// workflow backend to the UI search module.
fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    if query.len() > MAX_QUERY_BYTES || query.contains('\0') || query.chars().any(char::is_control)
    {
        return None;
    }
    let query: Vec<char> = query.trim().to_lowercase().chars().collect();
    if query.is_empty() {
        return None;
    }
    let haystack: Vec<char> = text.to_lowercase().chars().collect();
    if haystack.is_empty() {
        return None;
    }
    let mut query_index = 0;
    let mut score = 0i64;
    let mut previous = None;
    for (index, character) in haystack.iter().enumerate() {
        if query_index >= query.len() || *character != query[query_index] {
            continue;
        }
        score += 10;
        if index == 0 || !haystack[index - 1].is_alphanumeric() {
            score += 22;
        }
        if let Some(last) = previous {
            let gap = index.saturating_sub(last + 1);
            score += if gap == 0 { 9 } else { -(gap.min(12) as i64) };
        }
        previous = Some(index);
        query_index += 1;
    }
    (query_index == query.len()).then_some(score - haystack.len().min(100) as i64 / 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workflow_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "alter-workflow-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_single_keyword_and_argv_manifest() {
        let workflow = Workflow::from_json(
            r#"{
                "name": "Search GitHub",
                "keyword": "gh",
                "command": ["xdg-open", "https://github.com/search?q={query}"]
            }"#,
            "/tmp/github.json",
        )
        .expect("manifest should parse");
        assert_eq!(workflow.name, "Search GitHub");
        assert_eq!(workflow.keywords, ["gh"]);
        assert_eq!(workflow.command[0], "xdg-open");
        assert_eq!(workflow.id, "github");
        assert!(!workflow.script_filter);
        assert!(workflow.enabled);
    }

    #[test]
    fn parses_keyword_array_and_optional_fields() {
        let workflow = Workflow::from_json(
            r#"{
                "id": "translate",
                "title": "Translate",
                "keywords": ["tr", "translate", "tr"],
                "description": "Translate text",
                "command": ["translator", "--text", "{query}"],
                "working_directory": "~/bin",
                "environment": {"LANGUAGE": "en", "QUERY": "{query}"},
                "enabled": false
            }"#,
            "/tmp/translate.json",
        )
        .expect("manifest should parse");
        assert_eq!(workflow.id, "translate");
        assert_eq!(workflow.keywords, ["tr", "translate"]);
        assert_eq!(workflow.cwd.as_deref(), Some("~/bin"));
        assert_eq!(
            workflow.env.get("QUERY").map(String::as_str),
            Some("{query}")
        );
        assert!(!workflow.enabled);
    }

    #[test]
    fn rejects_shell_string_command() {
        let error = Workflow::from_json(
            r#"{"name":"Bad","keyword":"bad","command":"sh -c echo"}"#,
            "/tmp/bad.json",
        )
        .expect_err("a shell command string must not be accepted");
        assert!(matches!(error, WorkflowError::Parse { .. }));
    }

    #[test]
    fn substitutes_query_without_merging_argv_arguments() {
        let arguments = vec![
            "--query={query}".to_owned(),
            "literal words".to_owned(),
            "{query}/{query}".to_owned(),
        ];
        assert_eq!(
            substitute_arguments(&arguments, "rust lang"),
            ["--query=rust lang", "literal words", "rust lang/rust lang"]
        );
    }

    #[test]
    fn matches_keyword_and_extracts_remainder() {
        let workflow = Workflow::from_json(
            r#"{"id":"gh","name":"GitHub","keywords":["git","gh"],"command":["xdg-open","{query}"]}"#,
            "/tmp/gh.json",
        )
        .unwrap();
        let invocation = workflow.match_query("GH   rust lang").unwrap();
        assert_eq!(invocation.keyword, "gh");
        assert_eq!(invocation.query, "rust lang");
        assert!(invocation.invocation);
        assert!(workflow.match_query("ghost").is_none());
    }

    #[test]
    fn unicode_prefix_mismatch_does_not_panic() {
        let workflow = Workflow::from_json(
            r#"{"id":"签名","name":"签名","keyword":"签名","command":["true"]}"#,
            "/tmp/signature.json",
        )
        .unwrap();
        // The first character has a different UTF-8 width from the keyword.
        // Prefix matching must not slice at a non-character boundary.
        assert!(workflow.match_query("签字 hello").is_none());
        assert!(workflow.match_query("签名 hello").is_some());
    }

    #[test]
    fn prepare_expands_query_and_home_without_running_process() {
        let workflow = Workflow::from_json(
            r#"{
                "id":"open",
                "name":"Open",
                "keyword":"o",
                "command":["xdg-open","https://example.test/{query}"],
                "cwd":"~/work",
                "environment":{"ALTER_QUERY":"{query}"}
            }"#,
            "/tmp/open.json",
        )
        .unwrap();
        let prepared = workflow.prepare("a b").unwrap();
        assert_eq!(prepared.program(), "xdg-open");
        assert_eq!(prepared.args(), ["https://example.test/a b"]);
        assert_eq!(
            prepared.cwd(),
            Some(paths::home_dir().join("work").as_path())
        );
        assert_eq!(
            prepared.env().get("ALTER_QUERY").map(String::as_str),
            Some("a b")
        );
    }

    #[test]
    fn refuses_query_in_executable_and_disabled_workflows() {
        let executable = Workflow::from_json(
            r#"{"name":"Bad","keyword":"bad","command":["{query}","x"]}"#,
            "/tmp/bad-executable.json",
        )
        .unwrap();
        assert!(matches!(
            executable.prepare("echo"),
            Err(WorkflowError::InvalidManifest { .. })
        ));

        let disabled = Workflow::from_json(
            r#"{"name":"Off","keyword":"off","enabled":false,"command":["true"]}"#,
            "/tmp/off.json",
        )
        .unwrap();
        assert!(matches!(
            disabled.prepare("anything"),
            Err(WorkflowError::Disabled { .. })
        ));
    }

    #[test]
    fn bounds_repeated_query_expansion_before_allocating() {
        let workflow = Workflow::from_manifest(
            WorkflowManifest {
                id: Some("bounded".to_owned()),
                name: "Bounded".to_owned(),
                description: String::new(),
                keywords: vec!["bound".to_owned()],
                command: vec!["true".to_owned(), "{query}".repeat(32)],
                icon: None,
                cwd: None,
                env: BTreeMap::new(),
                script_filter: false,
                action: None,
                actions: Vec::new(),
                enabled: true,
            },
            "/tmp/bounded.json",
        )
        .unwrap();
        let query = "x".repeat(MAX_QUERY_BYTES);
        assert!(matches!(
            workflow.prepare(&query),
            Err(WorkflowError::InvalidQuery { .. })
        ));
    }

    #[test]
    fn loader_keeps_valid_manifests_and_reports_invalid_files() {
        let directory = temp_workflow_dir("loader");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("valid.json"),
            r#"{"id":"valid","name":"Valid","keyword":"v","command":["true"]}"#,
        )
        .unwrap();
        fs::write(directory.join("broken.json"), "{not json").unwrap();
        fs::write(directory.join("ignored.txt"), "not a manifest").unwrap();

        let report = load_workflows_from(&directory);
        assert_eq!(report.workflows.len(), 1);
        assert_eq!(report.workflows[0].id, "valid");
        assert_eq!(report.errors.len(), 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn catalog_searches_metadata_and_invocations() {
        let first = Workflow::from_json(
            r#"{"id":"gh","name":"GitHub","keyword":"gh","command":["true"]}"#,
            "/tmp/gh.json",
        )
        .unwrap();
        let second = Workflow::from_json(
            r#"{"id":"tr","name":"Translate","keyword":"tr","command":["true"]}"#,
            "/tmp/tr.json",
        )
        .unwrap();
        let catalog = WorkflowCatalog::new(vec![second, first]);
        let invocation = catalog.search("gh rust");
        assert_eq!(invocation.len(), 1);
        assert_eq!(invocation[0].query, "rust");
        let metadata = catalog.search("trans");
        assert_eq!(metadata[0].workflow.id, "tr");
        assert!(!metadata[0].invocation);
    }

    #[test]
    fn script_filter_parses_alfred_json_and_runs_with_a_timeout_bound() {
        let workflow = Workflow::from_manifest(
            WorkflowManifest {
                id: Some("filter".to_owned()),
                name: "Filter".to_owned(),
                description: String::new(),
                keywords: vec!["f".to_owned()],
                command: vec![
                    "printf".to_owned(),
                    r#"{"items":[{"title":"One","subtitle":"First","arg":"alpha","icon":"folder"},{"title":"二","arg":42}]}"#
                        .to_owned(),
                ],
                icon: None,
                cwd: None,
                env: BTreeMap::new(),
                script_filter: true,
                action: None,
                actions: Vec::new(),
                enabled: true,
            },
            "/tmp/filter.json",
        )
        .unwrap();
        assert!(workflow.script_filter);
        let items = workflow.script_filter_results("ignored").unwrap();
        assert_eq!(
            items,
            [
                WorkflowResultItem {
                    title: "One".to_owned(),
                    subtitle: "First".to_owned(),
                    arg: "alpha".to_owned(),
                    icon: Some("folder".to_owned()),
                },
                WorkflowResultItem {
                    title: "二".to_owned(),
                    subtitle: String::new(),
                    arg: "42".to_owned(),
                    icon: None,
                },
            ]
        );
    }

    #[test]
    fn script_filter_accepts_compact_tab_lines_and_bounds_output() {
        let items = parse_script_output(b"Alpha\tvalue-a\nBeta\n\n").unwrap();
        assert_eq!(items[0].arg, "value-a");
        assert_eq!(items[1].title, "Beta");
        assert_eq!(items[1].arg, "Beta");

        let oversized = vec![b'x'; MAX_SCRIPT_OUTPUT_BYTES + 1];
        assert!(parse_script_output(&oversized).is_err());
        assert!(parse_script_output(b"\xff").is_err());
    }

    #[test]
    fn script_filter_stops_a_stalled_process_at_the_deadline() {
        let workflow = Workflow::from_json(
            r#"{"id":"slow","name":"Slow","keyword":"slow","script_filter":true,"command":["sleep","2"]}"#,
            "/tmp/slow.json",
        )
        .unwrap();
        let started = Instant::now();
        let error = workflow
            .script_filter_results("")
            .expect_err("a stalled filter must time out");
        assert!(matches!(error, WorkflowError::Script { .. }));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn script_filter_action_expands_query_and_selected_argument_without_shelling() {
        let workflow = Workflow::from_manifest(
            WorkflowManifest {
                id: Some("action".to_owned()),
                name: "Action".to_owned(),
                description: String::new(),
                keywords: vec!["act".to_owned()],
                command: vec!["true".to_owned()],
                icon: None,
                cwd: Some("~/work/{arg}".to_owned()),
                env: BTreeMap::from([("ORIGINAL".to_owned(), "{query}".to_owned())]),
                script_filter: true,
                action: Some(vec![
                    "printf".to_owned(),
                    "{query}".to_owned(),
                    "{arg}".to_owned(),
                ]),
                actions: vec![
                    WorkflowActionManifest {
                        title: "打开".to_owned(),
                        subtitle: "用默认程序打开".to_owned(),
                        command: vec!["printf".to_owned(), "{arg}".to_owned()],
                        icon: Some("document-open".to_owned()),
                    },
                    WorkflowActionManifest {
                        title: "复制".to_owned(),
                        subtitle: String::new(),
                        command: vec!["printf".to_owned(), "{query}".to_owned()],
                        icon: None,
                    },
                ],
                enabled: true,
            },
            "/tmp/action.json",
        )
        .unwrap();
        assert_eq!(workflow.actions.len(), 3);
        assert_eq!(workflow.actions[1].title, "打开");
        assert_eq!(workflow.actions[2].title, "复制");
        let prepared = workflow
            .prepare_action(
                workflow.actions.first().unwrap(),
                "original query",
                "selected value",
            )
            .unwrap();
        assert_eq!(prepared.program(), "printf");
        assert_eq!(prepared.args(), ["original query", "selected value"]);
        assert_eq!(
            prepared.cwd(),
            Some(paths::home_dir().join("work/selected value").as_path())
        );
        assert_eq!(
            prepared.env().get("ORIGINAL").map(String::as_str),
            Some("original query")
        );

        let invalid = Workflow::from_json(
            r#"{"name":"Bad action","keyword":"bad","command":["true"],"action":["{arg}"]}"#,
            "/tmp/bad-action.json",
        )
        .expect_err("action executable must stay static");
        assert!(matches!(invalid, WorkflowError::InvalidManifest { .. }));
    }
}
