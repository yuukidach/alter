//! Persistent usage statistics used to improve launcher ranking.
//!
//! Usage data lives beside Alter's clipboard database, but in its own table so
//! it can be enabled incrementally without changing the existing clipboard
//! schema.  The API is intentionally independent from GTK and the search
//! engine: callers record a stable key (for example `app:org.gnome.Calculator`)
//! and can add [`score_bonus`] to their normal fuzzy-match score.

use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SECONDS_PER_HOUR: i64 = 60 * 60;
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageItem {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub use_count: i64,
    pub last_used_at: i64,
}

/// Create the usage table and indexes if they do not exist yet.
///
/// `path` is normally [`crate::clipboard::database_path`].  The helper does
/// not create or inspect the clipboard table, so it is safe to call while the
/// clipboard watcher is running.  SQLite's busy timeout gives a concurrent
/// clipboard write a short opportunity to finish.
pub fn ensure_schema(path: &Path) -> rusqlite::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    let connection = open(path)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_stats (
            \"key\" TEXT PRIMARY KEY NOT NULL CHECK(length(\"key\") > 0),
            title TEXT NOT NULL CHECK(length(title) > 0),
            kind TEXT NOT NULL CHECK(length(kind) > 0),
            use_count INTEGER NOT NULL DEFAULT 0 CHECK(use_count >= 0),
            last_used_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS usage_stats_recent_idx
            ON usage_stats(last_used_at DESC, use_count DESC);
        CREATE INDEX IF NOT EXISTS usage_stats_kind_idx
            ON usage_stats(kind, last_used_at DESC, use_count DESC);
        ",
    )?;
    Ok(())
}

