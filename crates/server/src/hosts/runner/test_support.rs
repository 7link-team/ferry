//! Isolated on-disk Runner test databases.
use sea_orm::{ConnectOptions, Database, DatabaseConnection};

/// A private database per test, on disk, deleted when the test ends.
///
/// Not `sqlite::memory:`: sqlx gives in-memory databases a shared cache, so
/// every test in the process ends up on one connection — and
/// `rows_affected()` reads SQLite's connection-global `changes()`. Two
/// tests interleaving on that connection make the single-use token check
/// report a row it did not touch, which is a false pass on the one
/// assertion here that is load-bearing for security.
pub(super) struct Scratch {
    pub(super) db: DatabaseConnection,
    path: std::path::PathBuf,
}

impl std::ops::Deref for Scratch {
    type Target = DatabaseConnection;
    fn deref(&self) -> &DatabaseConnection {
        &self.db
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

pub(super) async fn scratch_db() -> Scratch {
    let path =
        std::env::temp_dir().join(format!("ferry-runner-test-{}.sqlite", uuid::Uuid::new_v4()));
    let mut options = ConnectOptions::new("sqlite://ferry");
    let filename = path.clone();
    options.sqlx_logging(false).map_sqlx_sqlite_opts(move |o| {
        o.filename(&filename)
            .create_if_missing(true)
            .foreign_keys(true)
    });
    let db = Database::connect(options).await.expect("scratch sqlite");
    use sea_orm_migration::MigratorTrait;
    crate::migration::Migrator::up(&db, None)
        .await
        .expect("migrations apply");
    Scratch { db, path }
}
