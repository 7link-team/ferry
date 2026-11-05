use sea_orm_migration::prelude::*;

pub struct Migration;
pub struct LimitsMigration;
impl MigrationName for LimitsMigration {
    fn name(&self) -> &str {
        "platform_monitor_limits_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for LimitsMigration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        if !m.has_column("platform_state", "monitor_config").await? {
            m.alter_table(
                Table::alter()
                    .table(Alias::new("platform_state"))
                    .add_column(ColumnDef::new(Alias::new("monitor_config")).text().null())
                    .to_owned(),
            )
            .await?;
        }
        for (name, columns) in [
            ("idx_observations_instance_id", vec!["instance_id", "id"]),
            ("idx_observations_at_id", vec!["at", "id"]),
        ] {
            if !m.has_index("platform_observations", name).await? {
                let mut index = Index::create();
                index.name(name).table(Alias::new("platform_observations"));
                for column in columns {
                    index.col(Alias::new(column));
                }
                m.create_index(index.to_owned()).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Monitoring configuration must be retained".into(),
        ))
    }
}
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "platform_observations_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let mut table = super::create_table("platform_observations", m.get_database_backend());
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for name in ["at", "event_day", "user_id", "elapsed_ms", "response_bytes"] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        for (name, length) in [
            ("instance_id", 64),
            ("kind", 12),
            ("method", 12),
            ("route", 512),
            ("file_path", 1024),
        ] {
            table.col(
                ColumnDef::new(Alias::new(name))
                    .string_len(length)
                    .not_null(),
            );
        }
        table.col(ColumnDef::new(Alias::new("status")).integer().not_null());
        m.create_table(table).await?;
        for (name, columns) in [
            ("idx_observations_day_user", vec!["event_day", "user_id"]),
            ("idx_observations_kind_at", vec!["kind", "at"]),
        ] {
            if !m.has_index("platform_observations", name).await? {
                let mut index = Index::create();
                index.name(name).table(Alias::new("platform_observations"));
                for col in columns {
                    index.col(Alias::new(col));
                }
                m.create_index(index.to_owned()).await?;
            }
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Monitoring history must be retained".into()))
    }
}
