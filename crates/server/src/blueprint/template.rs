//! Restricted Go-template subset for blueprint role templates (design §6.2).
//!
//! Supported actions: `.Values.x`, `.Instance.Name|Index|Role|Host`,
//! `.Instance.Port "p"`, `.Deployment.Driver`, `add A B` (integers),
//! `range .Role "r"` (members expose `.Name .Index .Host .Port "p"`) and
//! `if X` / `else` / `end`. Blocks and parentheses nest at most 8 levels.
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Substitute only plain `.Values.*` actions; every other action is copied verbatim.
    Values,
    /// Evaluate everything; any `{{` left in the output is an error.
    Instance,
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    pub values: BTreeMap<String, Value>,
    pub instance: Option<Instance>,
    pub members: BTreeMap<String, Vec<Member>>,
    pub driver: String,
}

#[derive(Clone, Debug, Default)]
pub struct Instance {
    pub role: String,
    pub index: u32,
    pub name: String,
    pub host: String,
    pub ports: BTreeMap<String, u16>,
}

#[derive(Clone, Debug, Default)]
pub struct Member {
    pub name: String,
    pub index: u32,
    pub host: String,
    pub ports: BTreeMap<String, u16>,
}

/// Syntax check: balanced blocks, nesting depth and the expression whitelist.
pub fn check(text: &str) -> Result<()> {
    parse(text).map(|_| ())
}

pub fn render_text(text: &str, ctx: &Context, phase: Phase) -> Result<String> {
    let nodes = parse(text)?;
    let mut out = String::new();
    render(&nodes, ctx, phase, None, &mut out)?;
    if phase == Phase::Instance {
        unresolved(&out)?;
    }
    Ok(out)
}

/// 整串是单个表达式时返回类型值（端口为整数），否则返回字符串。
pub fn render_value(text: &str, ctx: &Context, phase: Phase) -> Result<Value> {
    let nodes = parse(text)?;
    let mut significant = nodes
        .iter()
        .filter(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
    if let (Some(Node::Output { expr, .. }), None) = (significant.next(), significant.next()) {
        if phase == Phase::Values && !matches!(expr, Expr::Values(_)) {
            return Ok(Value::String(text.to_owned()));
        }
        let value = eval(expr, ctx, None)?;
        unresolved(&stringify(&value))?;
        return Ok(value);
    }
    render_text(text, ctx, phase).map(Value::String)
}

/// True when the text holds any action that `Phase::Values` leaves for `Phase::Instance`
/// (invalid text also counts, so callers defer it and surface the error later).
pub fn has_instance_tokens(text: &str) -> bool {
    match parse(text) {
        Ok(nodes) => nodes.iter().any(|n| match n {
            Node::Text(_) => false,
            Node::Output { expr, .. } => !matches!(expr, Expr::Values(_)),
            _ => true,
        }),
        Err(_) => true,
    }
}

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

fn syntax<T>() -> Result<T> {
    fail(Message::new("blueprint.templateSyntax"))
}

fn unresolved(text: &str) -> Result<()> {
    if text.contains("{{") {
        return fail(Message::new("blueprint.templateUnresolved"));
    }
    Ok(())
}

// ---------------------------------------------------------------- lexing

#[derive(Debug, PartialEq)]
enum Token {
    Field(String),
    Str(String),
    Int(i64),
    Ident(String),
    Open,
    Close,
}

enum Segment<'a> {
    Text(&'a str),
    Action { raw: &'a str, inner: &'a str },
}

fn segments(text: &str) -> Result<Vec<Segment<'_>>> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(relative) = text[offset..].find("{{") {
        let start = offset + relative;
        let Some(len) = text[start + 2..].find("}}") else {
            return syntax();
        };
        let end = start + 2 + len;
        let inner = &text[start + 2..end];
        // Whitespace trim markers `{{-` / `-}}` are not part of the subset.
        if inner.starts_with('-') || inner.ends_with('-') {
            return syntax();
        }
        if start > offset {
            out.push(Segment::Text(&text[offset..start]));
        }
        out.push(Segment::Action {
            raw: &text[start..end + 2],
            inner,
        });
        offset = end + 2;
    }
    if offset < text.len() {
        out.push(Segment::Text(&text[offset..]));
    }
    Ok(out)
}

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn tokens(inner: &str) -> Result<Vec<Token>> {
    let bytes = inner.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'(' => {
                out.push(Token::Open);
                i += 1;
            }
            b')' => {
                out.push(Token::Close);
                i += 1;
            }
            b'.' => {
                i += 1;
                while i < bytes.len() && (word_byte(bytes[i]) || bytes[i] == b'.') {
                    i += 1;
                }
                if i == start + 1 {
                    return syntax();
                }
                out.push(Token::Field(inner[start + 1..i].to_owned()));
            }
            b'"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return syntax(),
                        Some(b'"') => break,
                        Some(b'\\') => {
                            match bytes.get(i + 1) {
                                Some(b'"') => s.push('"'),
                                Some(b'\\') => s.push('\\'),
                                _ => return syntax(),
                            }
                            i += 2;
                        }
                        Some(_) => {
                            let ch = inner[i..].chars().next().unwrap_or_default();
                            s.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                i += 1;
                out.push(Token::Str(s));
            }
            b'-' | b'0'..=b'9' => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let Ok(n) = inner[start..i].parse::<i64>() else {
                    return syntax();
                };
                out.push(Token::Int(n));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < bytes.len() && word_byte(bytes[i]) {
                    i += 1;
                }
                out.push(Token::Ident(inner[start..i].to_owned()));
            }
            _ => return syntax(),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- parsing

