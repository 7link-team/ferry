//! Catalog storage shared by workspace and platform sources: replace one source's entries atomically.
use super::index::PackageVersion;
use crate::{
    db,
    entities::marketplace_entries as entries,
    error::{ApiError, Result},
    state::AppState,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;

fn timeout() -> ApiError {
    ApiError::localized(axum::http::StatusCode::BAD_REQUEST, crate::i18n::Message::new("marketplace.syncTimeout"))
}

pub(crate) async fn replace_entries(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    workspace_id: i64,
    source_id: i64,
    actor: i64,
    packages: Vec<PackageVersion>,
    deadline: std::time::Instant,
) -> Result<usize> {
    entries::Entity::delete_many()
        .filter(entries::Column::WorkspaceId.eq(workspace_id))
        .filter(entries::Column::SourceId.eq(source_id))
        .exec(tx)
        .await?;
    let now = db::now();
    let total = packages.len();
    for p in packages {
        if std::time::Instant::now() >= deadline {
            return Err(timeout());
        }
        let mut meta = p.meta;
        if let Some(reason) = &p.incompatible_reason {
            meta["incompatible"] = json!(reason);
        }
        let files: Vec<_> = p.files.iter().map(|f| json!({"name":f.name,"content":f.content,"parameters":f.parameters})).collect();
        entries::Entity::insert(entries::ActiveModel {
            id: Set(s.next_id()?.into()),
            workspace_id: Set(workspace_id),
            source_id: Set(source_id),
            entry_key: Set(p.key),
            name: Set(p.name),
            description: Set(p.description),
            content: Set(serde_json::to_string_pretty(&p.content).unwrap()),
            parameters: Set(serde_json::to_string(&p.parameters).unwrap()),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            package_version: Set(p.version),
            kind: Set(p.kind),
            meta: Set(Some(meta.to_string())),
            files: Set(Some(serde_json::to_string(&files).unwrap())),
            readme: Set(Some(p.readme)),
            compatible: Set(i64::from(p.compatible)),
            digest: Set(p.digest),
            created_by: Set(actor),
            updated_by: Set(actor),
        })
        .exec(tx)
        .await?;
    }
    if std::time::Instant::now() >= deadline {
        return Err(timeout());
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::marketplace_entries as entries;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait};
    use serde_json::json;

    fn bundle(versions: &[&str]) -> String {
        let app = json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},"services":[{"name":"web","image":"busybox:1.36"}]});
        let versions: Vec<_> = versions.iter().map(|v| json!({"version":v,"blueprint":app,"files":[{"name":"web.conf","content":"listen 80;"}]})).collect();
        json!({"schemaVersion":2,"packages":[{"key":"web","kind":"app","meta":{"name":{"en-US":"Website"},"category":"web","topology":"single","drivers":["docker"]},"versions":versions}]}).to_string()
    }
    async fn store(s: &AppState, text: &str) -> usize {
        let rows = super::super::index::parse_bundle(text, &semver::Version::new(0, 3, 0), "en-US").unwrap();
        let tx = s.db.begin().await.unwrap();
        let n = replace_entries(s, &tx, 7, 70, 1, rows, std::time::Instant::now() + std::time::Duration::from_secs(20)).await.unwrap();
        tx.commit().await.unwrap();
        n
    }

    #[tokio::test]
    async fn replace_entries_stores_versions_files_digest_and_replaces_previous() {
        let (_dir, s) = crate::agents::tests::setup().await;
        assert_eq!(store(&s, &bundle(&["1.0.0", "1.1.0"])).await, 2);
        let rows = entries::Entity::find().filter(entries::Column::SourceId.eq(70)).order_by_asc(entries::Column::PackageVersion).all(&s.db).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].package_version, "1.0.0");
        assert_eq!(rows[0].workspace_id, 7);
        assert_eq!(rows[0].entry_key, "web");
        assert_eq!(rows[0].created_by, 1);
        assert_eq!(rows[0].digest.len(), 64);
        let files: serde_json::Value = serde_json::from_str(rows[0].files.as_deref().unwrap()).unwrap();
        assert_eq!(files[0]["name"], "web.conf");
        let meta: serde_json::Value = serde_json::from_str(rows[0].meta.as_deref().unwrap()).unwrap();
        assert_eq!(meta["name"]["en-US"], "Website");
        assert_eq!(store(&s, &bundle(&["2.0.0"])).await, 1);
        let rows = entries::Entity::find().filter(entries::Column::SourceId.eq(70)).all(&s.db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].package_version, "2.0.0");
    }

    #[tokio::test]
    async fn replace_entries_honours_deadline() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let rows = super::super::index::parse_bundle(&bundle(&["1.0.0"]), &semver::Version::new(0, 3, 0), "en-US").unwrap();
        let tx = s.db.begin().await.unwrap();
        assert!(replace_entries(&s, &tx, 7, 70, 1, rows, std::time::Instant::now()).await.is_err());
    }
}
