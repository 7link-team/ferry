use super::*;

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
/// Quote every potentially executable spreadsheet cell, including whitespace
/// prefixes. CSV quoting alone does not stop spreadsheet formula execution.
fn cell(value: &str) -> String {
    let trimmed = value.trim_start();
    if trimmed.starts_with(['=', '+', '-', '@']) || value.starts_with(['\t', '\r', '\n']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}
pub(super) fn render(value: &Value, format: &str) -> Result<(&'static str, &'static str, String)> {
    match format {
        "json" => Ok((
            "application/json; charset=utf-8",
            "attachment; filename=insight-report.json",
            encode(value)?,
        )),
        "html" => {
            let title = html(
                value["snapshot"]["config"]["name"]
                    .as_str()
                    .unwrap_or_default(),
            );
            let evidence = html(&serde_json::to_string_pretty(value).map_err(|_| bad("data"))?);
            Ok((
                "text/html; charset=utf-8",
                "attachment; filename=insight-report.html",
                format!(
                    "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'\"><title>{title}</title></head><body><h1>{title}</h1><pre>{evidence}</pre></body></html>"
                ),
            ))
        }
        "csv" => {
            let mut writer = csv::Writer::from_writer(Vec::new());
            let columns = [
                "repository_id",
                "sha",
                "author_key",
                "nickname",
                "author_name",
                "author_email",
                "author_date",
                "message",
                "additions",
                "deletions",
                "effectives",
            ];
            writer.write_record(columns).map_err(|_| bad("data"))?;
            if let Some(commits) = value["snapshot"]["commits"].as_array() {
                for commit in commits {
                    writer
                        .write_record(columns.iter().map(|column| {
                            let v = &commit[*column];
                            cell(
                                v.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| {
                                        if v.is_null() {
                                            String::new()
                                        } else {
                                            v.to_string()
                                        }
                                    })
                                    .as_str(),
                            )
                        }))
                        .map_err(|_| bad("data"))?;
                }
            }
            let bytes = writer.into_inner().map_err(|_| bad("data"))?;
            Ok((
                "text/csv; charset=utf-8",
                "attachment; filename=insight-report.csv",
                String::from_utf8(bytes).map_err(|_| bad("data"))?,
            ))
        }
        _ => Err(bad("format")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exported_evidence_escapes_html_and_spreadsheet_formulas() {
        let value = json!({"snapshot":{"config":{"name":"<script>alert(1)</script>"},"commits":[{"nickname":"=cmd()","author_name":"  +formula()","author_email":"@evil","message":"<img src=x onerror=alert(1)>,\"\n","sha":"abc"}]}});
        let (_, _, html) = render(&value, "html").unwrap();
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;script&gt;"));
        let (_, _, csv) = render(&value, "csv").unwrap();
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        let row = reader.records().next().unwrap().unwrap();
        assert_eq!(&row[3], "'=cmd()");
        assert_eq!(&row[4], "'  +formula()");
        assert_eq!(&row[5], "'@evil");
        assert_eq!(&row[7], "<img src=x onerror=alert(1)>,\"\n");
        assert_eq!(cell("\t=1"), "'\t=1");
        assert!(render(&value, "exe").is_err());
    }
}
