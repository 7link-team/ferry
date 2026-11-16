use super::*;

fn sample() -> Report {
    serde_json::from_value(json!({
        "title": "Codeup 每周代码洞察",
        "subtitle": "物业＋本地生活融合系统 · 会计系统",
        "period": "2026-11-09 至 2026-11-15",
        "summary": "本周共 **128** 次提交，覆盖 `7link-v3` 与 `E7ACCT` 两条产品线。\n\n> [!NOTE]\n> 仅根据提交描述归纳，不代表已上线。",
        "metrics": [
            {"label":"提交","value":128,"note":"含 12 次合并"},
            {"label":"贡献者","value":9},
            {"label":"仓库","value":"14 / 115"},
            {"label":"新增行","value":"+8,412","note":"删除 3,107"}
        ],
        "sections": [
            {"title":"物业＋本地生活融合系统","markdown":"## 后端\n- 新增：工单派发支持按楼栋排班\n- 修复：账单重复推送\n\n## App\n1. 优化首页加载\n2. 重构消息中心\n\n| 仓库 | 提交 | 作者 |\n|---|---:|:---:|\n| 7link-v3 | 42 | 张三 |\n| 7link-mos | 17 | 李四 |","page_break":true},
            {"title":"个人贡献","markdown":"按平台身份合并别名。","table":{"columns":["成员","提交","参与项目"],"rows":[["张三",42,"物业系统"],["李四",17,"会计系统"],["Alice",9,"物业系统"]]}}
        ]
    })).unwrap()
}

#[test]
fn renders_a_valid_cjk_pdf_with_cover_sections_and_tables() {
    let report = sample();
    let data = document(&report, "2026-11-16 09:00 (Asia/Shanghai)").unwrap();
    let rendered = layout(&data).unwrap();
    assert!(rendered.pdf.starts_with(b"%PDF-"));
    assert!(rendered.pages >= 2, "page break gives a second page");
    // The embedded CJK font is used (its name is in the PDF's font dictionary).
    let text = String::from_utf8_lossy(&rendered.pdf);
    assert!(text.contains("NotoSansSC"), "CJK font embedded");
    if let Ok(path) = std::env::var("FERRY_REPORT_SAMPLE") {
        std::fs::write(path, &rendered.pdf).unwrap();
    }
}

#[test]
fn report_text_is_never_typst_code() {
    // Typst markup and code in every user field stays literal text.
    let hostile = r##"#import "/etc/passwd": * #read("/data.json") $x$ #let a = 1 @label <x> = Heading ] ) } "##;
    let report: Report = serde_json::from_value(json!({
        "title": hostile, "subtitle": hostile, "period": hostile, "summary": hostile,
        "metrics": [{"label": hostile, "value": hostile, "note": hostile}],
        "sections": [{"title": hostile, "markdown": format!("{hostile}\n\n```\n{hostile}\n```\n\n[x](javascript:alert(1)) [y](file:///etc/passwd)"),
                      "table": {"columns": [hostile], "rows": [[hostile]]}}]
    }))
    .unwrap();
    let data = document(&report, hostile).unwrap();
    let rendered = layout(&data).expect("hostile text renders as text");
    assert!(rendered.pdf.starts_with(b"%PDF-"));
    // Unsafe link schemes are dropped to their text.
    let blocks = data["sections"][0]["blocks"].to_string();
    assert!(!blocks.contains("\"u\":\"javascript:"));
    assert!(!blocks.contains("\"u\":\"file:"));
}

#[test]
fn markdown_subset_becomes_a_block_tree() {
    let blocks = markdown::blocks(
        "# T\n\nA **b** _i_ ~~s~~ `c` [l](https://example.com)\n\n- one\n  - nested\n- two\n\n> [!WARNING]\n> careful\n\n---",
    );
    let kinds: Vec<&str> = blocks.iter().map(|b| b["k"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["h", "p", "ul", "callout", "hr"]);
    assert_eq!(blocks[2]["items"].as_array().unwrap().len(), 2);
    assert_eq!(blocks[3]["kind"], "warning");
    let inline = &blocks[1]["c"];
    let found: Vec<&str> = inline
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["k"].as_str().unwrap())
        .collect();
    for kind in ["b", "i", "s", "code", "a"] {
        assert!(found.contains(&kind), "{kind} in {found:?}");
    }
    assert_eq!(markdown::plain(inline), "A b i s c l");
}

#[test]
fn limits_and_required_title_are_named() {
    let mut report = sample();
    report.title = " ".into();
    assert_eq!(document(&report, "").unwrap_err(), Problem::MissingTitle);
    let mut report = sample();
    report.metrics = (0..=MAX_METRICS).map(|_| Metric::default()).collect();
    assert_eq!(
        document(&report, "").unwrap_err(),
        Problem::TooLarge("metrics", MAX_METRICS)
    );
    let mut report = sample();
    report.summary = "字".repeat(MAX_INPUT_BYTES);
    assert_eq!(
        document(&report, "").unwrap_err(),
        Problem::TooLarge("bytes", MAX_INPUT_BYTES)
    );
}

#[test]
fn table_columns_are_sized_by_their_longest_cell() {
    let cell = |t: &str| serde_json::json!([{"k":"t","t":t}]);
    let head = vec![cell("仓库"), cell("提交"), cell("作者")];
    let rows = vec![
        vec![cell("7link-v3"), cell("42"), cell("张三")],
        vec![cell("7link-mos-backend-service"), cell("17"), cell("李四")],
    ];
    let widths = super::markdown::column_widths(&head, &rows, 3);
    // The repository column is widest, the short numeric/name columns stay narrow.
    assert!(widths[0] > widths[1] && widths[0] > widths[2], "{widths:?}");
    assert_eq!(widths[1], 4, "short columns get the minimum width");
    // Very long cells are capped so one column cannot take the whole page.
    let long = vec![vec![cell(&"x".repeat(500)), cell("1"), cell("1")]];
    assert_eq!(super::markdown::column_widths(&head, &long, 3)[0], 40);
}
