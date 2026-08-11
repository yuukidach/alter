use crate::clipboard_meta;
use crate::paths;
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
const MAX_ITEMS: i64 = 500;

#[derive(Clone, Debug)]
pub struct ClipboardItem {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
    pub use_count: i64,
    pub external: bool,
    pub pinned: bool,
    pub file_path: Option<PathBuf>,
}

pub fn ensure_database(path: &Path) -> rusqlite::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }

    let connection = open(path)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS clipboard (
            id INTEGER PRIMARY KEY,
            content TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            last_used_at INTEGER,
            use_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS clipboard_created_at_idx
            ON clipboard(created_at DESC);
        ",
    )?;
    Ok(())
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_millis(500))?;
    Ok(connection)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn capture_stdin_with_retention(
    path: &Path,
    retention_days: u32,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    io::stdin()
        .take((MAX_CLIPBOARD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;

    if bytes.is_empty() || bytes.len() > MAX_CLIPBOARD_BYTES {
        return Ok(false);
    }

    // The watcher is started with `--type text`, so invalid UTF-8 is unusual.
    // Lossy conversion keeps one malformed clipboard item from killing the
    // watcher and still preserves all human-readable content.
    let content = String::from_utf8_lossy(&bytes).into_owned();
    if content.trim().is_empty() || content.contains('\0') {
        return Ok(false);
    }

    ensure_database(path)?;
    let connection = open(path)?;
    connection.execute(
        "INSERT INTO clipboard(content, created_at, last_used_at, use_count)
         VALUES (?1, ?2, NULL, 0)
         ON CONFLICT(content) DO UPDATE SET created_at = excluded.created_at",
        params![content, now()],
    )?;
    connection.execute(
        "DELETE FROM clipboard
         WHERE id NOT IN (SELECT id FROM clipboard ORDER BY created_at DESC LIMIT ?1)",
        params![MAX_ITEMS],
    )?;
    drop(connection);
    prune(path, retention_days)?;
    Ok(true)
}

pub fn recent_with_retention(
    path: &Path,
    limit: i64,
    retention_days: u32,
) -> rusqlite::Result<Vec<ClipboardItem>> {
    prune(path, retention_days)?;
    let mut items = recent_local(path, limit)?;
    let metadata = clipboard_metadata(path);
    apply_metadata(&mut items, &metadata);
    let cutoff = retention_cutoff(retention_days);
    // Clipse is already the clipboard daemon on the user's Hyprland setup.
    // Importing its JSON history makes Alter immediately useful without
    // starting a second watcher.  The integration is optional and silently
    // falls back to Alter's own SQLite store on other machines.
    if let Some(external_items) = recent_clipse(limit.max(0) as usize) {
        let mut seen: HashSet<String> = items.iter().map(clipboard_item_key).collect();
        let mut external_items: Vec<_> = external_items
            .into_iter()
            .filter(|item| seen.insert(clipboard_item_key(item)))
            .collect();
        apply_metadata(&mut external_items, &metadata);
        items.extend(external_items);
    }
    if let Some(cutoff) = cutoff {
        items.retain(|item| item.pinned || item.created_at >= cutoff);
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    items.truncate(limit.max(0) as usize);
    Ok(items)
}

fn clipboard_metadata(path: &Path) -> HashMap<String, (bool, bool)> {
    clipboard_meta::flagged_keys(path)
        .unwrap_or_default()
        .into_iter()
        .map(|item| (item.key, (item.pinned, item.hidden)))
        .collect()
}

fn apply_metadata(items: &mut Vec<ClipboardItem>, metadata: &HashMap<String, (bool, bool)>) {
    items.retain(|item| {
        let key = clipboard_item_key(item);
        metadata.get(&key).is_none_or(|(_, hidden)| !*hidden)
    });
    for item in items.iter_mut() {
        if let Some((pinned, _)) = metadata.get(&clipboard_item_key(item)).copied() {
            item.pinned |= pinned;
        }
    }
}

fn clipboard_item_key(item: &ClipboardItem) -> String {
    item.file_path
        .as_deref()
        .map(clipboard_meta::file_key)
        .unwrap_or_else(|| clipboard_meta::content_key(&item.content))
}

/// Remove local entries older than the configured retention period. A period
/// of zero is treated as unlimited, which keeps this helper useful for callers
/// that explicitly want to disable automatic expiry.
pub fn prune(path: &Path, retention_days: u32) -> rusqlite::Result<()> {
    if retention_days == 0 {
        ensure_database(path)?;
        return Ok(());
    }
    ensure_database(path)?;
    // Alter's pin state lives in a separate metadata table. Preserve old
    // local entries whose content is pinned there, otherwise a retention
    // cleanup would silently defeat Ctrl+Shift+P.
    let pinned_keys: HashSet<String> = clipboard_meta::flagged_keys(path)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item.pinned)
        .map(|item| item.key)
        .collect();
    let connection = open(path)?;
    let cutoff = retention_cutoff(retention_days).unwrap_or(i64::MIN);
    let mut statement =
        connection.prepare("SELECT id, content FROM clipboard WHERE created_at < ?1")?;
    let expired = statement
        .query_map(params![cutoff], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for (id, content) in expired {
        if !pinned_keys.contains(&clipboard_meta::content_key(&content)) {
            connection.execute("DELETE FROM clipboard WHERE id = ?1", params![id])?;
        }
    }
    Ok(())
}

fn retention_cutoff(retention_days: u32) -> Option<i64> {
    (retention_days != 0).then(|| now().saturating_sub(i64::from(retention_days) * 86_400))
}

fn recent_local(path: &Path, limit: i64) -> rusqlite::Result<Vec<ClipboardItem>> {
    ensure_database(path)?;
    let connection = open(path)?;
    let mut statement = connection.prepare(
        "SELECT id, content, created_at, use_count
         FROM clipboard ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = statement.query_map(params![limit], |row| {
        Ok(ClipboardItem {
            id: row.get(0)?,
            content: row.get(1)?,
            created_at: row.get(2)?,
            use_count: row.get(3)?,
            external: false,
            pinned: false,
            file_path: None,
        })
    })?;
    rows.collect()
}

#[derive(Debug, Deserialize)]
struct ClipseHistoryFile {
    #[serde(rename = "clipboardHistory", default)]
    clipboard_history: Vec<ClipseEntry>,
}

#[derive(Debug, Deserialize)]
struct ClipseEntry {
    value: Option<String>,
    recorded: Option<String>,
    #[serde(rename = "filePath")]
    file_path: Option<String>,
    pinned: Option<bool>,
}

fn clipse_config_path() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home_dir().join(".config"))
        .join("clipse/clipboard_history.json")
}

fn recent_clipse(limit: usize) -> Option<Vec<ClipboardItem>> {
    let path = clipse_config_path();
    let content = std::fs::read_to_string(path).ok()?;
    parse_clipse_history(&content, limit, now().saturating_sub(10))
}

fn parse_clipse_history(content: &str, limit: usize, base_time: i64) -> Option<Vec<ClipboardItem>> {
    let history: ClipseHistoryFile = serde_json::from_str(content).ok()?;
    Some(
        history
            .clipboard_history
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let file_path = item
                    .file_path
                    .as_deref()
                    .filter(|path| !path.is_empty() && *path != "null" && !path.contains('\0'))
                    .map(PathBuf::from);
                let value = item.value.filter(|value| !value.is_empty()).or_else(|| {
                    file_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                })?;
                if value.len() > MAX_CLIPBOARD_BYTES || value.contains('\0') {
                    return None;
                }
                // Clipse stores newest entries first.  Negative IDs cannot
                // collide with Alter's local SQLite IDs and are never updated
                // by mark_used().
                Some(ClipboardItem {
                    id: -((index as i64) + 1),
                    content: value,
                    created_at: item
                        .recorded
                        .as_deref()
                        .and_then(parse_recorded_timestamp)
                        .unwrap_or_else(|| base_time.saturating_sub(index as i64)),
                    use_count: 0,
                    external: true,
                    pinned: item.pinned.unwrap_or(false),
                    file_path,
                })
            })
            .take(limit)
            .collect(),
    )
}

