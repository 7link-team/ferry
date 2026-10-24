//! DDL compatibility retained until migrations move to schema builders (T3d).

use sea_orm::DbBackend;

/// Columns whose contents outgrow a MySQL `VARCHAR` key: JSON blobs.
/// MySQL forbids a DEFAULT on a TEXT/LONGTEXT column, so a column only
/// belongs here when it genuinely needs more than VARCHAR(255) *and* has no
/// default. `metrics` was promoted here once and broke the migration.
const LONG_COLUMNS: &[&str] = &["permissions", "details", "secret", "definition"];

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
