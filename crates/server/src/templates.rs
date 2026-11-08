//! Creation-time parameter expansion; runtime pipeline placeholders remain unchanged.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    error::{ApiError, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Called only while creating a workspace, inside its transaction. These are
/// ordinary workspace-owned templates, so editing/deleting one is permanent.
/// Add scenarios here only after real execution acceptance, without local IDs.
pub(crate) async fn seed_examples(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
) -> Result<()> {
    use crate::entities::pipeline_templates as rows;
    let content = convert_legacy_content(include_str!("templates/examples/dependency-build.yml"))?;
    validate_template(&content, "[]", Kind::Pipeline)?;
    let now = db::now();
    rows::Entity::insert(rows::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(workspace.into()),
        name: Set("示例：Go 上下游依赖构建与产物传递".into()),
        description: Set("真实 Agent 验证：golang/text v0.28.0 → golang/net v0.43.0。演示任务依赖、步骤/任务输出、产物上传下载和 SHA 校验。使用时为两个任务选择自己的主机；需要 Bash、Git、Go 1.23+，可访问 GitHub 和 Go 模块代理。仅手动运行，不部署。".into()),
        content: Set(content),
        parameters: Set("[]".into()),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    }).exec(conn).await?;
    let content =
        convert_legacy_content(include_str!("templates/examples/repository-report.json"))?;
    validate_template(&content, "[]", Kind::Pipeline)?;
    rows::Entity::insert(rows::ActiveModel {
        id: Set(s.next_id()?.into()), workspace_id: Set(workspace.into()),
        name: Set("示例：公开仓库信息与 JSON 报告".into()),
        description: Set("真实执行验证：查询 GitHub 公开仓库 golang/text，生成可下载的 JSON 报告。无需凭据或主机；可在节点表单修改仓库。仅手动运行，需要服务器可访问 GitHub API。".into()),
        content: Set(content), parameters: Set("[]".into()), version: Set(1),
        created_at: Set(now), updated_at: Set(now),
    }).exec(conn).await?;
    let content = include_str!("templates/examples/http-service.json");
    let parameters = json!([
        {"name":"port","label":crate::i18n::message("templates.examples.httpPort"),
         "type":"integer","min":1,"max":65535,"default":8080},
        {"name":"message","label":crate::i18n::message("templates.examples.httpMessage"),
         "type":"string","min_length":1,"max_length":2000,
         "default":crate::i18n::message("templates.examples.httpWelcome")}
    ])
    .to_string();
    validate_template(content, &parameters, Kind::App)?;
    crate::entities::app_templates::Entity::insert(crate::entities::app_templates::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(workspace.into()),
        name: Set(crate::i18n::message("templates.examples.httpName")),
        description: Set(crate::i18n::message("templates.examples.httpDescription")),
        content: Set(content.into()),
        parameters: Set(parameters),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(conn)
    .await?;
    let content = convert_legacy_content(include_str!("templates/examples/native-http-build.yml"))?;
    validate_template(&content, "[]", Kind::Pipeline)?;
    rows::Entity::insert(rows::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(workspace.into()),
        name: Set(crate::i18n::message("templates.examples.nativeBuildName")),
        description: Set(crate::i18n::message(
            "templates.examples.nativeBuildDescription",
        )),
        content: Set(content),
        parameters: Set("[]".into()),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(conn)
    .await?;
    let content = include_str!("templates/examples/native-http-service.json");
    let parameters = json!([
        {"name":"pipeline","label":crate::i18n::message("templates.examples.nativePipeline"),
         "type":"string","min_length":1,"max_length":200},
        {"name":"port","label":crate::i18n::message("templates.examples.httpPort"),
         "type":"integer","min":1,"max":65535,"default":8080},
        {"name":"message","label":crate::i18n::message("templates.examples.httpMessage"),
         "type":"string","min_length":1,"max_length":2000,
         "default":crate::i18n::message("templates.examples.httpWelcome")}
    ])
    .to_string();
    validate_template(content, &parameters, Kind::App)?;
    crate::entities::app_templates::Entity::insert(crate::entities::app_templates::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(workspace.into()),
        name: Set(crate::i18n::message("templates.examples.nativeName")),
        description: Set(crate::i18n::message("templates.examples.nativeDescription")),
        content: Set(content.into()),
        parameters: Set(parameters),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(conn)
    .await?;
    let content = include_str!("templates/examples/nats-config.conf");
    validate_template(content, "[]", Kind::Config)?;
    crate::entities::config_templates::Entity::insert(
        crate::entities::config_templates::ActiveModel {
            id: Set(s.next_id()?.into()),
            workspace_id: Set(workspace.into()),
            name: Set("example-nats-config".into()),
            description: Set(crate::i18n::message(
                "templates.examples.natsConfigDescription",
            )),
            content: Set(content.into()),
            parameters: Set("[]".into()),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        },
    )
    .exec(conn)
    .await?;
    let content = include_str!("templates/examples/nats-jetstream.json");
    let mut definitions = vec![
        json!({"name":"token","label":crate::i18n::message("templates.examples.natsToken"),"type":"string","secret":true}),
        json!({"name":"config","label":crate::i18n::message("templates.examples.natsConfig"),"type":"string","min_length":1,"max_length":60,"default":"example-nats-config"}),
    ];
    for (suffix, port) in [("a", 4222), ("b", 4223), ("c", 4224)] {
        definitions.push(json!({"name":format!("port_{suffix}"),"label":format!("{} {}",crate::i18n::message("templates.examples.natsPort"),suffix.to_uppercase()),"type":"integer","min":1,"max":65535,"default":port}));
        definitions.push(json!({"name":format!("advertise_{suffix}"),"label":format!("{} {}",crate::i18n::message("templates.examples.natsAdvertise"),suffix.to_uppercase()),"type":"string","min_length":1,"max_length":255}));
    }
    let parameters = serde_json::to_string(&definitions).map_err(|_| ApiError::missing())?;
    validate_template(content, &parameters, Kind::App)?;
    crate::entities::app_templates::Entity::insert(crate::entities::app_templates::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(workspace.into()),
        name: Set(crate::i18n::message("templates.examples.natsName")),
        description: Set(crate::i18n::message("templates.examples.natsDescription")),
        content: Set(content.into()),
        parameters: Set(parameters),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(conn)
    .await?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    App,
    Pipeline,
    Actions,
    Config,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Parameter {
    name: String,
    #[serde(default)]
    label: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    default: Option<Value>,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    choices: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(default)]
    secret: bool,
}
fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
fn parameter_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                && segment
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}
fn value_check(p: &Parameter, value: &Value) -> Result<()> {
    let valid = match p.kind.as_str() {
        "string" => value.as_str().is_some_and(|s| {
            !s.contains('\0')
                && s.len() <= 65536
                && p.min_length.is_none_or(|n| s.chars().count() >= n)
                && p.max_length.is_none_or(|n| s.chars().count() <= n)
                && p.choices
                    .as_ref()
                    .is_none_or(|choices| choices.iter().any(|v| v == s))
                && (!p.secret || crate::variables::valid_name(s))
        }),
        "integer" => value
            .as_i64()
            .is_some_and(|n| p.min.is_none_or(|min| n >= min) && p.max.is_none_or(|max| n <= max)),
        "boolean" => value.as_bool().is_some(),
        _ => false,
    };
    if !valid {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.parameterInvalid").with("name", p.name.clone()),
        ));
    }
    Ok(())
}
fn parameters(text: &str, _kind: Kind) -> Result<BTreeMap<String, Parameter>> {
    let items: Vec<Parameter> = serde_json::from_str(text).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.definitionsInvalid"),
        )
    })?;
    if items.len() > 64 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.parameterLimit"),
        ));
    }
    let mut out = BTreeMap::new();
    for p in items {
        if !parameter_name(&p.name)
            || p.label.chars().count() > 100
            || !["string", "integer", "boolean"].contains(&p.kind.as_str())
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.parameterMetadataInvalid"),
            ));
        }
        if (p.secret && p.kind != "string")
            || (p.kind != "string"
                && (p.choices.is_some() || p.min_length.is_some() || p.max_length.is_some()))
            || (p.kind != "integer" && (p.min.is_some() || p.max.is_some()))
            || p.min.zip(p.max).is_some_and(|(a, b)| a > b)
            || p.min_length.zip(p.max_length).is_some_and(|(a, b)| a > b)
            || p.choices
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 100 || v.iter().any(|s| s.is_empty()))
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.validationInvalid"),
            ));
        }
        if let Some(value) = &p.default {
            value_check(&p, value)?;
        }
        let name = p.name.clone();
        if out.insert(name, p).is_some() {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.parameterDuplicate"),
            ));
        }
    }
    Ok(out)
}
fn document(text: &str) -> Result<Value> {
    if text.len() > 262144 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.contentTooLarge"),
        ));
    }
    let value: Value = serde_yaml_ng::from_str(text).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.documentInvalid"),
        )
    })?;
    if !value.is_object() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.objectRequired"),
        ));
    }
    Ok(value)
}
fn expand(
    value: &Value,
    definitions: &BTreeMap<String, Parameter>,
    values: Option<&BTreeMap<String, Value>>,
    kind: Kind,
    field: &str,
    used: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Value> {
    if depth > 64 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.nestingTooDeep"),
        ));
    }
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                if key.contains("{{") && kind != Kind::Actions {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("templates.fieldNameForbidden"),
                    ));
                }
                if kind == Kind::Actions {
                    let mut offset = 0;
                    while let Some(relative) = key[offset..].find("{{") {
                        let start = offset + relative;
                        if start == 0 || key.as_bytes()[start - 1] != b'$' {
                            return Err(ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("templates.fieldNameForbidden"),
                            ));
                        }
                        offset = crate::pipelines::actions::expression_end(key, start + 2)? + 2;
                    }
                }
                if kind == Kind::Pipeline
                    && map.get("engine").and_then(Value::as_str)
                        == Some(crate::pipelines::actions::ENGINE)
                    && key == "workflow"
                {
                    let workflow: Value =
                        serde_yaml_ng::from_str(value.as_str().ok_or_else(|| {
                            ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("templates.workflowStringRequired"),
                            )
                        })?)
                        .map_err(|_| {
                            ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("templates.workflowInvalid"),
                            )
                        })?;
                    let mut workflow = expand(
                        &workflow,
                        definitions,
                        values,
                        Kind::Actions,
                        "",
                        used,
                        depth + 1,
                    )?;
                    if values.is_some() {
                        for job in workflow
                            .get_mut("jobs")
                            .and_then(Value::as_object_mut)
                            .into_iter()
                            .flat_map(|jobs| jobs.values_mut())
                        {
                            for step in job
                                .get_mut("steps")
                                .and_then(Value::as_array_mut)
                                .into_iter()
                                .flatten()
                            {
                                if step.get("uses").and_then(Value::as_str)
                                    == Some("actions/upload-artifact@v4")
                                    && step
                                        .pointer("/with/name")
                                        .and_then(Value::as_str)
                                        .is_some_and(|name| name.starts_with("legacy-"))
                                    && let Some(path) = step
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .and_then(|name| name.strip_prefix("Archive "))
                                {
                                    let name =
                                        crate::pipelines::artifacts::legacy_artifact_name(path);
                                    step["with"]["name"] = json!(name);
                                }
                            }
                            if job.pointer("/steps/0/id").and_then(Value::as_str)
                                == Some("prepare_workspace")
                                && job.pointer("/defaults/run/shell").and_then(Value::as_str)
                                    == Some("sh {0}")
                            {
                                let timeout = job["steps"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .skip(1)
                                    .filter_map(|step| {
                                        step.get("timeout-minutes").and_then(Value::as_u64)
                                    })
                                    .fold(30_u64, u64::saturating_add);
                                job["timeout-minutes"] = json!(timeout);
                            }
                        }
                    }
                    out.insert(
                        key.clone(),
                        json!(serde_yaml_ng::to_string(&workflow).map_err(|_| {
                            ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("templates.actionsGenerateFailed"),
                            )
                        })?),
                    );
                    continue;
                }
                out.insert(
                    key.clone(),
                    expand(
                        value,
                        definitions,
                        values,
                        kind,
                        &format!("{field}/{key}"),
                        used,
                        depth + 1,
                    )?,
                );
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => items
            .iter()
            .map(|v| {
                expand(
                    v,
                    definitions,
                    values,
                    kind,
                    &format!("{field}/*"),
                    used,
                    depth + 1,
                )
            })
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Value::String(text) => {
            let mut output = String::new();
            let mut offset = 0;
            while let Some(relative) = text[offset..].find("{{") {
                let start = offset + relative;
                if matches!(kind, Kind::Pipeline | Kind::Actions)
                    && start > 0
                    && text.as_bytes()[start - 1] == b'$'
                {
                    let end = crate::pipelines::actions::expression_end(text, start + 2)?;
                    output.push_str(&text[offset..end + 2]);
                    offset = end + 2;
                    continue;
                }
                let end = text[start + 2..]
                    .find("}}")
                    .map(|n| start + 2 + n)
                    .ok_or_else(|| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("templates.placeholderUnclosed"),
                        )
                    })?;
                let token = text[start + 2..end].trim();
                output.push_str(&text[offset..start]);
                let (parameter, ceil_minutes) = if kind == Kind::Actions {
                    match token.strip_prefix("ceilMinutes ") {
                        Some(parameter) => (parameter, true),
                        None => (token, false),
                    }
                } else {
                    (token, false)
                };
                if let Some(name) = parameter.strip_prefix(".Values.") {
                    let p = definitions.get(name).ok_or_else(|| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("templates.parameterUndeclared")
                                .with("name", name),
                        )
                    })?;
                    used.insert(name.to_owned());
                    let whole = start == 0 && end + 2 == text.len();
                    if ceil_minutes
                        && (!whole
                            || p.kind != "integer"
                            || !matches!(
                                field.split('/').collect::<Vec<_>>().as_slice(),
                                ["", "jobs", _, "steps", "*", "timeout-minutes"]
                            ))
                    {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("templates.ceilMinutesInvalid"),
                        ));
                    }
                    let actions_env = kind == Kind::Actions && actions_env_field(field);
                    let secret_field = match kind {
                        Kind::App => field == "/services/*/env/*/valueFrom",
                        Kind::Pipeline => field == "/env/*/valueFrom",
                        Kind::Actions => actions_env,
                        Kind::Config => true,
                    };
                    if p.secret && kind != Kind::Config && (!whole || !secret_field) {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("templates.secretPlacementInvalid"),
                        ));
                    }
                    if let Some(values) = values {
                        let replacement = &values[name];
                        if ceil_minutes {
                            let seconds = replacement
                                .as_i64()
                                .filter(|value| (1..=86400).contains(value))
                                .ok_or_else(|| {
                                    ApiError::localized(
                                        axum::http::StatusCode::BAD_REQUEST,
                                        crate::i18n::Message::new("templates.legacyTimeoutInvalid"),
                                    )
                                })?;
                            return Ok(json!((seconds + 59) / 60));
                        }
                        if p.secret && actions_env {
                            return Ok(json!(format!(
                                "${{{{ secrets['{}'] }}}}",
                                replacement.as_str().unwrap()
                            )));
                        }
                        if p.secret && kind == Kind::Config {
                            output.push_str(&format!(
                                "\0variable/{}\0",
                                replacement.as_str().unwrap()
                            ));
                            offset = end + 2;
                            continue;
                        }
                        if p.secret {
                            return Ok(json!(format!(
                                "variable/{}",
                                replacement.as_str().unwrap()
                            )));
                        }
                        if whole {
                            // Manifest environment values are strings even when a
                            // shared parameter also fills a numeric port field.
                            if kind == Kind::App && field == "/services/*/env/*/value" {
                                return Ok(json!(
                                    replacement
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| replacement.to_string())
                                ));
                            }
                            return Ok(replacement.clone());
                        }
                        output.push_str(
                            &replacement
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| replacement.to_string()),
                        );
                    } else {
                        output.push_str(&text[start..end + 2]);
                    }
                } else if matches!(kind, Kind::Pipeline | Kind::Actions)
                    && ["run", "branch", "repo_url", "commit"].contains(&token)
                    && token == &text[start + 2..end]
                {
                    output.push_str(&text[start..end + 2]);
                } else {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("templates.expressionUnsupported"),
                    ));
                }
                offset = end + 2;
            }
            output.push_str(&text[offset..]);
            Ok(json!(output))
        }
        _ => Ok(value.clone()),
    }
}
fn actions_env_field(field: &str) -> bool {
    let parts: Vec<_> = field.split('/').collect();
    matches!(
        parts.as_slice(),
        ["", "env", _] | ["", "jobs", _, "env", _] | ["", "jobs", _, "steps", "*", "env", _]
    )
}
fn validate_template(
    content: &str,
    text: &str,
    kind: Kind,
) -> Result<(Value, BTreeMap<String, Parameter>)> {
    let definitions = parameters(text, kind)?;
    let doc = if kind == Kind::Config {
        if content.len() > 262144 || content.contains('\0') {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.configInvalid"),
            ));
        }
        json!(content)
    } else {
        document(content)?
    };
    if kind == Kind::Pipeline && doc.get("engine").and_then(Value::as_str) == Some("ferry-graph") {
        if !definitions.is_empty() {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.parametersUnexpected"),
            ));
        }
        crate::workflows::graph::validate(&doc.to_string())?;
        return Ok((doc, definitions));
    }
    let mut used = BTreeSet::new();
    expand(&doc, &definitions, None, kind, "", &mut used, 0)?;
    if definitions.keys().any(|name| !used.contains(name)) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.parametersUnused"),
        ));
    }
    Ok((doc, definitions))
}
fn render_content(
    content: &str,
    parameters: &str,
    supplied: &BTreeMap<String, Value>,
    kind: Kind,
) -> Result<String> {
    let converted;
    let content = if kind == Kind::Pipeline {
        converted = convert_legacy_content(content)?;
        converted.as_str()
    } else {
        content
    };
    let (doc, definitions) = validate_template(content, parameters, kind)?;
    if supplied.keys().any(|k| !definitions.contains_key(k)) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.parametersUnexpected"),
        ));
    }
    if kind == Kind::Pipeline && doc.get("engine").and_then(Value::as_str) == Some("ferry-graph") {
        return Ok(content.to_owned());
    }
    let mut values = BTreeMap::new();
    for (name, p) in &definitions {
        let value = supplied.get(name).or(p.default.as_ref()).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.parameterRequired").with("name", name),
            )
        })?;
        value_check(p, value)?;
        let value = if kind == Kind::Pipeline && !p.secret {
            if let Some(value) = value.as_str() {
                json!(
                    value
                        .replace("{{run}}", "${{ github.run_number }}")
                        .replace("{{branch}}", "${{ github.ref_name }}")
                        .replace("{{commit}}", "${{ github.sha }}")
                        .replace("{{repo_url}}", "${{ env.FERRY_REPOSITORY_URL }}")
                )
            } else {
                value.clone()
            }
        } else {
            value.clone()
        };
        values.insert(name.clone(), value);
    }
    let out = expand(
        &doc,
        &definitions,
        Some(&values),
        kind,
        "",
        &mut BTreeSet::new(),
        0,
    )?;
    if kind == Kind::Config {
        return Ok(out
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| out.to_string()));
    }
    let text = serde_json::to_string_pretty(&out).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.configGenerateFailed"),
        )
    })?;
    match kind {
        Kind::App => {
            crate::deployments::manifest::parse(&text)?;
        }
        Kind::Pipeline => {
            crate::pipelines::validate_definition(&text)?;
        }
        Kind::Config => unreachable!(),
        Kind::Actions => unreachable!(),
    }
    Ok(text)
}
/// Normalize creation templates without expanding their creation-time parameters.
/// Backup storage is handled by the migration; renders and old imports use the same converter.
pub(crate) fn convert_legacy_content(content: &str) -> Result<String> {
    let doc = document(content)?;
    if doc.get("engine").and_then(Value::as_str) == Some("ferry-graph") {
        let mut graph = doc.as_object().cloned().unwrap();
        graph.remove("engine");
        // Canonical discriminator prefix supports correct DB pagination/filtering
        // without another template table, schema migration or JSON SQL dialect.
        let rest = serde_json::to_string(&graph).map_err(|_| ApiError::missing())?;
        return Ok(format!("{{\"engine\":\"ferry-graph\",{}", &rest[1..]));
    }
    if doc.get("engine").and_then(Value::as_str) == Some(crate::pipelines::actions::ENGINE) {
        return Ok(content.to_owned());
    }
    // A plain Actions workflow (YAML with jobs) gets the standard envelope so
    // the template picker can use it; its text, placeholders included, is kept.
    if doc.get("jobs").is_some() {
        return serde_json::to_string(&json!({
            "engine": crate::pipelines::actions::ENGINE,
            "workflow": content,
            "executor_bindings": [],
            "checkout_bindings": [],
            "graph_layout": {},
        }))
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("templates.pipelineConvertFailed"),
            )
        });
    }
    crate::pipelines::convert_legacy(&doc.to_string(), "", "")
}
#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    description: String,
    content: String,
    parameters: String,
    #[serde(default)]
    version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderBody {
    version: i64,
    #[serde(default)]
    values: BTreeMap<String, Value>,
}
fn validate_body(body: &Body, kind: Kind) -> Result<()> {
    if body.name.trim().is_empty() || body.name.chars().count() > 60 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("templates.nameInvalid"),
        ));
    }
    validate_template(&body.content, &body.parameters, kind)?;
    Ok(())
}
pub(crate) fn validate_app(content: &str, parameters: &str) -> Result<()> {
    validate_template(content, parameters, Kind::App).map(|_| ())
}
const SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &[],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-created_at",
};

