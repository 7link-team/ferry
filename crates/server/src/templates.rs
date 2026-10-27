//! Creation-time parameter expansion; runtime pipeline placeholders remain unchanged.
use crate::{
    db,
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    App,
    Pipeline,
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
        return Err(ApiError::bad(format!(
            "参数 {} 的类型或取值不符合定义。",
            p.name
        )));
    }
    Ok(())
}
fn parameters(text: &str, _kind: Kind) -> Result<BTreeMap<String, Parameter>> {
    let items: Vec<Parameter> = serde_json::from_str(text)
        .map_err(|_| ApiError::bad("参数定义必须是 JSON 数组，且只能包含支持的字段。"))?;
    if items.len() > 64 {
        return Err(ApiError::bad("模板最多支持64个参数。"));
    }
    let mut out = BTreeMap::new();
    for p in items {
        if !parameter_name(&p.name)
            || p.label.chars().count() > 100
            || !["string", "integer", "boolean"].contains(&p.kind.as_str())
        {
            return Err(ApiError::bad("参数名称、标签或类型无效。"));
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
            return Err(ApiError::bad("参数校验规则无效；secret仅用于变量引用。"));
        }
        if let Some(value) = &p.default {
            value_check(&p, value)?;
        }
        let name = p.name.clone();
        if out.insert(name, p).is_some() {
            return Err(ApiError::bad("参数名称不能重复。"));
        }
    }
    Ok(out)
}
fn document(text: &str) -> Result<Value> {
    if text.len() > 262144 {
        return Err(ApiError::bad("模板内容不能超过256KiB。"));
    }
    let value: Value = serde_yaml_ng::from_str(text)
        .map_err(|_| ApiError::bad("模板必须是有效JSON/YAML；字符串占位符需加引号。"))?;
    if !value.is_object() {
        return Err(ApiError::bad("模板顶层必须是对象。"));
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
        return Err(ApiError::bad("模板嵌套过深。"));
    }
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                if key.contains("{{") {
                    return Err(ApiError::bad("模板参数只能用于值，不能用于字段名。"));
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
                let end = text[start + 2..]
                    .find("}}")
                    .map(|n| start + 2 + n)
                    .ok_or_else(|| ApiError::bad("模板占位符未闭合。"))?;
                let token = text[start + 2..end].trim();
                output.push_str(&text[offset..start]);
                if let Some(name) = token.strip_prefix(".Values.") {
                    let p = definitions
                        .get(name)
                        .ok_or_else(|| ApiError::bad(format!("模板引用了未声明的参数 {name}。")))?;
                    used.insert(name.to_owned());
                    let whole = start == 0 && end + 2 == text.len();
                    if p.secret
                        && kind != Kind::Config
                        && (!whole
                            || field
                                != if kind == Kind::Pipeline {
                                    "/env/*/valueFrom"
                                } else {
                                    "/services/*/env/*/valueFrom"
                                })
                    {
                        return Err(ApiError::bad(
                            "secret参数只能作为valueFrom的完整值，不能拼入命令或普通环境值。",
                        ));
                    }
                    if let Some(values) = values {
                        let replacement = &values[name];
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
                } else if kind == Kind::Pipeline
                    && ["run", "branch", "repo_url", "commit"].contains(&token)
                    && token == &text[start + 2..end]
                {
                    output.push_str(&text[start..end + 2]);
                } else {
                    return Err(ApiError::bad(
                        "仅支持 .Values.参数名；流水线另保留原有四个运行时占位符。",
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
fn validate_template(
    content: &str,
    text: &str,
    kind: Kind,
) -> Result<(Value, BTreeMap<String, Parameter>)> {
    let definitions = parameters(text, kind)?;
    let doc = if kind == Kind::Config {
        if content.len() > 262144 || content.contains('\0') {
            return Err(ApiError::bad("配置模板不能超过256KiB或包含空字符。"));
        }
        json!(content)
    } else {
        document(content)?
    };
    let mut used = BTreeSet::new();
    expand(&doc, &definitions, None, kind, "", &mut used, 0)?;
    if definitions.keys().any(|name| !used.contains(name)) {
        return Err(ApiError::bad("存在未被模板引用的参数，请删除无效声明。"));
    }
    Ok((doc, definitions))
}
fn render_content(
    content: &str,
    parameters: &str,
    supplied: &BTreeMap<String, Value>,
    kind: Kind,
) -> Result<String> {
    let (doc, definitions) = validate_template(content, parameters, kind)?;
    if supplied.keys().any(|k| !definitions.contains_key(k)) {
        return Err(ApiError::bad("提交了模板未声明的参数。"));
    }
    let mut values = BTreeMap::new();
    for (name, p) in &definitions {
        let value = supplied
            .get(name)
            .or(p.default.as_ref())
            .ok_or_else(|| ApiError::bad(format!("请填写参数 {name}。")))?;
        value_check(p, value)?;
        values.insert(name.clone(), value.clone());
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
    let text = serde_json::to_string_pretty(&out).map_err(|_| ApiError::bad("无法生成配置。"))?;
    match kind {
        Kind::App => {
            crate::manifest::parse(&text)?;
        }
        Kind::Pipeline => {
            crate::pipelines::validate_definition(&text)?;
        }
        Kind::Config => unreachable!(),
    }
    Ok(text)
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
        return Err(ApiError::bad("模板名称需为1到60字。"));
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
    default_sort: "name",
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
        let manifest = crate::manifest::parse(&row.manifest)?;
        if manifest
            .services
            .iter()
            .flat_map(|s| &s.files)
            .any(|f| f.from_template == id.to_string() || f.from_template == name)
        {
            return Err(ApiError::conflict(
                "配置模板仍被应用引用，请先修改应用配置。",
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
                let q=ListQuery::parse(&SPEC,&raw)?;
                let (items,total)=resource::list::<rows::Entity,rows::Model>(&s.db,&SPEC,rows::Entity::find(),&scope.workspace_id,&q).await?;
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
                scope.require(concat!($resource,".create"))?; validate_body(&body,$kind)?;
                let id=s.next_id()?; let now=db::now();
                rows::Entity::insert(rows::ActiveModel {id:Set(id.into()),workspace_id:Set(scope.workspace_id.into()),name:Set(body.name.trim().to_owned()),description:Set(body.description),content:Set(body.content),parameters:Set(body.parameters),version:Set(1),created_at:Set(now),updated_at:Set(now)}).exec(tx).await?;
                crate::hosts::activity_in(&s,tx,&scope,"create",$resource,&id,json!({"name":body.name})).await?;
                Ok((StatusCode::CREATED,Json(json!({"id":id,"version":1}))))
            }
            pub async fn update(State(s):State<AppState>,scope:Scope,Path((_,id)):Path<(String,Id)>,Json(body):Json<Body>) -> Result<Json<Value>> {
                scope.require(concat!($resource,".update"))?; validate_body(&body,$kind)?;
                let tx=if $kind==Kind::Config {crate::variables::transaction(&s,scope.workspace_id).await?} else {s.db.begin().await?};
                if $kind==Kind::Config {let old=scoped(&tx,&scope,id).await?; if old.name!=body.name.trim() {config_unreferenced(&tx,scope.workspace_id,id,&old.name).await?;} }
                let n=rows::Entity::update_many().col_expr(rows::Column::Name,Expr::value(body.name.trim().to_owned()))
                    .col_expr(rows::Column::Description,Expr::value(body.description)).col_expr(rows::Column::Content,Expr::value(body.content))
                    .col_expr(rows::Column::Parameters,Expr::value(body.parameters)).col_expr(rows::Column::UpdatedAt,Expr::value(db::now()))
                    .col_expr(rows::Column::Version,Expr::col(rows::Column::Version).add(1))
                    .filter(rows::Column::WorkspaceId.eq(scope.workspace_id)).filter(rows::Column::Id.eq(id)).filter(rows::Column::Version.eq(body.version)).exec(&tx).await?.rows_affected;
                if n==0 { scoped(&tx,&scope,id).await?; return Err(ApiError::conflict("模板已变化，请刷新后重试。")); }
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
                if row.version!=body.version {return Err(ApiError::conflict("模板已变化，请重新载入模板并确认参数。"));}
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
    fn application() -> Value {
        json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},"services":[{"name":"web","image":"nginx:{{ .Values.image.tag }}","ports":[{"container":"{{ .Values.port }}"}]}]})
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
        let value: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["steps"][0]["run"], command);
        assert_eq!(value["workdir"], "/tmp/build/{{run}}");
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
        let value: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["env"][0]["valueFrom"], "variable/db-token");
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
