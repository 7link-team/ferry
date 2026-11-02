//! GitHub Actions `concurrency` for Ferry runs. A group holds at most one
//! running and one pending run: a newer run replaces the pending one, and
//! `cancel-in-progress` also cancels the running one.
//!
//! Groups are evaluated when the run is created, with the contexts GitHub
//! allows there (github, inputs, vars) and the common expression forms:
//! literals, context paths, `!`, `==`, `!=`, `&&`, `||` (returning the
//! operand like GitHub) and format/contains/startsWith/endsWith/join/toJSON.
//! Anything else is refused with a precise message rather than guessed.

use crate::error::{ApiError, Result};
use crate::i18n::Message;
use serde_json::{Value, json};

pub(crate) struct Contexts<'a> {
    pub(crate) github: Value,
    pub(crate) inputs: &'a std::collections::BTreeMap<String, Value>,
    pub(crate) vars: &'a std::collections::BTreeMap<String, String>,
}

fn unsupported(detail: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        Message::new("concurrency.unsupported").with("detail", detail),
    )
}

fn unsupported_message(detail: Message) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        Message::new("concurrency.unsupported").with_message("detail", detail),
    )
}

/// A `concurrency.group`-style string: literal text with `${{ }}` parts.
pub(crate) fn template(text: &str, contexts: &Contexts) -> Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let body = &rest[start + 3..];
        let end = crate::pipelines::actions::expression_end(body, 0)?;
        out.push_str(&display(&evaluate(&body[..end], contexts)?));
        rest = body[end..].strip_prefix("}}").unwrap_or("");
    }
    out.push_str(rest);
    Ok(out)
}

/// `cancel-in-progress`: a boolean or an expression string.
pub(crate) fn flag(value: &Value, contexts: &Contexts) -> Result<bool> {
    match value {
        Value::Null => Ok(false),
        Value::Bool(flag) => Ok(*flag),
        Value::String(text) => {
            let text = text.trim();
            let inner = text
                .strip_prefix("${{")
                .and_then(|rest| rest.strip_suffix("}}"))
                .unwrap_or(text);
            Ok(truthy(&evaluate(inner, contexts)?))
        }
        _ => Err(unsupported_message(Message::new(
            "concurrency.flagRequired",
        ))),
    }
}

