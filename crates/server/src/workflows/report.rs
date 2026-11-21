//! `report.render`: turns structured content (title, key figures, Markdown
//! sections, tables — typically the JSON an `ai.agent.run` step produced) into
//! a formatted document file.
//!
//! Extension point: each output format is one entry in [`FORMATS`] with a
//! renderer that maps the same [`crate::report_pdf::Report`] content to bytes.
//! Adding Word, Excel or slides means adding an entry here (and the value to
//! the contract's `format` enum); storage, links, the output shape and
//! notification attachments are shared through [`super::artifacts`].
use super::artifacts::{self, Origin};
use crate::content_assets::Store;
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use crate::report_pdf::{self, Problem, Report};
use crate::state::AppState;
use axum::http::StatusCode;
use ferry_protocol::sandbox::Item;
use serde_json::{Value, json};

/// One output format: its contract value, media type, file extension.
pub(crate) struct Format {
    pub name: &'static str,
    pub mime: &'static str,
    pub extension: &'static str,
}

pub(crate) const FORMATS: &[Format] = &[Format {
    name: "pdf",
    mime: "application/pdf",
    extension: "pdf",
}];

pub(crate) fn supports(spec: &Value) -> bool {
    spec["type"] == "report.render"
        && FORMATS
            .iter()
            .any(|format| spec["params"]["format"].as_str().unwrap_or("pdf") == format.name)
}

fn problem(problem: Problem) -> ApiError {
    match problem {
        Problem::MissingTitle => ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.reportTitleRequired"),
        ),
        Problem::TooLarge(what, limit) => ApiError::localized(
            StatusCode::PAYLOAD_TOO_LARGE,
            Message::new(match what {
                "sections" => "workflow.reportTooManySections",
                "metrics" => "workflow.reportTooManyMetrics",
                _ => "workflow.reportTooLarge",
            })
            .with(
                "limit",
                if what == "bytes" {
                    crate::content_assets::size_text(limit as i64)
                } else {
                    limit.to_string()
                },
            ),
        ),
        Problem::TooManyPages(limit) => ApiError::localized(
            StatusCode::PAYLOAD_TOO_LARGE,
            Message::new("workflow.reportTooManyPages").with("limit", limit.to_string()),
        ),
        Problem::Timeout(seconds) => ApiError::localized(
            StatusCode::SERVICE_UNAVAILABLE,
            Message::new("workflow.reportTimeout").with("seconds", seconds.to_string()),
        ),
        // The template is fixed, so a layout error is ours, not the user's.
        Problem::Layout(detail) => crate::error::internal_fault(format!("report layout: {detail}")),
    }
}

/// `generated` text from the run clock (`$now` encoding: `iso`, `zone`).
pub(crate) fn generated(clock: &Value) -> String {
    let iso = clock["iso"].as_str().unwrap_or_default();
    let shown = chrono::DateTime::parse_from_rfc3339(iso)
        .map(|at| at.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| iso.to_owned());
    match clock["zone"].as_str() {
        Some(zone) if !zone.is_empty() && zone != "local" => format!("{shown} ({zone})"),
        _ => shown,
    }
}

/// Renders one file per input item.
pub(crate) async fn execute(
    s: &AppState,
    storage: &Store<'_>,
    origin: &Origin<'_>,
    clock: &Value,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let mut output = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let p = params
            .get(index)
            .or_else(|| params.first())
            .ok_or_else(super::media::invalid)?;
        let format = p["format"].as_str().unwrap_or("pdf");
        let format = FORMATS
            .iter()
            .find(|f| f.name == format)
            .ok_or_else(super::media::invalid)?;
        let report: Report = serde_json::from_value(json!({
            "title": p["title"],
            "subtitle": p.get("subtitle").cloned().unwrap_or(json!("")),
            "period": p.get("period").cloned().unwrap_or(json!("")),
            "summary": p.get("summary").cloned().unwrap_or(json!("")),
            "metrics": p.get("metrics").cloned().unwrap_or(json!([])),
            "sections": p.get("sections").cloned().unwrap_or(json!([])),
            "language": p.get("language").cloned().unwrap_or(json!("zh")),
        }))
        .map_err(|error| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("workflow.reportContentInvalid").with("reason", error.to_string()),
            )
        })?;
        let (bytes, meta) = match format.name {
            "pdf" => {
                let rendered = report_pdf::render(&report, &generated(clock), &policy)
                    .await
                    .map_err(problem)?;
                (
                    rendered.pdf,
                    json!({"pages": rendered.pages, "format": "pdf"}),
                )
            }
            _ => return Err(super::media::invalid()),
        };
        let name = artifacts::file_name(
            p["file_name"].as_str().unwrap_or_default(),
            if report.title.trim().is_empty() {
                "report"
            } else {
                report.title.trim()
            },
            format.extension,
        );
        let ttl = p["link_ttl_days"]
            .as_i64()
            .unwrap_or(crate::file_links::DEFAULT_TTL_DAYS);
        let file =
            artifacts::store(s, storage, origin, &bytes, format.mime, &name, meta, ttl).await?;
        let field = p["output_field"]
            .as_str()
            .filter(|f| !f.is_empty())
            .unwrap_or("file");
        output.push(artifacts::with_file(item, field, file));
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