#[derive(Clone, Copy, Debug)]
enum Field {
    Name,
    Index,
    Role,
    Host,
}

#[derive(Debug)]
enum Expr {
    Int(i64),
    Values(String),
    Instance(Field),
    InstancePort(String),
    Member(Field),
    MemberPort(String),
    Driver,
    Add(Box<Expr>, Box<Expr>),
}

/// Block nodes keep their raw tags so `Phase::Values` can copy them byte-for-byte.
enum Node<'a> {
    Text(&'a str),
    Output {
        raw: &'a str,
        expr: Expr,
    },
    Range {
        open: &'a str,
        role: String,
        body: Vec<Node<'a>>,
        end: &'a str,
    },
    If {
        open: &'a str,
        cond: Expr,
        then: Vec<Node<'a>>,
        otherwise: Option<(&'a str, Vec<Node<'a>>)>,
        end: &'a str,
    },
}

enum Action {
    Range(String),
    If(Expr),
    Else,
    End,
    Output(Expr),
}

enum Stop<'a> {
    Eof,
    Else(&'a str),
    End(&'a str),
}

fn parse(text: &str) -> Result<Vec<Node<'_>>> {
    let mut iter = segments(text)?.into_iter();
    match block(&mut iter, 0, false)? {
        (nodes, Stop::Eof) => Ok(nodes),
        _ => syntax(),
    }
}

fn enter(depth: usize) -> Result<usize> {
    if depth >= MAX_DEPTH {
        return fail(Message::new("blueprint.templateDepth"));
    }
    Ok(depth + 1)
}

fn block<'a>(
    iter: &mut impl Iterator<Item = Segment<'a>>,
    depth: usize,
    in_range: bool,
) -> Result<(Vec<Node<'a>>, Stop<'a>)> {
    let mut nodes = Vec::new();
    while let Some(segment) = iter.next() {
        let (raw, inner) = match segment {
            Segment::Text(t) => {
                nodes.push(Node::Text(t));
                continue;
            }
            Segment::Action { raw, inner } => (raw, inner),
        };
        match action(&tokens(inner)?, in_range)? {
            Action::Output(expr) => nodes.push(Node::Output { raw, expr }),
            Action::Else => return Ok((nodes, Stop::Else(raw))),
            Action::End => return Ok((nodes, Stop::End(raw))),
            Action::Range(role) => {
                let (body, stop) = block(iter, enter(depth)?, true)?;
                let Stop::End(end) = stop else {
                    return syntax();
                };
                nodes.push(Node::Range {
                    open: raw,
                    role,
                    body,
                    end,
                });
            }
            Action::If(cond) => {
                let inner_depth = enter(depth)?;
                let (then, stop) = block(iter, inner_depth, in_range)?;
                let (otherwise, end) = match stop {
                    Stop::End(end) => (None, end),
                    Stop::Else(else_raw) => match block(iter, inner_depth, in_range)? {
                        (other, Stop::End(end)) => (Some((else_raw, other)), end),
                        _ => return syntax(),
                    },
                    Stop::Eof => return syntax(),
                };
                nodes.push(Node::If {
                    open: raw,
                    cond,
                    then,
                    otherwise,
                    end,
                });
            }
        }
    }
    Ok((nodes, Stop::Eof))
}

