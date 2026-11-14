//! Custom upstream configuration for one execution, written into an isolated
//! configuration home. The API key never touches disk: every generated file
//! references `KEY_ENV`, which only the child process receives.
use anyhow::{Context, Result, bail, ensure};
use ferry_protocol::agent::{Provider, Runtime};
use serde_json::{Map, Value, json};
use std::path::Path;

pub(super) const KEY_ENV: &str = "FERRY_PROVIDER_API_KEY";
const PROVIDER_ID: &str = "ferry";

/// What a prepared provider adds to the native command.
#[derive(Default, Debug)]
pub(super) struct Launch {
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    /// Model passed through the native protocol; None when the generated
    /// configuration already selects it under a CLI-specific alias.
    pub model: Option<String>,
}

pub(super) fn validate(runtime: Runtime, provider: &Provider) -> Result<()> {
    ensure!(
        runtime
            .provider_formats()
            .contains(&provider.format.as_str()),
        "this CLI cannot use the {} format directly; Ferry does not convert API formats",
        provider.format
    );
    ensure!(
        !provider.key.is_empty()
            && provider.key.len() <= 32
            && provider.key.bytes().all(|b| b.is_ascii_digit()),
        "invalid provider key"
    );
    let url = provider.base_url.as_str();
    ensure!(
        (url.starts_with("https://") || url.starts_with("http://"))
            && url.len() <= 512
            && !url.chars().any(|c| c.is_control() || c.is_whitespace()),
        "invalid provider address"
    );
    ensure!(
        !provider.api_key.is_empty()
            && provider.api_key.len() <= 16384
            && !provider.api_key.chars().any(char::is_control),
        "invalid provider key"
    );
    ensure!(provider.models.len() <= 2000, "too many models");
    for model in &provider.models {
        text(model).context("invalid model")?;
    }
    ensure!(
        provider.options.is_null() || provider.options.is_object(),
        "invalid provider options"
    );
    Ok(())
}

fn text(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty()
            && value.len() <= 256
            && !value.starts_with('-')
            && !value.chars().any(char::is_control),
        "invalid value"
    );
    Ok(value)
}

fn option<'a>(provider: &'a Provider, key: &str) -> Option<&'a str> {
    provider.options[key]
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
}
fn flag(provider: &Provider, key: &str) -> bool {
    provider.options[key].as_bool().unwrap_or(false)
}
fn number(provider: &Provider, key: &str) -> Option<u64> {
    provider.options[key].as_u64().filter(|n| *n > 0)
}
fn headers(provider: &Provider) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    if let Some(map) = provider.options["headers"].as_object() {
        for (name, value) in map {
            let value = value.as_str().context("invalid header")?;
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                    && value.len() <= 4096
                    && !value.chars().any(char::is_control),
                "invalid header"
            );
            out.insert(name.clone(), json!(value));
        }
    }
    Ok(out)
}

/// TOML basic strings accept JSON string escapes.
fn toml_str(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

async fn write(home: &Path, relative: &str, contents: String) -> Result<()> {
    let path = home.join(relative);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, contents)
        .await
        .with_context(|| format!("cannot write {relative}"))
}

