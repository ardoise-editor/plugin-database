//! Rows of made-up data for a table, as an `INSERT` to read and change before
//! it runs: each column is given a value of its type, and a column that
//! points to another table takes the key of a row that is there.

use super::protocol::{
    quote_ident_in, quote_literal_in, ColumnDef, ConstraintKind, Dialect, Structure,
};

/// How many rows are made.
pub const ROWS: usize = 20;

fn lower(kind: &str) -> String {
    kind.to_lowercase()
}

/// The longest a `varchar(n)` holds, if the type says.
fn length_of(kind: &str) -> Option<usize> {
    let open = kind.find('(')?;
    let close = kind[open..].find(')')? + open;
    kind[open + 1..close].split(',').next()?.trim().parse().ok()
}

fn is_whole(kind: &str) -> bool {
    ["int", "serial", "integer"]
        .iter()
        .any(|k| kind.contains(k))
        && !kind.contains("interval")
}

/// A value for row `i` (from 1) of a column of a type, as SQL.
fn value(dialect: Dialect, column: &ColumnDef, i: usize, unique_base: Option<&str>) -> String {
    let kind = lower(&column.kind);
    let day = 1 + i % 28;
    if kind.contains("bool") || kind == "tinyint(1)" || kind == "bit" {
        let on = i.is_multiple_of(2);
        return match (dialect, on) {
            (Dialect::Postgres | Dialect::Duckdb, true) => "true".into(),
            (Dialect::Postgres | Dialect::Duckdb, false) => "false".into(),
            (_, true) => "1".into(),
            (_, false) => "0".into(),
        };
    }
    if is_whole(&kind) {
        return match unique_base {
            Some(base) => format!("{base} + {i}"),
            None => i.to_string(),
        };
    }
    if ["numeric", "decimal", "real", "double", "float", "money"]
        .iter()
        .any(|k| kind.contains(k))
    {
        return format!("{:.2}", i as f64 * 1.5);
    }
    if kind.contains("timestamp") || kind.contains("datetime") {
        return format!("'2024-01-{day:02} 12:00:00'");
    }
    if kind.starts_with("date") {
        return format!("'2024-01-{day:02}'");
    }
    if kind.starts_with("time") {
        return "'12:00:00'".into();
    }
    if kind == "uuid" {
        return match dialect {
            Dialect::Postgres | Dialect::Duckdb => "gen_random_uuid()".into(),
            _ => format!("'00000000-0000-4000-8000-{i:012}'"),
        };
    }
    if kind.contains("json") {
        return format!("'{{\"n\": {i}}}'");
    }
    if kind.contains("bytea") || kind.contains("blob") || kind.contains("binary") {
        return match dialect {
            Dialect::Postgres => "'\\x00'".into(),
            _ => "X'00'".into(),
        };
    }
    let text_like = ["char", "text", "string", "clob", "name", "citext"]
        .iter()
        .any(|k| kind.contains(k));
    if text_like || dialect == Dialect::Sqlite {
        let mut text = format!("{}_{i}", column.name);
        if let Some(n) = length_of(&kind) {
            let keep = n.max(1);
            if text.chars().count() > keep {
                // The end carries the number: it is what tells the rows apart.
                let skip = text.chars().count() - keep;
                text = text.chars().skip(skip).collect();
            }
        }
        return quote_literal_in(dialect, &text);
    }
    // A type that is not known: left to the user.
    if column.nullable {
        "NULL".into()
    } else {
        "NULL /* a value of this type */".into()
    }
}

/// The column a foreign key of one column starts from, and what it points to.
fn foreign(structure: &Structure) -> Vec<(String, String, String)> {
    structure
        .constraints
        .iter()
        .filter(|c| c.kind == ConstraintKind::Foreign)
        .filter_map(|c| {
            let d = &c.definition;
            let (_, rest) = d.split_once("FOREIGN KEY (")?;
            let (cols, rest) = rest.split_once(')')?;
            let (_, rest) = rest.split_once("REFERENCES ")?;
            let (target, rest) = rest.split_once('(')?;
            let (refcols, _) = rest.split_once(')')?;
            if cols.contains(',') || refcols.contains(',') {
                return None;
            }
            Some((
                cols.trim().trim_matches(['"', '`']).to_string(),
                target.trim().to_string(),
                refcols.trim().trim_matches(['"', '`']).to_string(),
            ))
        })
        .collect()
}