struct Cursor<'t> {
    tokens: &'t [Token],
    pos: usize,
    in_range: bool,
}

impl Cursor<'_> {
    fn next(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.pos);
        self.pos += 1;
        token
    }

    fn string(&mut self) -> Result<String> {
        match self.next() {
            Some(Token::Str(s)) => Ok(s.clone()),
            _ => syntax(),
        }
    }
}

fn action(tokens: &[Token], in_range: bool) -> Result<Action> {
    let mut cur = Cursor {
        tokens,
        pos: 1,
        in_range,
    };
    let keyword = match tokens.first() {
        Some(Token::Ident(word)) => word.as_str(),
        _ => "",
    };
    let action = match keyword {
        "range" => {
            if cur.next() != Some(&Token::Field("Role".into())) {
                return syntax();
            }
            Action::Range(cur.string()?)
        }
        "if" => Action::If(expr(&mut cur, 0)?),
        "else" => Action::Else,
        "end" => Action::End,
        _ => {
            cur.pos = 0;
            Action::Output(expr(&mut cur, 0)?)
        }
    };
    if cur.pos < tokens.len() {
        return syntax();
    }
    Ok(action)
}

fn expr(cur: &mut Cursor, depth: usize) -> Result<Expr> {
    if let Some(Token::Ident(name)) = cur.tokens.get(cur.pos) {
        if name != "add" {
            return fail(
                Message::new("blueprint.templateFunctionUnknown").with("name", name.as_str()),
            );
        }
        cur.pos += 1;
        let a = argument(cur, depth)?;
        let b = argument(cur, depth)?;
        return Ok(Expr::Add(Box::new(a), Box::new(b)));
    }
    argument(cur, depth)
}

fn argument(cur: &mut Cursor, depth: usize) -> Result<Expr> {
    match cur.next() {
        Some(Token::Int(n)) => Ok(Expr::Int(*n)),
        Some(Token::Open) => {
            let inner = expr(cur, enter(depth)?)?;
            match cur.next() {
                Some(Token::Close) => Ok(inner),
                _ => syntax(),
            }
        }
        Some(Token::Field(path)) => {
            let path = path.clone();
            field(cur, &path)
        }
        Some(Token::Ident(name)) if name != "add" => {
            fail(Message::new("blueprint.templateFunctionUnknown").with("name", name.as_str()))
        }
        _ => syntax(),
    }
}

fn field(cur: &mut Cursor, path: &str) -> Result<Expr> {
    if let Some(name) = path.strip_prefix("Values.")
        && name.split('.').all(|s| !s.is_empty())
    {
        return Ok(Expr::Values(name.to_owned()));
    }
    let member = |f| cur.in_range.then_some(Expr::Member(f));
    let found = match path {
        "Instance.Name" => Some(Expr::Instance(Field::Name)),
        "Instance.Index" => Some(Expr::Instance(Field::Index)),
        "Instance.Role" => Some(Expr::Instance(Field::Role)),
        "Instance.Host" => Some(Expr::Instance(Field::Host)),
        "Deployment.Driver" => Some(Expr::Driver),
        "Name" => member(Field::Name),
        "Index" => member(Field::Index),
        "Host" => member(Field::Host),
        "Instance.Port" => return Ok(Expr::InstancePort(cur.string()?)),
        "Port" if cur.in_range => return Ok(Expr::MemberPort(cur.string()?)),
        _ => None,
    };
    found.map_or_else(
        || fail(Message::new("blueprint.templateFieldUnknown").with("field", format!(".{path}"))),
        Ok,
    )
}

// ---------------------------------------------------------------- rendering

