//! Explicit non-Compose host execution; all transport remains HostChannel.
use crate::{
    channel::HostChannel,
    deployments::{directory, project, quote, upload},
    entities::{deployment_revisions, deployments},
    error::{ApiError, Result},
    manifest,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

pub async fn docker_resolve(ch: &dyn HostChannel, rendered: &str) -> Result<String> {
    let mut artifact: Value =
        serde_json::from_str(rendered).map_err(|_| ApiError::bad("Docker产物无效。"))?;
    artifact
        .as_object_mut()
        .unwrap()
        .remove("x-variable-references");
    artifact
        .as_object_mut()
        .unwrap()
        .remove("x-config-references");
    for service in artifact["services"].as_object_mut().unwrap().values_mut() {
        let image = service["image"]
            .as_str()
            .ok_or_else(|| ApiError::bad("缺少镜像。"))?;
        let pull = ch
            .exec(
                &format!("docker pull {}", quote(image)),
                Duration::from_secs(120),
            )
            .await?;
        if pull.code != 0 {
            return Err(ApiError::bad(format!(
                "无法拉取镜像，未开始部署：{}",
                pull.stderr
            )));
        }
        let inspect = ch
            .exec(
                &format!(
                    "docker image inspect --format '{{{{json .RepoDigests}}}}' {}",
                    quote(image)
                ),
                Duration::from_secs(30),
            )
            .await?;
        let digests: Vec<String> = serde_json::from_str(inspect.stdout.trim())
            .map_err(|_| ApiError::bad("镜像没有可用的仓库摘要。"))?;
        let digest = digests
            .first()
            .and_then(|d| d.split_once('@').map(|(_, hash)| hash))
            .filter(|d| d.starts_with("sha256:"))
            .ok_or_else(|| ApiError::bad("镜像必须有固定仓库摘要。"))?;
        let mut repository = image.split('@').next().unwrap();
        if let Some((base, tag)) = repository.rsplit_once(':') {
            if !tag.contains('/') {
                repository = base;
            }
        }
        service["image"] = json!(format!("{repository}@{digest}"));
    }
    serde_json::to_string_pretty(&artifact)
        .map(|s| s + "\n")
        .map_err(|_| ApiError::bad("无法保存Docker产物。"))
}
fn arg(command: &mut String, flag: &str, value: &str) {
    command.push_str(&format!(" {flag} {}", quote(value)));
}
fn mount_field(name: &str, value: &str) -> String {
    format!("\"{name}={}\"", value.replace('"', "\"\""))
}

fn docker_commands(app: &deployments::Model, artifact: &Value, action: &str) -> Result<String> {
    let prefix = project(app);
    let dir = directory(app);
    let services = artifact["services"]
        .as_object()
        .ok_or_else(|| ApiError::bad("Docker服务定义无效。"))?;
    let mut output = String::new();
    let mut remaining: BTreeSet<String> = services.keys().cloned().collect();
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let ready: Vec<_> = remaining
            .iter()
            .filter(|name| {
                services[*name]["depends_on"].as_array().is_none_or(|deps| {
                    deps.iter()
                        .all(|d| !remaining.contains(d.as_str().unwrap_or("")))
                })
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            return Err(ApiError::bad("Docker服务依赖包含循环。"));
        }
        for name in ready {
            remaining.remove(&name);
            ordered.push(name);
        }
    }
    for name in &ordered {
        let service = &services[name];
        let container = format!("{prefix}-{name}");
        output.push_str(&format!("if docker container inspect {} >/dev/null 2>&1; then owner=$(docker inspect --format '{{{{index .Config.Labels \"workspace.deployment\"}}}}' {}); [ \"$owner\" = {} ] || {{ echo '容器名已被其他应用占用' >&2; exit 75; }}; ",quote(&container),quote(&container),quote(&prefix)));
        if action == "restart" {
            output.push_str(&format!(
                "docker restart {}; else echo '待重启容器不存在' >&2; exit 76; fi; ",
                quote(&container)
            ));
            continue;
        }
        output.push_str(&format!("docker rm -f {}; fi; ( ", quote(&container)));
        let mut command = "docker run -d".to_owned();
        arg(&mut command, "--name", &container);
        if let Some(labels) = service["labels"].as_object() {
            for (key, value) in labels {
                arg(
                    &mut command,
                    "--label",
                    &format!("{key}={}", value.as_str().unwrap()),
                );
            }
        }
        arg(
            &mut command,
            "--label",
            &format!("workspace.deployment={prefix}"),
        );
        if let Some(environment) = service["environment"].as_object() {
            for (key, value) in environment {
                arg(
                    &mut command,
                    "--env",
                    &format!("{key}={}", value.as_str().unwrap()),
                );
            }
        }
        if let Some(files) = service["env_file"].as_array() {
            for path in files {
                let path = path
                    .as_str()
                    .ok_or_else(|| ApiError::bad("变量文件无效。"))?;
                output.push_str(&format!(". {}; ", quote(&format!("{dir}/{path}"))));
            }
            if let Some(bindings) = artifact["x-variable-bindings"].as_array() {
                for binding in bindings
                    .iter()
                    .filter(|v| v["service"] == *name && v.get("environment").is_some())
                {
                    arg(
                        &mut command,
                        "--env",
                        binding["environment"].as_str().unwrap(),
                    );
                }
            }
        }
        if let Some(ports) = service["ports"].as_array() {
            for port in ports {
                if let Some(published) = port["published"].as_str() {
                    arg(
                        &mut command,
                        "--publish",
                        &format!(
                            "{published}:{}/{}",
                            port["target"],
                            port["protocol"].as_str().unwrap_or("tcp")
                        ),
                    );
                }
            }
        }
        if let Some(volumes) = service["volumes"].as_array() {
            for volume in volumes {
                let source = volume["source"].as_str().unwrap();
                let target = volume["target"].as_str().unwrap();
                if volume["type"] == "volume" {
                    let source = format!("{prefix}_{source}");
                    output.push_str(&format!(
                        "docker volume create {} >/dev/null; ",
                        quote(&source)
                    ));
                    arg(
                        &mut command,
                        "--mount",
                        &format!(
                            "type=volume,{},{}",
                            mount_field("src", &source),
                            mount_field("dst", target)
                        ),
                    );
                } else {
                    arg(
                        &mut command,
                        "--mount",
                        &format!(
                            "type=bind,{},{},readonly",
                            mount_field(
                                "src",
                                &format!("{dir}/{}", source.trim_start_matches("./"))
                            ),
                            mount_field("dst", target)
                        ),
                    );
                }
            }
        }
        for (path, flag) in [
            ("/deploy/resources/limits/cpus", "--cpus"),
            ("/deploy/resources/limits/memory", "--memory"),
            (
                "/deploy/resources/reservations/memory",
                "--memory-reservation",
            ),
        ] {
            if let Some(value) = service.pointer(path) {
                arg(
                    &mut command,
                    flag,
                    &value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                );
            }
        }
        if let Some(restart) = service["restart"].as_str() {
            arg(&mut command, "--restart", restart);
        }
        if let Some(logging) = service["logging"].as_object() {
            if let Some(driver) = logging.get("driver").and_then(Value::as_str) {
                arg(&mut command, "--log-driver", driver);
            }
            if let Some(options) = logging.get("options").and_then(Value::as_object) {
                for (key, value) in options {
                    arg(
                        &mut command,
                        "--log-opt",
                        &format!("{key}={}", value.as_str().unwrap()),
                    );
                }
            }
        }
        if let Some(health) = service["healthcheck"].as_object() {
            let test = health.get("test").and_then(Value::as_array);
            if health.get("disable") == Some(&json!(true))
                || test.is_some_and(|t| t.first() == Some(&json!("NONE")))
            {
                command.push_str(" --no-healthcheck");
            } else if let Some(test) = test {
                let cmd = if test[0] == "CMD-SHELL" {
                    test.iter()
                        .skip(1)
                        .map(|v| v.as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    test.iter()
                        .skip(1)
                        .map(|v| quote(v.as_str().unwrap()))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                arg(&mut command, "--health-cmd", &cmd);
                for (key, flag) in [
                    ("interval", "--health-interval"),
                    ("timeout", "--health-timeout"),
                    ("retries", "--health-retries"),
                    ("start_period", "--health-start-period"),
                    ("start_interval", "--health-start-interval"),
                ] {
                    if let Some(value) = health.get(key) {
                        arg(
                            &mut command,
                            flag,
                            &value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string()),
                        );
                    }
                }
            }
        }
        command.push(' ');
        command.push_str(&quote(service["image"].as_str().unwrap()));
        if let Some(args) = service["command"].as_array() {
            for value in args {
                command.push(' ');
                command.push_str(&quote(value.as_str().unwrap()));
            }
        }
        output.push_str(&command);
        output.push_str("; ); ");
    }
    // Only remove obsolete containers carrying this application's ownership label.
    if action != "restart" {
        let keep = ordered
            .iter()
            .map(|n| format!("{prefix}-{n}"))
            .collect::<Vec<_>>()
            .join("|");
        output.push_str(&format!("for old in $(docker ps -aq --filter {}); do name=$(docker inspect --format '{{{{.Name}}}}' \"$old\"); case \"${{name#/}}\" in {keep}) ;; *) docker rm -f \"$old\" ;; esac; done; ",quote(&format!("label=workspace.deployment={prefix}"))));
    }
    Ok(output)
}

pub async fn docker_execute(
    ch: &dyn HostChannel,
    app: &deployments::Model,
    rev: &deployment_revisions::Model,
    files: &BTreeMap<String, String>,
    protected: bool,
) -> Result<(bool, String)> {
    let artifact: Value =
        serde_json::from_str(&rev.artifact).map_err(|_| ApiError::bad("Docker历史产物无效。"))?;
    let dir = directory(app);
    let stage = format!("{dir}/revision-{}.json", rev.id);
    let current = format!("{dir}/compose.json");
    let prepare=ch.exec(&format!("set -eu; command -v flock >/dev/null; command -v sha256sum >/dev/null; docker version >/dev/null; umask 077; mkdir -p -- {}",quote(&dir)),Duration::from_secs(30)).await?;
    if prepare.code != 0 {
        return Ok((false, "Docker、flock、sha256sum或部署目录不可用。".into()));
    }
    upload(ch, &stage, rev.artifact.clone()).await?;
    let mut temporary: Vec<String> = Vec::new();
    for (name, text) in files {
        if name.contains('/')
            || !(name.starts_with(".variables-") && name.ends_with(".env")
                || name.starts_with(".config-") && name.ends_with(".txt"))
        {
            return Err(ApiError::bad("历史文件路径无效。"));
        }
        let path = format!("{dir}/{name}");
        if let Err(error) = upload(ch, &path, text.clone()).await {
            for path in &temporary {
                let _ = ch.delete_file(path).await;
            }
            return Err(error);
        }
        if name.starts_with(".variables-") {
            temporary.push(path);
        }
    }
    let manifest = manifest::parse_for(&rev.manifest, "docker")?;
    let timeout = manifest
        .strategy
        .as_ref()
        .map(|s| s.health_timeout)
        .unwrap_or(120);
    let mut command = format!(
        "set -eu; actual=$(if [ -e {path} ]; then cat -- {path}; fi | sha256sum); [ \"${{actual%% *}}\" = {expected} ] || {{ echo '目标产物已变化，请重新预览。' >&2; exit 73; }}; ",
        path = quote(&current),
        expected = quote(&rev.expected_hash)
    );
    if !temporary.is_empty() {
        command.push_str(&format!(
            "trap {} EXIT; ",
            quote(&format!(
                "rm -f -- {}",
                temporary
                    .iter()
                    .map(|s| quote(s))
                    .collect::<Vec<_>>()
                    .join(" ")
            ))
        ));
    }
    command.push_str(&docker_commands(app, &artifact, &rev.action)?);
    command.push_str(&format!("deadline=$(( $(date +%s) + {timeout} )); "));
    for name in artifact["services"].as_object().unwrap().keys() {
        let container = quote(&format!("{}-{name}", project(app)));
        command.push_str(&format!("while :; do state=$(docker inspect --format '{{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{else}}}}{{{{.State.Status}}}}{{{{end}}}}' {container}); case \"$state\" in healthy|running) break ;; exited|dead) echo '容器已退出' >&2; exit 74 ;; esac; [ $(date +%s) -lt \"$deadline\" ] || {{ echo '容器健康检查超时' >&2; exit 74; }}; sleep 1; done; "));
    }
    for (service, probe, period) in manifest::readiness(&manifest)? {
        command.push_str(&format!("until docker exec {} sh -c {} >/dev/null 2>&1; do [ $(date +%s) -lt \"$deadline\" ] || {{ echo 'readiness检查超时' >&2; exit 74; }}; next=$(( $(date +%s) + {period} )); while [ $(date +%s) -lt \"$next\" ] && [ $(date +%s) -lt \"$deadline\" ]; do sleep 1; done; done; ",quote(&format!("{}-{service}",project(app))),quote(&probe)));
    }
    command.push_str(&format!("cp -- {} {}", quote(&stage), quote(&current)));
    let result = ch
        .exec(
            &format!(
                "flock -n {} sh -c {}",
                quote(&format!("{dir}/execution.lock")),
                quote(&command)
            ),
            Duration::from_secs(u64::from(timeout) + 180),
        )
        .await;
    if !temporary.is_empty() {
        let cleanup = ch
            .exec(
                &format!(
                    "rm -f -- {}",
                    temporary
                        .iter()
                        .map(|p| quote(p))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                Duration::from_secs(15),
            )
            .await?;
        if cleanup.code != 0 {
            return Err(ApiError::bad("Docker变量文件清理失败，请核实目标。"));
        }
    }
    let out = result?;
    Ok((
        out.code == 0,
        if protected {
            format!(
                "Docker执行退出码：{}；引用变量的原始输出未保存。\n",
                out.code
            )
        } else {
            format!("{}{}\n退出码：{}\n", out.stdout, out.stderr, out.code)
        },
    ))
}

// EnvironmentFile uses shell-like quoting but performs no variable expansion.
// Preserve literal newlines; escaping '$' here would change the value.
pub(crate) fn systemd_environment(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
    )
}
fn unit_word(value: &str, exec: bool) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '%' => out.push_str("%%"),
            '$' if exec => out.push_str("$$"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
pub(crate) async fn native_resolve(
    s: &crate::state::AppState,
    app: &deployments::Model,
    rendered: &str,
) -> Result<String> {
    let mut doc: Value =
        serde_json::from_str(rendered).map_err(|_| ApiError::bad("native产物无效。"))?;
    for service in doc["services"].as_object_mut().unwrap().values_mut() {
        let reference: manifest::BuildArtifact =
            serde_json::from_value(service["artifact"].clone())
                .map_err(|_| ApiError::bad("构建产物引用无效。"))?;
        let row = crate::artifacts::resolve(&s.db, app.workspace_id.into(), &reference).await?;
        service["artifact"] = json!({"artifact_id":row.id.to_string(),"run_id":row.run_id.to_string(),"pipeline_id":row.pipeline_id.to_string(),"path":row.path,"sha256":row.sha256,"size":row.size,"mode":row.mode});
    }
    serde_json::to_string_pretty(&doc).map_err(|_| ApiError::bad("无法保存native产物。"))
}

/// Store exact units and probes in the revision, including overrides. Rollback
/// consumes these bytes, never a freshly rendered unit from a newer server.
pub(crate) fn native_finalize(app: &deployments::Model, artifact: &str) -> Result<String> {
    let mut doc: Value =
        serde_json::from_str(artifact).map_err(|_| ApiError::bad("native产物无效。"))?;
    let m = manifest::parse_for(&app.manifest, "native")?;
    let timeout = m.strategy.as_ref().map(|s| s.health_timeout).unwrap_or(120);
    let dir = directory(app);
    let prefix = project(app);
    let probes = manifest::native_probes(&m)?;
    let mut units = BTreeMap::new();
    let mut scripts = BTreeMap::new();
    for (name, service) in doc["services"].as_object().unwrap() {
        let executable = format!(
            "{dir}/.artifact-{}",
            service["artifact"]["sha256"].as_str().unwrap()
        );
        let mut unit = String::from("[Unit]\nAfter=network.target\n");
        for dependency in service["depends_on"].as_array().unwrap() {
            let target = format!("{prefix}-{}.service", dependency.as_str().unwrap());
            unit.push_str(&format!("Requires={target}\nAfter={target}\n"));
        }
        unit.push_str(&format!(
            "\n[Service]\nType=exec\nWorkingDirectory={}\nExecStart={}",
            dir.replace('%', "%%"),
            unit_word(&executable, true)
        ));
        for argument in service["command"].as_array().unwrap() {
            unit.push(' ');
            unit.push_str(&unit_word(argument.as_str().unwrap(), true));
        }
        unit.push_str(&format!(
            "\nRestart=on-failure\nRestartSec=2\nTimeoutStartSec={}\n",
            timeout + 10
        ));
        for (key, value) in service["environment"].as_object().unwrap() {
            unit.push_str(&format!(
                "Environment={}\n",
                unit_word(&format!("{key}={}", value.as_str().unwrap()), false)
            ));
        }
        if let Some(files) = service["env_file"].as_array() {
            for file in files {
                unit.push_str(&format!(
                    "EnvironmentFile={}\n",
                    format!("{dir}/{}", file.as_str().unwrap()).replace('%', "%%")
                ));
            }
        }
        for mount in service["volumes"].as_array().unwrap() {
            let source = mount["source"].as_str().unwrap();
            let target = mount["target"].as_str().unwrap();
            // ':' is systemd's source:destination separator, not an escapable filename character.
            if target.contains(':') || dir.contains(':') {
                return Err(ApiError::bad("native挂载路径不能包含冒号。"));
            }
            let (directive, path) = if mount["type"] == "volume" {
                ("BindPaths", format!("{dir}/volumes/{source}"))
            } else {
                (
                    "BindReadOnlyPaths",
                    format!("{dir}/{}", source.trim_start_matches("./")),
                )
            };
            unit.push_str(&format!(
                "{directive}={}:{}\n",
                unit_word(&path, false),
                unit_word(target, false)
            ));
        }
        if let Some(cpus) = service["limits"]["cpus"].as_str() {
            let cpus: f64 = cpus.parse().map_err(|_| ApiError::bad("CPU限制无效。"))?;
            unit.push_str(&format!("CPUQuota={:.2}%\n", cpus * 100.0));
        }
        if let Some(memory) = service["limits"]["memory"].as_str() {
            unit.push_str(&format!("MemoryMax={memory}\n"));
        }
        let mut script = format!("#!/bin/sh\nset -eu\ndeadline=$(( $(date +%s) + {timeout} ))\n");
        let mut has_probe = false;
        for (_, command, period) in probes.iter().filter(|(service, _, _)| service == name) {
            has_probe = true;
            script.push_str(&format!("until {command} >/dev/null 2>&1; do [ $(date +%s) -lt \"$deadline\" ] || exit 1; sleep {period}; done\n"));
        }
        if has_probe {
            use sha2::{Digest, Sha256};
            let script_name = format!(
                ".probe-{name}-{}.sh",
                hex::encode(Sha256::digest(script.as_bytes()))
            );
            unit.push_str(&format!(
                "ExecStartPost=/bin/sh {}\n",
                unit_word(&format!("{dir}/{script_name}"), true)
            ));
            scripts.insert(script_name, script);
        }
        unit.push_str("\n[Install]\nWantedBy=multi-user.target\n");
        if let Some(overridden) = doc["x-native-overrides"]["units"][name].as_str() {
            unit = overridden.to_owned();
        }
        units.insert(
            format!("{prefix}-{name}.service"),
            format!("# workspace.deployment={prefix}\n{unit}"),
        );
    }
    doc["x-native-units"] = json!(units);
    doc["x-native-scripts"] = json!(scripts);
    serde_json::to_string_pretty(&doc)
        .map(|s| s + "\n")
        .map_err(|_| ApiError::bad("无法保存systemd产物。"))
}

pub(crate) async fn native_execute(
    s: &crate::state::AppState,
    ch: &dyn HostChannel,
    app: &deployments::Model,
    rev: &deployment_revisions::Model,
    files: &BTreeMap<String, String>,
    protected: bool,
) -> Result<(bool, String)> {
    use tokio::io::AsyncReadExt;
    let artifact: Value =
        serde_json::from_str(&rev.artifact).map_err(|_| ApiError::bad("native历史产物无效。"))?;
    let dir = directory(app);
    let prefix = project(app);
    let staging = format!("{dir}/stage-{}", rev.id);
    let ready = ch.exec(&format!("set -eu; command -v flock >/dev/null; command -v sha256sum >/dev/null; systemctl show-environment >/dev/null; umask 077; mkdir -p -- {}",quote(&staging)),Duration::from_secs(30)).await?;
    if ready.code != 0 {
        return Ok((false, "systemd、flock、sha256sum或部署目录不可用。".into()));
    }
    let units = artifact["x-native-units"]
        .as_object()
        .ok_or_else(|| ApiError::bad("历史systemd单元缺失。"))?;
    let mut command = format!(
        "set -eu; umask 077; current={}; actual=$(if [ -e \"$current\" ]; then cat -- \"$current\"; fi | sha256sum); [ \"${{actual%% *}}\" = {} ] || {{ echo '目标产物已变化，请重新预览' >&2; exit 73; }}; ",
        quote(&format!("{dir}/compose.json")),
        quote(&rev.expected_hash)
    );
    // Validate ownership of every affected unit before stopping any service.
    command.push_str(&format!("for unit in /etc/systemd/system/{prefix}-*.service; do [ -e \"$unit\" ] || continue; [ \"$(head -n 1 \"$unit\")\" = {} ] || exit 75; done; ",quote(&format!("# workspace.deployment={prefix}"))));
    for name in units.keys() {
        if !name.starts_with(&format!("{prefix}-")) || name.contains('/') {
            return Err(ApiError::bad("历史单元名无效。"));
        }
        // Units installed through another systemd search path also count as occupied.
        command.push_str(&format!("found=$(systemctl show -p FragmentPath --value {} 2>/dev/null || :); if [ -n \"$found\" ]; then [ \"$(head -n 1 \"$found\")\" = {} ] || exit 75; fi; ",quote(name),quote(&format!("# workspace.deployment={prefix}"))));
    }
    if rev.action != "restart" {
        let mut uploaded = BTreeSet::new();
        for service in artifact["services"].as_object().unwrap().values() {
            let reference = &service["artifact"];
            let run = reference["run_id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| ApiError::bad("历史构建ID无效。"))?;
            let id = reference["artifact_id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| ApiError::bad("历史产物ID无效。"))?;
            let hash = reference["sha256"]
                .as_str()
                .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
                .ok_or_else(|| ApiError::bad("历史产物摘要无效。"))?;
            if !uploaded.insert(hash.to_owned()) {
                continue;
            }
            let local = crate::artifacts::directory(s, run).join(id.to_string());
            let file = tokio::fs::File::open(local)
                .await
                .map_err(|_| ApiError::bad("历史构建产物文件缺失。"))?;
            let stream = futures::stream::try_unfold(file, |mut file| async move {
                let mut buffer = vec![0u8; 65536];
                let n = file
                    .read(&mut buffer)
                    .await
                    .map_err(|_| ApiError::bad("读取构建产物失败。"))?;
                if n == 0 {
                    return Ok(None);
                }
                buffer.truncate(n);
                Ok(Some((bytes::Bytes::from(buffer), file)))
            });
            let target = format!("{staging}/.artifact-{hash}");
            ch.write_file(&target, 0o700, Box::pin(stream)).await?;
            command.push_str(&format!(
                "actual=$(sha256sum {}); [ \"${{actual%% *}}\" = {} ] || exit 77; mv -f -- {} {}; ",
                quote(&target),
                quote(hash),
                quote(&target),
                quote(&format!("{dir}/.artifact-{hash}"))
            ));
        }
        for (name, text) in files {
            if name.contains('/')
                || !(name.starts_with(".variables-") || name.starts_with(".config-"))
            {
                return Err(ApiError::bad("历史文件路径无效。"));
            }
            upload(ch, &format!("{staging}/{name}"), text.clone()).await?;
            command.push_str(&format!(
                "mv -f -- {} {}; ",
                quote(&format!("{staging}/{name}")),
                quote(&format!("{dir}/{name}"))
            ));
        }
        if let Some(volumes) = artifact["volumes"].as_object() {
            for name in volumes.keys() {
                command.push_str(&format!(
                    "mkdir -p -- {}; ",
                    quote(&format!("{dir}/volumes/{name}"))
                ));
            }
        }
        for (name, text) in artifact["x-native-scripts"]
            .as_object()
            .ok_or_else(|| ApiError::bad("历史探针缺失。"))?
        {
            if name.contains('/') {
                return Err(ApiError::bad("历史探针路径无效。"));
            }
            upload(
                ch,
                &format!("{staging}/{name}"),
                text.as_str().unwrap().to_owned(),
            )
            .await?;
            command.push_str(&format!(
                "mv -f -- {} {}; ",
                quote(&format!("{staging}/{name}")),
                quote(&format!("{dir}/{name}"))
            ));
        }
        let keep = units.keys().cloned().collect::<Vec<_>>().join("|");
        command.push_str(&format!("for unit in /etc/systemd/system/{prefix}-*.service; do [ -e \"$unit\" ] || continue; name=${{unit##*/}}; case \"$name\" in {keep}) ;; *) systemctl disable --now \"$name\"; rm -- \"$unit\" ;; esac; done; "));
        for (name, text) in units {
            upload(
                ch,
                &format!("{staging}/{name}"),
                text.as_str().unwrap().to_owned(),
            )
            .await?;
            command.push_str(&format!(
                "install -m 600 -- {} {}; ",
                quote(&format!("{staging}/{name}")),
                quote(&format!("/etc/systemd/system/{name}"))
            ));
        }
        command.push_str("systemctl daemon-reload; ");
    }
    // systemd resolves Requires/After, waits for ExecStartPost and propagates failure.
    let names = units
        .keys()
        .map(|name| quote(name))
        .collect::<Vec<_>>()
        .join(" ");
    for name in units.keys() {
        command.push_str(&format!("if [ \"$(systemctl show -p ActiveState --value {})\" = failed ]; then systemctl reset-failed {}; fi; ",quote(name),quote(name)));
    }
    command.push_str(&format!(
        "systemctl enable {names}; systemctl restart {names}; "
    ));
    for name in units.keys() {
        command.push_str(&format!("systemctl is-active --quiet {}; ", quote(name)));
    }
    upload(
        ch,
        &format!("{staging}/artifact.json"),
        rev.artifact.clone(),
    )
    .await?;
    command.push_str(&format!(
        "cp -- {} \"$current\"; rm -rf -- {}",
        quote(&format!("{staging}/artifact.json")),
        quote(&staging)
    ));
    let m = manifest::parse_for(&rev.manifest, "native")?;
    let timeout = m.strategy.as_ref().map(|s| s.health_timeout).unwrap_or(120);
    let result = ch
        .exec(
            &format!(
                "flock -n {} sh -c {}",
                quote(&format!("{dir}/execution.lock")),
                quote(&command)
            ),
            Duration::from_secs(u64::from(timeout) * units.len() as u64 + 120),
        )
        .await?;
    Ok((
        result.code == 0,
        if protected {
            format!("native执行退出码：{}；原始输出未保存。\n", result.code)
        } else {
            format!(
                "{}{}\n退出码：{}\n",
                result.stdout, result.stderr, result.code
            )
        },
    ))
}

/// Swarm distributes configuration through encrypted secret objects, never
/// through a bind path that exists only on the manager.
pub(crate) fn stack_finalize(app: &deployments::Model, artifact: &str) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut doc: Value =
        serde_json::from_str(artifact).map_err(|_| ApiError::bad("Swarm产物无效。"))?;
    let mut secrets = serde_json::Map::new();
    let mut sources = serde_json::Map::new();
    for service in doc["services"].as_object_mut().unwrap().values_mut() {
        let mut mounts = Vec::new();
        if let Some(volumes) = service["volumes"].as_array_mut() {
            volumes.retain(|volume| {
                if volume["type"]!="bind" {return true;}
                let source=volume["source"].as_str().unwrap().trim_start_matches("./");
                let key=format!("config-{}",hex::encode(Sha256::digest(source.as_bytes())));
                let digest=hex::encode(Sha256::digest(format!("{}:{source}",project(app)).as_bytes()));
                let name=format!("cfg-{}",&digest[..60]);
                secrets.insert(key.clone(),json!({"external":true,"name":name}));
                sources.insert(key.clone(),json!(source));
                mounts.push(json!({"source":key,"target":volume["target"],"uid":"0","gid":"0","mode":0o600}));
                false
            });
        }
        if !mounts.is_empty() {
            service["secrets"] = json!(mounts);
        }
        service["deploy"]["labels"] = json!({"workspace.deployment":project(app)});
    }
    doc["version"] = json!("3.8");
    if !secrets.is_empty() {
        doc["secrets"] = Value::Object(secrets);
    }
    doc["x-secret-sources"] = Value::Object(sources);
    serde_json::to_string_pretty(&doc)
        .map(|s| s + "\n")
        .map_err(|_| ApiError::bad("无法保存Swarm产物。"))
}

pub(crate) async fn stack_execute(
    ch: &dyn HostChannel,
    app: &deployments::Model,
    rev: &deployment_revisions::Model,
    files: &BTreeMap<String, String>,
    protected: bool,
) -> Result<(bool, String)> {
    let artifact: Value =
        serde_json::from_str(&rev.artifact).map_err(|_| ApiError::bad("Swarm历史产物无效。"))?;
    let mut runtime = artifact.clone();
    for service in runtime["services"].as_object_mut().unwrap().values_mut() {
        if let Some(paths) = service.as_object_mut().unwrap().remove("env_file") {
            let environment = service
                .as_object_mut()
                .unwrap()
                .entry("environment")
                .or_insert(json!({}));
            for path in paths
                .as_array()
                .ok_or_else(|| ApiError::bad("历史变量路径无效。"))?
            {
                let text = files
                    .get(path.as_str().unwrap_or(""))
                    .ok_or_else(|| ApiError::bad("历史变量快照缺失。"))?;
                for line in text.lines() {
                    let values: BTreeMap<String, String> = serde_json::from_str(line)
                        .map_err(|_| ApiError::bad("历史Swarm变量快照无效。"))?;
                    for (key, value) in values {
                        environment[key] = json!(value);
                    }
                }
            }
        }
        if let Some(ports) = service["ports"].as_array_mut() {
            for port in ports {
                if let Some(value) = port["published"].as_str() {
                    port["published"] = json!(
                        value
                            .parse::<u16>()
                            .map_err(|_| ApiError::bad("Swarm端口无效。"))?
                    );
                }
            }
        }
    }
    runtime
        .as_object_mut()
        .unwrap()
        .retain(|key, _| !key.starts_with("x-"));
    // Docker stack uses Compose interpolation too. Escape only the private
    // runtime document, after all snapshot environment values have been added.
    crate::manifest::escape(&mut runtime);
    let dir = directory(app);
    let prefix = project(app);
    let stage = format!("{dir}/stack-{}", rev.id);
    let ready=ch.exec(&format!("set -eu; command -v flock >/dev/null; command -v sha256sum >/dev/null; [ \"$(docker info --format '{{{{.Swarm.ControlAvailable}}}}')\" = true ]; umask 077; mkdir -p -- {}",quote(&stage)),Duration::from_secs(30)).await?;
    if ready.code != 0 {
        return Ok((
            false,
            "目标必须是可用的Swarm管理节点，且提供flock、sha256sum和可写部署目录。".into(),
        ));
    }
    let result=async {
        upload(ch,&format!("{stage}/stack.json"),runtime.to_string()).await?;
        upload(ch,&format!("{stage}/artifact.json"),rev.artifact.clone()).await?;
        let mut command=format!("set -eu; trap {} EXIT; actual=$(if [ -e {current} ]; then cat -- {current}; fi | sha256sum); [ \"${{actual%% *}}\" = {} ] || {{ echo '目标产物已变化，请重新预览' >&2; exit 73; }}; ",quote(&format!("rm -rf -- {}",quote(&stage))),quote(&rev.expected_hash),current=quote(&format!("{dir}/compose.json")));
        command.push_str(&format!("for service in $(docker service ls -q --filter {}); do owner=$(docker service inspect --format '{{{{index .Spec.Labels \"workspace.deployment\"}}}}' \"$service\"); [ \"$owner\" = {} ] || exit 75; done; ",quote(&format!("label=com.docker.stack.namespace={prefix}")),quote(&prefix)));
        for name in artifact["services"].as_object().unwrap().keys() {
            let name=quote(&format!("{prefix}_{name}"));
            command.push_str(&format!("if docker service inspect {name} >/dev/null 2>&1; then [ \"$(docker service inspect --format '{{{{index .Spec.Labels \"workspace.deployment\"}}}}' {name})\" = {} ] || exit 75; fi; ",quote(&prefix)));
        }
        if let Some(sources)=artifact["x-secret-sources"].as_object() {
            for (key,source) in sources {
                let source=source.as_str().ok_or_else(||ApiError::bad("Swarm配置引用无效。"))?;
                let content=files.get(source).ok_or_else(||ApiError::bad("历史配置快照缺失。"))?;
                if content.len()>500*1024 {return Err(ApiError::bad("Swarm单个配置文件不能超过500KiB。"));}
                let name=artifact["secrets"][key]["name"].as_str().ok_or_else(||ApiError::bad("Swarm配置名称无效。"))?;
                let path=format!("{stage}/{key}");
                upload(ch,&path,content.clone()).await?;
                command.push_str(&format!("if docker secret inspect {name} >/dev/null 2>&1; then [ \"$(docker secret inspect --format '{{{{index .Spec.Labels \"workspace.deployment\"}}}}' {name})\" = {owner} ] || exit 75; else docker secret create --label {label} {name} {path} >/dev/null; fi; ",name=quote(name),owner=quote(&prefix),label=quote(&format!("workspace.deployment={prefix}")),path=quote(&path)));
            }
        }
        if rev.action=="restart" {
            for name in artifact["services"].as_object().unwrap().keys() {
                command.push_str(&format!("docker service update --force --detach=true {} >/dev/null; ",quote(&format!("{prefix}_{name}"))));
            }
        } else {
            command.push_str(&format!("docker stack deploy --prune --resolve-image never -c {} {}; ",quote(&format!("{stage}/stack.json")),quote(&prefix)));
        }
        let m=manifest::parse_for(&rev.manifest,"stack")?;
        let timeout=m.strategy.as_ref().map(|s|s.health_timeout).unwrap_or(120);
        command.push_str(&format!("deadline=$(( $(date +%s) + {timeout} )); "));
        for (name,service) in artifact["services"].as_object().unwrap() {
            let name=quote(&format!("{prefix}_{name}"));
            let count=service["deploy"]["replicas"].as_u64().unwrap_or(1);
            command.push_str(&format!("while :; do update=$(docker service inspect --format '{{{{if .UpdateStatus}}}}{{{{.UpdateStatus.State}}}}{{{{end}}}}' {name}); case \"$update\" in paused|rollback_paused|rollback_completed) echo 'Swarm更新失败或被暂停' >&2; exit 74;; esac; states=$(docker service ps --filter desired-state=running --format '{{{{.CurrentState}}}}' {name}); running=0; ready=true; while IFS= read -r state; do case \"$state\" in Running\\ *) running=$((running+1));; *) ready=false;; esac; done <<STATES\n$states\nSTATES\nif [ \"$ready\" = true ] && [ \"$running\" -eq {count} ] && [ \"$update\" != updating ]; then break; fi; [ $(date +%s) -lt \"$deadline\" ] || {{ echo 'Swarm副本未全部就绪，已超时' >&2; exit 74; }}; sleep 1; done; "));
        }
        command.push_str(&format!("cp -- {} {}",quote(&format!("{stage}/artifact.json")),quote(&format!("{dir}/compose.json"))));
        let result=ch.exec(&format!("flock -n {} sh -c {}",quote(&format!("{dir}/execution.lock")),quote(&command)),Duration::from_secs(u64::from(timeout)+180)).await?;
        Ok::<_,ApiError>((result.code==0,if protected {format!("Swarm执行退出码：{}；原始输出未保存。\n",result.code)}else{format!("{}{}\n退出码：{}\n",result.stdout,result.stderr,result.code)}))
    }.await;
    let cleanup = ch
        .exec(
            &format!("rm -rf -- {}", quote(&stage)),
            Duration::from_secs(15),
        )
        .await?;
    if cleanup.code != 0 {
        return Err(ApiError::bad("Swarm临时配置清理失败，请核实目标。"));
    }
    result
}