/// Record one explicit selection of an item.
///
/// The key should be stable across launches, while title and kind may be
/// refreshed whenever an item is selected.  No shell or external process is
/// involved; one SQLite upsert is used so concurrent launcher invocations do
/// not lose increments.
pub fn record_use(path: &Path, key: &str, title: &str, kind: &str) -> rusqlite::Result<()> {
    ensure_schema(path)?;
    let connection = open(path)?;
    connection.execute(
        "INSERT INTO usage_stats(\"key\", title, kind, use_count, last_used_at)
         VALUES (?1, ?2, ?3, 1, ?4)
         ON CONFLICT(\"key\") DO UPDATE SET
             title = excluded.title,
             kind = excluded.kind,
             use_count = usage_stats.use_count + 1,
             last_used_at = excluded.last_used_at",
        params![key, title, kind, now()],
    )?;
    Ok(())
}

/// Return a bounded ranking bonus for one key.
///
/// The bonus combines a small frequency component (capped at 80 points) with
/// a recency component (30 points for the last hour, decaying to zero after
/// thirty days).  Missing keys simply receive zero, which lets callers use
/// `score_bonus(...).unwrap_or_default()` while a database is unavailable.
pub fn score_bonus(path: &Path, key: &str) -> rusqlite::Result<i64> {
    ensure_schema(path)?;
    let connection = open(path)?;
    let values = connection
        .query_row(
            "SELECT use_count, last_used_at FROM usage_stats WHERE \"key\" = ?1",
            params![key],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    Ok(values
        .map(|(use_count, last_used_at)| score_bonus_from_values(use_count, last_used_at, now()))
        .unwrap_or_default())
}

/// Return all recently used items, newest first.
pub fn recent_items(path: &Path, limit: usize) -> rusqlite::Result<Vec<UsageItem>> {
    recent_items_filtered(path, None, limit)
}

/// Return recently used items for one kind, newest first.
pub fn recent_items_for_kind(
    path: &Path,
    kind: &str,
    limit: usize,
) -> rusqlite::Result<Vec<UsageItem>> {
    recent_items_filtered(path, Some(kind), limit)
}

fn recent_items_filtered(
    path: &Path,
    kind: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<UsageItem>> {
    ensure_schema(path)?;
    let connection = open(path)?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut items = Vec::new();
    if let Some(kind) = kind {
        let mut statement = connection.prepare(
            "SELECT \"key\", title, kind, use_count, last_used_at
             FROM usage_stats WHERE kind = ?1
             ORDER BY last_used_at DESC, use_count DESC, \"key\" ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![kind, limit], row_to_item)?;
        for row in rows {
            items.push(row?);
        }
    } else {
        let mut statement = connection.prepare(
            "SELECT \"key\", title, kind, use_count, last_used_at
             FROM usage_stats
             ORDER BY last_used_at DESC, use_count DESC, \"key\" ASC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], row_to_item)?;
        for row in rows {
            items.push(row?);
        }
    }
    Ok(items)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageItem> {
    Ok(UsageItem {
        key: row.get(0)?,
        title: row.get(1)?,
        kind: row.get(2)?,
        use_count: row.get(3)?,
        last_used_at: row.get(4)?,
    })
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

fn score_bonus_from_values(use_count: i64, last_used_at: i64, at: i64) -> i64 {
    let frequency = use_count.clamp(0, 20) * 4;
    let age = at.saturating_sub(last_used_at).max(0);
    let recency = if age <= SECONDS_PER_HOUR {
        30
    } else if age <= SECONDS_PER_DAY {
        20
    } else if age <= 7 * SECONDS_PER_DAY {
        10
    } else if age <= 30 * SECONDS_PER_DAY {
        4
    } else {
        0
    };
    frequency + recency
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn temporary_database(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        std::env::temp_dir().join(format!(
            "alter-usage-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_database(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn creates_schema_and_records_usage() {
        let path = temporary_database("schema");
        ensure_schema(&path).unwrap();
        record_use(&path, "app:calculator", "Calculator", "app").unwrap();
        record_use(&path, "app:calculator", "Calculator", "app").unwrap();

        let items = recent_items(&path, 10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "app:calculator");
        assert_eq!(items[0].title, "Calculator");
        assert_eq!(items[0].kind, "app");
        assert_eq!(items[0].use_count, 2);
        assert!(items[0].last_used_at > 0);
        remove_database(&path);
    }

    #[test]
    fn record_use_refreshes_metadata_without_resetting_count() {
        let path = temporary_database("metadata");
        record_use(&path, "file:notes", "Old title", "file").unwrap();
        record_use(&path, "file:notes", "New title", "document").unwrap();
        let item = recent_items(&path, 1).unwrap().pop().unwrap();
        assert_eq!(item.title, "New title");
        assert_eq!(item.kind, "document");
        assert_eq!(item.use_count, 2);
        remove_database(&path);
    }

    #[test]
    fn score_bonus_is_zero_for_unknown_and_increases_with_use() {
        let path = temporary_database("score");
        assert_eq!(score_bonus(&path, "missing"), Ok(0));
        record_use(&path, "app:one", "One", "app").unwrap();
        let first = score_bonus(&path, "app:one").unwrap();
        record_use(&path, "app:one", "One", "app").unwrap();
        let second = score_bonus(&path, "app:one").unwrap();
        assert!(first > 0);
        assert!(second > first);
        remove_database(&path);
    }

    #[test]
    fn score_bonus_recency_decays_and_frequency_is_capped() {
        let now = 100 * SECONDS_PER_DAY;
        assert_eq!(score_bonus_from_values(1, now, now), 34);
        assert_eq!(
            score_bonus_from_values(100, now - 31 * SECONDS_PER_DAY, now),
            80
        );
        assert!(
            score_bonus_from_values(2, now - 2 * SECONDS_PER_DAY, now)
                > score_bonus_from_values(2, now - 31 * SECONDS_PER_DAY, now)
        );
    }

    #[test]
    fn recent_items_are_newest_first_and_can_filter_kind() {
        let path = temporary_database("recent");
        ensure_schema(&path).unwrap();
        {
            let connection = open(&path).unwrap();
            connection
                .execute(
                    "INSERT INTO usage_stats(\"key\", title, kind, use_count, last_used_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params!["old", "Old", "app", 20, 10_i64],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO usage_stats(\"key\", title, kind, use_count, last_used_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params!["new", "New", "file", 1, 20_i64],
                )
                .unwrap();
        }
        let all = recent_items(&path, 10).unwrap();
        assert_eq!(
            all.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(),
            ["new", "old"]
        );
        let files = recent_items_for_kind(&path, "file", 10).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].key, "new");
        assert!(recent_items(&path, 0).unwrap().is_empty());
        remove_database(&path);
    }

    #[test]
    fn schema_is_idempotent() {
        let path = temporary_database("idempotent");
        ensure_schema(&path).unwrap();
        ensure_schema(&path).unwrap();
        record_use(&path, "x", "X", "test").unwrap();
        assert_eq!(recent_items(&path, 1).unwrap().len(), 1);
        remove_database(&path);
    }

    #[test]
    fn schema_accepts_a_database_path_without_a_parent_directory() {
        let name = format!(
            "alter-usage-relative-{}-{}.sqlite3",
            std::process::id(),
            now()
        );
        let path = PathBuf::from(&name);
        ensure_schema(&path).unwrap();
        record_use(&path, "relative", "Relative", "test").unwrap();
        assert_eq!(recent_items(&path, 1).unwrap().len(), 1);
        remove_database(&path);
    }
}