fn render(
    nodes: &[Node],
    ctx: &Context,
    phase: Phase,
    member: Option<&Member>,
    out: &mut String,
) -> Result<()> {
    for node in nodes {
        match (node, phase) {
            (Node::Text(t), _) => out.push_str(t),
            (
                Node::Output {
                    expr: e @ Expr::Values(_),
                    ..
                },
                Phase::Values,
            ) => {
                let text = stringify(&eval(e, ctx, member)?);
                // Phase-1 output is re-parsed later; a value must not inject actions.
                unresolved(&text)?;
                out.push_str(&text);
            }
            (Node::Output { raw, .. }, Phase::Values) => out.push_str(raw),
            (Node::Output { expr, .. }, Phase::Instance) => {
                out.push_str(&stringify(&eval(expr, ctx, member)?))
            }
            (
                Node::Range {
                    open, body, end, ..
                },
                Phase::Values,
            ) => {
                out.push_str(open);
                render(body, ctx, phase, member, out)?;
                out.push_str(end);
            }
            (Node::Range { role, body, .. }, Phase::Instance) => {
                let Some(members) = ctx.members.get(role) else {
                    return fail(
                        Message::new("blueprint.templateRoleUnknown").with("role", role.as_str()),
                    );
                };
                for m in members {
                    render(body, ctx, phase, Some(m), out)?;
                }
            }
            (
                Node::If {
                    open,
                    then,
                    otherwise,
                    end,
                    ..
                },
                Phase::Values,
            ) => {
                out.push_str(open);
                render(then, ctx, phase, member, out)?;
                if let Some((else_raw, other)) = otherwise {
                    out.push_str(else_raw);
                    render(other, ctx, phase, member, out)?;
                }
                out.push_str(end);
            }
            (
                Node::If {
                    cond,
                    then,
                    otherwise,
                    ..
                },
                Phase::Instance,
            ) => {
                if truthy(&eval(cond, ctx, member)?) {
                    render(then, ctx, phase, member, out)?;
                } else if let Some((_, other)) = otherwise {
                    render(other, ctx, phase, member, out)?;
                }
            }
        }
    }
    Ok(())
}

fn port(ports: &BTreeMap<String, u16>, name: &str) -> Result<Value> {
    match ports.get(name) {
        Some(p) => Ok(json!(p)),
        None => fail(Message::new("blueprint.templatePortUnknown").with("port", name)),
    }
}

fn eval(expr: &Expr, ctx: &Context, member: Option<&Member>) -> Result<Value> {
    let instance = || {
        ctx.instance
            .as_ref()
            .map_or_else(|| fail(Message::new("blueprint.templateUnresolved")), Ok)
    };
    // The parser admits member fields only inside `range`, where a member is always set.
    let current = || member.map_or_else(syntax, Ok);
    Ok(match expr {
        Expr::Int(n) => json!(n),
        Expr::Values(path) => match lookup(&ctx.values, path) {
            Some(v) => v.clone(),
            None => {
                return fail(
                    Message::new("blueprint.templateValueMissing").with("name", path.as_str()),
                );
            }
        },
        Expr::Instance(f) => {
            let i = instance()?;
            match f {
                Field::Name => json!(i.name),
                Field::Index => json!(i.index),
                Field::Role => json!(i.role),
                Field::Host => json!(i.host),
            }
        }
        Expr::InstancePort(name) => port(&instance()?.ports, name)?,
        Expr::Member(f) => {
            let m = current()?;
            match f {
                Field::Name => json!(m.name),
                Field::Index => json!(m.index),
                Field::Host => json!(m.host),
                Field::Role => return syntax(),
            }
        }
        Expr::MemberPort(name) => port(&current()?.ports, name)?,
        Expr::Driver => json!(ctx.driver),
        Expr::Add(a, b) => {
            let a = eval(a, ctx, member)?.as_i64();
            let b = eval(b, ctx, member)?.as_i64();
            match (a, b) {
                (Some(a), Some(b)) => match a.checked_add(b) {
                    Some(sum) => json!(sum),
                    None => return fail(Message::new("blueprint.templateIntegerRequired")),
                },
                _ => return fail(Message::new("blueprint.templateIntegerRequired")),
            }
        }
    })
}

/// Full dotted name first (parameter names may contain dots), then nested objects.
fn lookup<'v>(values: &'v BTreeMap<String, Value>, path: &str) -> Option<&'v Value> {
    values.get(path).or_else(|| {
        path.match_indices('.')
            .find_map(|(i, _)| nested(values.get(&path[..i])?, &path[i + 1..]))
    })
}

fn nested<'v>(value: &'v Value, path: &str) -> Option<&'v Value> {
    let map = value.as_object()?;
    map.get(path).or_else(|| {
        path.match_indices('.')
            .find_map(|(i, _)| nested(map.get(&path[..i])?, &path[i + 1..]))
    })
}