/// Writes the CLI's native configuration under `home` and returns what the
/// command needs. `requested` is the model chosen for this execution.
pub(super) async fn prepare(
    home: &Path,
    runtime: Runtime,
    provider: &Provider,
    requested: Option<&str>,
) -> Result<Launch> {
    validate(runtime, provider)?;
    let model = requested
        .map(str::to_owned)
        .or_else(|| option(provider, "default_model").map(str::to_owned))
        .or_else(|| provider.models.first().cloned())
        .context("a custom API needs a model")?;
    text(&model).context("invalid model")?;
    let base = provider.base_url.trim_end_matches('/').to_owned();
    let headers = headers(provider)?;
    let key = provider.api_key.clone();
    let mut launch = Launch::default();
    let mut catalog: Vec<String> = provider.models.clone();
    if !catalog.contains(&model) {
        catalog.insert(0, model.clone());
    }
    match runtime {
        Runtime::Claude => {
            let field = match option(provider, "auth_field") {
                None | Some("ANTHROPIC_AUTH_TOKEN") => "ANTHROPIC_AUTH_TOKEN",
                Some("ANTHROPIC_API_KEY") => "ANTHROPIC_API_KEY",
                Some(_) => bail!("invalid Claude authentication field"),
            };
            // Claude appends /v1/messages itself.
            let root = base.strip_suffix("/v1").unwrap_or(&base).to_owned();
            launch.env.push(("ANTHROPIC_BASE_URL".into(), root));
            launch.env.push((field.into(), key));
            launch.env.push(("ANTHROPIC_MODEL".into(), model.clone()));
            for (option_key, env) in [
                ("sonnet_model", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
                ("opus_model", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
                ("haiku_model", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
                ("fable_model", "ANTHROPIC_DEFAULT_FABLE_MODEL"),
                ("subagent_model", "CLAUDE_CODE_SUBAGENT_MODEL"),
            ] {
                if let Some(value) = option(provider, option_key) {
                    launch.env.push((env.into(), text(value)?.into()));
                }
            }
            launch.env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
            if flag(provider, "tool_search") {
                launch
                    .env
                    .push(("ENABLE_TOOL_SEARCH".into(), "true".into()));
            }
            if !headers.is_empty() {
                let lines: Vec<String> = headers
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or_default()))
                    .collect();
                launch
                    .env
                    .push(("ANTHROPIC_CUSTOM_HEADERS".into(), lines.join("\n")));
            }
            let mut settings = json!({});
            if flag(provider, "hide_attribution") {
                settings["includeCoAuthoredBy"] = json!(false);
                settings["attribution"] = json!({"commit":"","pr":""});
            }
            write(home, ".claude/settings.json", settings.to_string()).await?;
            launch.model = Some(model);
        }
        Runtime::Codex => {
            let mut toml = format!(
                "model_provider = \"{PROVIDER_ID}\"\nmodel = {}\n",
                toml_str(&model)
            );
            if let Some(effort) = option(provider, "reasoning_effort") {
                ensure!(
                    ["minimal", "low", "medium", "high", "xhigh"].contains(&effort),
                    "invalid reasoning effort"
                );
                toml.push_str(&format!("model_reasoning_effort = \"{effort}\"\n"));
            }
            if let Some(n) = number(provider, "context_window") {
                toml.push_str(&format!("model_context_window = {n}\n"));
            }
            if let Some(n) = number(provider, "auto_compact_limit") {
                toml.push_str(&format!("model_auto_compact_token_limit = {n}\n"));
            }
            toml.push_str(&format!(
                "\n[model_providers.{PROVIDER_ID}]\nname = \"Ferry\"\nbase_url = {}\nwire_api = \"responses\"\nenv_key = \"{KEY_ENV}\"\nrequires_openai_auth = false\n",
                toml_str(&base)
            ));
            if !headers.is_empty() {
                let pairs: Vec<String> = headers
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{} = {}",
                            toml_str(k),
                            toml_str(v.as_str().unwrap_or_default())
                        )
                    })
                    .collect();
                toml.push_str(&format!("http_headers = {{ {} }}\n", pairs.join(", ")));
            }
            let subagent = option(provider, "subagent_model");
            let subagent_effort = option(provider, "subagent_reasoning_effort");
            if subagent.is_some() || subagent_effort.is_some() {
                toml.push_str("\n[agents]\n");
                if let Some(m) = subagent {
                    toml.push_str(&format!(
                        "default_subagent_model = {}\n",
                        toml_str(text(m)?)
                    ));
                }
                if let Some(e) = subagent_effort {
                    ensure!(
                        ["minimal", "low", "medium", "high", "xhigh"].contains(&e),
                        "invalid reasoning effort"
                    );
                    toml.push_str(&format!("default_subagent_reasoning_effort = \"{e}\"\n"));
                }
            }
            write(home, ".codex/config.toml", toml).await?;
            launch.env.push((KEY_ENV.into(), key));
            launch.model = Some(model);
        }
        Runtime::Gemini => {
            write(
                home,
                ".gemini/settings.json",
                json!({"security":{"auth":{"selectedType":"gemini-api-key"}},"model":{"name":model}})
                    .to_string(),
            )
            .await?;
            launch.env.push(("GEMINI_API_KEY".into(), key));
            // Gemini appends /v1beta/models/... itself.
            let root = base.strip_suffix("/v1beta").unwrap_or(&base).to_owned();
            launch.env.push(("GOOGLE_GEMINI_BASE_URL".into(), root));
            launch.env.push(("GEMINI_MODEL".into(), model));
        }
        Runtime::Qwen => {
            write(
                home,
                ".qwen/settings.json",
                json!({"security":{"auth":{"selectedType":"openai"}},"model":{"name":model}})
                    .to_string(),
            )
            .await?;
            launch.env.push(("OPENAI_API_KEY".into(), key));
            launch.env.push(("OPENAI_BASE_URL".into(), base));
            launch.env.push(("OPENAI_MODEL".into(), model));
        }
        Runtime::Opencode => {
            let npm = match provider.format.as_str() {
                "openai_chat" => "@ai-sdk/openai-compatible",
                "openai_responses" => "@ai-sdk/openai",
                "anthropic" => "@ai-sdk/anthropic",
                _ => "@ai-sdk/google",
            };
            let mut models = Map::new();
            for id in &catalog {
                let mut entry = json!({"name": id});
                if let (Some(context), Some(output)) = (
                    number(provider, "context_window"),
                    number(provider, "max_output_tokens"),
                ) {
                    entry["limit"] = json!({"context":context,"output":output});
                }
                models.insert(id.clone(), entry);
            }
            let mut options = json!({"baseURL": base, "apiKey": format!("{{env:{KEY_ENV}}}")});
            if !headers.is_empty() {
                options["headers"] = Value::Object(headers);
            }
            let config = json!({
                "$schema": "https://opencode.ai/config.json",
                "autoupdate": false,
                "share": "disabled",
                "provider": {PROVIDER_ID: {"npm": npm, "name": "Ferry", "options": options, "models": models}},
                "model": format!("{PROVIDER_ID}/{model}"),
            });
            write(home, ".config/opencode/opencode.json", config.to_string()).await?;
            launch.env.push((KEY_ENV.into(), key));
        }
        Runtime::Pi => {
            let api = match provider.format.as_str() {
                "openai_chat" => "openai-completions",
                "openai_responses" => "openai-responses",
                "anthropic" => "anthropic-messages",
                _ => "google-generative-ai",
            };
            let models: Vec<Value> = catalog
                .iter()
                .map(|id| {
                    let mut entry = json!({"id": id, "name": id, "reasoning": flag(provider, "reasoning"), "input": ["text"]});
                    if let Some(n) = number(provider, "context_window") {
                        entry["contextWindow"] = json!(n);
                    }
                    if let Some(n) = number(provider, "max_output_tokens") {
                        entry["maxTokens"] = json!(n);
                    }
                    entry
                })
                .collect();
            let mut entry =
                json!({"baseUrl": base, "api": api, "apiKey": KEY_ENV, "models": models});
            if !headers.is_empty() {
                entry["headers"] = Value::Object(headers);
            }
            write(
                home,
                ".pi/agent/models.json",
                json!({"providers": {PROVIDER_ID: entry}}).to_string(),
            )
            .await?;
            launch.env.push((KEY_ENV.into(), key));
            launch
                .args
                .extend(["--provider".into(), PROVIDER_ID.into()]);
            launch.model = Some(model);
        }
        Runtime::Kimi => {
            let kind = match provider.format.as_str() {
                "openai_chat" => "openai",
                "openai_responses" => "openai_responses",
                "anthropic" => "anthropic",
                _ => "google-genai",
            };
            let context = number(provider, "context_window").unwrap_or(131_072);
            let mut toml = format!(
                "default_model = {}\n\n[providers.{PROVIDER_ID}]\ntype = \"{kind}\"\nbase_url = {}\napi_key_env = \"{KEY_ENV}\"\n",
                toml_str(&format!("{PROVIDER_ID}/{model}")),
                toml_str(&base)
            );
            if !headers.is_empty() {
                let pairs: Vec<String> = headers
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{} = {}",
                            toml_str(k),
                            toml_str(v.as_str().unwrap_or_default())
                        )
                    })
                    .collect();
                toml.push_str(&format!("custom_headers = {{ {} }}\n", pairs.join(", ")));
            }
            let mut capabilities = vec!["\"tool_use\""];
            if flag(provider, "reasoning") {
                capabilities.push("\"thinking\"");
            }
            if flag(provider, "image_input") {
                capabilities.push("\"image_in\"");
            }
            for id in &catalog {
                toml.push_str(&format!(
                    "\n[models.{}]\nprovider = \"{PROVIDER_ID}\"\nmodel = {}\nmax_context_size = {context}\ncapabilities = [{}]\n",
                    toml_str(&format!("{PROVIDER_ID}/{id}")),
                    toml_str(id),
                    capabilities.join(", ")
                ));
            }
            write(home, ".kimi-code/config.toml", toml).await?;
            launch.env.push((KEY_ENV.into(), key));
        }
        Runtime::Grok => {
            let backend = if provider.format == "openai_chat" {
                "chat"
            } else {
                "responses"
            };
            let context = number(provider, "context_window").unwrap_or(500_000);
            let mut toml = format!("[models]\ndefault = {}\n", toml_str(&model));
            for id in &catalog {
                toml.push_str(&format!(
                    "\n[model.{}]\nmodel = {}\nname = {}\nbase_url = {}\nenv_key = \"{KEY_ENV}\"\napi_backend = \"{backend}\"\ncontext_window = {context}\n",
                    toml_str(id),
                    toml_str(id),
                    toml_str(id),
                    toml_str(&base)
                ));
            }
            write(home, ".grok/config.toml", toml).await?;
            launch.env.push((KEY_ENV.into(), key));
        }
        Runtime::Dsh | Runtime::Copilot | Runtime::Codebuddy => {
            bail!("this CLI only uses its own login and configuration")
        }
    }
    Ok(launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(format: &str, options: Value) -> Provider {
        Provider {
            key: "42".into(),
            format: format.into(),
            base_url: "http://127.0.0.1:9/v1/".into(),
            api_key: "sk-secret-value".into(),
            models: vec!["m1".into(), "m2".into()],
            options,
        }
    }

    async fn files(home: &Path) -> String {
        let mut all = String::new();
        let mut stack = vec![home.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
            while let Some(entry) = entries.next_entry().await.unwrap() {
                if entry.file_type().await.unwrap().is_dir() {
                    stack.push(entry.path());
                } else {
                    all.push_str(&tokio::fs::read_to_string(entry.path()).await.unwrap());
                }
            }
        }
        all
    }

    #[tokio::test]
    async fn every_supported_cli_gets_config_without_the_key_on_disk() {
        for (runtime, format, file) in [
            (Runtime::Claude, "anthropic", ".claude/settings.json"),
            (Runtime::Codex, "openai_responses", ".codex/config.toml"),
            (Runtime::Gemini, "gemini", ".gemini/settings.json"),
            (Runtime::Qwen, "openai_chat", ".qwen/settings.json"),
            (
                Runtime::Opencode,
                "anthropic",
                ".config/opencode/opencode.json",
            ),
            (Runtime::Pi, "openai_chat", ".pi/agent/models.json"),
            (Runtime::Kimi, "openai_responses", ".kimi-code/config.toml"),
            (Runtime::Grok, "openai_chat", ".grok/config.toml"),
        ] {
            let home = tempfile::tempdir().unwrap();
            let launch = prepare(
                home.path(),
                runtime,
                &provider(format, json!({"format":format})),
                Some("m2"),
            )
            .await
            .unwrap();
            assert!(home.path().join(file).is_file(), "{runtime:?}");
            let written = files(home.path()).await;
            assert!(
                !written.contains("sk-secret-value"),
                "{runtime:?} wrote the key"
            );
            assert!(
                launch.env.iter().any(|(_, v)| v == "sk-secret-value"),
                "{runtime:?} must pass the key in the environment"
            );
            if file.ends_with(".json") {
                serde_json::from_str::<Value>(
                    &tokio::fs::read_to_string(home.path().join(file))
                        .await
                        .unwrap(),
                )
                .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn codex_and_claude_carry_their_options() {
        let home = tempfile::tempdir().unwrap();
        let options = json!({"format":"openai_responses","reasoning_effort":"high","context_window":400000,
            "subagent_model":"m1","headers":{"X-Team":"a\"b"}});
        let launch = prepare(
            home.path(),
            Runtime::Codex,
            &provider("openai_responses", options),
            None,
        )
        .await
        .unwrap();
        let toml = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
        for expected in [
            "model_provider = \"ferry\"",
            "model = \"m1\"",
            "model_reasoning_effort = \"high\"",
            "model_context_window = 400000",
            "base_url = \"http://127.0.0.1:9/v1\"",
            "wire_api = \"responses\"",
            "env_key = \"FERRY_PROVIDER_API_KEY\"",
            "\"X-Team\" = \"a\\\"b\"",
            "default_subagent_model = \"m1\"",
        ] {
            assert!(toml.contains(expected), "{expected} missing in {toml}");
        }
        assert_eq!(launch.model.as_deref(), Some("m1"));

        let home = tempfile::tempdir().unwrap();
        let options = json!({"format":"anthropic","auth_field":"ANTHROPIC_API_KEY","opus_model":"big","hide_attribution":true});
        let launch = prepare(
            home.path(),
            Runtime::Claude,
            &provider("anthropic", options),
            Some("m2"),
        )
        .await
        .unwrap();
        let env: std::collections::HashMap<_, _> = launch.env.into_iter().collect();
        assert_eq!(env["ANTHROPIC_API_KEY"], "sk-secret-value");
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:9");
        assert_eq!(env["ANTHROPIC_MODEL"], "m2");
        assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL"], "big");
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["includeCoAuthoredBy"], false);
    }

    #[tokio::test]
    async fn mismatched_formats_and_native_only_clis_are_refused() {
        let home = tempfile::tempdir().unwrap();
        for (runtime, format) in [
            (Runtime::Claude, "openai_chat"),
            (Runtime::Codex, "anthropic"),
            (Runtime::Copilot, "openai_chat"),
            (Runtime::Dsh, "openai_chat"),
        ] {
            assert!(
                prepare(home.path(), runtime, &provider(format, json!({})), None)
                    .await
                    .is_err(),
                "{runtime:?} {format}"
            );
        }
        let mut bad = provider("anthropic", json!({}));
        bad.base_url = "file:///etc/passwd".into();
        assert!(validate(Runtime::Claude, &bad).is_err());
        let mut bad = provider("anthropic", json!({}));
        bad.key = "../x".into();
        assert!(validate(Runtime::Claude, &bad).is_err());
    }
}
