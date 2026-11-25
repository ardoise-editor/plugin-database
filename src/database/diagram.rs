//! The diagram of the tables of a schema and of the foreign keys between
//! them: where each table goes, what is drawn, and the same as Mermaid text.

use super::protocol::Catalog;

pub const BOX_WIDTH: f32 = 270.0;
pub const HEADER: f32 = 28.0;
pub const ROW: f32 = 18.0;
const GAP_X: f32 = 70.0;
const GAP_Y: f32 = 50.0;
const MARGIN: f32 = 24.0;
/// Most columns drawn in a box (the rest are counted).
const MAX_ROWS: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramColumn {
    pub name: String,
    pub kind: String,
    /// Probably the key of the table: called `id`, or pointed to by another
    /// table.
    pub key: bool,
    /// Points to another table.
    pub foreign: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagramTable {
    pub schema: String,
    pub name: String,
    pub columns: Vec<DiagramColumn>,
    /// Columns that are not drawn.
    pub hidden: usize,
    pub x: f32,
    pub y: f32,
    pub height: f32,
}

/// A foreign key: from a table to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: usize,
    pub to: usize,
    pub columns: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagram {
    pub tables: Vec<DiagramTable>,
    pub links: Vec<Link>,
    pub width: f32,
    pub height: f32,
}

/// The diagram of the tables of `schema` (those of other schemas that they
/// point to come too, named with their schema).
pub fn build(catalog: &Catalog, schema: &str) -> Diagram {
    let mut tables: Vec<DiagramTable> = Vec::new();
    let index = |tables: &Vec<DiagramTable>, s: &str, n: &str| {
        tables.iter().position(|t| t.schema == s && t.name == n)
    };
    for column in catalog.columns.iter().filter(|c| c.schema == schema) {
        let at = match index(&tables, &column.schema, &column.table) {
            Some(at) => at,
            None => {
                tables.push(DiagramTable {
                    schema: column.schema.clone(),
                    name: column.table.clone(),
                    columns: Vec::new(),
                    hidden: 0,
                    x: 0.0,
                    y: 0.0,
                    height: 0.0,
                });
                tables.len() - 1
            }
        };
        tables[at].columns.push(DiagramColumn {
            name: column.name.clone(),
            kind: column.kind.clone(),
            key: column.name == "id",
            foreign: false,
        });
    }
    let mut links = Vec::new();
    for fk in catalog.foreign_keys.iter().filter(|f| f.schema == schema) {
        let Some(from) = index(&tables, &fk.schema, &fk.table) else {
            continue;
        };
        // A table of another schema is added, with its columns.
        let to = match index(&tables, &fk.ref_schema, &fk.ref_table) {
            Some(to) => to,
            None => {
                tables.push(DiagramTable {
                    schema: fk.ref_schema.clone(),
                    name: fk.ref_table.clone(),
                    columns: catalog
                        .columns
                        .iter()
                        .filter(|c| c.schema == fk.ref_schema && c.table == fk.ref_table)
                        .map(|c| DiagramColumn {
                            name: c.name.clone(),
                            kind: c.kind.clone(),
                            key: c.name == "id",
                            foreign: false,
                        })
                        .collect(),
                    hidden: 0,
                    x: 0.0,
                    y: 0.0,
                    height: 0.0,
                });
                tables.len() - 1
            }
        };
        for name in &fk.columns {
            if let Some(c) = tables[from].columns.iter_mut().find(|c| c.name == *name) {
                c.foreign = true;
            }
        }
        for name in &fk.ref_columns {
            if let Some(c) = tables[to].columns.iter_mut().find(|c| c.name == *name) {
                c.key = true;
            }
        }
        links.push(Link {
            from,
            to,
            columns: fk.columns.join(", "),
        });
    }
    layout(&mut tables, &links);
    let width = tables.iter().map(|t| t.x + BOX_WIDTH).fold(0.0, f32::max) + MARGIN;
    let height = tables.iter().map(|t| t.y + t.height).fold(0.0, f32::max) + MARGIN;
    Diagram {
        tables,
        links,
        width,
        height,
    }
}

/// The boxes in rows, the most connected first, so that the links that
/// matter are short.
fn layout(tables: &mut [DiagramTable], links: &[Link]) {
    let mut degree = vec![0usize; tables.len()];
    for l in links {
        degree[l.from] += 1;
        degree[l.to] += 1;
    }
    let mut order: Vec<usize> = (0..tables.len()).collect();
    order.sort_by(|a, b| {
        degree[*b]
            .cmp(&degree[*a])
            .then_with(|| tables[*a].name.cmp(&tables[*b].name))
    });
    for t in tables.iter_mut() {
        if t.columns.len() > MAX_ROWS {
            t.hidden = t.columns.len() - MAX_ROWS;
            t.columns.truncate(MAX_ROWS);
        }
        t.height = HEADER + ROW * (t.columns.len() + usize::from(t.hidden > 0)) as f32 + 8.0;
    }
    let per_row = (tables.len() as f32).sqrt().ceil().max(1.0) as usize;
    let mut y = MARGIN;
    for row in order.chunks(per_row) {
        let mut tallest: f32 = 0.0;
        for (n, &i) in row.iter().enumerate() {
            tables[i].x = MARGIN + n as f32 * (BOX_WIDTH + GAP_X);
            tables[i].y = y;
            tallest = tallest.max(tables[i].height);
        }
        y += tallest + GAP_Y;
    }
}

fn word(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "x".to_string()
    } else {
        cleaned
    }
}

