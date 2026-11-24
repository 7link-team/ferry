//! Automatic tasks have execution provenance, but never impersonate human messages.
use super::*;
use crate::entities::{ai_chat_agents as snapshots, ai_chat_turns as links};
use crate::i18n::Locale;
use sea_orm::DatabaseTransaction;
use sha2::{Digest, Sha256};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route(
        "/{workspace}/conversations/{id}/turns/{turn}/introduction-retries",
        post(retry),
    )
}

pub(super) async fn created(
    s: &AppState,
    tx: &DatabaseTransaction,
    scope: &Scope,
    id: Id,
) -> Result<()> {
    if groups::is_group(tx, id).await? {
        for a in snapshots::Entity::find()
            .filter(snapshots::Column::ConversationId.eq(id))
            .all(tx)
            .await?
        {
            added(s, tx, scope, id, a.clone()).await?;
        }
    } else {
        enqueue(s, tx, scope, id, None).await?;
    }
    Ok(())
}
pub(super) async fn added(
    s: &AppState,
    tx: &DatabaseTransaction,
    scope: &Scope,
    id: Id,
    a: snapshots::Model,
) -> Result<()> {
    enqueue(s, tx, scope, id, Some(a)).await
}
async fn enqueue(
    s: &AppState,
    tx: &DatabaseTransaction,
    scope: &Scope,
    id: Id,
    snapshot: Option<snapshots::Model>,
) -> Result<()> {
    let policy = crate::accounts::conversation_policy::Configuration::load(tx).await?;
    if !policy.introduction_enabled {
        return Ok(());
    }
    let mut c = conversations::Entity::find_by_id(id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let text = snapshot
        .as_ref()
        .map_or(c.config.as_str(), |a| a.config.as_str());
    let mut config: Configuration = serde_json::from_str(text).map_err(|_| invalid())?;
    if !is_zero_id(&config.introduction_turn_id) {
        return Ok(());
    }
    let turn = s.next_id()?;
    config.introduction_turn_id = turn;
    let locale = crate::i18n::current_locale().tag();
    let permitted =
        scope.require("conversations.run").is_ok() && scope.require("conversations.read").is_ok();
    let row = model(
        s,
        scope,
        id,
        turn,
        i64::from(turn),
        locale,
        format!("introduction:{turn}"),
        permitted,
    )?;
    turns::Entity::insert(row.into_active_model())
        .exec(tx)
        .await?;
    if let Some(mut a) = snapshot {
        // message_id=0 denotes an automatic task without a human parent.
        links::Entity::insert(
            links::Model {
                id: turn.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
                message_id: 0,
                chat_agent_id: a.id,
                hop: 0,
                caller_chat_agent_id: 0,
            }
            .into_active_model(),
        )
        .exec(tx)
        .await?;
        a.config = appearance::preserve(&a.config, &serde_json::to_string(&config).unwrap());
        a.into_active_model().reset_all().update(tx).await?;
    } else {
        c.config = appearance::preserve(&c.config, &serde_json::to_string(&config).unwrap());
        c.into_active_model().reset_all().update(tx).await?;
    }
    Ok(())
}
fn model(
    _s: &AppState,
    scope: &Scope,
    room: Id,
    id: Id,
    root: i64,
    locale: &str,
    client_id: String,
    permitted: bool,
) -> Result<turns::Model> {
    let now = db::now();
    Ok(turns::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        conversation_id: room.into(),
        owner_id: scope.user.id.into(),
        state: if permitted { "queued" } else { "failed" }.into(),
        lease_owner: String::new(),
        expires_at: 0,
        version: 1,
        client_id,
        prompt: String::new(),
        attachments: "[]".into(),
        native_session_id: String::new(),
        candidate_session: String::new(),
        error: if permitted {
            String::new()
        } else {
            crate::i18n::store_message(&Message::new("conversations.introductionPermission"))
        },
        usage: "null".into(),
        decisions: "{}".into(),
        created_at: now,
        updated_at: now,
        purpose: "introduction".into(),
        introduction_id: root,
        introduction_locale: locale.into(),
    })
}