async fn config_unreferenced(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    id: Id,
    name: &str,
) -> Result<()> {
    use crate::entities::deployments;
    for row in deployments::Entity::find()
        .filter(deployments::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?
    {
        let manifest = crate::deployments::manifest::parse(&row.manifest)?;
        if manifest
            .services
            .iter()
            .flat_map(|s| &s.files)
            .any(|f| f.from_template == id.to_string() || f.from_template == name)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("templates.applicationReferenced"),
            ));
        }
    }
    Ok(())
}
// Both resources have the same contract, but distinct tenant tables and permissions.
macro_rules! template_api {
    ($module:ident,$entity:ident,$resource:literal,$kind:expr,$field:literal) => {
        pub mod $module {
            use super::*;
            use crate::entities::$entity as rows;
            fn json_row(r: &rows::Model) -> Value { json!({"id":Id::from(r.id),"name":r.name,"description":r.description,"content":r.content,"parameters":r.parameters,"version":r.version,"created_at":r.created_at,"updated_at":r.updated_at}) }
            async fn scoped(conn: &impl sea_orm::ConnectionTrait, scope: &Scope, id: Id) -> Result<rows::Model> {
                rows::Entity::find_by_id(i64::from(id)).filter(rows::Column::WorkspaceId.eq(scope.workspace_id)).one(conn).await?.ok_or_else(ApiError::missing)
            }
            pub async fn list(State(s):State<AppState>,scope:Scope,Query(raw):Query<HashMap<String,String>>) -> Result<Json<Value>> {
                scope.require(concat!($resource,".read"))?;
                let mut list_raw=raw.clone();
                if $kind==Kind::Pipeline {list_raw.remove("workflow_kind");}
                let q=ListQuery::parse(&SPEC,&list_raw)?;
                let mut selection=rows::Entity::find();
                if $kind==Kind::Pipeline {
                    match raw.get("workflow_kind").map(String::as_str) {
                        Some("nodes") => selection=selection.filter(rows::Column::Content.starts_with("{\"engine\":\"ferry-graph\",")),
                        Some("actions") => selection=selection.filter(rows::Column::Content.not_like("{\"engine\":\"ferry-graph\",%")),
                        None => {},
                        _ => return Err(ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new("templates.documentInvalid"))),
                    }
                }
                let (items,total)=resource::list::<rows::Entity,rows::Model>(&s.db,&SPEC,selection,&scope.workspace_id,&q).await?;
                Ok(Json(json!({"items":items.iter().map(json_row).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size})))
            }
            pub async fn get(State(s):State<AppState>,scope:Scope,Path((_,id)):Path<(String,Id)>) -> Result<Json<Value>> {
                scope.require(concat!($resource,".read"))?; Ok(Json(json_row(&scoped(&s.db,&scope,id).await?)))
            }
            pub async fn create(State(s):State<AppState>,scope:Scope,Json(body):Json<Body>) -> Result<(StatusCode,Json<Value>)> {
                let tx=crate::variables::transaction(&s,scope.workspace_id).await?;
                let result=create_in(&s,&tx,&scope,body).await?;
                tx.commit().await?; Ok(result)
            }
            pub(crate) async fn create_in(s:&AppState,tx:&sea_orm::DatabaseTransaction,scope:&Scope,body:Body) -> Result<(StatusCode,Json<Value>)> {
                scope.require(concat!($resource,".create"))?;
                let mut body=body;
                if $kind==Kind::Pipeline {body.content=convert_legacy_content(&body.content)?;}
                validate_body(&body,$kind)?;
                let id=s.next_id()?; let now=db::now();
                rows::Entity::insert(rows::ActiveModel {id:Set(id.into()),workspace_id:Set(scope.workspace_id.into()),name:Set(body.name.trim().to_owned()),description:Set(body.description),content:Set(body.content),parameters:Set(body.parameters),version:Set(1),created_at:Set(now),updated_at:Set(now)}).exec(tx).await?;
                crate::hosts::activity_in(&s,tx,&scope,"create",$resource,&id,json!({"name":body.name})).await?;
                Ok((StatusCode::CREATED,Json(json!({"id":id,"version":1}))))
            }
            pub async fn update(State(s):State<AppState>,scope:Scope,Path((_,id)):Path<(String,Id)>,Json(body):Json<Body>) -> Result<Json<Value>> {
                scope.require(concat!($resource,".update"))?;
                let mut body=body;
                if $kind==Kind::Pipeline {body.content=convert_legacy_content(&body.content)?;}
                validate_body(&body,$kind)?;
                let tx=if $kind==Kind::Config {crate::variables::transaction(&s,scope.workspace_id).await?} else {s.db.begin().await?};
                if $kind==Kind::Config {let old=scoped(&tx,&scope,id).await?; if old.name!=body.name.trim() {config_unreferenced(&tx,scope.workspace_id,id,&old.name).await?;} }
                let n=rows::Entity::update_many().col_expr(rows::Column::Name,Expr::value(body.name.trim().to_owned()))
                    .col_expr(rows::Column::Description,Expr::value(body.description)).col_expr(rows::Column::Content,Expr::value(body.content))
                    .col_expr(rows::Column::Parameters,Expr::value(body.parameters)).col_expr(rows::Column::UpdatedAt,Expr::value(db::now()))
                    .col_expr(rows::Column::Version,Expr::col(rows::Column::Version).add(1))
                    .filter(rows::Column::WorkspaceId.eq(scope.workspace_id)).filter(rows::Column::Id.eq(id)).filter(rows::Column::Version.eq(body.version)).exec(&tx).await?.rows_affected;
                if n==0 { scoped(&tx,&scope,id).await?; return Err(ApiError::localized(axum::http::StatusCode::CONFLICT, crate::i18n::Message::new("templates.changed"))); }
                crate::hosts::activity_in(&s,&tx,&scope,"update",$resource,&id,json!({"name":body.name})).await?; tx.commit().await?;
                Ok(Json(json!({"id":id,"version":body.version+1})))
            }
            pub async fn delete(State(s):State<AppState>,scope:Scope,Path((_,id)):Path<(String,Id)>) -> Result<StatusCode> {
                scope.require(concat!($resource,".delete"))?; let tx=if $kind==Kind::Config {crate::variables::transaction(&s,scope.workspace_id).await?} else {s.db.begin().await?};
                if $kind==Kind::Config {let old=scoped(&tx,&scope,id).await?; config_unreferenced(&tx,scope.workspace_id,id,&old.name).await?;}
                let n=rows::Entity::delete_many().filter(rows::Column::WorkspaceId.eq(scope.workspace_id)).filter(rows::Column::Id.eq(id)).exec(&tx).await?.rows_affected;
                if n==0 {return Err(ApiError::missing());}
                crate::hosts::activity_in(&s,&tx,&scope,"delete",$resource,&id,json!({})).await?; tx.commit().await?; Ok(StatusCode::NO_CONTENT)
            }
            pub async fn render(State(s):State<AppState>,scope:Scope,Path((_,id)):Path<(String,Id)>,Json(body):Json<RenderBody>) -> Result<Json<Value>> {
                scope.require(concat!($resource,".read"))?; let row=scoped(&s.db,&scope,id).await?;
                if row.version!=body.version {return Err(ApiError::localized(axum::http::StatusCode::CONFLICT, crate::i18n::Message::new("templates.reloadRequired")));}
                let text=render_content(&row.content,&row.parameters,&body.values,$kind)?;
                let text=if $kind==Kind::Config { config_preview(&text) } else { text };
                Ok(Json(json!({$field:text,"template_id":id,"template_version":row.version})))
            }
        }
    }
}
template_api!(
    configs,
    config_templates,
    "config-templates",
    Kind::Config,
    "content"
);
/// NUL-delimited references are internal tokens, never accepted as literal input.
pub(crate) fn config_preview(text: &str) -> String {
    text.split('\0')
        .enumerate()
        .map(|(index, part)| {
            if index % 2 == 1 {
                format!("<{}>", part)
            } else {
                part.to_owned()
            }
        })
        .collect()
}
pub(crate) fn config_content(
    content: &str,
    parameters: &str,
    values: &BTreeMap<String, Value>,
) -> Result<String> {
    render_content(content, parameters, values, Kind::Config)
}
template_api!(apps, app_templates, "app-templates", Kind::App, "manifest");
template_api!(
    pipelines,
    pipeline_templates,
    "pipeline-templates",
    Kind::Pipeline,
    "definition"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workspace_examples_and_template_kind_lists_are_isolated_and_transactional() {
        use crate::{
            accounts::{auth::User, workspace},
            entities::pipeline_templates as rows,
        };
        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "template-examples-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
        });
        let mut workspaces = Vec::new();
        for slug in ["examples-a", "examples-b", "examples-rollback"] {
            let tx = s.db.begin().await.unwrap();
            let result = workspace::create_in(
                &s,
                1.into(),
                serde_json::from_value(json!({"name":slug,"slug":slug})).unwrap(),
                &tx,
            )
            .await
            .unwrap();
            let id: Id = serde_json::from_value(result["id"].clone()).unwrap();
            if slug == "examples-rollback" {
                tx.rollback().await.unwrap();
                assert!(
                    crate::entities::config_templates::Entity::find()
                        .filter(crate::entities::config_templates::Column::WorkspaceId.eq(id))
                        .all(&s.db)
                        .await
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    rows::Entity::find()
                        .filter(rows::Column::WorkspaceId.eq(id))
                        .all(&s.db)
                        .await
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    crate::entities::app_templates::Entity::find()
                        .filter(crate::entities::app_templates::Column::WorkspaceId.eq(id))
                        .all(&s.db)
                        .await
                        .unwrap()
                        .is_empty()
                );
            } else {
                tx.commit().await.unwrap();
                workspaces.push(id);
            }
        }
        let scope = |workspace_id| Scope {
            workspace_id,
            user: User {
                id: 1.into(),
                email: "test@example.invalid".into(),
                name: "Test".into(),
                is_superadmin: false,
            },
            permissions: ["pipeline-templates.read".to_owned()].into_iter().collect(),
        };
        let list =
            |workspace_id, raw| pipelines::list(State(s.clone()), scope(workspace_id), Query(raw));
        let query = |kind: &str| {
            HashMap::from([
                ("workflow_kind".into(), kind.into()),
                ("page_size".into(), "1".into()),
            ])
        };
        let first = list(workspaces[0], query("actions")).await.unwrap().0;
        let second = list(workspaces[1], query("actions")).await.unwrap().0;
        assert_eq!(first["total"], 2);
        assert_eq!(second["total"], 2);
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
        let mut all_actions = query("actions");
        all_actions.insert("page_size".into(), "200".into());
        let actions = list(workspaces[0], all_actions).await.unwrap().0;
        let example = actions["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["content"].as_str().unwrap().contains("v0.28.0"))
            .unwrap();
        let native_build = actions["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["content"]
                    .as_str()
                    .unwrap()
                    .contains("Native HTTP artifact build")
            })
            .unwrap();
        let build = render_content(
            native_build["content"].as_str().unwrap(),
            "[]",
            &BTreeMap::new(),
            Kind::Pipeline,
        )
        .unwrap();
        let build: Value = serde_json::from_str(&build).unwrap();
        assert_eq!(build["executor_bindings"], json!([]));
        let build: Value = serde_yaml_ng::from_str(build["workflow"].as_str().unwrap()).unwrap();
        assert_eq!(build["on"], "workflow_dispatch");
        assert_eq!(build["jobs"]["build"]["steps"][1]["with"]["path"], "app");
        assert_eq!(example["parameters"], "[]");
        let content = example["content"].as_str().unwrap();
        let rendered = render_content(content, "[]", &BTreeMap::new(), Kind::Pipeline).unwrap();
        let envelope: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(envelope["executor_bindings"], json!([]));
        let workflow: Value =
            serde_yaml_ng::from_str(envelope["workflow"].as_str().unwrap()).unwrap();
        assert_eq!(workflow["on"], "workflow_dispatch");
        assert_eq!(workflow["jobs"]["idna"]["needs"], json!(["text"]));
        assert!(
            workflow["jobs"]["text"]["steps"][0]["run"]
                .as_str()
                .unwrap()
                .contains("v0.28.0")
        );
        let seeded = list(workspaces[0], query("nodes")).await.unwrap().0;
        assert_eq!(seeded["total"], 1);
        let graph = render_content(
            seeded["items"][0]["content"].as_str().unwrap(),
            "[]",
            &BTreeMap::new(),
            Kind::Pipeline,
        )
        .unwrap();
        let graph: Value = serde_json::from_str(&graph).unwrap();
        assert_eq!(graph["nodes"][1]["params"]["repo"], "text");
        assert!(graph.get("pin_data").is_none());
        assert!(graph.get("settings").is_none());
        let app_scope = |workspace_id| Scope {
            permissions: ["app-templates.read".into()].into_iter().collect(),
            ..scope(workspace_id)
        };
        let apps_a = apps::list(
            State(s.clone()),
            app_scope(workspaces[0]),
            Query(HashMap::new()),
        )
        .await
        .unwrap()
        .0;
        let apps_b = apps::list(
            State(s.clone()),
            app_scope(workspaces[1]),
            Query(HashMap::new()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(apps_a["total"], 3);
        assert_eq!(apps_b["total"], 3);
        assert_ne!(apps_a["items"][0]["id"], apps_b["items"][0]["id"]);
        let app = apps_a["items"].as_array().unwrap().iter().find(|item| {
            serde_json::from_str::<Value>(item["content"].as_str().unwrap()).unwrap()["metadata"]["name"] == "example-http"
        }).unwrap();
        let native = apps_a["items"].as_array().unwrap().iter().find(|item| {
            serde_json::from_str::<Value>(item["content"].as_str().unwrap()).unwrap()["metadata"]["name"] == "example-native-http"
        }).unwrap();
        let native_values = BTreeMap::from([
            ("pipeline".into(), json!("build-native")),
            ("port".into(), json!(23181)),
            ("message".into(), json!("literal '\" $HOME\n你好")),
        ]);
        let native_render = render_content(
            native["content"].as_str().unwrap(),
            native["parameters"].as_str().unwrap(),
            &native_values,
            Kind::App,
        )
        .unwrap();
        let native_doc: Value = serde_json::from_str(&native_render).unwrap();
        assert_eq!(native_doc["services"][0]["env"][0]["value"], "23181");
        assert_eq!(
            native_doc["services"][0]["env"][1]["value"],
            native_values["message"]
        );
        assert_eq!(native_doc["services"][0]["ports"][0]["container"], 23181);
        assert_eq!(
            native_doc["services"][0]["artifact"]["pipeline"],
            "build-native"
        );
        crate::deployments::manifest::parse_for(&native_render, "native").unwrap();
        let nats = apps_a["items"].as_array().unwrap().iter().find(|item| {
            serde_json::from_str::<Value>(item["content"].as_str().unwrap()).unwrap()["metadata"]["name"] == "example-nats"
        }).unwrap();
        let nats_values = BTreeMap::from([
            ("token".into(), json!("nats-client-token")),
            ("advertise_a".into(), json!("a.example.invalid:4222")),
            ("advertise_b".into(), json!("b.example.invalid:4223")),
            ("advertise_c".into(), json!("c.example.invalid:4224")),
        ]);
        let nats_text = render_content(
            nats["content"].as_str().unwrap(),
            nats["parameters"].as_str().unwrap(),
            &nats_values,
            Kind::App,
        )
        .unwrap();
        let nats_doc: Value = serde_json::from_str(&nats_text).unwrap();
        assert_eq!(nats_doc["services"].as_array().unwrap().len(), 3);
        for (index, suffix) in ["a", "b", "c"].iter().enumerate() {
            let service = &nats_doc["services"][index];
            assert_eq!(service["env"][2]["valueFrom"], "variable/nats-client-token");
            assert_eq!(service["volumes"][0]["name"], format!("nats-{suffix}-data"));
            assert_eq!(service["files"][0]["fromTemplate"], "example-nats-config");
            assert_eq!(
                service["health"]["readiness"]["httpGet"]["path"],
                "/healthz?js-enabled-only=true"
            );
            assert_eq!(
                service["health"]["liveness"]["httpGet"]["path"],
                "/healthz?js-enabled-only=true"
            );
        }
        for mode in ["compose", "stack"] {
            let manifest = crate::deployments::manifest::parse_for(&nats_text, mode).unwrap();
            let rendered = crate::deployments::manifest::render_for(&manifest, mode).unwrap();
            let rendered: Value = serde_json::from_str(&rendered).unwrap();
            assert_eq!(rendered["volumes"].as_object().unwrap().len(), 3);
            for suffix in ["a", "b", "c"] {
                let service = &rendered["services"][format!("nats-{suffix}")];
                if mode == "stack" {
                    assert_eq!(
                        service["deploy"]["placement"]["constraints"],
                        json!([format!("node.labels.ferry-nats == {suffix}")])
                    );
                } else {
                    assert!(service["deploy"].get("placement").is_none());
                }
            }
        }
        assert!(
            render_content(
                nats["content"].as_str().unwrap(),
                nats["parameters"].as_str().unwrap(),
                &BTreeMap::new(),
                Kind::App
            )
            .is_err()
        );
        let configs = configs::list(
            State(s.clone()),
            Scope {
                permissions: ["config-templates.read".into()].into_iter().collect(),
                ..scope(workspaces[0])
            },
            Query(HashMap::new()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(configs["total"], 1);
        assert_eq!(
            configs["items"][0]["content"],
            include_str!("templates/examples/nats-config.conf")
        );
        for invalid in [
            BTreeMap::new(),
            BTreeMap::from([("pipeline".into(), json!(""))]),
            BTreeMap::from([
                ("pipeline".into(), json!("build")),
                ("port".into(), json!(65536)),
            ]),
        ] {
            assert!(
                render_content(
                    native["content"].as_str().unwrap(),
                    native["parameters"].as_str().unwrap(),
                    &invalid,
                    Kind::App
                )
                .is_err()
            );
        }
        let app_id: Id = serde_json::from_value(app["id"].clone()).unwrap();
        assert!(
            apps::get(
                State(s.clone()),
                app_scope(workspaces[1]),
                Path((String::new(), app_id))
            )
            .await
            .is_err()
        );
        let values = BTreeMap::from([
            ("port".into(), json!(23180)),
            ("message".into(), json!("literal ' \" $HOME $(false)\n你好")),
        ]);
        let manifest = render_content(
            app["content"].as_str().unwrap(),
            app["parameters"].as_str().unwrap(),
            &values,
            Kind::App,
        )
        .unwrap();
        let manifest = crate::deployments::manifest::parse(&manifest).unwrap();
        assert_eq!(manifest.services[0].ports[0].published, Some(23180));
        // Template text must remain data: never interpolate it into a shell command.
        let doc: Value = serde_json::from_str(
            &render_content(
                app["content"].as_str().unwrap(),
                app["parameters"].as_str().unwrap(),
                &values,
                Kind::App,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(doc["services"][0]["env"][0]["value"], values["message"]);
        assert!(
            !doc["services"][0]["command"][2]
                .as_str()
                .unwrap()
                .contains("$(false)")
        );
        for port in [json!(0), json!(65536), json!("8080")] {
            let mut invalid = values.clone();
            invalid.insert("port".into(), port);
            assert!(
                render_content(
                    app["content"].as_str().unwrap(),
                    app["parameters"].as_str().unwrap(),
                    &invalid,
                    Kind::App
                )
                .is_err()
            );
        }
        crate::entities::app_templates::Entity::delete_by_id(i64::from(app_id))
            .exec(&s.db)
            .await
            .unwrap();
        assert_eq!(
            apps::list(
                State(s.clone()),
                app_scope(workspaces[0]),
                Query(HashMap::new())
            )
            .await
            .unwrap()
            .0["total"],
            2
        );
        assert_eq!(
            apps::list(
                State(s.clone()),
                app_scope(workspaces[1]),
                Query(HashMap::new())
            )
            .await
            .unwrap()
            .0["total"],
            3
        );
        // Include a canonical graph discriminator to check SQL filtering, count
        // and pagination alongside the seeded Actions template.
        rows::Entity::insert(rows::ActiveModel {
            id: Set(s.next_id().unwrap().into()),
            workspace_id: Set(workspaces[0].into()),
            name: Set("A node template".into()),
            description: Set(String::new()),
            content: Set("{\"engine\":\"ferry-graph\",\"schema\":1}".into()),
            parameters: Set("[]".into()),
            version: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        assert_eq!(
            list(workspaces[0], query("nodes")).await.unwrap().0["total"],
            2
        );
        assert_eq!(
            list(workspaces[0], query("actions")).await.unwrap().0["total"],
            2
        );
        assert_eq!(
            list(workspaces[0], HashMap::new()).await.unwrap().0["total"],
            4
        );
        assert_eq!(
            list(workspaces[1], query("nodes")).await.unwrap().0["total"],
            1
        );
        assert!(list(workspaces[0], query("invalid")).await.is_err());
        let mut invalid = query("actions");
        invalid.insert("unknown".into(), "x".into());
        assert!(list(workspaces[0], invalid).await.is_err());
        assert!(
            configs::list(
                State(s.clone()),
                Scope {
                    permissions: ["config-templates.read".into()].into_iter().collect(),
                    ..scope(workspaces[0])
                },
                Query(query("actions"))
            )
            .await
            .is_err()
        );
        // Templates remain ordinary workspace data: deleting one does not
        // re-create it on subsequent reads or affect another workspace.
        let example_id: Id = serde_json::from_value(example["id"].clone()).unwrap();
        rows::Entity::delete_by_id(i64::from(example_id))
            .exec(&s.db)
            .await
            .unwrap();
        assert_eq!(
            list(workspaces[0], query("actions")).await.unwrap().0["total"],
            1
        );
        assert_eq!(
            list(workspaces[1], query("actions")).await.unwrap().0["total"],
            2
        );
    }

    #[test]
    fn invalid_parameter_messages_keep_names_literal_for_all_readers() {
        let parameter: Parameter = serde_json::from_value(
            json!({"name":"feature.tag", "label":"用户 {name} 原文", "type":"integer"}),
        )
        .unwrap();
        let error = value_check(&parameter, &json!("wrong type")).unwrap_err();
        assert_eq!(error.2.as_ref().unwrap().key, "templates.parameterInvalid");
        for locale in crate::i18n::Locale::ALL {
            assert!(error.render(locale).contains("feature.tag"));
            assert_eq!(
                crate::i18n::render_stored(locale, &error.stored()),
                error.render(locale)
            );
            if locale != crate::i18n::Locale::EnUs {
                assert_ne!(error.render(locale), error.1);
            }
        }
        assert_eq!(parameter.label, "用户 {name} 原文");
    }
    fn application() -> Value {
        json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},"services":[{"name":"web","image":"nginx:{{ .Values.image.tag }}","ports":[{"container":"{{ .Values.port }}"}]}]})
    }
    #[test]
    fn upgrade_wraps_plain_actions_templates_and_converts_only_legacy_ones() {
        // A template saved as a plain Actions YAML workflow (as alpha.32 allowed).
        let plain = "name: Build\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo {{ .Values.command }}\n";
        let wrapped: Value = serde_json::from_str(&convert_legacy_content(plain).unwrap()).unwrap();
        assert_eq!(wrapped["engine"], crate::pipelines::actions::ENGINE);
        assert_eq!(wrapped["workflow"], plain);
        let again = convert_legacy_content(&wrapped.to_string()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&again).unwrap(), wrapped);
        let legacy = r#"{"workdir":"/tmp/{{run}}","steps":[{"name":"Build","run":"{{ .Values.command }}"}]}"#;
        let converted: Value =
            serde_json::from_str(&convert_legacy_content(legacy).unwrap()).unwrap();
        assert_eq!(converted["engine"], crate::pipelines::actions::ENGINE);
        assert!(
            converted["workflow"]
                .as_str()
                .unwrap()
                .contains("{{ .Values.command }}")
        );
    }
    #[test]
    fn actions_expressions_survive_template_validation_and_rendering() {
        let workflow = r#"concurrency: ${{ github.ref }}
jobs:
  build:
    runs-on: self-hosted
    if: ${{ vars.ENABLED == 'yes' }}
    env:
      TOKEN: ${{ secrets.TOKEN }}
      QUOTED: ${{ format('it''s }} {{ .Values.not_a_parameter }}', vars.NAME) }}
    steps:
      - run: echo {{ .Values.message }} ${{ secrets[inputs.key] }}
"#;
        let content =
            json!({"engine":crate::pipelines::actions::ENGINE, "workflow":workflow}).to_string();
        let parameters = r#"[{"name":"message","type":"string","default":"hello"}]"#;
        validate_template(&content, parameters, Kind::Pipeline).unwrap();
        let rendered =
            render_content(&content, parameters, &BTreeMap::new(), Kind::Pipeline).unwrap();
        let document = crate::pipelines::actions::document(&rendered).unwrap();
        let expected: Value =
            serde_yaml_ng::from_str(&workflow.replace("{{ .Values.message }}", "hello")).unwrap();
        let actual: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
        assert_eq!(actual, expected);
        assert!(document.workflow.contains("{{ .Values.not_a_parameter }}"));
        assert_eq!(
            crate::pipelines::actions::references(&document)
                .unwrap()
                .secrets
                .names,
            BTreeSet::from(["TOKEN".into()])
        );
        assert!(
            crate::pipelines::actions::references(&document)
                .unwrap()
                .secrets
                .deliver_all
        );
        let unknown = content.replace("{{ .Values.message }}", "{{ unsupported }}");
        assert!(validate_template(&unknown, "[]", Kind::Pipeline).is_err());
    }

    #[test]
    fn actions_secret_parameters_render_scoped_references_in_environment_values() {
        let workflow = r#"env:
  GLOBAL_TOKEN: '{{ .Values.token }}'
jobs:
  build:
    runs-on: self-hosted
    env:
      JOB_TOKEN: '{{ .Values.token }}'
      EXISTING: ${{ secrets.EXISTING }}
    steps:
      - run: echo ok
        env:
          STEP_TOKEN: '{{ .Values.token }}'
"#;
        let content =
            json!({"engine":crate::pipelines::actions::ENGINE,"workflow":workflow}).to_string();
        let parameters = r#"[{"name":"token","type":"string","secret":true}]"#;
        let values = BTreeMap::from([("token".into(), json!("db-token"))]);
        let rendered = render_content(&content, parameters, &values, Kind::Pipeline).unwrap();
        let document = crate::pipelines::actions::document(&rendered).unwrap();
        let workflow: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
        for value in [
            &workflow["env"]["GLOBAL_TOKEN"],
            &workflow["jobs"]["build"]["env"]["JOB_TOKEN"],
            &workflow["jobs"]["build"]["steps"][0]["env"]["STEP_TOKEN"],
        ] {
            assert_eq!(value, "${{ secrets['db-token'] }}");
        }
        assert_eq!(
            workflow["jobs"]["build"]["env"]["EXISTING"],
            "${{ secrets.EXISTING }}"
        );
        let references = crate::pipelines::actions::references(&document).unwrap();
        assert_eq!(
            references.secrets.names,
            BTreeSet::from(["db-token".into(), "EXISTING".into()])
        );
        for workflow in [
            "jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: '{{ .Values.token }}'\n",
            "env:\n  TOKEN: 'prefix {{ .Values.token }}'\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo ok\n",
        ] {
            let content =
                json!({"engine":crate::pipelines::actions::ENGINE,"workflow":workflow}).to_string();
            assert!(validate_template(&content, parameters, Kind::Pipeline).is_err());
        }
        assert!(
            render_content(
                &content,
                parameters,
                &BTreeMap::from([("token".into(), json!("bad' }} injected"))]),
                Kind::Pipeline
            )
            .is_err()
        );
    }
    #[test]
    fn converted_artifact_parameter_names_follow_the_rendered_logical_path() {
        let legacy = json!({"workdir":"/tmp/build", "steps":[{"name":"build","run":"true"}],
            "artifacts":["{{ .Values.artifact }}"]})
        .to_string();
        let content = convert_legacy_content(&legacy).unwrap();
        let rendered = render_content(
            &content,
            r#"[{"name":"artifact","type":"string"}]"#,
            &BTreeMap::from([("artifact".into(), json!("bin/app"))]),
            Kind::Pipeline,
        )
        .unwrap();
        let document = crate::pipelines::actions::document(&rendered).unwrap();
        let workflow: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
        let upload = workflow["jobs"]["build"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["uses"] == "actions/upload-artifact@v4")
            .unwrap();
        assert_eq!(upload["name"], "Archive bin/app");
        assert_eq!(
            upload["with"]["name"],
            crate::pipelines::artifacts::legacy_artifact_name("bin/app")
        );
        assert_eq!(upload["with"]["path"], "/tmp/build/bin/app");
    }
    #[test]
    fn converted_timeout_parameters_keep_seconds_in_other_fields() {
        let content = json!({
            "workdir":"/tmp/build",
            "steps":[{"name":"build","run":"echo {{ .Values.timeout }}",
                "timeout_seconds":"{{ .Values.timeout }}"}]
        })
        .to_string();
        let parameters = r#"[{"name":"timeout","type":"integer","default":61}]"#;
        let converted = convert_legacy_content(&content).unwrap();
        assert_eq!(convert_legacy_content(&converted).unwrap(), converted);
        assert!(converted.contains("ceilMinutes .Values.timeout"));
        for (seconds, minutes) in [(1, 1), (60, 1), (61, 2), (86400, 1440)] {
            let rendered = render_content(
                &converted,
                parameters,
                &BTreeMap::from([("timeout".into(), json!(seconds))]),
                Kind::Pipeline,
            )
            .unwrap();
            let document = crate::pipelines::actions::document(&rendered).unwrap();
            let workflow: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
            let step = workflow["jobs"]["build"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["name"] == "build")
                .unwrap();
            assert_eq!(step["timeout-minutes"], minutes);
            assert_eq!(step["run"], format!("echo {seconds}"));
            assert_eq!(workflow["jobs"]["build"]["timeout-minutes"], minutes + 30);
        }
        for seconds in [0, -1, 86401] {
            assert!(
                render_content(
                    &converted,
                    parameters,
                    &BTreeMap::from([("timeout".into(), json!(seconds))]),
                    Kind::Pipeline
                )
                .is_err()
            );
        }
        let rendered =
            render_content(&converted, parameters, &BTreeMap::new(), Kind::Pipeline).unwrap();
        let document = crate::pipelines::actions::document(&rendered).unwrap();
        let workflow: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
        assert_eq!(
            workflow["jobs"]["build"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["name"] == "build")
                .unwrap()["timeout-minutes"],
            2
        );
    }
    #[test]
    fn typed_expansion_defaults_and_json_escaping() {
        let defs=json!([{"name":"image.tag","type":"string","default":"alpine","enum":["alpine","latest"]},{"name":"port","type":"integer","default":8080,"min":1,"max":65535}]).to_string();
        let text = render_content(
            &application().to_string(),
            &defs,
            &BTreeMap::new(),
            Kind::App,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["services"][0]["image"], "nginx:alpine");
        assert_eq!(value["services"][0]["ports"][0]["container"], 8080);
        assert!(
            render_content(
                &application().to_string(),
                &defs,
                &BTreeMap::from([("port".into(), json!("8080"))]),
                Kind::App
            )
            .is_err()
        );
        assert!(
            render_content(
                &application().to_string(),
                &defs,
                &BTreeMap::from([("port".into(), json!(65536))]),
                Kind::App
            )
            .is_err()
        );
        assert!(
            render_content(
                &application().to_string(),
                &defs,
                &BTreeMap::from([("other".into(), json!(1))]),
                Kind::App
            )
            .is_err()
        );
        let pipeline=json!({"workdir":"/tmp/build/{{run}}","steps":[{"name":"step","run":"{{ .Values.command }}"}]}).to_string();
        let command = "printf 'quote\" newline\n dollar$'; echo {{branch}}";
        let rendered = render_content(
            &pipeline,
            r#"[{"name":"command","type":"string"}]"#,
            &BTreeMap::from([("command".into(), json!(command))]),
            Kind::Pipeline,
        )
        .unwrap();
        let document = crate::pipelines::actions::document(&rendered).unwrap();
        let value: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
        let setup = &value["jobs"]["build"]["steps"][0];
        assert_eq!(setup["id"], "prepare_workspace");
        assert_eq!(setup["working-directory"], "${{ github.workspace }}");
        assert_eq!(
            setup["env"]["FERRY_LEGACY_WORKDIR"],
            "/tmp/build/${{ github.run_number }}"
        );
        assert_eq!(
            setup["run"],
            "[ -d \"$FERRY_LEGACY_WORKDIR\" ] || mkdir -p -- \"$FERRY_LEGACY_WORKDIR\""
        );
        assert_eq!(
            value["jobs"]["build"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["name"] == "step")
                .unwrap()["run"],
            command.replace("{{branch}}", "${{ github.ref_name }}")
        );
        assert_eq!(
            value["jobs"]["build"]["defaults"]["run"]["working-directory"],
            "/tmp/build/${{ github.run_number }}"
        );
    }
    #[test]
    fn declarations_missing_values_and_unsupported_expressions_are_refused() {
        let doc =
            r#"{"workdir":"/tmp/test","steps":[{"name":"test","run":"{{ .Values.command }}"}]}"#;
        let defs = r#"[{"name":"command","type":"string","min_length":1}]"#;
        assert!(render_content(doc, defs, &BTreeMap::new(), Kind::Pipeline).is_err());
        for invalid in [
            r#"[{"name":"command","type":"string","default":null}]"#,
            r#"[{"name":"command","type":"string","default":"" ,"min_length":1}]"#,
            r#"[{"name":"command","type":"string"},{"name":"command","type":"string"}]"#,
            r#"[{"name":"unused","type":"string"}]"#,
        ] {
            assert!(validate_template(doc, invalid, Kind::Pipeline).is_err());
        }
        for token in [
            "{{ .Values.missing }}",
            "{{ include file }}",
            "{{ .Values.command",
            "{{ run }}",
        ] {
            assert!(
                validate_template(
                    &doc.replace("{{ .Values.command }}", token),
                    defs,
                    Kind::Pipeline
                )
                .is_err()
            );
        }
        let invalid_business =
            r#"{"workdir":"relative","steps":[{"name":"test","run":"{{ .Values.command }}"}]}"#;
        assert!(
            render_content(
                invalid_business,
                defs,
                &BTreeMap::from([("command".into(), json!("echo ok"))]),
                Kind::Pipeline
            )
            .is_err()
        );
    }
    #[test]
    fn secret_parameters_only_generate_variable_references_in_environment() {
        let mut doc = json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},"services":[{"name":"web","image":"nginx:alpine","env":[{"name":"TOKEN","valueFrom":"{{ .Values.token }}"}]}]});
        let defs = r#"[{"name":"token","type":"string","secret":true}]"#;
        let values = BTreeMap::from([("token".into(), json!("db-token"))]);
        let rendered = render_content(&doc.to_string(), defs, &values, Kind::App).unwrap();
        let value: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(
            value["services"][0]["env"][0]["valueFrom"],
            "variable/db-token"
        );
        assert!(
            render_content(
                &doc.to_string(),
                defs,
                &BTreeMap::from([("token".into(), json!("secret with spaces"))]),
                Kind::App
            )
            .is_err()
        );
        doc["services"][0]["env"][0] = json!({"name":"TOKEN","value":"{{ .Values.token }}"});
        assert!(validate_template(&doc.to_string(), defs, Kind::App).is_err());
        doc["services"][0]["env"] = json!([]);
        doc["metadata"]["labels"] = json!({"valueFrom":"{{ .Values.token }}"});
        assert!(validate_template(&doc.to_string(), defs, Kind::App).is_err());
        let pipeline = json!({"workdir":"/tmp/{{run}}","env":[{"name":"TOKEN","valueFrom":"{{ .Values.token }}"}],"steps":[{"name":"build","run":"true"}]});
        let rendered =
            render_content(&pipeline.to_string(), defs, &values, Kind::Pipeline).unwrap();
        let document = crate::pipelines::actions::document(&rendered).unwrap();
        let value: Value = serde_yaml_ng::from_str(&document.workflow).unwrap();
        assert_eq!(
            value["jobs"]["build"]["env"]["TOKEN"],
            "${{ secrets['db-token'] }}"
        );
    }
    #[test]
    fn config_secrets_are_references_and_literals_cannot_forge_markers() {
        let params = r#"[{"name":"password","type":"string","secret":true},{"name":"port","type":"integer","default":8080}]"#;
        let output = config_content(
            "port={{ .Values.port }}\npassword={{ .Values.password }}",
            params,
            &BTreeMap::from([("password".into(), json!("db-password"))]),
        )
        .unwrap();
        assert_eq!(output, "port=8080\npassword=\0variable/db-password\0");
        assert_eq!(
            config_preview(&output),
            "port=8080\npassword=<variable/db-password>"
        );
        assert!(config_content("bad\0text", "[]", &BTreeMap::new()).is_err());
        assert!(
            config_content(
                "{{ .Values.password }}",
                params,
                &BTreeMap::from([("password".into(), json!("bad\0value"))])
            )
            .is_err()
        );
    }
    #[test]
    fn boolean_defaults_and_no_recursive_expansion() {
        let defs = parameters(
            r#"[{"name":"flag","type":"boolean","default":false},{"name":"text","type":"string"}]"#,
            Kind::App,
        )
        .unwrap();
        let values = BTreeMap::from([
            ("flag".into(), json!(false)),
            ("text".into(), json!("{{ .Values.flag }}")),
        ]);
        let out = expand(
            &json!({"flag":"{{ .Values.flag }}","text":"{{ .Values.text }}"}),
            &defs,
            Some(&values),
            Kind::App,
            "",
            &mut BTreeSet::new(),
            0,
        )
        .unwrap();
        assert_eq!(out["flag"], false);
        assert_eq!(out["text"], "{{ .Values.flag }}");
    }
}
