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
/// Bounds on what is walked under `output/`.
pub(crate) const OUTPUT_DIR: &str = "output";

/// The node's `files` settings, when file output is enabled.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileMode {
    /// Lower-case extensions; empty means any format.
    pub formats: Vec<String>,
    pub max_files: usize,
    pub max_bytes: u64,
    pub max_file_bytes: u64,
    pub max_entries: usize,
    pub instructions: String,
}
impl FileMode {
    /// A conversation turn: any format, the node defaults.
    pub(crate) fn conversation() -> Self {
        Self {
            formats: Vec::new(),
            max_files: usize::MAX,
            max_bytes: u64::MAX,
            max_file_bytes: u64::MAX,
            max_entries: usize::MAX,
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
    // Legacy per-node quota fields remain accepted; only platform policy sets quotas.
    for key in ["max_files", "max_total_mb"] {
        if !files[key].is_null() && files[key].as_u64().is_none_or(|value| value == 0) {
            return Err(invalid(path(key)));
        }
    }
    let max_files = usize::MAX;
    let mut formats = Vec::new();
    match &files["formats"] {
        Value::Null => {}
        Value::String(any) if any == "any" => {}
        Value::Array(list) => {
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
        Value::String(text) => text.clone(),
        _ => return Err(invalid(path("instructions"))),
    };
    Ok(Some(FileMode {
        formats,
        max_files,
        max_bytes: u64::MAX,
        max_file_bytes: u64::MAX,
        max_entries: usize::MAX,
        instructions,
    }))
}

pub(crate) fn apply_policy(
    mode: &mut FileMode,
    policy: &ferry_protocol::resources::ResourcePolicy,
) {
    mode.max_files = ferry_protocol::resources::byte_limit(policy.file_count);
    mode.max_entries = ferry_protocol::resources::byte_limit(policy.directory_entries);
    mode.max_bytes = policy.file_total_bytes.unwrap_or(u64::MAX);
    mode.max_file_bytes = policy.file_upload_bytes.unwrap_or(u64::MAX);
}

/// CLIs whose writes the Runner can confine to the working directory.
pub(crate) fn runtime_supported(runtime: Runtime) -> bool {
    matches!(runtime, Runtime::Codex | Runtime::Claude)
}

fn append_limits(text: &mut String, mode: &FileMode) {
    if mode.max_files != usize::MAX {
        text.push_str(&format!("At most {} files.\n", mode.max_files));
    }
    if mode.max_file_bytes != u64::MAX {
        text.push_str(&format!(
            "Each file at most {} bytes.\n",
            mode.max_file_bytes
        ));
    }
    if mode.max_bytes != u64::MAX {
        text.push_str(&format!("At most {} bytes in total.\n", mode.max_bytes));
    }
    text.push_str("Give files clear names with the right extension.\n");
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
    append_limits(&mut text, mode);
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

async fn matches_file(mime: &str, path: &std::path::Path) -> Result<bool> {
    use tokio::io::AsyncReadExt;
    let text = mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/yaml" | "image/svg+xml"
        );
    let mut file = tokio::fs::File::open(path).await?;
    if !text {
        let mut prefix = [0; 12];
        let mut count = 0;
        while count < prefix.len() {
            let read = file.read(&mut prefix[count..]).await?;
            if read == 0 {
                break;
            }
            count += read;
        }
        return Ok(matches_content(mime, &prefix[..count]));
    }
    let mut buffer = vec![0; 64 * 1024];
    let mut incomplete = Vec::new();
    let mut suffix = Vec::new();
    let mut svg = false;
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let mut scan = std::mem::take(&mut suffix);
        scan.extend_from_slice(&buffer[..count]);
        svg |= scan.windows(4).any(|bytes| bytes == b"<svg");
        suffix = scan[scan.len().saturating_sub(3)..].to_vec();
        incomplete.extend_from_slice(&buffer[..count]);
        match std::str::from_utf8(&incomplete) {
            Ok(_) => incomplete.clear(),
            Err(error) if error.error_len().is_none() => {
                incomplete = incomplete[error.valid_up_to()..].to_vec();
            }
            Err(_) => return Ok(false),
        }
    }
    Ok(incomplete.is_empty() && (mime != "image/svg+xml" || svg))
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

/// One collected file, spooled privately and removed when its owner is dropped.
pub(crate) struct Collected {
    /// Path under `output/`, `/`-separated.
    pub path: String,
    pub name: String,
    pub mime: &'static str,
    pub content: tempfile::TempPath,
    pub size: u64,
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
    append_limits(&mut text, &mode);
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
    let mut total = 0u64;
    let mut seen = 0usize;
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
            seen = seen
                .checked_add(1)
                .ok_or_else(|| invalid("/files".into()))?;
            if seen > mode.max_entries {
                return Err(rejected(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    limit_error("workflow.agentFilesTooMany")
                        .with("max", mode.max_entries.to_string()),
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
                "directory" => pending.push((path, depth + 1)),
                "file" => {
                    if entry.size > mode.max_file_bytes {
                        return Err(rejected(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            Message::new("workflow.agentFileTooLarge")
                                .with("name", sanitize(&path))
                                .with(
                                    "limit",
                                    crate::content_assets::size_text(mode.max_file_bytes as i64),
                                ),
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
        use tokio::io::AsyncWriteExt;
        let content = tempfile::NamedTempFile::new().map_err(ApiError::from)?;
        let mut output = tokio::fs::File::from_std(content.reopen().map_err(ApiError::from)?);
        let mut size = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| invalid("/files".into()))?;
            if size > mode.max_file_bytes {
                return Err(rejected(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Message::new("workflow.agentFileTooLarge")
                        .with("name", name.clone())
                        .with(
                            "limit",
                            crate::content_assets::size_text(mode.max_file_bytes as i64),
                        ),
                ));
            }
            if total
                .checked_add(size)
                .is_none_or(|value| value > mode.max_bytes)
            {
                return Err(rejected(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Message::new("workflow.agentFilesTotal").with(
                        "limit",
                        crate::content_assets::size_text(mode.max_bytes as i64),
                    ),
                ));
            }
            output.write_all(&chunk).await.map_err(ApiError::from)?;
        }
        output.flush().await.map_err(ApiError::from)?;
        drop(output);
        total = total
            .checked_add(size)
            .ok_or_else(|| invalid("/files".into()))?;
        let mime = mime(&name);
        if !matches_file(mime, content.path()).await? {
            return Err(rejected(
                StatusCode::UNPROCESSABLE_ENTITY,
                Message::new("workflow.agentFileMismatch").with("name", name.clone()),
            ));
        }
        files.push(Collected {
            path,
            name,
            mime,
            content: content.into_temp_path(),
            size,
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

/// One workflow file as node output, stored through the shared file-artifact helper.
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
    // The shared file-artifact helper: same output shape and signed download
    // link as report.render, so notifications can attach these files too.
    let stored = super::artifacts::store_file(
        s,
        &storage,
        &super::artifacts::Origin {
            workspace: run.workspace_id,
            workflow: run.workflow_id,
            execution: run.id,
            node,
        },
        file.content.as_ref(),
        file.mime,
        &file.name,
        json!({"path": file.path}),
        crate::file_links::DEFAULT_TTL_DAYS,
    )
    .await?;
    Ok((stored.binary, stored.json))
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
        assert_eq!((on.max_files, on.max_bytes), (usize::MAX, u64::MAX));
        assert!(on.formats.is_empty());
        let picked = mode(
            &json!({"files":{"enabled":true,"formats":[".PPTX","pdf","pdf"],"max_files":3,"max_total_mb":500}}),
            "",
        )
        .unwrap()
        .unwrap();
        assert_eq!(picked.formats, ["pptx", "pdf"]);
        assert_eq!(picked.max_bytes, u64::MAX);
        assert_eq!(picked.max_files, usize::MAX);
        let mut limited = picked;
        apply_policy(
            &mut limited,
            &ferry_protocol::resources::ResourcePolicy {
                file_count: Some(3),
                file_total_bytes: Some(500),
                file_upload_bytes: Some(200),
                ..Default::default()
            },
        );
        assert_eq!(
            (limited.max_files, limited.max_bytes, limited.max_file_bytes),
            (3, 500, 200)
        );
        for bad in [
            json!({"files":{"enabled":true,"max_files":0}}),
            json!({"files":{"enabled":true,"formats":["../x"]}}),
            json!({"files":{"enabled":true,"formats":"pdf"}}),
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
            max_file_bytes: u64::MAX,
            max_entries: usize::MAX,
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