/// Parse Clipse's `YYYY-MM-DD HH:MM:SS[.fraction]` timestamp without adding a
/// heavyweight date/time dependency. Clipse writes local time; treating it as
/// UTC is conservative for expiry, and future values are clamped by callers.
fn parse_recorded_timestamp(value: &str) -> Option<i64> {
    let (date, time) = value.trim().split_once(' ')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let clock = time.split('.').next()?;
    let mut time_parts = clock.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next()?.parse().ok()?;
    if time_parts.next().is_some()
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }

    days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)
}

// Howard Hinnant's Gregorian calendar conversion, returning days since the
// Unix epoch (1970-01-01).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let month = i64::from(month);
    let day = i64::from(day);
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year / 400
    } else {
        (adjusted_year - 399) / 400
    };
    let year_of_era = adjusted_year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn mark_used(path: &Path, id: i64) -> rusqlite::Result<()> {
    if id < 0 {
        return Ok(());
    }
    ensure_database(path)?;
    let connection = open(path)?;
    connection.execute(
        "UPDATE clipboard SET last_used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
        params![now(), id],
    )?;
    Ok(())
}

pub fn copy_to_wayland(content: &str) -> io::Result<()> {
    copy_reader_to_wayland(Cursor::new(content.as_bytes()), "text/plain;charset=utf-8")
}