/// The diagram as Mermaid text (`erDiagram`).
pub fn to_mermaid(diagram: &Diagram) -> String {
    let name = |t: &DiagramTable| word(&t.name);
    let mut out = String::from("erDiagram\n");
    for t in &diagram.tables {
        out.push_str(&format!("  {} {{\n", name(t)));
        for c in &t.columns {
            let mark = match (c.key, c.foreign) {
                (true, true) => " PK, FK",
                (true, false) => " PK",
                (false, true) => " FK",
                (false, false) => "",
            };
            out.push_str(&format!("    {} {}{mark}\n", word(&c.kind), word(&c.name)));
        }
        out.push_str("  }\n");
    }
    for l in &diagram.links {
        out.push_str(&format!(
            "  {} ||--o{{ {} : \"{}\"\n",
            name(&diagram.tables[l.to]),
            name(&diagram.tables[l.from]),
            l.columns.replace('"', "'")
        ));
    }
    out
}

/// A diagram on show: its picture and the same as Mermaid text (to copy).
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub title: String,
    pub diagram: Diagram,
    pub mermaid: String,
}

/// Text put in SVG.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The diagram as an SVG picture, in the colors of the theme.
/// The fonts of the texts, the first installed one is taken. A generic name
/// alone is not enough: the renderer reads `sans-serif` as Arial and
/// `monospace` as Courier New, which many systems lack, and draws no text.
const SANS: &str = "DejaVu Sans, Liberation Sans, Noto Sans, Arial, Helvetica, sans-serif";
const MONO: &str =
    "DejaVu Sans Mono, Liberation Mono, Noto Sans Mono, Menlo, Consolas, Courier New, monospace";

/// The characters of a column's name there is room for, before its type.
const NAME_CHARS: usize = 16;

/// A name cut to `max` characters, an ellipsis saying so.
fn short(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    name.chars().take(max - 1).chain(['…']).collect()
}

