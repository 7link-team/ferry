//! File output for `ai.agent.run`: the Agent's CLI writes deliverables into
//! `./output/` of a fresh, Runner-managed working directory (writes confined to
//! that directory), and the server collects them afterwards as workflow files.
use crate::{
    content_assets::{DEFAULT_QUOTA, Store},
    entities::workflow_executions,
    error::{ApiError, Result},
    hosts::channel::HostChannel,
    i18n::Message,
    state::AppState,
};
use axum::http::StatusCode;
use ferry_protocol::{agent::Runtime, sandbox::Asset};
use futures::StreamExt;
use serde_json::{Value, json};

/// One stored file may not exceed the content store's object limit.
pub(crate) const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_FILES: u64 = 10;
const MAX_FILES: u64 = 50;
const DEFAULT_TOTAL_MB: u64 = 100;
const MAX_TOTAL_MB: u64 = 500;
const MAX_INSTRUCTIONS: usize = 4 * 1024;
const MAX_FORMATS: usize = 30;
/// Bounds on what is walked under `output/`.
const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 500;
pub(crate) const OUTPUT_DIR: &str = "output";

/// The node's `files` settings, when file output is enabled.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileMode {
    /// Lower-case extensions; empty means any format.
    pub formats: Vec<String>,
    pub max_files: usize,
    pub max_bytes: u64,
    pub instructions: String,
}
impl FileMode {
    /// A conversation turn: any format, the node defaults.
    pub(crate) fn conversation() -> Self {
        Self {
            formats: Vec::new(),
            max_files: DEFAULT_FILES as usize,
            max_bytes: DEFAULT_TOTAL_MB * 1024 * 1024,
            instructions: String::new(),
        }
    }
}

fn invalid(path: String) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.agentFilesInvalid").with("path", path),
    )
}

/// `None` when file output is off. Absent numbers take their defaults.
pub(super) fn mode(params: &Value, base: &str) -> Result<Option<FileMode>> {
    let files = &params["files"];
    if files.is_null() || files["enabled"] != true {
        return Ok(None);
    }
    let path = |field: &str| format!("{base}/params/files/{field}");
    let number = |field: &str, default: u64, max: u64| match &files[field] {
        Value::Null => Ok(default),
        value => value
            .as_u64()
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| invalid(path(field))),
    };
    let max_files = number("max_files", DEFAULT_FILES, MAX_FILES)? as usize;
    let max_mb = number("max_total_mb", DEFAULT_TOTAL_MB, MAX_TOTAL_MB)?;
    let mut formats = Vec::new();
    match &files["formats"] {
        Value::Null => {}
        Value::String(any) if any == "any" => {}
        Value::Array(list) => {
            if list.len() > MAX_FORMATS {
                return Err(invalid(path("formats")));
            }
            for value in list {
                let format = value
                    .as_str()
                    .map(|f| f.trim().trim_start_matches('.').to_ascii_lowercase())
                    .filter(|f| {
                        !f.is_empty()
                            && f.len() <= 10
                            && f.bytes().all(|b| b.is_ascii_alphanumeric())
                    })
                    .ok_or_else(|| invalid(path("formats")))?;
                if !formats.contains(&format) {
                    formats.push(format);
                }
            }
        }
        _ => return Err(invalid(path("formats"))),
    }
    let instructions = match &files["instructions"] {
        Value::Null => String::new(),
        Value::String(text) if text.len() <= MAX_INSTRUCTIONS => text.clone(),
        _ => return Err(invalid(path("instructions"))),
    };
    Ok(Some(FileMode {
        formats,
        max_files,
        max_bytes: max_mb * 1024 * 1024,
        instructions,
    }))
}

/// CLIs whose writes the Runner can confine to the working directory.
pub(crate) fn runtime_supported(runtime: Runtime) -> bool {
    matches!(runtime, Runtime::Codex | Runtime::Claude)
}

