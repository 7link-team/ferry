//! Markdown → a plain block tree the report template draws.
//!
//! The tree is data, never Typst markup: the template only places these strings
//! as text, so nothing a user or an AI writes can run Typst code, read files or
//! change the layout beyond the supported elements.
use pulldown_cmark::{
    Alignment, BlockQuoteKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use serde_json::{Value, json};

/// Nesting and size bounds keep the template's recursion and layout bounded.
pub(crate) const MAX_DEPTH: usize = 6;

#[derive(Default)]
struct Frame {
    kind: FrameKind,
    blocks: Vec<Value>,
    inlines: Vec<Value>,
    items: Vec<Vec<Value>>,
}

#[derive(Default, Clone)]
enum FrameKind {
    #[default]
    Root,
    Paragraph,
    Heading(u8),
    List(Option<u64>),
    Item,
    Quote(Option<&'static str>),
    Code,
    Table {
        align: Vec<&'static str>,
        head: Vec<Value>,
        rows: Vec<Vec<Value>>,
        row: Vec<Value>,
    },
    Cell,
    Inline(&'static str, String),
}

/// Parses the supported Markdown subset: headings, paragraphs, lists, emphasis,
/// strike-through, inline and block code, links, tables, quotes, GitHub-style
/// callouts (`> [!NOTE]`) and rules. Raw HTML and images become plain text.
pub(crate) fn blocks(markdown: &str) -> Vec<Value> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_GFM;
    let mut stack = vec![Frame::default()];
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::Paragraph => FrameKind::Paragraph,
                    Tag::Heading { level, .. } => FrameKind::Heading(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        _ => 4,
                    }),
                    Tag::List(start) => FrameKind::List(start),
                    Tag::Item => FrameKind::Item,
                    Tag::BlockQuote(kind) => FrameKind::Quote(kind.map(|kind| match kind {
                        BlockQuoteKind::Note => "note",
                        BlockQuoteKind::Tip => "tip",
                        BlockQuoteKind::Important => "important",
                        BlockQuoteKind::Warning => "warning",
                        BlockQuoteKind::Caution => "caution",
                    })),
                    Tag::CodeBlock(_) => FrameKind::Code,
                    Tag::Table(align) => FrameKind::Table {
                        align: align
                            .iter()
                            .map(|a| match a {
                                Alignment::Center => "center",
                                Alignment::Right => "right",
                                _ => "left",
                            })
                            .collect(),
                        head: vec![],
                        rows: vec![],
                        row: vec![],
                    },
                    Tag::TableHead | Tag::TableRow => continue,
                    Tag::TableCell => FrameKind::Cell,
                    Tag::Emphasis => FrameKind::Inline("i", String::new()),
                    Tag::Strong => FrameKind::Inline("b", String::new()),
                    Tag::Strikethrough => FrameKind::Inline("s", String::new()),
                    Tag::Link { dest_url, .. } => FrameKind::Inline("a", dest_url.to_string()),
                    // Images and other inline containers keep only their text.
                    _ => FrameKind::Inline("span", String::new()),
                };
                if stack.len() > MAX_DEPTH * 4 {
                    // Deep nesting is flattened into the current frame's text.
                    stack.push(Frame {
                        kind: FrameKind::Inline("span", String::new()),
                        ..Default::default()
                    });
                } else {
                    stack.push(Frame {
                        kind,
                        ..Default::default()
                    });
                }
            }
            Event::End(end) => {
                if matches!(end, TagEnd::TableHead | TagEnd::TableRow) {
                    finish_row(&mut stack, matches!(end, TagEnd::TableHead));
                    continue;
                }
                let Some(frame) = stack.pop() else { break };
                let parent = stack.last_mut().expect("root frame");
                close(frame, parent);
            }
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => {
                push_text(stack.last_mut().expect("root"), &text);
            }
            Event::Code(code) => {
                let frame = stack.last_mut().expect("root");
                frame.inlines.push(json!({"k":"code","t":code.to_string()}));
            }
            Event::SoftBreak => push_text(stack.last_mut().expect("root"), " "),
            Event::HardBreak => stack
                .last_mut()
                .expect("root")
                .inlines
                .push(json!({"k":"br"})),
            Event::Rule => push_block(stack.last_mut().expect("root"), json!({"k":"hr"})),
            Event::TaskListMarker(done) => push_text(
                stack.last_mut().expect("root"),
                if done { "☑ " } else { "☐ " },
            ),
            Event::FootnoteReference(name) => {
                push_text(stack.last_mut().expect("root"), &format!("[{name}]"))
            }
        }
    }
    while stack.len() > 1 {
        let frame = stack.pop().unwrap();
        close(frame, stack.last_mut().unwrap());
    }
    let mut root = stack.pop().unwrap();
    flush_inlines(&mut root);
    root.blocks
}