pub fn svg(d: &Diagram, dark: bool) -> String {
    let (text, muted, accent, elevated, border, soft, background) = if dark {
        (
            "#d7dae0", "#8b919a", "#4f9cf9", "#2b2d31", "#4a4d53", "#24364f", "#1e1f22",
        )
    } else {
        (
            "#1f2328", "#6b7280", "#2563eb", "#ffffff", "#c9ced6", "#dbe7fb", "#f6f7f9",
        )
    };
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{SANS}"><rect width="{w}" height="{h}" fill="{background}"/>"#,
        w = d.width.max(1.0),
        h = d.height.max(1.0)
    );
    // The links first, under the boxes: from the side of a box that faces
    // the other one.
    for l in &d.links {
        let (a, b) = (&d.tables[l.from], &d.tables[l.to]);
        let (ax, bx) = if a.x < b.x {
            (a.x + BOX_WIDTH, b.x)
        } else if a.x > b.x {
            (a.x, b.x + BOX_WIDTH)
        } else {
            (a.x + BOX_WIDTH, b.x + BOX_WIDTH)
        };
        let (ay, by) = (a.y + HEADER / 2.0, b.y + HEADER / 2.0);
        let points = if a.x == b.x {
            format!("{ax},{ay} {},{ay} {},{by} {bx},{by}", ax + 40.0, bx + 40.0)
        } else {
            format!("{ax},{ay} {bx},{by}")
        };
        out.push_str(&format!(
            r#"<polyline points="{points}" fill="none" stroke="{muted}" stroke-width="1.2"><title>{}</title></polyline><circle cx="{bx}" cy="{by}" r="3.5" fill="{accent}"/>"#,
            escape(&l.columns)
        ));
    }
    for t in &d.tables {
        let title = if t.schema.is_empty() {
            t.name.clone()
        } else {
            format!("{}.{}", t.schema, t.name)
        };
        out.push_str(&format!(
            r#"<rect x="{x}" y="{y}" width="{BOX_WIDTH}" height="{h}" fill="{elevated}" stroke="{border}"/><rect x="{x}" y="{y}" width="{BOX_WIDTH}" height="{HEADER}" fill="{soft}"/><text x="{tx}" y="{ty}" font-size="13" font-weight="600" fill="{text}">{title}</text>"#,
            x = t.x,
            y = t.y,
            h = t.height,
            tx = t.x + 10.0,
            ty = t.y + 18.0,
            title = escape(&title)
        ));
        for (i, c) in t.columns.iter().enumerate() {
            let y = t.y + HEADER + 16.0 + i as f32 * ROW;
            let mark = match (c.key, c.foreign) {
                (true, true) => "PK FK",
                (true, false) => "PK",
                (false, true) => "FK",
                (false, false) => "",
            };
            let kind: String = c.kind.chars().take(14).collect();
            let name = short(&c.name, NAME_CHARS);
            out.push_str(&format!(
                r#"<text x="{}" y="{y}" font-size="10.5" font-family="{MONO}" fill="{muted}">{mark}</text><text x="{}" y="{y}" font-size="12" font-family="{MONO}" fill="{}">{}</text><text x="{}" y="{y}" font-size="11" font-family="{MONO}" fill="{muted}">{}</text>"#,
                t.x + 8.0,
                t.x + 46.0,
                if c.foreign { accent } else { text },
                escape(&name),
                t.x + BOX_WIDTH - 96.0,
                escape(&kind)
            ));
        }
        if t.hidden > 0 {
            out.push_str(&format!(
                r#"<text x="{}" y="{}" font-size="11" fill="{muted}">… +{}</text>"#,
                t.x + 46.0,
                t.y + HEADER + 16.0 + t.columns.len() as f32 * ROW,
                t.hidden
            ));
        }
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_column_name_is_cut_before_the_type() {
        assert_eq!(short("customer_id", NAME_CHARS), "customer_id");
        let cut = short("a_very_long_column_name", NAME_CHARS);
        assert_eq!(cut.chars().count(), NAME_CHARS);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn the_texts_name_fonts_that_are_installed_not_only_a_generic_one() {
        let drawn = svg(&build(&catalog(), "shop"), true);
        assert!(drawn.contains(r#"font-family="DejaVu Sans, "#));
        assert!(drawn.contains(r#"font-family="DejaVu Sans Mono, "#));
        for generic in ["sans-serif", "monospace"] {
            assert!(!drawn.contains(&format!(r#"font-family="{generic}""#)));
        }
    }
    use crate::database::protocol::{ForeignKey, TableColumn};

    fn column(schema: &str, table: &str, name: &str, kind: &str) -> TableColumn {
        TableColumn {
            schema: schema.into(),
            table: table.into(),
            name: name.into(),
            kind: kind.into(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            columns: vec![
                column("shop", "customers", "id", "integer"),
                column("shop", "customers", "name", "text"),
                column("shop", "orders", "id", "integer"),
                column("shop", "orders", "customer_id", "integer"),
                column("shop", "orders", "total", "numeric(8,2)"),
                column("other", "countries", "code", "text"),
                column("other", "ignored", "x", "text"),
            ],
            foreign_keys: vec![
                ForeignKey {
                    schema: "shop".into(),
                    table: "orders".into(),
                    columns: vec!["customer_id".into()],
                    ref_schema: "shop".into(),
                    ref_table: "customers".into(),
                    ref_columns: vec!["id".into()],
                },
                ForeignKey {
                    schema: "shop".into(),
                    table: "customers".into(),
                    columns: vec!["name".into()],
                    ref_schema: "other".into(),
                    ref_table: "countries".into(),
                    ref_columns: vec!["code".into()],
                },
                ForeignKey {
                    schema: "other".into(),
                    table: "ignored".into(),
                    columns: vec!["x".into()],
                    ref_schema: "other".into(),
                    ref_table: "countries".into(),
                    ref_columns: vec!["code".into()],
                },
            ],
        }
    }

    #[test]
    fn the_tables_of_the_schema_and_the_ones_they_point_to_are_drawn() {
        let d = build(&catalog(), "shop");
        let names: Vec<&str> = d.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"countries") && !names.contains(&"ignored"));
        assert_eq!(
            d.links.len(),
            2,
            "the links of other schemas' tables stay out"
        );
        let orders = d.tables.iter().find(|t| t.name == "orders").unwrap();
        let by = |name: &str| orders.columns.iter().find(|c| c.name == name).unwrap();
        assert!(by("id").key && !by("id").foreign);
        assert!(by("customer_id").foreign && !by("customer_id").key);
        let countries = d.tables.iter().find(|t| t.name == "countries").unwrap();
        assert!(countries.columns[0].key, "pointed to: a key");
    }

    #[test]
    fn the_boxes_do_not_overlap_and_the_diagram_holds_them() {
        let d = build(&catalog(), "shop");
        for (i, a) in d.tables.iter().enumerate() {
            assert!(a.x + BOX_WIDTH <= d.width && a.y + a.height <= d.height);
            for b in &d.tables[i + 1..] {
                let apart = a.x + BOX_WIDTH <= b.x
                    || b.x + BOX_WIDTH <= a.x
                    || a.y + a.height <= b.y
                    || b.y + b.height <= a.y;
                assert!(apart, "{} and {} overlap", a.name, b.name);
            }
        }
        assert_eq!(build(&catalog(), "nothing").tables.len(), 0);
    }

    #[test]
    fn long_tables_are_cut_and_counted() {
        let mut c = Catalog::default();
        for i in 0..20 {
            c.columns
                .push(column("s", "wide", &format!("c{i}"), "text"));
        }
        let d = build(&c, "s");
        assert_eq!(
            (d.tables[0].columns.len(), d.tables[0].hidden),
            (MAX_ROWS, 6)
        );
    }

    #[test]
    fn mermaid_names_the_tables_the_keys_and_the_links() {
        let text = to_mermaid(&build(&catalog(), "shop"));
        assert!(text.starts_with("erDiagram\n"));
        assert!(text.contains("  orders {\n    integer id PK\n    integer customer_id FK\n    numeric_8_2_ total\n  }"));
        assert!(text.contains("  customers ||--o{ orders : \"customer_id\""));
    }
}
