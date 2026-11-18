use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "session_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(ColumnDef::new(Alias::new("session_config")).text().null())
                    .to_owned(),
            )
            .await?;
        }
        m.create_table(
            Table::create()
                .if_not_exists()
                .table(Alias::new("session_refresh_tokens"))
                .col(
                    ColumnDef::new(Alias::new("session_id"))
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Alias::new("token_hash"))
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("previous_hash"))
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(Alias::new("access_ttl_ms"))
                        .big_integer()
                        .not_null()
                        .default(1_800_000),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Alias::new("created_by"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Alias::new("rotated_at"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Alias::new("rotated_by"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Alias::new("expires_by"))
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .check(Expr::col(Alias::new("access_ttl_ms")).gt(0))
                .index(
                    Index::create()
                        .name("session_refresh_token_hash")
                        .unique()
                        .col(Alias::new("token_hash")),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(
                            Alias::new("session_refresh_tokens"),
                            Alias::new("session_id"),
                        )
                        .to(Alias::new("sessions"), Alias::new("id"))
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        if !m
            .has_index("session_refresh_tokens", "session_refresh_previous_hash")
            .await?
        {
            m.create_index(
                Index::create()
                    .name("session_refresh_previous_hash")
                    .table(Alias::new("session_refresh_tokens"))
                    .col(Alias::new("previous_hash"))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let conn = m.get_connection();
        let configured = if m.has_column("platform_state", "session_config").await? {
            conn.query_one_raw(sea_orm::Statement::from_string(
                m.get_database_backend(),
                "SELECT id FROM platform_state WHERE session_config IS NOT NULL LIMIT 1".to_owned(),
            ))
            .await?
            .is_some()
        } else {
            false
        };
        let sessions = if m.has_table("session_refresh_tokens").await? {
            conn.query_one_raw(sea_orm::Statement::from_string(
                m.get_database_backend(),
                "SELECT session_id FROM session_refresh_tokens LIMIT 1".to_owned(),
            ))
            .await?
            .is_some()
        } else {
            false
        };
        if configured || sessions {
            return Err(DbErr::Custom(
                "Retain session policy and revoke new-style sessions before rollback".into(),
            ));
        }
        m.drop_table(
            Table::drop()
                .if_exists()
                .table(Alias::new("session_refresh_tokens"))
                .to_owned(),
        )
        .await?;
        if m.has_column("platform_state", "session_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .drop_column(Alias::new("session_config"))
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};
    #[tokio::test]
    async fn migration_preserves_old_rows_cascades_revocation_and_blocks_lossy_rollback() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys=ON")
            .await
            .unwrap();
        db.execute_unprepared(
            "CREATE TABLE platform_state (id BIGINT PRIMARY KEY, site_config TEXT)",
        )
        .await
        .unwrap();
        db.execute_unprepared("INSERT INTO platform_state VALUES (1,'keep')")
            .await
            .unwrap();
        db.execute_unprepared("CREATE TABLE sessions (id BIGINT PRIMARY KEY)")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO sessions VALUES (1)")
            .await
            .unwrap();
        let m = SchemaManager::new(&db);
        Migration.up(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        db.execute_unprepared(
            "INSERT INTO session_refresh_tokens(session_id,token_hash) VALUES(1,'hash')",
        )
        .await
        .unwrap();
        assert!(Migration.down(&m).await.is_err());
        db.execute_unprepared("DELETE FROM sessions WHERE id=1")
            .await
            .unwrap();
        Migration.down(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        db.execute_unprepared("UPDATE platform_state SET session_config='{}'")
            .await
            .unwrap();
        assert!(Migration.down(&m).await.is_err());
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT site_config FROM platform_state".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "site_config").unwrap(), "keep");
    }
}
