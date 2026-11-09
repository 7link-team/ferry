use super::*;

/// Preview holes: a public token mapped to a loopback port on a runner host.
/// Columns and indexes follow docs/preview-holes-design.md section 10.
pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "holes_v1"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let backend = m.get_database_backend();
        let mut table = create_table("holes", backend);
        table.col(
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .primary_key(),
        );
        for name in [
            "workspace_id",
            "host_id",
            "target_port",
            "expires_at",
            "created_at",
            "updated_at",
        ] {
            table.col(ColumnDef::new(Alias::new(name)).big_integer().not_null());
        }
        for name in [
            "revoked_at",
            "revoked_by",
            "probed_at",
            "probed_by",
            "created_by",
            "updated_by",
        ] {
            table.col(
                ColumnDef::new(Alias::new(name))
                    .big_integer()
                    .not_null()
                    .default(0_i64),
            );
        }
        table.col(
            ColumnDef::new(Alias::new("version"))
                .big_integer()
                .not_null()
                .default(1_i64),
        );
        table.col(
            ColumnDef::new(Alias::new("strip_prefix"))
                .boolean()
                .not_null()
                .default(true),
        );
        table.col(
            ColumnDef::new(Alias::new("name"))
                .string_len(128)
                .not_null(),
        );
        table.col(
            ColumnDef::new(Alias::new("token_hash"))
                .string_len(64)
                .not_null(),
        );
        table.col(long_text("token_sealed", backend).not_null().to_owned());
        table.col(
            ColumnDef::new(Alias::new("probe_state"))
                .string_len(16)
                .not_null()
                .default("unknown"),
        );
        table.col(
            ColumnDef::new(Alias::new("probe_error"))
                .string_len(512)
                .not_null()
                .default(""),
        );
        m.create_table(table.to_owned()).await?;
        m.create_index(
            Index::create()
                .name("holes_token_hash")
                .table(Alias::new("holes"))
                .col(Alias::new("token_hash"))
                .unique()
                .to_owned(),
        )
        .await?;
        for (name, columns) in [
            ("holes_workspace", &["workspace_id", "id"][..]),
            ("holes_host", &["host_id"][..]),
            ("holes_expires", &["expires_at"][..]),
        ] {
            let mut index = Index::create();
            index.name(name).table(Alias::new("holes"));
            for column in columns {
                index.col(Alias::new(*column));
            }
            m.create_index(index.to_owned()).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