/// This is a membership-generation check, not an admission toggle check: claimed
/// tasks obey normal Stop semantics even if the global toggle subsequently changes.
pub(super) async fn validate(
    conn: &impl ConnectionTrait,
    row: &turns::Model,
    c: &conversations::Model,
) -> Result<()> {
    if row.purpose == "message" {
        return Ok(());
    }
    if row.purpose != "introduction"
        || row.introduction_id <= 0
        || !Locale::ALL
            .iter()
            .any(|l| l.tag() == row.introduction_locale)
    {
        return Err(invalid());
    }
    let config = if let Some(link) = links::Entity::find_by_id(row.id).one(conn).await? {
        let member = snapshots::Entity::find_by_id(link.chat_agent_id)
            .filter(snapshots::Column::ConversationId.eq(c.id))
            .filter(snapshots::Column::State.eq("active"))
            .one(conn)
            .await?
            .ok_or_else(|| error(StatusCode::CONFLICT, "conversations.introductionStale"))?;
        member.config
    } else {
        c.config.clone()
    };
    let config: Configuration = serde_json::from_str(&config).map_err(|_| invalid())?;
    if i64::from(config.introduction_turn_id) != row.introduction_id {
        return Err(error(
            StatusCode::CONFLICT,
            "conversations.introductionStale",
        ));
    }
    Ok(())
}

