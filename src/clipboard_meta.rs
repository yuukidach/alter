//! Alter-owned metadata for clipboard entries.
//!
//! Clipse remains the source of clipboard history on systems where it is
//! installed.  Alter must not rewrite Clipse's JSON just to remember a UI
//! preference, so pinned/hidden state is kept in a small table in Alter's own
//! SQLite database.  Callers choose a stable key; [`content_key`] and
//! [`file_key`] provide collision-resistant conventions for the two common
//! clipboard entry types.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fs;
use std::io;
use std::path::Path;

/// Prefix used by [`content_key`].
pub const CONTENT_KEY_PREFIX: &str = "content:";
/// Versioned prefix used for the hashed representation returned by
/// [`content_key`].
pub const CONTENT_HASH_KEY_PREFIX: &str = "content:sha256:";
/// Prefix used by [`file_key`].
pub const FILE_KEY_PREFIX: &str = "file:";

const LEGACY_CONTENT_KEY_PREFIX: &str = CONTENT_KEY_PREFIX;
const CONTENT_KEY_MIGRATION: i64 = 1;

/// Create a stable key for textual clipboard content.
///
/// Only a SHA-256 digest is retained. This lets callers match entries imported
/// from Clipse without copying potentially sensitive clipboard text into the
/// metadata table. The versioned prefix keeps the representation explicit and
/// distinct from a file path with identical bytes.
pub fn content_key(content: &str) -> String {
    let digest = sha256(content.as_bytes());
    let mut key = String::with_capacity(CONTENT_HASH_KEY_PREFIX.len() + digest.len() * 2);
    key.push_str(CONTENT_HASH_KEY_PREFIX);
    for byte in digest {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        key.push(HEX[usize::from(byte >> 4)] as char);
        key.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    key
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_length = (input.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((input.len() + 72) & !63);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut state = INITIAL;
    for block in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, bytes) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(bytes.try_into().expect("four-byte SHA-256 word"));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = h
                .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
                .wrapping_add((e & f) ^ ((!e) & g))
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let sum0 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
                .wrapping_add((a & b) ^ (a & c) ^ (b & c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(sum1);
            d = c;
            c = b;
            b = a;
            a = sum0.wrapping_add(sum1);
        }

        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut digest = [0_u8; 32];
    for (chunk, value) in digest.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    digest
}

/// Create a stable key for a file clipboard entry without touching the file
/// system.  Relative paths are retained as supplied; callers that need a
/// canonical identity can pass an absolute path first.
pub fn file_key(path: &Path) -> String {
    let path = path.to_string_lossy();
    let mut key = String::with_capacity(FILE_KEY_PREFIX.len() + path.len());
    key.push_str(FILE_KEY_PREFIX);
    key.push_str(&path);
    key
}

/// A snapshot of Alter metadata for one clipboard key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardMetadata {
    pub key: String,
    pub pinned: bool,
    pub hidden: bool,
}

/// Ensure the metadata table exists.
///
/// The function creates the parent directory when needed, but never reads or
/// writes Clipse's configuration.  It is safe to call repeatedly while the
/// clipboard watcher has the same SQLite file open.
pub fn ensure_schema(path: &Path) -> rusqlite::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    let mut connection = open(path)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS clipboard_metadata (
            \"key\" TEXT PRIMARY KEY NOT NULL CHECK(length(\"key\") > 0),
            pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
            hidden INTEGER NOT NULL DEFAULT 0 CHECK(hidden IN (0, 1)),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS clipboard_metadata_pinned_idx
            ON clipboard_metadata(pinned, updated_at DESC);
        CREATE INDEX IF NOT EXISTS clipboard_metadata_hidden_idx
            ON clipboard_metadata(hidden, updated_at DESC);
        CREATE TABLE IF NOT EXISTS clipboard_metadata_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        ",
    )?;
    migrate_legacy_content_keys(&mut connection)?;
    Ok(())
}