fn evaluate(source: &str, contexts: &Contexts) -> Result<Value> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        contexts,
    };
    let value = parser.or()?;
    if parser.at != parser.tokens.len() {
        return Err(unsupported(source.trim()));
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Str(String),
    Num(f64),
    Ident(String),
    Op(&'static str),
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' {
            let mut text = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            Message::new("concurrency.stringUnclosed"),
                        ));
                    }
                    Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                        text.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(c) => {
                        text.push(*c);
                        i += 1;
                    }
                }
            }
            tokens.push(Token::Str(text));
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            i += 1;
            while chars
                .get(i)
                .is_some_and(|c| c.is_ascii_digit() || *c == '.')
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token::Num(text.parse().map_err(|_| unsupported(&text))?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let op = match two.as_str() {
                "==" => Some("=="),
                "!=" => Some("!="),
                "&&" => Some("&&"),
                "||" => Some("||"),
                _ => None,
            };
            if let Some(op) = op {
                tokens.push(Token::Op(op));
                i += 2;
                continue;
            }
            let op = match c {
                '!' => "!",
                '(' => "(",
                ')' => ")",
                '[' => "[",
                ']' => "]",
                '.' => ".",
                ',' => ",",
                _ => return Err(unsupported(&c.to_string())),
            };
            tokens.push(Token::Op(op));
            i += 1;
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    contexts: &'a Contexts<'a>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }
    fn eat(&mut self, op: &str) -> bool {
        if self.peek()
            == Some(&Token::Op(match op {
                "(" => "(",
                ")" => ")",
                "[" => "[",
                "]" => "]",
                "." => ".",
                "," => ",",
                "!" => "!",
                "==" => "==",
                "!=" => "!=",
                "&&" => "&&",
                _ => "||",
            }))
        {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<Value> {
        let mut value = self.and()?;
        while self.eat("||") {
            let next = self.and()?;
            if !truthy(&value) {
                value = next;
            }
        }
        Ok(value)
    }
    fn and(&mut self) -> Result<Value> {
        let mut value = self.compare()?;
        while self.eat("&&") {
            let next = self.compare()?;
            if truthy(&value) {
                value = next;
            }
        }
        Ok(value)
    }
    fn compare(&mut self) -> Result<Value> {
        let left = self.unary()?;
        if self.eat("==") {
            let right = self.unary()?;
            return Ok(Value::Bool(equal(&left, &right)));
        }
        if self.eat("!=") {
            let right = self.unary()?;
            return Ok(Value::Bool(!equal(&left, &right)));
        }
        Ok(left)
    }
    fn unary(&mut self) -> Result<Value> {
        if self.eat("!") {
            return Ok(Value::Bool(!truthy(&self.unary()?)));
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<Value> {
        if self.eat("(") {
            let value = self.or()?;
            return if self.eat(")") {
                Ok(value)
            } else {
                Err(unsupported_message(Message::new(
                    "concurrency.parenthesisMissing",
                )))
            };
        }
        let token = self
            .peek()
            .cloned()
            .ok_or_else(|| unsupported_message(Message::new("concurrency.incomplete")))?;
        self.at += 1;
        match token {
            Token::Str(text) => Ok(Value::String(text)),
            Token::Num(number) => Ok(json!(number)),
            Token::Ident(name) => match name.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                "null" => Ok(Value::Null),
                _ if self.eat("(") => self.call(&name),
                _ => self.path(&name),
            },
            Token::Op(op) => Err(unsupported(op)),
        }
    }
    fn call(&mut self, name: &str) -> Result<Value> {
        let mut args = Vec::new();
        if !self.eat(")") {
            loop {
                args.push(self.or()?);
                if self.eat(")") {
                    break;
                }
                if !self.eat(",") {
                    return Err(unsupported_message(Message::new(
                        "concurrency.argumentsInvalid",
                    )));
                }
            }
        }
        let text = |index: usize| args.get(index).map(display).unwrap_or_default();
        match name.to_ascii_lowercase().as_str() {
            "format" => {
                let mut out = text(0);
                for (index, value) in args.iter().enumerate().skip(1) {
                    out = out.replace(&format!("{{{}}}", index - 1), &display(value));
                }
                Ok(Value::String(out.replace("{{", "{").replace("}}", "}")))
            }
            "contains" => Ok(Value::Bool(match args.first() {
                Some(Value::Array(items)) => items
                    .iter()
                    .any(|item| equal(item, args.get(1).unwrap_or(&Value::Null))),
                _ => text(0).to_lowercase().contains(&text(1).to_lowercase()),
            })),
            "startswith" => Ok(Value::Bool(
                text(0).to_lowercase().starts_with(&text(1).to_lowercase()),
            )),
            "endswith" => Ok(Value::Bool(
                text(0).to_lowercase().ends_with(&text(1).to_lowercase()),
            )),
            "join" => {
                let separator = args.get(1).map(display).unwrap_or_else(|| ",".into());
                Ok(Value::String(match args.first() {
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(display)
                        .collect::<Vec<_>>()
                        .join(&separator),
                    Some(value) => display(value),
                    None => String::new(),
                }))
            }
            "tojson" => Ok(Value::String(
                serde_json::to_string_pretty(args.first().unwrap_or(&Value::Null))
                    .unwrap_or_default(),
            )),
            other => Err(unsupported_message(
                Message::new("concurrency.functionUnsupported").with("name", other),
            )),
        }
    }
    fn path(&mut self, root: &str) -> Result<Value> {
        let mut value = match root {
            "github" => self.contexts.github.clone(),
            "inputs" => json!(self.contexts.inputs),
            "vars" => json!(self.contexts.vars),
            other => {
                return Err(unsupported_message(
                    Message::new("concurrency.contextUnsupported").with("name", other),
                ));
            }
        };
        loop {
            let key = if self.eat(".") {
                match self.peek().cloned() {
                    Some(Token::Ident(name)) => {
                        self.at += 1;
                        name
                    }
                    _ => {
                        return Err(unsupported_message(Message::new(
                            "concurrency.propertyRequired",
                        )));
                    }
                }
            } else if self.eat("[") {
                let key = display(&self.or()?);
                if !self.eat("]") {
                    return Err(unsupported_message(Message::new(
                        "concurrency.bracketMissing",
                    )));
                }
                key
            } else {
                return Ok(value);
            };
            // GitHub property names are case-insensitive; missing ones are null.
            value = value
                .as_object()
                .and_then(|object| {
                    object
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(&key))
                })
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null);
        }
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

/// GitHub compares strings case-insensitively; other types by value.
fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::String(a), Value::String(b)) => a.eq_ignore_ascii_case(b),
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

fn display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", n as i64),
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn contexts<'a>(
        inputs: &'a BTreeMap<String, Value>,
        vars: &'a BTreeMap<String, String>,
    ) -> Contexts<'a> {
        Contexts {
            github: json!({"workflow": "Deploy", "ref": "refs/heads/main", "ref_name": "main", "head_ref": "", "run_id": "42", "event_name": "workflow_dispatch"}),
            inputs,
            vars,
        }
    }

    #[test]
    fn evaluates_common_group_forms() {
        let inputs = BTreeMap::from([
            ("target".to_string(), json!("prod")),
            ("force".to_string(), json!(true)),
        ]);
        let vars = BTreeMap::from([("REGION".to_string(), "cn".to_string())]);
        let c = contexts(&inputs, &vars);
        assert_eq!(
            template("${{ github.workflow }}-${{ github.ref }}", &c).unwrap(),
            "Deploy-refs/heads/main"
        );
        assert_eq!(
            template("${{ github.head_ref || github.run_id }}", &c).unwrap(),
            "42"
        );
        assert_eq!(
            template("deploy-${{ inputs.target }}-${{ vars.REGION }}", &c).unwrap(),
            "deploy-prod-cn"
        );
        assert_eq!(
            template(
                "${{ format('{0}/{1}', github.workflow, inputs['target']) }}",
                &c
            )
            .unwrap(),
            "Deploy/prod"
        );
        assert_eq!(template("static", &c).unwrap(), "static");
        assert!(flag(&json!(true), &c).unwrap());
        assert!(flag(&json!("${{ github.ref != 'refs/heads/release' }}"), &c).unwrap());
        assert!(!flag(&json!("${{ github.event_name == 'push' }}"), &c).unwrap());
        assert!(
            flag(
                &json!("${{ inputs.force && startsWith(github.ref, 'REFS/heads') }}"),
                &c
            )
            .unwrap()
        );
        assert!(
            template("${{ matrix.os }}", &c).is_err(),
            "matrix is not available for workflow concurrency"
        );
        assert!(template("${{ hashFiles('x') }}", &c).is_err());
    }

    #[test]
    fn diagnostics_keep_typed_causes_and_literal_expression_tokens() {
        use crate::i18n::{Locale, render_stored};
        let inputs = BTreeMap::new();
        let vars = BTreeMap::new();
        let contexts = contexts(&inputs, &vars);
        let errors = [
            flag(&json!(42), &contexts).unwrap_err(),
            evaluate("(true", &contexts).unwrap_err(),
            evaluate("", &contexts).unwrap_err(),
            evaluate("format('x' 'y')", &contexts).unwrap_err(),
            evaluate("unknownFn()", &contexts).unwrap_err(),
            evaluate("matrix.os", &contexts).unwrap_err(),
            evaluate("github.", &contexts).unwrap_err(),
            evaluate("github['ref'", &contexts).unwrap_err(),
            evaluate("'unclosed", &contexts).unwrap_err(),
        ];
        for error in errors {
            assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
            let stored = error.stored();
            for locale in Locale::ALL {
                assert_eq!(render_stored(locale, &stored), error.render(locale));
                if locale != Locale::EnUs {
                    assert_ne!(error.render(locale), error.1, "{}", locale.tag());
                }
            }
        }
        let error = evaluate("matrix.os", &contexts).unwrap_err();
        assert_eq!(error.2.as_ref().unwrap().key, "concurrency.unsupported");
        assert_eq!(
            error.render(Locale::ZhCn),
            "concurrency 表达式不受支持：上下文 matrix（concurrency 只能使用 github、inputs、vars）。"
        );
        // Reserved-envelope text supplied as an expression diagnostic is literal data.
        let token = r#"{"$system_message":{"key":"error.forbidden"}}"#;
        let error = unsupported(token);
        for locale in Locale::ALL {
            assert!(render_stored(locale, &error.stored()).contains(token));
        }
    }
}