/// Same task envelope for both engines; provider role=user is a wire role, not a
/// public human message. Administrator instructions never enter public turn.prompt.
pub(super) async fn prompt(
    s: &AppState,
    row: &turns::Model,
    c: &conversations::Model,
    scope: &Scope,
    policy: &crate::accounts::conversation_policy::Configuration,
    config: &Configuration,
) -> Result<String> {
    if row.purpose != "introduction" {
        let attachments: Vec<Attachment> =
            serde_json::from_str(&row.attachments).map_err(|_| invalid())?;
        return Ok(run::turn_prompt(
            &policy.instructions,
            &config.system_prompt,
            &row.prompt,
            &attachments,
        ));
    }
    validate(&s.db, row, c).await?;
    let agent = available(&s.db, scope, c.agent_id.into()).await?;
    let mut permissions: Vec<String> =
        if is_zero_id(&crate::platform_tools::role_id(&s.db, &agent).await?) {
            Vec::new()
        } else {
            crate::platform_tools::authorized_scope(s, scope.clone(), &agent)
                .await?
                .permissions
                .into_iter()
                .collect()
        };
    permissions.sort();
    let definition = crate::entities::definition_revisions::Entity::find_by_id(c.revision_id)
        .filter(crate::entities::definition_revisions::Column::WorkspaceId.eq(c.workspace_id))
        .filter(crate::entities::definition_revisions::Column::SubjectKind.eq("agent"))
        .filter(crate::entities::definition_revisions::Column::SubjectId.eq(c.agent_id))
        .one(&s.db)
        .await?
        .map(|r| serde_json::from_str::<Value>(&r.definition))
        .transpose()
        .map_err(|_| invalid())?
        .unwrap_or(json!({}));
    let locale = Locale::from_tag(&row.introduction_locale).ok_or_else(invalid)?;
    let task = crate::i18n::render(locale, &Message::new("conversations.introductionTask"));
    let facts = json!({"name":config.name,"role":definition["role"],"description":definition["description"],"effective_permissions":permissions,"locale":locale.tag()});
    Ok(format!(
        "<platform_instructions source=\"platform settings\">\n{}\n</platform_instructions>\n<agent_instructions source=\"agent configuration, not the user's words\">\n{}\n</agent_instructions>\n<automatic_task origin=\"platform introduction; not a human message\">\n{}\n<administrator_guidance>\n{}\n</administrator_guidance>\n<configured_facts>\n{}\n</configured_facts>\n</automatic_task>",
        policy.instructions, config.system_prompt, task, policy.introduction_instructions, facts
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retry {
    client_id: String,
}
async fn retry(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, turn)): Path<(String, Id, Id)>,
    Json(body): Json<Retry>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.run")?;
    scope.require("conversations.read")?;
    // Reuse the existing client token validation without admitting an empty user message.
    check_message("introduction retry", &body.client_id, &[])?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    let original = find_turn(&tx, &c, turn).await?;
    if original.purpose != "introduction" {
        return Err(invalid());
    }
    validate(&tx, &original, &c).await?;
    let execution = groups::execution(&tx, &original, c.clone()).await?;
    available(&tx, &scope, execution.agent_id.into()).await?;
    let config: Configuration = serde_json::from_str(&execution.config).map_err(|_| invalid())?;
    if config.runtime == Runtime::Api {
        scope.require("credentials.use")?;
    }
    let hash = Sha256::digest(
        format!(
            "{}:{}:{}",
            original.introduction_id, scope.user.id, body.client_id
        )
        .as_bytes(),
    );
    let client = format!("introduction-retry:{hash:x}");
    if let Some(old) = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::ClientId.eq(&client))
        .one(&tx)
        .await?
    {
        return Ok((StatusCode::OK, Json(json!({"id":Id::from(old.id)}))));
    }
    if !crate::accounts::conversation_policy::Configuration::load(&tx)
        .await?
        .introduction_enabled
    {
        return Err(error(
            StatusCode::CONFLICT,
            "conversations.introductionDisabled",
        ));
    }
    if c.state != "active" {
        return Err(error(StatusCode::CONFLICT, "conversations.archived"));
    }
    let chain = chain(&tx, id, original.introduction_id).await?;
    if chain.iter().any(|t| t.state == "completed") {
        return Err(error(
            StatusCode::CONFLICT,
            "conversations.introductionCompleted",
        ));
    }
    if chain.iter().any(|t| active(&t.state)) || has_active(&tx, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    let next = s.next_id()?;
    let row = model(
        &s,
        &scope,
        id,
        next,
        original.introduction_id,
        &original.introduction_locale,
        client,
        true,
    )?;
    turns::Entity::insert(row.into_active_model())
        .exec(&tx)
        .await?;
    if let Some(link) = links::Entity::find_by_id(original.id).one(&tx).await? {
        links::Entity::insert(
            links::Model {
                id: next.into(),
                ..link
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
    }
    c.updated_at = db::now();
    c.version += 1;
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "run",
        "conversations",
        &id,
        json!({"turn_id":next,"introduction_id":Id::from(original.introduction_id)}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"id":next}))))
}
async fn chain(conn: &impl ConnectionTrait, id: Id, root: i64) -> Result<Vec<turns::Model>> {
    Ok(turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::IntroductionId.eq(root))
        .all(conn)
        .await?)
}
pub(super) async fn decorate(
    s: &AppState,
    scope: &Scope,
    c: &conversations::Model,
    value: &mut Value,
) -> Result<()> {
    let policy = crate::accounts::conversation_policy::Configuration::load(&s.db).await?;
    let all = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(c.id))
        .all(&s.db)
        .await?;
    let busy = all.iter().any(|t| active(&t.state));
    if let Some(items) = value["turns"].as_array_mut() {
        for item in items {
            let Some(row) = all
                .iter()
                .find(|t| json!(Id::from(t.id)) == item["id"] && t.purpose == "introduction")
            else {
                continue;
            };
            let chain: Vec<_> = all
                .iter()
                .filter(|t| t.introduction_id == row.introduction_id)
                .collect();
            let allowed = scope.require("conversations.run").is_ok()
                && scope.require("conversations.read").is_ok();
            let valid = validate(&s.db, row, c).await.is_ok();
            let executable = match groups::execution(&s.db, row, c.clone()).await {
                Ok(execution) => match available(&s.db, scope, execution.agent_id.into()).await {
                    Ok(_) => serde_json::from_str::<Configuration>(&execution.config).is_ok_and(
                        |config| {
                            config.runtime != Runtime::Api
                                || scope.require("credentials.use").is_ok()
                        },
                    ),
                    Err(_) => false,
                },
                Err(_) => false,
            };
            let completed = chain.iter().any(|t| t.state == "completed");
            let retry = policy.introduction_enabled
                && allowed
                && valid
                && executable
                && !busy
                && !completed
                && c.state == "active";
            let uncertain = chain.iter().any(|t| {
                matches!(t.state.as_str(), "interrupted" | "timed_out")
                    || !t.candidate_session.is_empty() && t.state != "completed"
            }) || crate::entities::ai_calls::Entity::find()
                .filter(crate::entities::ai_calls::Column::ConversationId.eq(c.id))
                .filter(
                    crate::entities::ai_calls::Column::ConversationTurnId
                        .is_in(chain.iter().map(|t| t.id)),
                )
                .filter(crate::entities::ai_calls::Column::State.is_in(["sent", "unknown"]))
                .count(&s.db)
                .await?
                > 0;
            let reason = if completed {
                Some("conversations.introductionCompleted")
            } else if !valid {
                Some("conversations.introductionStale")
            } else if !policy.introduction_enabled {
                Some("conversations.introductionDisabled")
            } else if !allowed {
                Some("conversations.introductionPermission")
            } else if busy {
                Some("conversations.busy")
            } else {
                None
            };
            item["can_retry_introduction"] = json!(retry);
            item["introduction_retry_reason"] = reason
                .map(|k| {
                    json!(crate::i18n::render(
                        crate::i18n::current_locale(),
                        &Message::new(k)
                    ))
                })
                .unwrap_or(Value::Null);
            item["introduction_retry_warning"] = if uncertain {
                json!(crate::i18n::render(
                    crate::i18n::current_locale(),
                    &Message::new("conversations.introductionRetryWarning")
                ))
            } else {
                Value::Null
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::admin;
    use crate::conversations::tests::seeded;
    use sea_orm::sea_query::Expr;

    async fn policy(s: &AppState, enabled: bool) {
        crate::entities::platform_state::Entity::update_many()
            .col_expr(crate::entities::platform_state::Column::ConversationConfig,
                Expr::value(json!({"introduction_enabled":enabled,"introduction_instructions":"PRIVATE INTRO GUIDANCE"}).to_string()))
            .exec(&s.db).await.unwrap();
    }
    async fn create_room(s: &AppState, scope: Scope, agent: i64, group: bool) -> Id {
        let (_,Json(v))=super::super::create(State(s.clone()),scope,Json(serde_json::from_value(
            json!({"agent_id":Id::from(agent),"kind":if group {"group"}else{"private"},"agent_ids":if group {vec![Id::from(agent)]}else{vec![]}})
        ).unwrap())).await.unwrap();
        serde_json::from_value(v["id"].clone()).unwrap()
    }
    async fn rows(s: &AppState, id: Id) -> Vec<turns::Model> {
        turns::Entity::find()
            .filter(turns::Column::ConversationId.eq(id))
            .order_by_asc(turns::Column::Id)
            .all(&s.db)
            .await
            .unwrap()
    }
    async fn end(s: &AppState, id: i64, state: &str) {
        turns::Entity::update_many()
            .col_expr(turns::Column::State, Expr::value(state))
            .filter(turns::Column::Id.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
    }
    async fn members(s: &AppState, id: Id, agents: Vec<Id>) -> Result<Json<Value>> {
        let version = crate::entities::ai_chat_members::Entity::find()
            .filter(crate::entities::ai_chat_members::Column::ConversationId.eq(id))
            .order_by_asc(crate::entities::ai_chat_members::Column::Id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .id;
        super::super::interactions::members(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(
                serde_json::from_value(
                    json!({"member_ids":[],"agent_ids":agents,"version":Id::from(version)}),
                )
                .unwrap(),
            ),
        )
        .await
    }

    #[tokio::test]
    async fn introduction_missing_run_records_failure_without_breaking_creation_and_explicit_retry_is_idempotent()
     {
        let (_dir, s, old) = seeded().await;
        policy(&s, true).await;
        let agent = find(&s.db, &admin(1), old).await.unwrap().agent_id;
        let mut denied = admin(1);
        denied.permissions.remove("conversations.run");
        let id = create_room(&s, denied, agent, false).await;
        let root = rows(&s, id).await.remove(0);
        assert_eq!(root.state, "failed");
        assert_eq!(
            crate::i18n::decode_stored(&root.error).unwrap().key,
            "conversations.introductionPermission"
        );
        assert!(
            crate::entities::ai_calls::Entity::find()
                .all(&s.db)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            super::super::run::claim(&s, root.id.into())
                .await
                .unwrap()
                .is_none()
        );
        let request = || {
            Json(Retry {
                client_id: "explicit-token".into(),
            })
        };
        let (a, b) = tokio::join!(
            retry(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id, root.id.into())),
                request()
            ),
            retry(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id, root.id.into())),
                request()
            )
        );
        let (status_a, Json(first)) = a.unwrap();
        let (status_b, Json(concurrent)) = b.unwrap();
        assert_eq!(first, concurrent);
        assert!([StatusCode::ACCEPTED, StatusCode::OK].contains(&status_a));
        assert!([StatusCode::ACCEPTED, StatusCode::OK].contains(&status_b));
        assert_ne!(status_a, status_b);
        let (status, Json(same)) = retry(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, root.id.into())),
            request(),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first, same);
        assert_eq!(rows(&s, id).await.len(), 2);
        let retry_id: Id = serde_json::from_value(first["id"].clone()).unwrap();
        end(&s, retry_id.into(), "completed").await;
        let again = retry(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, root.id.into())),
            Json(Retry {
                client_id: "another-token".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(again.0, StatusCode::CONFLICT);
        let Json(v) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap();
        assert!(
            v["turns"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["can_retry_introduction"] == false)
        );
    }

    #[tokio::test]
    async fn introduction_disable_cancels_only_unclaimed_turns_and_does_not_backfill_old_rooms() {
        let (_dir, s, old) = seeded().await;
        let agent = find(&s.db, &admin(1), old).await.unwrap().agent_id;
        let disabled = create_room(&s, admin(1), agent, false).await;
        assert!(rows(&s, disabled).await.is_empty());
        policy(&s, true).await;
        let queued = create_room(&s, admin(1), agent, false).await;
        let root = rows(&s, queued).await.remove(0);
        policy(&s, false).await;
        assert!(
            super::super::run::claim(&s, root.id.into())
                .await
                .unwrap()
                .is_none()
        );
        let row = rows(&s, queued).await.remove(0);
        assert_eq!(row.state, "cancelled");
        assert_eq!(
            crate::i18n::decode_stored(&row.error).unwrap().key,
            "conversations.introductionDisabled"
        );
        policy(&s, true).await;
        for id in [old, disabled] {
            let _ = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
                .await
                .unwrap();
            assert!(rows(&s, id).await.is_empty());
        }
        let running = create_room(&s, admin(1), agent, false).await;
        let root = rows(&s, running).await.remove(0);
        let (row, _, owner) = super::super::run::claim(&s, root.id.into())
            .await
            .unwrap()
            .unwrap();
        policy(&s, false).await;
        let (tx, _, _) = super::super::run::leased(&s, &row, &owner).await.unwrap();
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn introduction_group_membership_dedupes_and_readd_rejects_the_old_root() {
        let (_dir, s, old) = seeded().await;
        policy(&s, true).await;
        let agent = find(&s.db, &admin(1), old).await.unwrap().agent_id;
        let id = create_room(&s, admin(1), agent, true).await;
        let root = rows(&s, id).await.remove(0);
        let link = links::Entity::find_by_id(root.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(link.message_id, 0);
        end(&s, root.id, "failed").await;
        members(&s, id, vec![agent.into()]).await.unwrap();
        assert_eq!(rows(&s, id).await.len(), 1);
        members(&s, id, vec![]).await.unwrap();
        members(&s, id, vec![agent.into()]).await.unwrap();
        let attempts = rows(&s, id).await;
        assert_eq!(attempts.len(), 2);
        assert_ne!(attempts[0].introduction_id, attempts[1].introduction_id);
        let c = find(&s.db, &admin(1), id).await.unwrap();
        assert!(validate(&s.db, &root, &c).await.is_err());
        let Json(v) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap();
        assert!(
            v["turns"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["message_id"].is_null() && t["agent_id"] == json!(Id::from(agent)))
        );
        assert!(
            retry(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id, root.id.into())),
                Json(Retry {
                    client_id: "stale".into()
                })
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn introduction_private_prompt_locale_and_backfill_never_fabricate_user_content() {
        let (_dir, s, old) = seeded().await;
        policy(&s, true).await;
        let agent = find(&s.db, &admin(1), old).await.unwrap().agent_id;
        let id =
            crate::i18n::with_locale(Locale::MsMy, create_room(&s, admin(1), agent, false)).await;
        let row = rows(&s, id).await.remove(0);
        assert_eq!(row.introduction_locale, "ms-MY");
        let c = find(&s.db, &admin(1), id).await.unwrap();
        let config: Configuration = serde_json::from_str(&c.config).unwrap();
        let p = prompt(
            &s,
            &row,
            &c,
            &admin(1),
            &crate::accounts::conversation_policy::Configuration::load(&s.db)
                .await
                .unwrap(),
            &config,
        )
        .await
        .unwrap();
        assert!(p.contains("PRIVATE INTRO GUIDANCE"));
        assert!(p.contains("Jana pengenalan"));
        assert!(p.contains("<automatic_task"));
        assert!(!p.contains("<user_message>"));
        messages::append_text(
            &s.db,
            &row,
            &c,
            s.next_id().unwrap().into(),
            "greeting",
            "Actual generated greeting",
            true,
        )
        .await
        .unwrap();
        messages::backfill(&s.db).await.unwrap();
        messages::backfill(&s.db).await.unwrap();
        let Json(v) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap();
        assert!(!v.to_string().contains("PRIVATE INTRO GUIDANCE"));
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["role"], "assistant");
        assert!(v["messages"][0]["reply_to"].is_null());
    }

    #[tokio::test]
    async fn introduction_stale_membership_blocks_leased_output_and_unknown_attempt_warns_before_retry()
     {
        let (_dir, s, old) = seeded().await;
        policy(&s, true).await;
        let agent = find(&s.db, &admin(1), old).await.unwrap().agent_id;
        let id = create_room(&s, admin(1), agent, false).await;
        let root = rows(&s, id).await.remove(0);
        let (row, _, owner) = super::super::run::claim(&s, root.id.into())
            .await
            .unwrap()
            .unwrap();
        end(&s, row.id, "interrupted").await;
        let Json(v) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap();
        assert!(v["turns"][0]["introduction_retry_warning"].is_string());
        assert_eq!(v["turns"][0]["can_retry_introduction"], true);
        let c = find(&s.db, &admin(1), id).await.unwrap();
        let mut config: Configuration = serde_json::from_str(&c.config).unwrap();
        config.introduction_turn_id = s.next_id().unwrap();
        end(&s, row.id, "running").await;
        conversations::Entity::update_many()
            .col_expr(
                conversations::Column::Config,
                Expr::value(serde_json::to_string(&config).unwrap()),
            )
            .filter(conversations::Column::Id.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(super::super::run::leased(&s, &row, &owner).await.is_err());
        assert!(
            retry(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id, row.id.into())),
                Json(Retry {
                    client_id: "old-membership".into()
                })
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn introduction_schema_is_idempotent_and_down_preserves_provenance() {
        let (_dir, s, id) = seeded().await;
        use crate::migration::introductions::Migration;
        use sea_orm_migration::{MigrationTrait, SchemaManager};
        let m = SchemaManager::new(&s.db);
        Migration.up(&m).await.unwrap();
        Migration.up(&m).await.unwrap();
        let turn = super::super::tests::sent(&s, id, "migration-normal").await;
        let row = crate::entities::ai_turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.purpose, "message");
        assert_eq!(row.introduction_id, 0);
        assert!(row.introduction_locale.is_empty());
        s.db.execute_unprepared(&format!("UPDATE ai_turns SET purpose='introduction',introduction_id={},introduction_locale='en-US' WHERE id={}",i64::from(turn),i64::from(turn))).await.unwrap();
        assert!(
            Migration
                .down(&m)
                .await
                .unwrap_err()
                .to_string()
                .contains("Retain introduction")
        );
        assert!(m.has_column("ai_turns", "purpose").await.unwrap());
        crate::entities::ai_turns::Entity::delete_many()
            .filter(crate::entities::ai_turns::Column::Id.eq(turn))
            .exec(&s.db)
            .await
            .unwrap();
        Migration.down(&m).await.unwrap();
        assert!(!m.has_column("ai_turns", "purpose").await.unwrap());
        Migration.up(&m).await.unwrap();
        assert!(
            m.has_index("ai_turns", "ai_turns_introduction")
                .await
                .unwrap()
        );
    }
}
