//! Schema migrations as raw SQL, applied in order and never edited once
//! shipped. The full target schema is in docs/ferry-运维平台产品与技术方案.md;
//! `Initial` carries only the identity and tenancy bedrock every module needs.

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Initial), Box::new(HostResources)]
    }
}

const INITIAL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS users (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL,
        name TEXT NOT NULL,
        password_hash TEXT NOT NULL,
        active INTEGER NOT NULL DEFAULT 1,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE UNIQUE INDEX IF NOT EXISTS users_email ON users (email)",
    "CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        workspace_id TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        seen_at BIGINT NOT NULL,
        expires_at BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id)",
    "CREATE TABLE IF NOT EXISTS workspaces (
        id TEXT PRIMARY KEY,
        slug TEXT NOT NULL,
        name TEXT NOT NULL,
        owner_id TEXT NOT NULL,
        join_code TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE UNIQUE INDEX IF NOT EXISTS workspaces_slug ON workspaces (slug)",
    "CREATE TABLE IF NOT EXISTS workspace_roles (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        permissions TEXT NOT NULL,
        builtin INTEGER NOT NULL DEFAULT 0,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS workspace_roles_workspace ON workspace_roles (workspace_id)",
    "CREATE TABLE IF NOT EXISTS workspace_members (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        role_id TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'active',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL)",
    "CREATE UNIQUE INDEX IF NOT EXISTS workspace_members_unique ON workspace_members (workspace_id, user_id)",
    "CREATE TABLE IF NOT EXISTS activity (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        actor_id TEXT NOT NULL,
        actor_name TEXT NOT NULL,
        action TEXT NOT NULL,
        resource TEXT NOT NULL,
        resource_id TEXT NOT NULL,
        details TEXT NOT NULL,
        created_at BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS activity_workspace ON activity (workspace_id, created_at)",
];

struct Initial;
impl MigrationName for Initial {
    fn name(&self) -> &str {
        "initial_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Initial {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in INITIAL {
            m.get_connection().execute_unprepared(sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}

const HOST_RESOURCES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS host_groups (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        parent_id TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version INTEGER NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS host_groups_workspace ON host_groups (workspace_id, name)",
    "CREATE TABLE IF NOT EXISTS hosts (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        address TEXT NOT NULL DEFAULT '',
        port BIGINT NOT NULL DEFAULT 22,
        username TEXT NOT NULL DEFAULT '',
        connection TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'unknown',
        group_id TEXT NOT NULL DEFAULT '',
        description TEXT NOT NULL DEFAULT '',
        host_key TEXT NOT NULL DEFAULT '',
        runner_key_hash TEXT NOT NULL DEFAULT '',
        seen_at BIGINT NOT NULL DEFAULT 0,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version INTEGER NOT NULL DEFAULT 1)",
    "CREATE INDEX IF NOT EXISTS hosts_workspace ON hosts (workspace_id, name)",
    "CREATE INDEX IF NOT EXISTS hosts_group ON hosts (workspace_id, group_id)",
    "CREATE TABLE IF NOT EXISTS tags (
        id TEXT PRIMARY KEY,
        workspace_id TEXT NOT NULL,
        name TEXT NOT NULL,
        color TEXT NOT NULL DEFAULT '',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        version INTEGER NOT NULL DEFAULT 1)",
    "CREATE UNIQUE INDEX IF NOT EXISTS tags_unique ON tags (workspace_id, name)",
    "CREATE TABLE IF NOT EXISTS host_tags (
        host_id TEXT NOT NULL,
        tag_id TEXT NOT NULL,
        PRIMARY KEY (host_id, tag_id))",
];

struct HostResources;
impl MigrationName for HostResources {
    fn name(&self) -> &str {
        "host_resources_v1"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for HostResources {
    async fn up(&self, m: &SchemaManager) -> std::result::Result<(), DbErr> {
        for sql in HOST_RESOURCES {
            m.get_connection().execute_unprepared(sql).await?;
        }
        Ok(())
    }
    async fn down(&self, _: &SchemaManager) -> std::result::Result<(), DbErr> {
        Err(DbErr::Custom("不自动删除业务数据".into()))
    }
}
