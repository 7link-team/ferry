// Fixed, trusted report template. Every value comes from /data.json and is
// placed as text; nothing from the data is evaluated as Typst markup or code.
#let data = json("/data.json")

#let navy = rgb("#16365f")
#let navy-soft = rgb("#d6e3f3")
#let teal = rgb("#0e9f8e")
#let teal-soft = rgb("#e3f5f2")
#let ink = rgb("#1f2933")
#let muted = rgb("#66788a")
#let rule = rgb("#d8e0e8")
#let zebra = rgb("#f4f7fa")

#set document(title: data.title)
#set text(font: "Noto Sans SC", size: 10pt, lang: data.lang, fill: ink)
#set par(leading: 0.72em, spacing: 1.05em)
#set page(
  paper: "a4",
  margin: (top: 2.3cm, bottom: 2.1cm, x: 1.9cm),
  header: context {
    if counter(page).get().first() > 1 {
      set text(size: 7.5pt, fill: muted)
      grid(columns: (1fr, auto), gutter: 8pt, [#data.title], [#data.period])
      v(-5pt)
      line(length: 100%, stroke: 0.5pt + rule)
    }
  },
  footer: context {
    set text(size: 7.5pt, fill: muted)
    line(length: 100%, stroke: 0.5pt + rule)
    v(-5pt)
    let current = str(counter(page).get().first())
    let total = str(counter(page).final().first())
    grid(
      columns: (1fr, auto),
      [#data.labels.generated #data.generated],
      [#data.labels.page.replace("{page}", current).replace("{total}", total)],
    )
  },
)

#show heading: it => block(above: 1.3em, below: 0.6em, breakable: false, {
  if it.level == 1 {
    text(size: 15pt, weight: "bold", fill: navy, it.body)
    v(-6pt)
    line(length: 100%, stroke: 1.4pt + teal)
  } else if it.level == 2 {
    text(size: 12pt, weight: "bold", fill: navy, it.body)
  } else {
    text(size: 10.5pt, weight: "bold", fill: navy, it.body)
  }
})
#show raw: set text(font: "Noto Sans SC", size: 8.5pt)
#show link: set text(fill: teal)

#let inline(xs) = for x in xs {
  if x.k == "t" { [#x.t] }
  else if x.k == "b" { strong(inline(x.c)) }
  else if x.k == "i" { emph(inline(x.c)) }
  else if x.k == "s" { strike(inline(x.c)) }
  else if x.k == "code" {
    box(fill: zebra, inset: (x: 3pt), outset: (y: 2.5pt), radius: 2pt, text(size: 0.88em, fill: navy)[#x.t])
  }
  else if x.k == "a" { link(x.u, inline(x.c)) }
  else if x.k == "br" { linebreak() }
}

#let callout-style = (
  note: (navy, navy-soft),
  tip: (teal, teal-soft),
  important: (rgb("#6b3fa0"), rgb("#efe7f8")),
  warning: (rgb("#b7791f"), rgb("#fdf3e1")),
  caution: (rgb("#c0392b"), rgb("#fbe9e7")),
)

#let blocks(bs) = for b in bs {
  if b.k == "p" { par(inline(b.c)) }
  else if b.k == "h" { heading(level: calc.min(b.l + 1, 4), inline(b.c)) }
  else if b.k == "ul" { list(..b.items.map(item => blocks(item))) }
  else if b.k == "ol" { enum(start: b.start, ..b.items.map(item => blocks(item))) }
  else if b.k == "code" {
    block(width: 100%, fill: zebra, inset: 8pt, radius: 4pt, raw(b.t, block: true))
  }
  else if b.k == "quote" {
    block(width: 100%, inset: (left: 10pt, y: 3pt), stroke: (left: 2pt + rule), text(fill: muted, blocks(b.b)))
  }
  else if b.k == "callout" {
    let (accent, soft) = callout-style.at(b.kind, default: (navy, navy-soft))
    block(width: 100%, fill: soft, stroke: (left: 3pt + accent), radius: (right: 4pt), inset: 9pt, blocks(b.b))
  }
  else if b.k == "table" {
    let columns = b.head.len()
    let place(name) = if name == "right" { right } else if name == "center" { center } else { left }
    block(width: 100%, radius: 4pt, clip: true, stroke: 0.5pt + rule, table(
      // Content-sized columns; the last one takes the remaining width.
      columns: (auto,) * (columns - 1) + (1fr,),
      stroke: none,
      inset: (x: 7pt, y: 5pt),
      fill: (_, y) => if y == 0 { navy } else if calc.even(y) { zebra } else { white },
      align: (x, y) => place(b.align.at(x)) + horizon,
      table.header(..b.head.map(cell => text(fill: white, weight: "bold", inline(cell)))),
      ..b.rows.map(row => row.map(cell => inline(cell))).flatten(),
    ))
  }
  else if b.k == "hr" { line(length: 100%, stroke: 0.5pt + rule) }
}

#let metric(m) = block(
  width: 100%, fill: rgb("#f6f9fc"), stroke: (left: 3pt + teal), radius: (right: 5pt), inset: (x: 10pt, y: 9pt),
  {
    text(size: 8pt, fill: muted)[#m.label]
    v(2pt)
    text(size: 17pt, weight: "bold", fill: navy)[#m.value]
    // An empty note keeps its line so cards in a row have the same height.
    v(1pt)
    text(size: 7.5pt, fill: muted)[#if m.note != "" { m.note } else { sym.zwj }]
  },
)

// Cover: title band, period chip, key figures, summary.
#block(width: 100%, fill: navy, radius: 6pt, inset: (x: 20pt, y: 22pt), {
  text(size: 22pt, weight: "bold", fill: white)[#data.title]
  if data.subtitle != "" {
    v(6pt)
    text(size: 11.5pt, fill: navy-soft)[#data.subtitle]
  }
  if data.period != "" {
    v(8pt)
    box(fill: teal, radius: 3pt, inset: (x: 8pt, y: 4pt), text(size: 8.5pt, fill: white)[#data.period])
  }
})
#v(14pt)
#if data.metrics.len() > 0 {
  let per-row = calc.min(data.metrics.len(), 4)
  grid(columns: (1fr,) * per-row, rows: auto, gutter: 10pt, ..data.metrics.map(metric))
  v(10pt)
}
#blocks(data.summary)

#for s in data.sections {
  if s.page_break { pagebreak(weak: true) }
  heading(level: 1, s.title)
  blocks(s.blocks)
}