/// Copy a file-backed Clipse entry to the Wayland clipboard.
///
/// Common image formats are streamed directly with their image MIME type so
/// applications can paste the image itself. Other files and directories are
/// advertised as an RFC 3986 encoded `text/uri-list`, which is the standard
/// clipboard representation understood by Wayland file managers. No shell is
/// involved and neither the path nor file contents are written to output.
pub fn copy_file_to_wayland(path: &Path) -> io::Result<()> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    std::fs::metadata(&absolute_path)?;

    if let Some(mime_type) = image_mime_type(&absolute_path) {
        return copy_reader_to_wayland(File::open(&absolute_path)?, mime_type);
    }

    let uri_list = format!("{}\r\n", file_uri(&absolute_path));
    copy_reader_to_wayland(Cursor::new(uri_list.into_bytes()), "text/uri-list")
}

fn copy_reader_to_wayland(mut source: impl Read, mime_type: &str) -> io::Result<()> {
    let mut child = Command::new("wl-copy")
        .args(["--type", mime_type])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "wl-copy stdin was unavailable",
        ));
    };
    let copy_result = io::copy(&mut source, &mut stdin).map(|_| ());
    drop(stdin);
    // wl-copy normally daemonizes after receiving the content.  Waiting here
    // only waits for the short foreground hand-off and keeps errors visible.
    let status = child.wait();
    copy_result?;
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("wl-copy failed to accept clipboard data"))
    }
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("png") {
        Some("image/png")
    } else if ["jpg", "jpeg", "jpe"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some("image/jpeg")
    } else if extension.eq_ignore_ascii_case("webp") {
        Some("image/webp")
    } else if extension.eq_ignore_ascii_case("gif") {
        Some("image/gif")
    } else if extension.eq_ignore_ascii_case("bmp") {
        Some("image/bmp")
    } else if ["tif", "tiff"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some("image/tiff")
    } else if ["svg", "svgz"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some("image/svg+xml")
    } else if extension.eq_ignore_ascii_case("avif") {
        Some("image/avif")
    } else {
        None
    }
}

fn file_uri(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut uri = String::with_capacity("file://".len() + bytes.len());
    uri.push_str("file://");
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(byte));
        } else {
            uri.push('%');
            uri.push(HEX[usize::from(byte >> 4)] as char);
            uri.push(HEX[usize::from(byte & 0x0f)] as char);
        }
    }
    uri
}

pub struct ClipboardWatcher {
    child: Child,
}

impl ClipboardWatcher {
    pub fn start(executable: &Path) -> io::Result<Option<Self>> {
        if clipse_monitor_running() && clipse_config_path().is_file() {
            return Ok(None);
        }
        let child = Command::new("wl-paste")
            .args(["--no-newline", "--type", "text", "--watch"])
            .arg(executable)
            .arg("--capture")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Some(Self { child }))
    }
}

