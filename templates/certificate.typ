// Drive health certificate. Inputs (sys.inputs):
//   data    - DriveReport as JSON string
//   display - pre-formatted display model as JSON string (see src/pdf.rs)
#let report = json(bytes(sys.inputs.data))
#let d = json(bytes(sys.inputs.display))
#let dense = d.style == "dense"
#let roomy = d.style == "roomy"

// ---------- palette ----------
#let ink = luma(15%)
#let muted = luma(42%)
#let faint = luma(62%)
#let hair = luma(80%)
#let panel = luma(96.5%)
#let green = rgb("#1b7a3a")
#let amber = rgb("#b86e00")
#let red = rgb("#b42318")
#let gray = luma(45%)
#let grade-color = (PASS: green, CAUTION: amber, FAIL: red, UNKNOWN: gray).at(d.grade)
#let grade-word = (
  PASS: "Healthy",
  CAUTION: "Use with caution",
  FAIL: "Not recommended",
  UNKNOWN: "Insufficient data",
).at(d.grade)
#let level-color(l) = (ok: green, info: ink, warn: amber, bad: red, na: faint).at(l)
#let bar-color(l) = (ok: green, info: luma(70%), warn: amber, bad: red, na: hair).at(l)

#let mono = "IBM Plex Mono"
#let base = if roomy { 8.8pt } else if dense { 7.6pt } else { 8.3pt }
#let small = if roomy { 7.8pt } else if dense { 6.9pt } else { 7.3pt }
#let tiny = if roomy { 7pt } else { 6.6pt }
#let gap = if roomy { 11pt } else if dense { 6pt } else { 8pt }

#set document(
  title: "Drive Health Certificate",
  author: "ddreport",
  keywords: (report.identity.model, report.identity.serial, d.grade).filter(k => k != none),
)
#set page(
  paper: d.paper,
  margin: (x: 0.45in, top: 0.4in, bottom: 0.38in),
)
#set text(font: "IBM Plex Sans", size: base, fill: ink, number-type: "lining")
#set par(leading: 0.45em, spacing: 0.5em)
#set table(inset: (x: 3pt, y: 1.6pt), stroke: none)

// ---------- building blocks ----------
#let section(title, extra: none) = block(
  above: gap,
  below: 3pt,
  width: 100%,
  stroke: (bottom: 0.6pt + hair),
  inset: (bottom: 2pt),
)[
  #text(size: 7.4pt, weight: "bold", fill: muted, tracking: 0.08em, upper(title))
  #if extra != none { h(1fr); text(size: 7pt, fill: faint, extra) }
]

#let mark(l) = {
  let c = level-color(l)
  let glyph = (ok: "✓", info: "i", warn: "!", bad: "✗", na: "–").at(l)
  box(
    baseline: 18%,
    circle(radius: 4.2pt, fill: c, stroke: none, align(center + horizon,
      text(font: mono, size: 5.6pt, weight: "bold", fill: white, glyph))),
  )
}

#let kv-grid(rows, key-width: 0.95in) = grid(
  columns: (key-width, 1fr),
  row-gutter: 3.2pt,
  column-gutter: 6pt,
  ..rows.map(r => (text(fill: muted, r.k), r.v)).flatten(),
)

#let more-note(n, what) = if n > 0 {
  text(size: 7pt, fill: muted, style: "italic")[+#n more #what not shown]
}

#let status-text(s, l) = text(
  fill: level-color(l),
  weight: if l == "ok" { "regular" } else { "bold" },
  s,
)