fn push_text(frame: &mut Frame, text: &str) {
    if let Some(last) = frame.inlines.last_mut() {
        if last["k"] == "t" {
            let joined = format!("{}{text}", last["t"].as_str().unwrap_or_default());
            last["t"] = json!(joined);
            return;
        }
    }
    frame.inlines.push(json!({"k":"t","t":text}));
}

/// Adds a block after turning any loose text before it into its own paragraph.
fn push_block(frame: &mut Frame, block: Value) {
    flush_inlines(frame);
    frame.blocks.push(block);
}

/// Loose inline content directly inside a block container becomes a paragraph.
fn flush_inlines(frame: &mut Frame) {
    if !frame.inlines.is_empty() {
        let inlines = std::mem::take(&mut frame.inlines);
        frame.blocks.push(json!({"k":"p","c":inlines}));
    }
}

fn finish_row(stack: &mut [Frame], head: bool) {
    if let Some(Frame {
        kind: FrameKind::Table {
            head: h, rows, row, ..
        },
        ..
    }) = stack.last_mut()
    {
        let cells = std::mem::take(row);
        if head {
            *h = cells;
        } else {
            rows.push(cells);
        }
    }
}

fn close(mut frame: Frame, parent: &mut Frame) {
    match frame.kind.clone() {
        FrameKind::Root => {}
        FrameKind::Paragraph => {
            if !frame.inlines.is_empty() {
                push_block(parent, json!({"k":"p","c":frame.inlines}));
            }
        }
        FrameKind::Heading(level) => {
            push_block(parent, json!({"k":"h","l":level,"c":frame.inlines}));
        }
        FrameKind::List(start) => {
            let items = frame.items;
            push_block(
                parent,
                match start {
                    Some(start) => json!({"k":"ol","start":start.max(1),"items":items}),
                    None => json!({"k":"ul","items":items}),
                },
            );
        }
        FrameKind::Item => {
            flush_inlines(&mut frame);
            parent.items.push(frame.blocks);
        }
        FrameKind::Quote(kind) => {
            flush_inlines(&mut frame);
            push_block(
                parent,
                match kind {
                    Some(kind) => json!({"k":"callout","kind":kind,"b":frame.blocks}),
                    None => json!({"k":"quote","b":frame.blocks}),
                },
            );
        }
        FrameKind::Code => {
            let text: String = frame
                .inlines
                .iter()
                .filter_map(|i| i["t"].as_str())
                .collect();
            push_block(parent, json!({"k":"code","t":text.trim_end_matches('\n')}));
        }
        FrameKind::Table {
            align, head, rows, ..
        } => {
            let columns = head
                .len()
                .max(rows.iter().map(Vec::len).max().unwrap_or(0))
                .max(1);
            let pad = |mut cells: Vec<Value>| {
                cells.resize(columns, json!([]));
                cells.truncate(columns);
                cells
            };
            let mut align: Vec<&str> = align;
            align.resize(columns, "left");
            push_block(
                parent,
                json!({
                    "k":"table",
                    "align":align,
                    "head":pad(head),
                    "rows":rows.into_iter().map(pad).collect::<Vec<_>>(),
                }),
            );
        }
        FrameKind::Cell => {
            if let FrameKind::Table { row, .. } = &mut parent.kind {
                row.push(Value::Array(frame.inlines));
            }
        }
        FrameKind::Inline(kind, url) => {
            if frame.inlines.is_empty() {
                return;
            }
            match kind {
                "span" => parent.inlines.extend(frame.inlines),
                "a" if safe_link(&url) => parent
                    .inlines
                    .push(json!({"k":"a","u":url,"c":frame.inlines})),
                // Other schemes (javascript:, file:, data:...) keep their text only.
                "a" => parent.inlines.extend(frame.inlines),
                kind => parent.inlines.push(json!({"k":kind,"c":frame.inlines})),
            }
        }
    }
}

pub(crate) fn safe_link(url: &str) -> bool {
    url.len() <= 2048
        && reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https" | "mailto"))
}

/// Plain text of an inline list, for limits and tests.
#[cfg(test)]
pub(crate) fn plain(inlines: &Value) -> String {
    let mut out = String::new();
    for i in inlines.as_array().into_iter().flatten() {
        if let Some(t) = i["t"].as_str() {
            out.push_str(t);
        }
        if let Some(c) = i.get("c") {
            out.push_str(&plain(c));
        }
    }
    out
}
