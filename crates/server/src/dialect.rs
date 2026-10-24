//! The three supported databases, from one set of SQL.
//!
//! Application code writes SQLite-flavoured SQL with `?` placeholders. This
//! module is the only place that knows the differences, and it stays small on
//! purpose: the queries are plain CRUD, so the whole dialect surface is
//! placeholders plus a handful of DDL types. Resist growing it with
//! per-query special cases — rewrite the query to be portable instead.
//!
//! Values are never interpolated here. Placeholders are renumbered; the bound
//! parameters still travel separately.

use sea_orm::DbBackend;

/// Columns whose contents outgrow a MySQL `VARCHAR` key: JSON blobs.
/// MySQL forbids a DEFAULT on a TEXT/LONGTEXT column, so a column only
/// belongs here when it genuinely needs more than VARCHAR(255) *and* has no
/// default. `metrics` was promoted here once and broke the migration.
const LONG_COLUMNS: &[&str] = &["permissions", "details", "secret", "definition"];

/// Rewrites a parameterized query for the target backend.
pub fn query(backend: DbBackend, sql: &str) -> String {
    match backend {
        // Both take `?` and need nothing else from Ferry's SQL.
        DbBackend::Sqlite | DbBackend::MySql => sql.to_owned(),
        DbBackend::Postgres => numbered_placeholders(sql),
        // DbBackend is non-exhaustive upstream. A backend this module has not
        // been taught is a configuration error, not something to guess at.
        other => unreachable!("unsupported database backend: {other:?}"),
    }
}

/// Counts the real placeholders, ignoring any `?` inside a string literal.
pub fn placeholder_count(sql: &str) -> usize {
    let mut count = 0;
    let mut in_literal = false;
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\'' => {
                if in_literal && chars.peek() == Some(&'\'') {
                    chars.next();
                    continue;
                }
                in_literal = !in_literal;
            }
            '?' if !in_literal => count += 1,
            _ => {}
        }
    }
    count
}

/// `?, ?, ?` -> `$1, $2, $3`, skipping anything inside a string literal.
fn numbered_placeholders(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len() + 8);
    let mut index = 0;
    let mut in_literal = false;
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\'' => {
                // '' inside a literal is an escaped quote, not the end of it.
                if in_literal && chars.peek() == Some(&'\'') {
                    result.push('\'');
                    result.push(chars.next().unwrap());
                    continue;
                }
                in_literal = !in_literal;
                result.push('\'');
            }
            '?' if !in_literal => {
                index += 1;
                result.push('$');
                result.push_str(&index.to_string());
            }
            _ => result.push(character),
        }
    }
    result
}

/// Rewrites one DDL statement for the target backend. Returns `None` when the
/// statement should be skipped entirely on that backend.
pub fn schema(backend: DbBackend, sql: &str) -> Option<String> {
    match backend {
        // The migrations are written in its dialect.
        DbBackend::Sqlite => Some(sql.to_owned()),
        // TEXT and `IF NOT EXISTS` on an index are both fine here.
        DbBackend::Postgres => Some(sql.to_owned()),
        DbBackend::MySql => Some(mysql_schema(sql)),
        other => unreachable!("unsupported database backend: {other:?}"),
    }
}

pub(crate) fn mysql_schema(sql: &str) -> String {
    // MySQL cannot index a TEXT column without a prefix length, and every TEXT
    // column here is either a key, a short label, or a JSON blob.
    let mut out = String::with_capacity(sql.len() + 64);
    let mut words = sql.split_inclusive(char::is_whitespace).peekable();
    let mut previous = String::new();
    while let Some(chunk) = words.next() {
        let bare = chunk.trim();
        if bare == "TEXT" {
            let column = previous.trim_start_matches(['(', ',']).trim();
            let replacement = if LONG_COLUMNS.contains(&column) {
                "LONGTEXT"
            } else if column == "id" || column.ends_with("_id") {
                // UUIDs are 36 characters; keep index keys small.
                "VARCHAR(64)"
            } else {
                "VARCHAR(255)"
            };
            out.push_str(&chunk.replace("TEXT", replacement));
        } else {
            out.push_str(chunk);
        }
        previous = bare.to_owned();
    }

    // `CREATE INDEX IF NOT EXISTS` is not valid MySQL; the caller checks for
    // the index instead, so the clause is simply dropped here.
    let out = out
        .replace("CREATE INDEX IF NOT EXISTS ", "CREATE INDEX ")
        .replace("CREATE UNIQUE INDEX IF NOT EXISTS ", "CREATE UNIQUE INDEX ");

    if out.trim_start().starts_with("CREATE TABLE") {
        // utf8mb4_bin so comparisons stay case- and accent-sensitive, matching
        // SQLite's BINARY default. Without it 'Admin' and 'admin' collide in
        // the unique indexes and a slug check silently changes meaning.
        format!("{out} DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin")
    } else {
        out
    }
}