/// Text form of a value, matching `templates.rs`: strings verbatim, others as JSON.
fn stringify(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// Truthiness: `true`, non-zero numbers, non-empty strings, lists and objects.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn member(i: u32) -> Member {
        Member {
            name: format!("nats-{i}"),
            index: i,
            host: format!("10.0.0.{}", i + 1),
            ports: BTreeMap::from([("cluster".into(), 6222), ("client".into(), 4222)]),
        }
    }

    fn ctx() -> Context {
        Context {
            values: BTreeMap::from([
                ("client_port".into(), json!(4222)),
                ("image.tag".into(), json!("2.10")),
                ("cfg".into(), json!({ "mode": "ha", "replicas": 2 })),
                ("name".into(), json!("demo")),
                ("empty".into(), json!("")),
                ("tags".into(), json!(["a"])),
                ("injected".into(), json!("{{ .Instance.Name }}")),
            ]),
            instance: Some(Instance {
                role: "redis".into(),
                index: 1,
                name: "redis-1".into(),
                host: "10.0.0.2".into(),
                ports: BTreeMap::from([("data".into(), 6379), ("bus".into(), 16379)]),
            }),
            members: BTreeMap::from([("nats".into(), (0..3).map(member).collect())]),
            driver: "native".into(),
        }
    }

    fn at_index(index: u32) -> Context {
        let mut c = ctx();
        c.instance.as_mut().unwrap().index = index;
        c
    }

    fn key(e: crate::error::ApiError) -> String {
        e.2.map(|m| m.key).unwrap_or_default()
    }

    fn text_err(text: &str, phase: Phase) -> String {
        key(render_text(text, &ctx(), phase).expect_err("expected error"))
    }

    #[test]
    fn values_phase_keeps_instance_tokens() {
        let text = "{{ .Values.name }}-{{  .Instance.Name }}:{{.Instance.Port \"data\"}} \
                    {{ range .Role \"nats\" }}{{ .Host }}/{{ .Values.name }}{{ end }} \
                    {{ if .Instance.Index }}x{{else}}y{{ end }} {{ add .Values.client_port 1 }} {{ .Deployment.Driver }}";
        let out = render_text(text, &Context::default(), Phase::Values).unwrap_err();
        assert_eq!(key(out), "blueprint.templateValueMissing");
        let out = render_text(text, &ctx(), Phase::Values).unwrap();
        assert_eq!(
            out,
            "demo-{{  .Instance.Name }}:{{.Instance.Port \"data\"}} \
             {{ range .Role \"nats\" }}{{ .Host }}/demo{{ end }} \
             {{ if .Instance.Index }}x{{else}}y{{ end }} {{ add .Values.client_port 1 }} {{ .Deployment.Driver }}"
        );
        assert!(has_instance_tokens(text));
        assert!(has_instance_tokens("{{ add .Values.client_port 1 }}"));
        assert!(!has_instance_tokens(
            "plain {{ .Values.name }} and {{ .Values.image.tag }}"
        ));
        assert!(!has_instance_tokens("no actions"));
        // Phase-1 output re-renders to the same result as a one-shot instance render.
        let full = render_text(text, &ctx(), Phase::Instance).unwrap();
        assert_eq!(render_text(&out, &ctx(), Phase::Instance).unwrap(), full);
        assert_eq!(
            full,
            "demo-redis-1:6379 10.0.0.1/demo10.0.0.2/demo10.0.0.3/demo x 4223 native"
        );
    }

    #[test]
    fn values_lookup_prefers_dotted_key_then_nested() {
        let c = ctx();
        assert_eq!(
            render_text("{{ .Values.image.tag }}", &c, Phase::Values).unwrap(),
            "2.10"
        );
        assert_eq!(
            render_text("{{ .Values.cfg.mode }}", &c, Phase::Values).unwrap(),
            "ha"
        );
        assert_eq!(
            render_value("{{ .Values.cfg.replicas }}", &c, Phase::Values).unwrap(),
            json!(2)
        );
        assert_eq!(
            render_value(" {{ .Values.cfg }} ", &c, Phase::Values).unwrap(),
            json!({ "mode": "ha", "replicas": 2 })
        );
        assert_eq!(
            text_err("{{ .Values.cfg.nope }}", Phase::Values),
            "blueprint.templateValueMissing"
        );
    }

    #[test]
    fn nats_routes_render_with_range() {
        let text = "{{ range .Role \"nats\" }}{{ if .Index }},{{ end }}nats://{{ .Host }}:{{ .Port \"cluster\" }}{{ end }}";
        check(text).unwrap();
        assert_eq!(
            render_text(text, &ctx(), Phase::Instance).unwrap(),
            "nats://10.0.0.1:6222,nats://10.0.0.2:6222,nats://10.0.0.3:6222"
        );
        // Values phase leaves the whole block untouched.
        assert_eq!(render_text(text, &ctx(), Phase::Values).unwrap(), text);
        // Nested ranges see the inner member; outer context stays reachable.
        let nested = "{{ range .Role \"nats\" }}{{ range .Role \"nats\" }}{{ .Index }}{{ end }}|{{ .Instance.Name }};{{ end }}";
        assert_eq!(
            render_text(nested, &ctx(), Phase::Instance).unwrap(),
            "012|redis-1;012|redis-1;012|redis-1;"
        );
    }

    #[test]
    fn if_else_by_index() {
        let text = "{{ if .Instance.Index }}replica{{ else }}master{{ end }}";
        assert_eq!(
            render_text(text, &at_index(0), Phase::Instance).unwrap(),
            "master"
        );
        assert_eq!(
            render_text(text, &at_index(2), Phase::Instance).unwrap(),
            "replica"
        );
        let c = ctx();
        for (cond, want) in [
            ("{{ if .Values.empty }}t{{ else }}f{{ end }}", "f"),
            ("{{ if .Values.name }}t{{ else }}f{{ end }}", "t"),
            ("{{ if .Values.tags }}t{{ end }}", "t"),
            (
                "{{ if (add .Instance.Index -1) }}t{{ else }}f{{ end }}",
                "f",
            ),
            (
                "{{ if .Deployment.Driver }}{{ if .Values.empty }}a{{ else }}b{{ end }}{{ end }}",
                "b",
            ),
        ] {
            assert_eq!(
                render_text(cond, &c, Phase::Instance).unwrap(),
                want,
                "{cond}"
            );
        }
    }

    #[test]
    fn whole_port_expression_is_typed() {
        let c = ctx();
        assert_eq!(
            render_value("{{ .Instance.Port \"data\" }}", &c, Phase::Instance).unwrap(),
            json!(6379)
        );
        assert_eq!(
            render_value("  {{ .Instance.Index }}\n", &c, Phase::Instance).unwrap(),
            json!(1)
        );
        assert_eq!(
            render_value("{{ .Instance.Name }}", &c, Phase::Instance).unwrap(),
            json!("redis-1")
        );
        assert_eq!(
            render_value("{{ .Values.client_port }}", &c, Phase::Values).unwrap(),
            json!(4222)
        );
        assert_eq!(
            render_value("port {{ .Instance.Port \"data\" }}", &c, Phase::Instance).unwrap(),
            json!("port 6379")
        );
        // Values phase cannot type an instance expression; the text is kept verbatim.
        assert_eq!(
            render_value("{{ .Instance.Port \"data\" }}", &c, Phase::Values).unwrap(),
            json!("{{ .Instance.Port \"data\" }}")
        );
    }

    #[test]
    fn add_offsets_port() {
        let c = ctx();
        assert_eq!(
            render_value(
                "{{ add (.Instance.Port \"data\") 10000 }}",
                &c,
                Phase::Instance
            )
            .unwrap(),
            json!(16379)
        );
        assert_eq!(
            render_value(
                "{{ add .Instance.Port \"data\" 10000 }}",
                &c,
                Phase::Instance
            )
            .unwrap(),
            json!(16379)
        );
        assert_eq!(
            render_value(
                "{{ add .Values.client_port .Instance.Index }}",
                &c,
                Phase::Instance
            )
            .unwrap(),
            json!(4223)
        );
        assert_eq!(
            render_text(
                "{{ range .Role \"nats\" }}{{ add .Port \"cluster\" .Index }} {{ end }}",
                &c,
                Phase::Instance
            )
            .unwrap(),
            "6222 6223 6224 "
        );
        assert_eq!(
            render_value("{{ add (add 1 2) 3 }}", &c, Phase::Instance).unwrap(),
            json!(6)
        );
        assert_eq!(
            text_err("{{ add .Values.name 1 }}", Phase::Instance),
            "blueprint.templateIntegerRequired"
        );
        assert_eq!(
            text_err("{{ add 9223372036854775807 1 }}", Phase::Instance),
            "blueprint.templateIntegerRequired"
        );
    }

    #[test]
    fn unbalanced_or_deep_blocks_rejected() {
        let syntax = "blueprint.templateSyntax";
        for bad in [
            "{{ if .Instance.Index }}x",
            "{{ range .Role \"nats\" }}x",
            "x{{ end }}",
            "{{ else }}",
            "{{ if .Instance.Index }}a{{ else }}b{{ else }}c{{ end }}",
            "{{ range .Role \"nats\" }}a{{ else }}b{{ end }}",
            "{{ .Values.name ",
            "{{- .Values.name }}",
            "{{ .Values.name -}}",
            "{{ }}",
            "{{ .Values.name .Values.name }}",
            "{{ range .Values.name }}{{ end }}",
            "{{ add 1 }}",
            "{{ .Instance.Port }}",
            "{{ \"lit\" }}",
            "{{ add (1 2 }}",
        ] {
            assert_eq!(key(check(bad).unwrap_err()), syntax, "{bad}");
        }
        let nest = |n: usize| "{{ if .Instance.Index }}".repeat(n) + &"{{ end }}".repeat(n);
        check(&nest(8)).unwrap();
        assert_eq!(key(check(&nest(9)).unwrap_err()), "blueprint.templateDepth");
        let ranges = "{{ range .Role \"nats\" }}".repeat(9) + &"{{ end }}".repeat(9);
        assert_eq!(key(check(&ranges).unwrap_err()), "blueprint.templateDepth");
        let parens = format!("{{{{ add {}1{} 1 }}}}", "(add 1 ".repeat(9), ")".repeat(9));
        assert_eq!(key(check(&parens).unwrap_err()), "blueprint.templateDepth");
    }

    #[test]
    fn unknown_function_rejected() {
        for bad in [
            "{{ join .Values.tags \",\" }}",
            "{{ printf \"%d\" 1 }}",
            "{{ if default .Values.x }}{{ end }}",
        ] {
            assert_eq!(
                key(check(bad).unwrap_err()),
                "blueprint.templateFunctionUnknown",
                "{bad}"
            );
        }
        for bad in [
            "{{ .Name }}",
            "{{ .Port \"cluster\" }}",
            "{{ .Instance.Foo }}",
            "{{ .Role \"nats\" }}",
            "{{ .Values }}",
            "{{ .Values..x }}",
            "{{ .Deployment.Name }}",
        ] {
            assert_eq!(
                key(check(bad).unwrap_err()),
                "blueprint.templateFieldUnknown",
                "{bad}"
            );
        }
    }

    #[test]
    fn unresolved_after_instance_phase_rejected() {
        assert_eq!(
            text_err("{{ .Values.injected }}", Phase::Instance),
            "blueprint.templateUnresolved"
        );
        assert_eq!(
            text_err("a{{ .Values.injected }}", Phase::Values),
            "blueprint.templateUnresolved"
        );
        let mut c = ctx();
        c.instance = None;
        let e = render_text("{{ .Instance.Name }}", &c, Phase::Instance).unwrap_err();
        assert_eq!(key(e), "blueprint.templateUnresolved");
        let e = render_value("{{ .Values.injected }}", &ctx(), Phase::Instance).unwrap_err();
        assert_eq!(key(e), "blueprint.templateUnresolved");
    }

    #[test]
    fn unknown_role_or_port_rejected() {
        assert_eq!(
            text_err("{{ range .Role \"redis\" }}{{ end }}", Phase::Instance),
            "blueprint.templateRoleUnknown"
        );
        assert_eq!(
            text_err(
                "{{ range .Role \"nats\" }}{{ .Port \"gossip\" }}{{ end }}",
                Phase::Instance
            ),
            "blueprint.templatePortUnknown"
        );
        assert_eq!(
            text_err("{{ .Instance.Port \"gossip\" }}", Phase::Instance),
            "blueprint.templatePortUnknown"
        );
        // Values phase does not evaluate instance data, so unknown names surface only later.
        assert!(
            render_text(
                "{{ range .Role \"redis\" }}{{ end }}",
                &ctx(),
                Phase::Values
            )
            .is_ok()
        );
    }
}