/// An `INSERT` of made-up rows for a table.
pub fn insert_rows(
    dialect: Dialect,
    schema: &str,
    table: &str,
    structure: &Structure,
    rows: usize,
) -> String {
    let name = if dialect == Dialect::Sqlite {
        quote_ident_in(dialect, table)
    } else {
        format!(
            "{}.{}",
            quote_ident_in(dialect, schema),
            quote_ident_in(dialect, table)
        )
    };
    let keys = foreign(structure);
    // What the server fills in itself is left to it.
    let given: Vec<&ColumnDef> = structure
        .columns
        .iter()
        .filter(|c| {
            let default = c.default.as_deref().unwrap_or("");
            c.identity.is_empty()
                && !c.generated
                && !default.starts_with("nextval(")
                && !(c.nullable && !default.is_empty())
                && !(default.contains("auto_increment"))
        })
        .collect();
    if given.is_empty() {
        return format!("-- {name}: every column is filled by the server.\n");
    }
    let unique = |column: &str| {
        structure.constraints.iter().any(|c| {
            matches!(c.kind, ConstraintKind::Primary | ConstraintKind::Unique)
                && c.definition.contains(column)
        })
    };
    let columns: Vec<String> = given
        .iter()
        .map(|c| quote_ident_in(dialect, &c.name))
        .collect();
    let mut lines = Vec::new();
    for i in 1..=rows {
        let values: Vec<String> = given
            .iter()
            .map(|c| {
                if let Some((_, target, refcol)) = keys.iter().find(|(col, _, _)| *col == c.name) {
                    let target = target
                        .split('.')
                        .map(|p| quote_ident_in(dialect, p.trim_matches(['"', '`'])))
                        .collect::<Vec<_>>()
                        .join(".");
                    // The keys that are there, one after the other.
                    let key = quote_ident_in(dialect, refcol);
                    return format!(
                        "(SELECT {key} FROM (SELECT {key}, ROW_NUMBER() OVER (ORDER BY {key}) AS n \
                         FROM {target}) AS fk_rows WHERE n = {i} % (SELECT COUNT(*) FROM {target}) + 1)"
                    );
                }
                let base = unique(&c.name).then(|| {
                    format!(
                        "(SELECT COALESCE(MAX({}), 0) FROM {name})",
                        quote_ident_in(dialect, &c.name)
                    )
                });
                value(dialect, c, i, base.as_deref())
            })
            .collect();
        lines.push(format!("  ({})", values.join(", ")));
    }
    format!(
        "INSERT INTO {name} ({})\nVALUES\n{};\n",
        columns.join(", "),
        lines.join(",\n")
    )
}

#[cfg(test)]
mod tests {
    use super::super::protocol::ConstraintDef;
    use super::*;

    fn column(name: &str, kind: &str) -> ColumnDef {
        ColumnDef {
            name: name.into(),
            kind: kind.into(),
            nullable: false,
            default: None,
            identity: String::new(),
            generated: false,
            comment: String::new(),
        }
    }

    #[test]
    fn each_column_gets_a_value_of_its_type() {
        let mut id = column("id", "integer");
        id.identity = "a".into();
        let structure = Structure {
            columns: vec![
                id,
                column("name", "character varying(6)"),
                column("price", "numeric(8,2)"),
                column("active", "boolean"),
                column("born", "date"),
                column("owner_id", "integer"),
            ],
            constraints: vec![ConstraintDef {
                name: "fk".into(),
                kind: ConstraintKind::Foreign,
                definition: "FOREIGN KEY (owner_id) REFERENCES public.owners(id)".into(),
            }],
            ..Structure::default()
        };
        let sql = insert_rows(Dialect::Postgres, "public", "pets", &structure, 2);
        // The identity is the server's own.
        assert!(sql.starts_with(
            "INSERT INTO \"public\".\"pets\" (\"name\", \"price\", \"active\", \"born\", \"owner_id\")"
        ));
        // A text that is too long keeps its end, the number.
        assert!(sql.contains("'name_1'") && sql.contains("1.50") && sql.contains("true"));
        assert!(sql.contains("'2024-01-02'"));
        assert!(sql.contains(
            "(SELECT \"id\" FROM (SELECT \"id\", ROW_NUMBER() OVER (ORDER BY \"id\") AS n FROM \"public\".\"owners\") AS fk_rows WHERE n = 1 % (SELECT COUNT(*) FROM \"public\".\"owners\") + 1)"
        ));
        assert_eq!(sql.matches("\n  (").count(), 2);
    }

    #[test]
    fn a_key_goes_on_from_the_last_one() {
        let structure = Structure {
            columns: vec![column("id", "integer")],
            constraints: vec![ConstraintDef {
                name: "pk".into(),
                kind: ConstraintKind::Primary,
                definition: "PRIMARY KEY (id)".into(),
            }],
            ..Structure::default()
        };
        let sql = insert_rows(Dialect::Mysql, "shop", "t", &structure, 1);
        assert!(sql.contains("(SELECT COALESCE(MAX(`id`), 0) FROM `shop`.`t`) + 1"));
    }

    #[test]
    fn a_table_the_server_fills_alone_is_said() {
        let mut id = column("id", "integer");
        id.default = Some("nextval('t_id_seq'::regclass)".into());
        let structure = Structure {
            columns: vec![id],
            ..Structure::default()
        };
        assert!(insert_rows(Dialect::Postgres, "public", "t", &structure, 3).starts_with("--"));
    }
}