fn migrate_legacy_content_keys(connection: &mut Connection) -> rusqlite::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let already_applied = transaction
        .query_row(
            "SELECT 1 FROM clipboard_metadata_migrations WHERE version = ?1",
            params![CONTENT_KEY_MIGRATION],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    let legacy_entries = {
        let mut statement = transaction.prepare(
            "SELECT \"key\", pinned, hidden, updated_at
             FROM clipboard_metadata
             WHERE \"key\" LIKE 'content:%'
               AND \"key\" NOT LIKE 'content:sha256:%'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for (legacy_key, pinned, hidden, updated_at) in legacy_entries {
        // The migration can safely be checked on every schema setup. This
        // also catches a legacy writer that added a plaintext key after the
        // first run, while leaving already-hashed keys untouched.
        if is_hashed_content_key(&legacy_key) {
            continue;
        }
        let Some(content) = legacy_key.strip_prefix(LEGACY_CONTENT_KEY_PREFIX) else {
            continue;
        };
        let hashed_key = content_key(content);
        transaction.execute(
            "INSERT INTO clipboard_metadata(\"key\", pinned, hidden, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(\"key\") DO UPDATE SET
                 pinned = MAX(clipboard_metadata.pinned, excluded.pinned),
                 hidden = MAX(clipboard_metadata.hidden, excluded.hidden),
                 updated_at = MAX(clipboard_metadata.updated_at, excluded.updated_at)",
            params![hashed_key, pinned, hidden, updated_at],
        )?;
        transaction.execute(
            "DELETE FROM clipboard_metadata WHERE \"key\" = ?1",
            params![legacy_key],
        )?;
    }
    if !already_applied {
        transaction.execute(
            "INSERT INTO clipboard_metadata_migrations(version) VALUES (?1)",
            params![CONTENT_KEY_MIGRATION],
        )?;
    }
    transaction.commit()
}

fn is_hashed_content_key(key: &str) -> bool {
    let Some(digest) = key.strip_prefix(CONTENT_HASH_KEY_PREFIX) else {
        return false;
    };
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Set or clear the pinned bit for `key`, preserving its hidden bit.
pub fn set_pinned(path: &Path, key: &str, pinned: bool) -> rusqlite::Result<()> {
    set_flag(path, key, "pinned", pinned)
}

/// Return whether `key` is pinned.  Missing metadata is equivalent to false.
pub fn is_pinned(path: &Path, key: &str) -> rusqlite::Result<bool> {
    get_flag(path, key, "pinned")
}

/// Set or clear the hidden bit for `key`, preserving its pinned bit.
pub fn set_hidden(path: &Path, key: &str, hidden: bool) -> rusqlite::Result<()> {
    set_flag(path, key, "hidden", hidden)
}

/// Return whether `key` is hidden.  Missing metadata is equivalent to false.
pub fn is_hidden(path: &Path, key: &str) -> rusqlite::Result<bool> {
    get_flag(path, key, "hidden")
}

/// Read both flags at once for callers rendering a clipboard row.
pub fn metadata(path: &Path, key: &str) -> rusqlite::Result<ClipboardMetadata> {
    validate_key(key)?;
    ensure_schema(path)?;
    let connection = open(path)?;
    let values = connection
        .query_row(
            "SELECT \"key\", pinned, hidden
             FROM clipboard_metadata WHERE \"key\" = ?1",
            params![key],
            |row| {
                Ok(ClipboardMetadata {
                    key: row.get(0)?,
                    pinned: row.get::<_, i64>(1)? != 0,
                    hidden: row.get::<_, i64>(2)? != 0,
                })
            },
        )
        .optional()?;
    Ok(values.unwrap_or_else(|| ClipboardMetadata {
        key: key.to_owned(),
        pinned: false,
        hidden: false,
    }))
}

/// Return all keys with one or both metadata flags set.
///
/// This is intentionally a small convenience API for a future clipboard
/// search layer; it does not expose or modify Clipse entries.
pub fn flagged_keys(path: &Path) -> rusqlite::Result<Vec<ClipboardMetadata>> {
    ensure_schema(path)?;
    let connection = open(path)?;
    let mut statement = connection.prepare(
        "SELECT \"key\", pinned, hidden FROM clipboard_metadata
         WHERE pinned != 0 OR hidden != 0
         ORDER BY updated_at DESC, \"key\" ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ClipboardMetadata {
            key: row.get(0)?,
            pinned: row.get::<_, i64>(1)? != 0,
            hidden: row.get::<_, i64>(2)? != 0,
        })
    })?;
    rows.collect()
}