/// The prompt section describing where and how to deliver files.
pub(super) fn contract(mode: &FileMode, runtime: Runtime, tools: &[String]) -> String {
    let mut text = String::from(
        "<file_output>\nThis step must produce files. Write every final deliverable into the ./output/ directory of your current working directory (create it if needed); only files there are collected. You may write only inside the working directory and have no network access.\n",
    );
    if mode.formats.is_empty() {
        text.push_str("Any file format is accepted.\n");
    } else {
        text.push_str(&format!(
            "Accepted formats (by extension): {}.\n",
            mode.formats.join(", ")
        ));
    }
    text.push_str(&format!(
        "At most {} files, each at most {} MiB, {} MiB in total. Give files clear names with the right extension.\n",
        mode.max_files,
        MAX_FILE_BYTES / 1024 / 1024,
        mode.max_bytes / 1024 / 1024
    ));
    if runtime == Runtime::Claude {
        text.push_str("In this mode you can create text files directly (Markdown, CSV, JSON, HTML, SVG...), but you cannot run programs, so binary formats (PDF, DOCX, XLSX, PPTX, images, video) cannot be produced here; say so instead of writing a fake file.\n");
    } else {
        text.push_str("You may run programs inside the working directory to build binary formats (documents, spreadsheets, slides, charts, video). Use only tools that already exist on this host; do not install anything.\n");
    }
    if tools.is_empty() {
        text.push_str("No file-production tools were detected on this host.\n");
    } else {
        text.push_str("File-production tools detected on this host:\n");
        for tool in tools.iter().take(20) {
            text.push_str("- ");
            text.push_str(&tool.chars().take(300).collect::<String>());
            text.push('\n');
        }
    }
    text.push_str("If no available tool can produce a requested format, say so plainly instead of faking it. In your final message, list each file you wrote and the tools you used.\n");
    if !mode.instructions.trim().is_empty() {
        text.push_str("Instructions for the files from the workflow author:\n");
        text.push_str(mode.instructions.trim());
        text.push('\n');
    }
    text.push_str("</file_output>");
    text
}