// ---------- body ----------
#let body = [
  // Header
  #grid(
    columns: (auto, 1fr),
    column-gutter: 12pt,
    align: (left + bottom, right + bottom),
    [
      #text(size: 21pt, weight: "bold", tracking: -0.01em)[Drive Health Certificate]
      #v(-4pt)
      #text(size: 8.6pt, fill: muted, style: "italic")[
        Self-reported S.M.A.R.T. health snapshot of a used storage device
      ]
    ],
    text(size: 7.4pt, grid(
      columns: (auto, auto),
      column-gutter: 6pt,
      row-gutter: 2.6pt,
      align: (right, left),
      text(fill: muted)[Generated], d.generated,
      ..if d.host != "" { (text(fill: muted)[Host], d.host) },
      ..if d.source != "" { (text(fill: muted)[Source], d.source) },
    )),
  )
  #v(-2pt)
  #line(length: 100%, stroke: 1.4pt + ink)
  #v(2pt)

  // Verdict
  #grid(
    columns: (1.55in, 1fr),
    column-gutter: 12pt,
    block(
      width: 100%,
      fill: grade-color,
      radius: 5pt,
      inset: (x: 8pt, y: 9pt),
      align(center, {
        text(size: 6.8pt, fill: white.transparentize(15%), tracking: 0.12em, weight: "bold")[OVERALL VERDICT]
        v(1pt)
        text(size: if d.grade.len() > 5 { 17pt } else { 27pt }, weight: "bold", fill: white, d.grade)
        v(-3pt)
        text(size: 8pt, fill: white, style: "italic", grade-word)
      }),
    ),
    {
      if d.summary != "" {
        text(size: 10pt, weight: "bold", d.summary)
        v(1pt)
      }
      let fs = d.findings.items
      if fs.len() == 0 {
        text(fill: muted, style: "italic")[No findings reported.]
      } else {
        let cols = if fs.len() > 4 { 2 } else { 1 }
        grid(
          columns: (1fr,) * cols,
          column-gutter: 10pt,
          row-gutter: 3.4pt,
          ..fs.map(f => grid(
            columns: (10pt, 1fr),
            column-gutter: 2pt,
            mark(f.level),
            text(size: 7.8pt, f.text),
          )),
        )
      }
      more-note(d.findings.more, "findings")
    },
  )

  // Identity + metrics
  #v(gap - 4pt)
  #grid(
    columns: (1fr, 1.08fr),
    column-gutter: 14pt,
    {
      section("Drive identity")
      text(size: if d.model.len() > 30 { 10pt } else { 12pt }, weight: "bold", d.model)
      if d.vendor_line != "" {
        linebreak()
        text(fill: muted, d.vendor_line)
      }
      v(1pt)
      block(
        width: 100%,
        stroke: 0.8pt + ink,
        radius: 3pt,
        inset: (x: 6pt, y: 4pt),
        {
          let label = text(size: 6.6pt, weight: "bold", fill: muted, tracking: 0.08em)[SERIAL NUMBER]
          if d.serial.len() <= 20 {
            grid(
              columns: (auto, 1fr),
              align: (left + horizon, right + horizon),
              label, text(font: mono, size: 11pt, weight: "bold", d.serial),
            )
          } else {
            label
            linebreak()
            text(font: mono, size: if d.serial.len() > 28 { 8pt } else { 9.5pt }, weight: "bold", d.serial)
          }
        },
      )
      text(size: 6.6pt, fill: muted, style: "italic")[Match this serial to the label on the drive.]
      v(1pt)
      kv-grid(d.identity)
      if d.bridge.len() > 0 {
        section("USB bridge")
        kv-grid(d.bridge)
      }
    },
    {
      section("Key health metrics")
      grid(
        columns: (1fr,) * 3,
        gutter: 4pt,
        ..d.tiles.map(t => block(
          width: 100%,
          height: if roomy { 41pt } else if dense { 34pt } else { 38pt },
          fill: panel,
          radius: 3pt,
          inset: (x: 5pt, y: 4pt),
          stroke: (left: 2.2pt + bar-color(t.level)),
          {
            text(size: 6.6pt, fill: muted, upper(t.label))
            linebreak()
            text(size: if t.value.len() > 11 { 8.5pt } else { 11pt }, weight: "bold", fill: if t.level in ("warn", "bad") { level-color(t.level) } else { ink }, t.value)
            if t.sub != "" {
              linebreak()
              text(size: 6.6pt, fill: muted, t.sub)
            }
          },
        )),
      )
    },
  )

  // SMART attributes
  #if d.attributes.items.len() > 0 or d.attr_note != none [#section(
    "SMART attributes",
    extra: if d.attributes.items.len() > 0 [Normalized values: higher is better; FAILING = at or below threshold],
  )]
  #if d.attr_note != none {
    text(fill: muted, style: "italic", d.attr_note)
  } else if d.attributes.items.len() > 0 {
    let rows = d.attributes.items
    let attr-table(rs) = table(
      // Extra room before Raw and Status keeps adjacent numbers apart.
      inset: (x, _) => (left: if x in (5, 6) { 7pt } else { 2.4pt }, right: 2.4pt, y: 1.6pt),
      columns: (auto, 1fr, auto, auto, auto, auto, auto),
      align: (right, left, right, right, right, right, left),
      fill: (_, y) => if y > 0 and calc.odd(y) { panel },
      table.header(
        ..("ID", "Attribute", "Val", "Wst", "Thr", "Raw", "Status").map(h => text(size: 6.6pt, weight: "bold", fill: muted, h)),
      ),
      ..rs.map(a => (
        text(fill: muted, a.id),
        a.name,
        text(font: mono, size: tiny, a.value),
        text(font: mono, size: tiny, a.worst),
        text(font: mono, size: tiny, a.thresh),
        text(font: mono, size: tiny, fill: if a.level == "ok" { ink } else { level-color(a.level) }, a.raw),
        status-text(a.status, a.level),
      )).flatten(),
    )
    set text(size: calc.min(small, 7.4pt))
    if rows.len() > 14 {
      let half = calc.ceil(rows.len() / 2)
      grid(
        columns: (1fr, 1fr),
        column-gutter: 10pt,
        attr-table(rows.slice(0, half)), attr-table(rows.slice(half)),
      )
    } else {
      attr-table(rows)
    }
    more-note(d.attributes.more, "attributes")
  }

  // Self-tests + statistics
  #let tests(full) = {
    section("Self-test log", extra: if d.self_tests.items.len() > 0 [newest first])
    if d.self_tests.items.len() > 0 {
      set text(size: small)
      table(
        columns: if full { (auto, auto, auto, auto) } else { (auto, 1fr, auto, auto) },
        inset: (x: if full { 6pt } else { 3pt }, y: 1.6pt),
        align: (left, left, right, right),
        fill: (_, y) => if y > 0 and calc.odd(y) { panel },
        table.header(
          ..("Test", "Result", "Hours", "Error LBA").map(h => text(size: 6.6pt, weight: "bold", fill: muted, h)),
        ),
        ..d.self_tests.items.map(t => (
          t.kind,
          status-text(t.status, t.level),
          text(font: mono, size: tiny, t.hours),
          text(font: mono, size: tiny, t.lba),
        )).flatten(),
      )
      more-note(d.self_tests.more, "log entries")
    }
    if d.self_test_note != none {
      text(size: 7pt, fill: muted, style: "italic", d.self_test_note)
    }
  }
  #let stats = if d.stats.items.len() > 0 {
    section("Device statistics")
    set text(size: small)
    grid(
      columns: (1fr, auto),
      align: (left, right),
      row-gutter: 2.6pt,
      column-gutter: 6pt,
      ..d.stats.items.map(s => (text(fill: muted, s.k), text(font: mono, size: tiny, s.v))).flatten(),
    )
    more-note(d.stats.more, "statistics")
  }
  #if stats == none {
    tests(true)
  } else {
    v(gap - 4pt)
    grid(columns: (1.25fr, 1fr), column-gutter: 14pt, tests(false), stats)
  }

  // Verification steps, only when there is room
  #if d.verify_steps {
    section("How to verify this report")
    let step(n, body) = grid(
      columns: (12pt, 1fr),
      box(circle(radius: 5pt, stroke: 0.7pt + muted, align(center + horizon, text(size: 6.6pt, weight: "bold", fill: muted, str(n))))),
      text(size: small, body),
    )
    grid(
      columns: (1fr, 1fr, 1fr),
      column-gutter: 12pt,
      step(1)[Match the serial number above to the label on the drive.],
      step(2)[Read SMART data yourself (e.g. `smartctl -x`, CrystalDiskInfo). Power-on hours should be #if d.poh != none [at least *#d.poh*] else [no lower than when tested].],
      step(3)[Reallocated, pending and uncorrectable sector counts should match this report and stay stable.],
    )
  }

  // Collection warnings
  #if d.warnings.items.len() > 0 {
    v(3pt)
    text(size: 6.8pt, fill: muted)[
      *Collection notes:* #d.warnings.items.join("; ")
      #if d.warnings.more > 0 [ (+#d.warnings.more more)]
    ]
  }
]

#let footer = block(width: 100%, stroke: (top: 1pt + ink), inset: (top: 6pt))[
  #{
      set text(size: 7pt)
      text(weight: "bold")[Raw data SHA-256]
      h(4pt)
      text(font: mono, size: 6.4pt, if d.sha256 != "" { d.sha256 } else { "not available" })
      linebreak()
      text(fill: muted)[
        The hash identifies the exact raw diagnostic output this certificate was generated from.
      ]
      v(1pt)
      text(style: "italic", d.disclaimer)
      h(1fr)
      text(fill: faint, d.tool_version)
  }
]

#if d.clip {
  block(height: 1fr, width: 100%, clip: true, body)
} else {
  body
  v(1fr)
}
#footer