fn set_flag(path: &Path, key: &str, flag: &str, value: bool) -> rusqlite::Result<()> {
    validate_key(key)?;
    ensure_schema(path)?;
    let connection = open(path)?;
    // `flag` is selected only from these two literals by the public helpers;
    // keeping the SQL branches explicit avoids interpolating caller input.
    let value = i64::from(value);
    match flag {
        "pinned" => {
            connection.execute(
                "INSERT INTO clipboard_metadata(\"key\", pinned, hidden, updated_at)
                 VALUES (?1, ?2, 0, unixepoch())
                 ON CONFLICT(\"key\") DO UPDATE SET
                     pinned = excluded.pinned,
                     updated_at = excluded.updated_at",
                params![key, value],
            )?;
        }
        "hidden" => {
            connection.execute(
                "INSERT INTO clipboard_metadata(\"key\", pinned, hidden, updated_at)
                 VALUES (?1, 0, ?2, unixepoch())
                 ON CONFLICT(\"key\") DO UPDATE SET
                     hidden = excluded.hidden,
                     updated_at = excluded.updated_at",
                params![key, value],
            )?;
        }
        _ => unreachable!("clipboard metadata flag is private and validated by callers"),
    }
    Ok(())
}

fn get_flag(path: &Path, key: &str, flag: &str) -> rusqlite::Result<bool> {
    validate_key(key)?;
    ensure_schema(path)?;
    let connection = open(path)?;
    let value: Option<i64> = match flag {
        "pinned" => connection
            .query_row(
                "SELECT pinned FROM clipboard_metadata WHERE \"key\" = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?,
        "hidden" => connection
            .query_row(
                "SELECT hidden FROM clipboard_metadata WHERE \"key\" = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?,
        _ => unreachable!("clipboard metadata flag is private and validated by callers"),
    };
    Ok(value.is_some_and(|value| value != 0))
}

fn validate_key(key: &str) -> rusqlite::Result<()> {
    if key.trim().is_empty() {
        return Err(invalid_key_error("key cannot be empty"));
    }
    if key.contains('\0') {
        return Err(invalid_key_error("key cannot contain NUL"));
    }
    Ok(())
}

fn invalid_key_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::new(
        io::ErrorKind::InvalidInput,
        message.to_owned(),
    )))
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_millis(500))?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temporary_database(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        std::env::temp_dir().join(format!(
            "alter-clipboard-meta-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_database(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn schema_is_idempotent_and_missing_keys_are_clear() {
        let path = temporary_database("schema");
        ensure_schema(&path).unwrap();
        ensure_schema(&path).unwrap();
        assert!(!is_pinned(&path, "missing").unwrap());
        assert!(!is_hidden(&path, "missing").unwrap());
        assert_eq!(
            metadata(&path, "missing").unwrap(),
            ClipboardMetadata {
                key: "missing".to_owned(),
                pinned: false,
                hidden: false,
            }
        );
        remove_database(&path);
    }

    #[test]
    fn content_keys_use_known_sha256_and_do_not_retain_plaintext() {
        assert_eq!(
            content_key(""),
            "content:sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            content_key("abc"),
            "content:sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let key = content_key("private clipboard text");
        assert_eq!(key.len(), CONTENT_HASH_KEY_PREFIX.len() + 64);
        assert!(!key.contains("private clipboard text"));
        assert_eq!(key, content_key("private clipboard text"));
        assert_ne!(key, content_key("different clipboard text"));
    }

    #[test]
    fn migrates_legacy_plaintext_keys_without_losing_flags() {
        let path = temporary_database("legacy-migration");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE clipboard_metadata (
                        \"key\" TEXT PRIMARY KEY NOT NULL,
                        pinned INTEGER NOT NULL DEFAULT 0,
                        hidden INTEGER NOT NULL DEFAULT 0,
                        updated_at INTEGER NOT NULL DEFAULT 0
                    );",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO clipboard_metadata(\"key\", pinned, hidden, updated_at)
                     VALUES (?1, 1, 1, 42)",
                    params!["content:private legacy value"],
                )
                .unwrap();
        }

        ensure_schema(&path).unwrap();
        let hashed = content_key("private legacy value");
        assert!(is_pinned(&path, &hashed).unwrap());
        assert!(is_hidden(&path, &hashed).unwrap());
        let connection = Connection::open(&path).unwrap();
        let plaintext_rows: i64 = connection
            .query_row(
                "SELECT count(*) FROM clipboard_metadata WHERE \"key\" = ?1",
                params!["content:private legacy value"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(plaintext_rows, 0);
        drop(connection);

        // Re-running schema setup uses the migration marker and must not hash
        // the already-versioned key a second time.
        ensure_schema(&path).unwrap();
        assert!(is_pinned(&path, &hashed).unwrap());

        // A stale/legacy process may write one final plaintext key after the
        // marker exists. The next schema check still scrubs that row.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO clipboard_metadata(\"key\", pinned, hidden, updated_at)
                 VALUES (?1, 0, 1, 43)",
                params!["content:late legacy value"],
            )
            .unwrap();
        drop(connection);
        ensure_schema(&path).unwrap();
        assert!(is_hidden(&path, &content_key("late legacy value")).unwrap());
        remove_database(&path);
    }

    #[test]
    fn flags_are_independent_and_toggle_correctly() {
        let path = temporary_database("flags");
        let key = content_key("hello");
        set_pinned(&path, &key, true).unwrap();
        assert!(is_pinned(&path, &key).unwrap());
        assert!(!is_hidden(&path, &key).unwrap());

        set_hidden(&path, &key, true).unwrap();
        assert!(is_pinned(&path, &key).unwrap());
        assert!(is_hidden(&path, &key).unwrap());

        set_pinned(&path, &key, false).unwrap();
        assert!(!is_pinned(&path, &key).unwrap());
        assert!(is_hidden(&path, &key).unwrap());

        set_hidden(&path, &key, false).unwrap();
        assert_eq!(
            flagged_keys(&path).unwrap(),
            Vec::<ClipboardMetadata>::new()
        );
        remove_database(&path);
    }

    #[test]
    fn content_and_file_keys_do_not_collide() {
        let path = temporary_database("keys");
        let content = content_key("/tmp/report.txt");
        let file = file_key(Path::new("/tmp/report.txt"));
        assert_ne!(content, file);
        set_pinned(&path, &content, true).unwrap();
        set_hidden(&path, &file, true).unwrap();
        assert!(is_pinned(&path, &content).unwrap());
        assert!(!is_hidden(&path, &content).unwrap());
        assert!(!is_pinned(&path, &file).unwrap());
        assert!(is_hidden(&path, &file).unwrap());
        let flagged = flagged_keys(&path).unwrap();
        assert_eq!(flagged.len(), 2);
        remove_database(&path);
    }

    #[test]
    fn rejects_empty_and_nul_keys_without_creating_rows() {
        let path = temporary_database("invalid");
        assert!(set_pinned(&path, "", true).is_err());
        assert!(set_hidden(&path, "\0", true).is_err());
        ensure_schema(&path).unwrap();
        let connection = open(&path).unwrap();
        let count: i64 = connection
            .query_row("SELECT count(*) FROM clipboard_metadata", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        remove_database(&path);
    }

    #[test]
    fn metadata_does_not_touch_clipboard_history_table() {
        let path = temporary_database("isolation");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute(
                    "CREATE TABLE clipboard (id INTEGER PRIMARY KEY, content TEXT NOT NULL)",
                    [],
                )
                .unwrap();
            connection
                .execute("INSERT INTO clipboard(content) VALUES ('keep me')", [])
                .unwrap();
        }
        set_hidden(&path, &content_key("keep me"), true).unwrap();
        let connection = Connection::open(&path).unwrap();
        let value: String = connection
            .query_row("SELECT content FROM clipboard WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "keep me");
        remove_database(&path);
    }
}