/// Media type by extension; unknown extensions are generic bytes.
pub(crate) fn mime(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "doc" => "application/msword",
        "xls" => "application/vnd.ms-excel",
        "ppt" => "application/vnd.ms-powerpoint",
        "csv" => "text/csv",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Whether the content looks like what its extension claims; text types must be UTF-8.
pub(super) fn matches_content(mime: &str, bytes: &[u8]) -> bool {
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    let riff = |kind: &[u8]| bytes.len() >= 12 && starts(b"RIFF") && &bytes[8..12] == kind;
    match mime {
        "application/pdf" => starts(b"%PDF-"),
        "application/zip"
        | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            starts(b"PK\x03\x04")
        }
        "application/msword" | "application/vnd.ms-excel" | "application/vnd.ms-powerpoint" => {
            starts(b"\xD0\xCF\x11\xE0")
        }
        "image/png" => starts(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => starts(b"\xFF\xD8\xFF"),
        "image/gif" => starts(b"GIF87a") || starts(b"GIF89a"),
        "image/webp" => riff(b"WEBP"),
        "audio/wav" => riff(b"WAVE"),
        "video/mp4" | "video/quicktime" => bytes.len() >= 12 && &bytes[4..8] == b"ftyp",
        "video/webm" => starts(b"\x1A\x45\xDF\xA3"),
        "audio/mpeg" => {
            starts(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
        }
        "image/svg+xml" => std::str::from_utf8(bytes).is_ok_and(|text| text.contains("<svg")),
        mime if mime.starts_with("text/")
            || matches!(
                mime,
                "application/json" | "application/xml" | "application/yaml"
            ) =>
        {
            std::str::from_utf8(bytes).is_ok()
        }
        _ => true,
    }
}

/// A display-safe file name: no separators, control or reserved characters.
pub(crate) fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.');
    let mut out: String = cleaned.chars().take(120).collect();
    if out.is_empty() {
        out = "file".into();
    }
    out
}

/// `dir` and `rest` joined with the separator the Runner's path already uses.
pub(crate) fn join(dir: &str, rest: &str) -> String {
    let separator = if dir.contains('\\') && !dir.contains('/') {
        '\\'
    } else {
        '/'
    };
    let mut path = dir.trim_end_matches(['/', '\\']).to_owned();
    for part in rest.split('/').filter(|part| !part.is_empty()) {
        path.push(separator);
        path.push_str(part);
    }
    path
}

fn limit_error(key: &'static str) -> Message {
    Message::new(key)
}

fn rejected(status: StatusCode, message: Message) -> ApiError {
    ApiError::localized(status, message)
}

/// One collected file, still in memory.
pub(crate) struct Collected {
    /// Path under `output/`, `/`-separated.
    pub path: String,
    pub name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Like `collect`, but no files is not an error: a conversation turn produces
/// files only when the user asked for some.
pub(crate) async fn collect_any(
    channel: &dyn HostChannel,
    dir: &str,
    mode: &FileMode,
) -> Result<Vec<Collected>> {
    match collect(channel, dir, mode).await {
        Err(error) if error.2.as_ref().map(|m| m.key.as_str()) == Some("workflow.agentNoFiles") => {
            Ok(Vec::new())
        }
        other => other,
    }
}

/// Deletes collected files from the host's output folder (best effort), so the
/// next turn starts with an empty one.
pub(crate) async fn remove(channel: &dyn HostChannel, dir: &str, files: &[Collected]) {
    for file in files {
        let _ = channel
            .delete_file(&join(dir, &format!("{OUTPUT_DIR}/{}", file.path)))
            .await;
    }
}

/// The prompt section when this conversation cannot deliver files, and why.
pub(crate) fn conversation_unavailable(reason: &str) -> String {
    format!(
        "<file_output>\nThis conversation cannot deliver files: {reason}. If the user asks for a file, say so plainly with that reason and offer the content as text instead. Do not claim to have created a file.\n</file_output>\n\n"
    )
}

/// The prompt section for a conversation turn that may produce files.
pub(crate) fn conversation_contract(runtime: Runtime, confined: bool, tools: &[String]) -> String {
    let mode = FileMode::conversation();
    let mut text = String::from(
        "<file_output>\nIf the user asks you to create a file (document, spreadsheet, slides, chart, image, video, Markdown, JSON...), write the finished file into the ./output/ directory of your current working directory; every file there is attached to your reply and then removed from the host, so do not leave drafts in it. ",
    );
    if confined {
        text.push_str(
            "You may write only inside your working directory and have no network access. ",
        );
    }
    text.push_str(&format!(
        "At most {} files, each at most {} MiB, {} MiB in total.\n",
        mode.max_files,
        MAX_FILE_BYTES / 1024 / 1024,
        mode.max_bytes / 1024 / 1024
    ));
    if runtime == Runtime::Claude && confined {
        text.push_str("You can create text files directly (Markdown, CSV, JSON, HTML, SVG), but you cannot run programs here, so say so instead of producing binary formats.\n");
    } else {
        text.push_str("Use only tools that already exist on this host; do not install anything.\n");
    }
    if !tools.is_empty() {
        text.push_str("File-production tools on this host: ");
        text.push_str(
            &tools
                .iter()
                .take(20)
                .map(|tool| tool.chars().take(200).collect::<String>())
                .collect::<Vec<_>>()
                .join("; "),
        );
        text.push('\n');
    }
    text.push_str("Name each file you produced in your reply.\n</file_output>\n\n");
    text
}

/// Every regular file under `<dir>/output`, within the node's limits. Symbolic
/// links are never followed; anything over a limit fails the node.
pub(super) async fn collect(
    channel: &dyn HostChannel,
    dir: &str,
    mode: &FileMode,
) -> Result<Vec<Collected>> {
    let mut pending = vec![(String::new(), 0usize)];
    let mut found = Vec::new();
    let mut seen = 0usize;
    let mut total = 0u64;
    while let Some((relative, depth)) = pending.pop() {
        let entries = match channel
            .list_dir(&join(dir, &format!("{OUTPUT_DIR}/{relative}")))
            .await
        {
            Ok(entries) => entries,
            // No output directory at all reads as "no files".
            Err(_) if relative.is_empty() => Vec::new(),
            Err(error) => return Err(error),
        };
        for entry in entries {
            seen += 1;
            if seen > MAX_ENTRIES {
                return Err(rejected(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    limit_error("workflow.agentFilesTooMany")
                        .with("max", mode.max_files.to_string()),
                ));
            }
            if entry.name.is_empty() || entry.name == "." || entry.name == ".." {
                continue;
            }
            let path = if relative.is_empty() {
                entry.name.clone()
            } else {
                format!("{relative}/{}", entry.name)
            };
            match entry.kind.as_str() {
                "directory" if depth + 1 < MAX_DEPTH => pending.push((path, depth + 1)),
                "file" => {
                    if entry.size > MAX_FILE_BYTES {
                        return Err(rejected(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            Message::new("workflow.agentFileTooLarge")
                                .with("name", sanitize(&path))
                                .with("limit", (MAX_FILE_BYTES / 1024 / 1024).to_string()),
                        ));
                    }
                    found.push((path, entry.size));
                }
                // Links and anything else are skipped, never followed.
                _ => {}
            }
        }
    }
    found.sort();
    if found.len() > mode.max_files {
        return Err(rejected(
            StatusCode::PAYLOAD_TOO_LARGE,
            limit_error("workflow.agentFilesTooMany").with("max", mode.max_files.to_string()),
        ));
    }
    let mut files = Vec::new();
    for (path, _) in found {
        let name = sanitize(path.rsplit('/').next().unwrap_or(&path));
        let extension = name
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        if !mode.formats.is_empty() && !mode.formats.contains(&extension) {
            return Err(rejected(
                StatusCode::UNPROCESSABLE_ENTITY,
                Message::new("workflow.agentFileFormat")
                    .with("name", name.clone())
                    .with("formats", mode.formats.join(", ")),
            ));
        }
        let (_, mut stream) = channel
            .read_file(&join(dir, &format!("{OUTPUT_DIR}/{path}")))
            .await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len() as u64 + chunk.len() as u64 > MAX_FILE_BYTES {
                return Err(rejected(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Message::new("workflow.agentFileTooLarge")
                        .with("name", name.clone())
                        .with("limit", (MAX_FILE_BYTES / 1024 / 1024).to_string()),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        total += bytes.len() as u64;
        if total > mode.max_bytes {
            return Err(rejected(
                StatusCode::PAYLOAD_TOO_LARGE,
                Message::new("workflow.agentFilesTotal")
                    .with("limit", (mode.max_bytes / 1024 / 1024).to_string()),
            ));
        }
        let mime = mime(&name);
        if !matches_content(mime, &bytes) {
            return Err(rejected(
                StatusCode::UNPROCESSABLE_ENTITY,
                Message::new("workflow.agentFileMismatch").with("name", name.clone()),
            ));
        }
        files.push(Collected {
            path,
            name,
            mime,
            bytes,
        });
    }
    if files.is_empty() {
        return Err(rejected(
            StatusCode::UNPROCESSABLE_ENTITY,
            Message::new("workflow.agentNoFiles"),
        ));
    }
    Ok(files)
}

/// Removes the per-run working directory, best effort and bounded: files first,
/// then directories bottom-up.
pub(super) async fn cleanup(channel: &dyn HostChannel, dir: &str) {
    let mut order = Vec::new();
    let mut pending = vec![(dir.to_owned(), 0usize)];
    while let Some((path, depth)) = pending.pop() {
        order.push((path.clone(), true));
        let Ok(entries) = channel.list_dir(&path).await else {
            continue;
        };
        for entry in entries {
            if order.len() > 2000
                || entry.name.is_empty()
                || entry.name == "."
                || entry.name == ".."
            {
                continue;
            }
            let child = join(&path, &entry.name);
            if entry.kind == "directory" && depth < 8 {
                pending.push((child, depth + 1));
            } else {
                order.push((child, false));
            }
        }
    }
    // Files and links before the directories that hold them, deepest first.
    order.sort_by_key(|(path, directory)| (*directory, std::cmp::Reverse(path.len())));
    for (path, _) in order {
        let _ = channel.delete_file(&path).await;
    }
}

/// `GET /nodes/{id}/file-tools`: what this Runner's execution user can use to
/// produce files, and whether it can confine an Agent's writes to a directory.
pub async fn host_tools(
    axum::extract::State(s): axum::extract::State<AppState>,
    scope: crate::accounts::workspace::Scope,
    crate::error::Path((_, id)): crate::error::Path<(String, crate::snowflake::Id)>,
) -> Result<crate::error::Json<Value>> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    scope.require("hosts.read")?;
    scope.require("ai-agents.probe")?;
    let host = crate::entities::hosts::Entity::find_by_id(id)
        .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if !crate::models::runner_connections().contains(&host.connection.as_str()) {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("hosts.fileToolsRunnerOnly"),
        ));
    }
    let channel = crate::hosts::remote::open(&s, &scope.workspace_id, &id).await?;
    // An empty directory is a catalogue inspection: nothing is created on the host.
    let probe = channel.agent_probe(Runtime::Codex, "", None).await?;
    Ok(crate::error::Json(json!({
        "tools": probe.file_tools,
        "write_workdir": probe.write_workdir,
    })))
}

/// One workflow file as node output. This is the only place that stores a file,
/// so a shared file-artifact helper can replace it later.
pub(super) async fn store(
    s: &AppState,
    run: &workflow_executions::Model,
    node: &str,
    file: &Collected,
) -> Result<(Asset, Value)> {
    let backend = crate::artifact_store::ArtifactStore::for_state(s).await?;
    let storage = Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    };
    let asset = storage
        .stage_bytes(
            run.workspace_id,
            &file.bytes,
            file.mime,
            run.id,
            node,
            run.triggered_by,
        )
        .await?;
    let tx = sea_orm::TransactionTrait::begin(&s.db).await?;
    storage
        .attach(&tx, run.workspace_id, asset.id, "workflow", run.workflow_id)
        .await?;
    tx.commit().await?;
    let reference = Asset {
        asset: format!("sha256:{}", asset.sha256),
        mime: asset.mime.clone(),
        name: file.name.clone(),
        size: asset.size as u64,
    };
    let info = json!({
        "artifact_id": asset.id.to_string(),
        "name": file.name,
        "mime": asset.mime,
        "size": asset.size,
        "sha256": asset.sha256,
        "download_url": format!(
            "/api/workspaces/{}/workflows/{}/assets/{}",
            run.workspace_id, run.workflow_id, asset.sha256
        ),
        "meta": {"path": file.path},
    });
    Ok((reference, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_settings_default_and_reject_bad_values() {
        assert_eq!(mode(&json!({}), "").unwrap(), None);
        assert_eq!(mode(&json!({"files":{"enabled":false}}), "").unwrap(), None);
        let on = mode(&json!({"files":{"enabled":true}}), "")
            .unwrap()
            .unwrap();
        assert_eq!((on.max_files, on.max_bytes), (10, 100 * 1024 * 1024));
        assert!(on.formats.is_empty());
        let picked = mode(
            &json!({"files":{"enabled":true,"formats":[".PPTX","pdf","pdf"],"max_files":3,"max_total_mb":500}}),
            "",
        )
        .unwrap()
        .unwrap();
        assert_eq!(picked.formats, ["pptx", "pdf"]);
        assert_eq!(picked.max_bytes, 500 * 1024 * 1024);
        for bad in [
            json!({"files":{"enabled":true,"max_total_mb":501}}),
            json!({"files":{"enabled":true,"max_files":0}}),
            json!({"files":{"enabled":true,"max_files":51}}),
            json!({"files":{"enabled":true,"formats":["../x"]}}),
            json!({"files":{"enabled":true,"formats":"pdf"}}),
            json!({"files":{"enabled":true,"instructions":"x".repeat(5000)}}),
        ] {
            let error = mode(&bad, "/nodes/0").unwrap_err();
            assert!(
                format!("{error:?}").contains("workflow.agentFilesInvalid"),
                "{bad}"
            );
        }
    }

    #[test]
    fn content_must_match_its_extension() {
        assert!(matches_content(mime("r.pdf"), b"%PDF-1.7\n..."));
        assert!(!matches_content(mime("r.pdf"), b"<html>not a pdf</html>"));
        assert!(matches_content(mime("deck.PPTX"), b"PK\x03\x04rest"));
        assert!(!matches_content(mime("deck.pptx"), b"slides as text"));
        assert!(matches_content(mime("c.png"), b"\x89PNG\r\n\x1a\nIHDR"));
        assert!(matches_content(mime("v.mp4"), b"\0\0\0\x18ftypisom...."));
        assert!(matches_content(mime("d.svg"), b"<svg xmlns='x'></svg>"));
        assert!(!matches_content(mime("d.svg"), b"<html></html>"));
        assert!(matches_content(mime("t.csv"), "名称,值\n".as_bytes()));
        assert!(!matches_content(mime("t.json"), b"\xff\xfe\x00"));
        assert_eq!(mime("unknown.bin"), "application/octet-stream");
        assert!(matches_content(mime("unknown.bin"), b"\x00\x01"));
    }

    #[test]
    fn names_and_paths_are_safe() {
        assert_eq!(sanitize("周报 v1.pdf"), "周报 v1.pdf");
        assert_eq!(sanitize("..\\..\\evil:name?.md"), "_.._evil_name_.md");
        assert_eq!(sanitize("\u{0}"), "_");
        assert_eq!(sanitize(""), "file");
        assert_eq!(join("/w/run", "output/a/b.pdf"), "/w/run/output/a/b.pdf");
        assert_eq!(
            join(r"\\?\D:\state\run", "output/b.pdf"),
            r"\\?\D:\state\run\output\b.pdf"
        );
    }

    #[test]
    fn the_contract_names_formats_limits_tools_and_runtime_limits() {
        let mode = FileMode {
            formats: vec!["pptx".into()],
            max_files: 2,
            max_bytes: 5 * 1024 * 1024,
            instructions: "Use the company colours.".into(),
        };
        let codex = contract(&mode, Runtime::Codex, &["python 3.12 (python-pptx)".into()]);
        assert!(codex.contains("./output/") && codex.contains("pptx"));
        assert!(codex.contains("python-pptx") && codex.contains("do not install"));
        assert!(codex.contains("company colours"));
        let claude = contract(&mode, Runtime::Claude, &[]);
        assert!(claude.contains("cannot run programs"));
        assert!(claude.contains("No file-production tools"));
        assert!(runtime_supported(Runtime::Codex) && runtime_supported(Runtime::Claude));
        assert!(!runtime_supported(Runtime::Gemini));
    }
}
