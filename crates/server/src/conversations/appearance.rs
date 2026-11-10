//! A room's presentation is independent of immutable Agent execution snapshots.
use super::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Appearance {
    pub pattern: String,
    pub color: String,
    pub image: Option<Id>,
}

pub(super) fn preserve(old: &str, new: &str) -> String {
    let mut next: Value = serde_json::from_str(new).unwrap_or(json!({}));
    if let Ok(previous) = serde_json::from_str::<Value>(old) {
        if let Some(value) = previous.get("appearance") {
            next["appearance"] = value.clone();
        }
    }
    next.to_string()
}

pub(super) async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Appearance>,
) -> Result<Json<Value>> {
    scope.require("conversations.update")?;
    if !["plain", "dots", "grid", "diagonal"].contains(&body.pattern.as_str())
        || (!body.color.is_empty()
            && !(body.color.len() == 7
                && body.color.starts_with('#')
                && body.color[1..].bytes().all(|v| v.is_ascii_hexdigit())))
    {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    if c.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    if let Some(file) = body.image {
        let file = files::resolve(&tx, scope.workspace_id, id, file).await?;
        if !["image/png", "image/jpeg", "image/webp", "image/avif"].contains(&file.mime.as_str())
            || file.size > 5 * 1024 * 1024
        {
            return Err(invalid());
        }
    }
    let mut config: Value = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    config["appearance"] = serde_json::to_value(&body).unwrap();
    c.config = config.to_string();
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "conversations",
        &id,
        json!({"appearance":body}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(serde_json::to_value(body).unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::admin;
    #[tokio::test]
    async fn persists_and_rejects_cross_room_or_unauthorized_changes() {
        let (_dir, s, id) = super::super::tests::seeded().await;
        let body = || Appearance {
            pattern: "dots".into(),
            color: "#182634".into(),
            image: None,
        };
        update(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(body()),
        )
        .await
        .unwrap();
        let c = conversations::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&c.config).unwrap()["appearance"]["pattern"],
            "dots"
        );
        assert!(preserve(&c.config, "{\"model\":\"new\"}").contains("dots"));
        let mut other = admin(1);
        other.user.id = 999.into();
        assert!(
            update(
                State(s.clone()),
                other,
                Path(("1".into(), id)),
                Json(body())
            )
            .await
            .is_err()
        );
        assert!(
            update(
                State(s.clone()),
                admin(2),
                Path(("2".into(), id)),
                Json(body())
            )
            .await
            .is_err()
        );
        let mut bad = body();
        bad.color = "url(https://invalid)".into();
        assert!(
            update(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id)),
                Json(bad)
            )
            .await
            .is_err()
        );
        let mut missing = body();
        missing.image = Some(999.into());
        assert!(
            update(State(s), admin(1), Path(("1".into(), id)), Json(missing))
                .await
                .is_err()
        );
    }
}