/// `CREATE [UNIQUE] INDEX name ON table (...)` -> `(table, name)`, so the
/// caller can ask whether it already exists before issuing it on MySQL.
pub fn index_target(sql: &str) -> Option<(String, String)> {
    let words: Vec<&str> = sql.split_whitespace().collect();
    let (name, table) = match words.as_slice() {
        [
            "CREATE",
            "INDEX",
            "IF",
            "NOT",
            "EXISTS",
            name,
            "ON",
            table,
            ..,
        ] => (name, table),
        [
            "CREATE",
            "UNIQUE",
            "INDEX",
            "IF",
            "NOT",
            "EXISTS",
            name,
            "ON",
            table,
            ..,
        ] => (name, table),
        ["CREATE", "INDEX", name, "ON", table, ..] => (name, table),
        ["CREATE", "UNIQUE", "INDEX", name, "ON", table, ..] => (name, table),
        _ => return None,
    };
    Some((
        table.split('(').next()?.to_string(),
        name.split('(').next()?.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_counted_without_counting_literals() {
        assert_eq!(
            placeholder_count("SELECT ? WHERE a = ? AND b = 'why? really'"),
            2
        );
        assert_eq!(
            placeholder_count("INSERT INTO t VALUES (?,?,'unknown',?,0,?,?,1)"),
            5
        );
        assert_eq!(placeholder_count("SELECT 1"), 0);
        assert_eq!(
            placeholder_count("SELECT ? WHERE a = 'it''s ? here' AND b = ?"),
            2
        );
    }

    #[test]
    fn sqlite_and_mysql_keep_question_marks() {
        let sql = "SELECT id FROM hosts WHERE workspace_id = ? AND state IN (?,?)";
        assert_eq!(query(DbBackend::Sqlite, sql), sql);
        assert_eq!(query(DbBackend::MySql, sql), sql);
    }

    #[test]
    fn postgres_numbers_placeholders_in_order() {
        assert_eq!(
            query(
                DbBackend::Postgres,
                "SELECT id FROM hosts WHERE workspace_id = ? AND state IN (?,?) LIMIT ? OFFSET ?"
            ),
            "SELECT id FROM hosts WHERE workspace_id = $1 AND state IN ($2,$3) LIMIT $4 OFFSET $5"
        );
    }

    #[test]
    fn a_question_mark_inside_a_literal_is_not_a_placeholder() {
        assert_eq!(
            query(DbBackend::Postgres, "SELECT ? WHERE note = 'why? really'"),
            "SELECT $1 WHERE note = 'why? really'"
        );
        // An escaped quote must not be read as the end of the literal.
        assert_eq!(
            query(
                DbBackend::Postgres,
                "SELECT ? WHERE a = 'it''s ? here' AND b = ?"
            ),
            "SELECT $1 WHERE a = 'it''s ? here' AND b = $2"
        );
    }

    #[test]
    fn mysql_sizes_text_columns_by_their_role() {
        let out = mysql_schema(
            "CREATE TABLE IF NOT EXISTS activity (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                actor_name TEXT NOT NULL,
                details TEXT NOT NULL)",
        );
        assert!(out.contains("id VARCHAR(64) PRIMARY KEY"), "{out}");
        assert!(out.contains("workspace_id VARCHAR(64)"), "{out}");
        assert!(out.contains("actor_name VARCHAR(255)"), "{out}");
        assert!(out.contains("details LONGTEXT"), "{out}");
        assert!(out.ends_with("COLLATE utf8mb4_bin"), "{out}");
        // No bare TEXT may survive, or the index on it fails to create.
        assert!(!out.contains(" TEXT "), "{out}");
    }

    #[test]
    fn a_sealed_private_key_does_not_land_in_a_255_byte_column() {
        // An SSH key seals to several kilobytes of hex; VARCHAR(255) would
        // truncate it and the credential would fail to decrypt later, far
        // from the cause.
        let out = mysql_schema("CREATE TABLE credentials (id TEXT, secret TEXT NOT NULL)");
        assert!(out.contains("secret LONGTEXT"), "{out}");
        assert!(!out.contains("secret VARCHAR"), "{out}");
    }

    #[test]
    fn mysql_drops_if_not_exists_from_an_index() {
        let out = mysql_schema("CREATE UNIQUE INDEX IF NOT EXISTS users_email ON users (email)");
        assert_eq!(out, "CREATE UNIQUE INDEX users_email ON users (email)");
        assert!(!out.contains("CHARACTER SET"), "only tables get a charset");
    }

    #[test]
    fn index_targets_are_recognised_in_every_spelling() {
        for (sql, want) in [
            (
                "CREATE INDEX IF NOT EXISTS hosts_workspace ON hosts (workspace_id, name)",
                ("hosts", "hosts_workspace"),
            ),
            (
                "CREATE UNIQUE INDEX IF NOT EXISTS tags_unique ON tags (workspace_id, name)",
                ("tags", "tags_unique"),
            ),
            (
                "CREATE INDEX hosts_group ON hosts(workspace_id, group_id)",
                ("hosts", "hosts_group"),
            ),
        ] {
            let (table, name) = index_target(sql).expect(sql);
            assert_eq!((table.as_str(), name.as_str()), want);
        }
        assert!(index_target("CREATE TABLE hosts (id TEXT)").is_none());
    }
}