fn clipse_monitor_running() -> bool {
    let uid = current_uid().to_string();
    ["wl-paste.*clipse.*wl-store", "clipse.*wl-store"]
        .iter()
        .any(|pattern| {
            Command::new("pgrep")
                .args(["-u", &uid, "-f", pattern])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })
}

#[cfg(unix)]
fn current_uid() -> u32 {
    Command::new("id")
        .args(["-u"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn database_path() -> PathBuf {
    paths::database_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_deduplicates_clipboard_items() {
        let path = std::env::temp_dir().join(format!("alter-test-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        ensure_database(&path).unwrap();
        let connection = open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard(content, created_at) VALUES ('hello', 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clipboard(content, created_at) VALUES ('world', 2)",
                [],
            )
            .unwrap();
        assert_eq!(recent_local(&path, 10).unwrap().len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parses_clipse_text_and_file_history() {
        let fixture = r#"{
            "clipboardHistory": [
                {"value": "newest", "recorded": "2026-01-01", "filePath": "null"},
                {"value": "image-path", "recorded": "2025-01-01", "filePath": "/tmp/image.png"},
                {"value": null, "recorded": "2025-01-01", "filePath": "/tmp/report.pdf", "pinned": true},
                {"value": "older", "recorded": "2025-01-01", "filePath": "null"}
            ]
        }"#;
        let items = parse_clipse_history(fixture, 10, 100).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].content, "newest");
        assert!(items[0].external);
        assert!(items[0].id < 0);
        assert_eq!(
            items[1].file_path.as_deref(),
            Some(Path::new("/tmp/image.png"))
        );
        assert_eq!(items[2].content, "/tmp/report.pdf");
        assert_eq!(
            items[2].file_path.as_deref(),
            Some(Path::new("/tmp/report.pdf"))
        );
        assert!(items[2].pinned);
        assert!(items[3].file_path.is_none());
    }

    #[test]
    fn file_clipboard_helpers_choose_mime_and_encode_uri() {
        assert_eq!(image_mime_type(Path::new("photo.PNG")), Some("image/png"));
        assert_eq!(image_mime_type(Path::new("photo.JpEg")), Some("image/jpeg"));
        assert_eq!(
            image_mime_type(Path::new("animation.webp")),
            Some("image/webp")
        );
        assert_eq!(image_mime_type(Path::new("document.pdf")), None);
        assert_eq!(
            file_uri(Path::new("/tmp/截图 #1.txt")),
            "file:///tmp/%E6%88%AA%E5%9B%BE%20%231.txt"
        );
    }

    #[test]
    fn file_entries_use_file_metadata_keys() {
        let path = PathBuf::from("/tmp/private-image.png");
        let item = ClipboardItem {
            id: -1,
            content: "display label".to_owned(),
            created_at: 1,
            use_count: 0,
            external: true,
            pinned: false,
            file_path: Some(path.clone()),
        };
        assert_eq!(clipboard_item_key(&item), clipboard_meta::file_key(&path));
    }

    #[test]
    fn parses_recorded_timestamp_and_prunes_expired_local_items() {
        assert_eq!(parse_recorded_timestamp("1970-01-01 00:00:00"), Some(0));
        assert_eq!(
            parse_recorded_timestamp("1970-01-02 00:00:00.123"),
            Some(86_400)
        );

        let path = std::env::temp_dir().join(format!(
            "alter-retention-test-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        ensure_database(&path).unwrap();
        let connection = open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard(content, created_at) VALUES (?1, ?2)",
                params!["expired", now().saturating_sub(3 * 86_400)],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO clipboard(content, created_at) VALUES (?1, ?2)",
                params!["fresh", now()],
            )
            .unwrap();
        drop(connection);

        // A local item pinned through Alter metadata must survive the same
        // retention pass that removes other expired rows.
        clipboard_meta::set_pinned(&path, &clipboard_meta::content_key("expired"), true).unwrap();

        prune(&path, 2).unwrap();
        let local_items = recent_local(&path, 10).unwrap();
        assert!(local_items.iter().any(|item| item.content == "expired"));
        assert!(local_items.iter().any(|item| item.content == "fresh"));
        let _ = std::fs::remove_file(path);
    }
}
