//! PDF reports for the `report.render` workflow node.
//!
//! Typst lays out a fixed, trusted template (`template.typ`). The report's
//! content reaches it only as JSON data through an in-memory `/data.json`;
//! Markdown is parsed here into a plain block tree, so report text is always
//! drawn as text and can never run Typst code. The compiler's world is sealed:
//! the template, the data and the embedded fonts are the only files it can
//! reach, and it has no package or network access.
mod markdown;
#[cfg(test)]
mod tests;

use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Dict, IntoValue};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;

static REGULAR: &[u8] = include_bytes!("../../assets/report-fonts/NotoSansSC-Regular.otf");
static BOLD: &[u8] = include_bytes!("../../assets/report-fonts/NotoSansSC-Bold.otf");
const TEMPLATE: &str = include_str!("template.typ");

/// Upper bounds for one report; a larger request is refused before layout.
pub(crate) const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_PAGES: usize = 200;
pub(crate) const MAX_SECTIONS: usize = 100;
pub(crate) const MAX_METRICS: usize = 24;
pub(crate) const RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

struct Fonts {
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

static FONTS: LazyLock<Fonts> = LazyLock::new(|| {
    let fonts: Vec<Font> = [REGULAR, BOLD]
        .into_iter()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    Fonts {
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
    }
});

/// At most two reports lay out at once; each holds a blocking thread.
static SLOTS: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(2));

/// What the node asks for, after expressions are resolved.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Report {
    pub title: String,
    pub subtitle: String,
    pub period: String,
    /// Markdown shown under the key figures on the first page.
    pub summary: String,
    pub metrics: Vec<Metric>,
    pub sections: Vec<Section>,
    /// `zh` or `en`: labels and hyphenation.
    pub language: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Metric {
    pub label: String,
    pub value: Value,
    pub note: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Section {
    pub title: String,
    pub markdown: String,
    /// Optional structured table drawn after the Markdown.
    pub table: Option<Table>,
    pub page_break: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// A rendered report.
pub(crate) struct Rendered {
    pub pdf: Vec<u8>,
    pub pages: usize,
}

/// Why a report could not be produced, in user terms.
#[derive(Debug, PartialEq)]
pub(crate) enum Problem {
    /// The title is required.
    MissingTitle,
    /// The content exceeds a limit: `(what, limit)`.
    TooLarge(&'static str, usize),
    /// Layout produced more pages than allowed.
    TooManyPages(usize),
    /// Layout did not finish in time.
    Timeout,
    /// The layout engine rejected the document; logged with the details.
    Layout(String),
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

/// Builds the template's data. Separate from layout so it is cheap to test.
pub(crate) fn document(report: &Report, generated: &str) -> Result<Value, Problem> {
    let title = report.title.trim();
    if title.is_empty() {
        return Err(Problem::MissingTitle);
    }
    if report.sections.len() > MAX_SECTIONS {
        return Err(Problem::TooLarge("sections", MAX_SECTIONS));
    }
    if report.metrics.len() > MAX_METRICS {
        return Err(Problem::TooLarge("metrics", MAX_METRICS));
    }
    let english = report.language.starts_with("en");
    let sections: Vec<Value> = report
        .sections
        .iter()
        .map(|section| {
            let mut blocks = markdown::blocks(&section.markdown);
            if let Some(table) = &section.table {
                if !table.columns.is_empty() {
                    let cell = |value: &Value| json!([{"k":"t","t":text(value)}]);
                    let columns = table.columns.len();
                    blocks.push(json!({
                        "k":"table",
                        "align":table.rows.first().map(|row| row.iter().take(columns).map(|v| if v.is_number() {"right"} else {"left"}).collect::<Vec<_>>()).unwrap_or_default().into_iter().chain(std::iter::repeat("left")).take(columns).collect::<Vec<_>>(),
                        "head":table.columns.iter().map(|c| json!([{"k":"t","t":c}])).collect::<Vec<_>>(),
                        "rows":table.rows.iter().map(|row| {
                            let mut cells: Vec<Value> = row.iter().take(columns).map(cell).collect();
                            cells.resize(columns, json!([]));
                            cells
                        }).collect::<Vec<_>>(),
                    }));
                }
            }
            json!({"title":section.title.trim(),"blocks":blocks,"page_break":section.page_break})
        })
        .collect();
    let data = json!({
        "title": title,
        "subtitle": report.subtitle.trim(),
        "period": report.period.trim(),
        "generated": generated,
        "lang": if english { "en" } else { "zh" },
        "labels": if english {
            json!({"generated":"Generated","page":"Page {page} of {total}"})
        } else {
            json!({"generated":"生成于","page":"第 {page} / {total} 页"})
        },
        "metrics": report.metrics.iter().map(|m| json!({"label":m.label.trim(),"value":text(&m.value),"note":m.note.trim()})).collect::<Vec<_>>(),
        "summary": markdown::blocks(&report.summary),
        "sections": sections,
    });
    let size = data.to_string().len();
    if size > MAX_INPUT_BYTES {
        return Err(Problem::TooLarge("bytes", MAX_INPUT_BYTES));
    }
    Ok(data)
}

/// Lays out the report and exports the PDF, off the async runtime and bounded
/// in time, pages and concurrency.
pub(crate) async fn render(report: &Report, generated: &str) -> Result<Rendered, Problem> {
    let data = document(report, generated)?;
    let _slot = SLOTS.acquire().await.map_err(|_| Problem::Timeout)?;
    let job = tokio::task::spawn_blocking(move || layout(&data));
    match tokio::time::timeout(RENDER_TIMEOUT, job).await {
        Ok(Ok(result)) => result,
        Ok(Err(join)) => Err(Problem::Layout(join.to_string())),
        // The thread finishes on its own; its result is discarded.
        Err(_) => Err(Problem::Timeout),
    }
}

/// Synchronous layout + PDF export of prepared template data.
pub(crate) fn layout(data: &Value) -> Result<Rendered, Problem> {
    let world = ReportWorld::new(data);
    let document = typst::compile::<PagedDocument>(&world)
        .output
        .map_err(|errors| {
            Problem::Layout(
                errors
                    .iter()
                    .map(|e| e.message.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
    let pages = document.pages().len();
    if pages > MAX_PAGES {
        return Err(Problem::TooManyPages(MAX_PAGES));
    }
    let pdf = typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|errors| {
        Problem::Layout(
            errors
                .iter()
                .map(|e| e.message.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    Ok(Rendered { pdf, pages })
}

fn file_id(path: &str) -> FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new(path).expect("static path"),
    )
    .intern()
}

struct ReportWorld {
    library: LazyHash<Library>,
    main: Source,
    data_id: FileId,
    data: Bytes,
}

impl ReportWorld {
    fn new(data: &Value) -> Self {
        Self {
            // No inputs, no packages: the template reads only /data.json.
            library: LazyHash::new(Library::builder().with_inputs(Dict::new()).build()),
            main: Source::new(file_id("/main.typ"), TEMPLATE.to_owned()),
            data_id: file_id("/data.json"),
            data: Bytes::from_string(data.to_string()),
        }
    }
}

impl World for ReportWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.book
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.data_id {
            Ok(self.data.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }
    fn font(&self, index: usize) -> Option<Font> {
        FONTS.fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        // The template takes its date from the data; `datetime.today()` is not used.
        None
    }
}

#[allow(dead_code)]
fn _assert_into_value() {
    let _ = "x".into_value();
}
