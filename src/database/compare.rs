//! Two schemas side by side: the SQL that makes the second one like the first.
//!
//! Only what the catalog tells is compared: the tables, their columns and the
//! types of these. What would lose data (a table or a column the first schema
//! does not have) is written as a comment, to be read before it is run.

use super::protocol::{quote_ident_in, Dialect, TableColumn};

fn of_schema<'a>(columns: &'a [TableColumn], schema: &str) -> Vec<&'a TableColumn> {
    columns.iter().filter(|c| c.schema == schema).collect()
}

fn tables(columns: &[&TableColumn]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for c in columns {
        if !names.contains(&c.table) {
            names.push(c.table.clone());
        }
    }
    names
}

fn qualified(dialect: Dialect, schema: &str, table: &str) -> String {
    if dialect == Dialect::Sqlite {
        quote_ident_in(dialect, table)
    } else {
        format!(
            "{}.{}",
            quote_ident_in(dialect, schema),
            quote_ident_in(dialect, table)
        )
    }
}

/// The statements that turn `target_schema` of `target` into `source_schema`
/// of `source`. Empty when they are the same.
pub fn migration(
    dialect: Dialect,
    source: &[TableColumn],
    source_schema: &str,
    target: &[TableColumn],
    target_schema: &str,
) -> String {
    let (from, to) = (
        of_schema(source, source_schema),
        of_schema(target, target_schema),
    );
    let mut out: Vec<String> = Vec::new();
    for table in tables(&from) {
        let wanted: Vec<&&TableColumn> = from.iter().filter(|c| c.table == table).collect();
        let name = qualified(dialect, target_schema, &table);
        let have: Vec<&&TableColumn> = to.iter().filter(|c| c.table == table).collect();
        if have.is_empty() {
            let cols: Vec<String> = wanted
                .iter()
                .map(|c| format!("  {} {}", quote_ident_in(dialect, &c.name), c.kind))
                .collect();
            out.push(format!("CREATE TABLE {name} (\n{}\n);", cols.join(",\n")));
            continue;
        }
        for c in &wanted {
            let col = quote_ident_in(dialect, &c.name);
            match have.iter().find(|h| h.name == c.name) {
                None => out.push(format!("ALTER TABLE {name} ADD COLUMN {col} {};", c.kind)),
                Some(h) if h.kind != c.kind => out.push(match dialect {
                    Dialect::Postgres | Dialect::Duckdb => {
                        format!("ALTER TABLE {name} ALTER COLUMN {col} TYPE {};", c.kind)
                    }
                    Dialect::Mysql => format!("ALTER TABLE {name} MODIFY COLUMN {col} {};", c.kind),
                    Dialect::Sqlite => format!(
                        "-- {name}.{col}: {} should be {} (SQLite cannot change a type: make the table again)",
                        h.kind, c.kind
                    ),
                }),
                Some(_) => {}
            }
        }
        for h in &have {
            if !wanted.iter().any(|c| c.name == h.name) {
                out.push(format!(
                    "-- ALTER TABLE {name} DROP COLUMN {};",
                    quote_ident_in(dialect, &h.name)
                ));
            }
        }
    }
    let wanted_tables = tables(&from);
    for table in tables(&to) {
        if !wanted_tables.contains(&table) {
            out.push(format!(
                "-- DROP TABLE {};",
                qualified(dialect, target_schema, &table)
            ));
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(schema: &str, table: &str, name: &str, kind: &str) -> TableColumn {
        TableColumn {
            schema: schema.into(),
            table: table.into(),
            name: name.into(),
            kind: kind.into(),
        }
    }

    #[test]
    fn the_second_schema_is_made_like_the_first() {
        let first = [
            col("a", "users", "id", "integer"),
            col("a", "users", "email", "text"),
            col("a", "users", "age", "bigint"),
            col("a", "orders", "id", "integer"),
        ];
        let second = [
            col("b", "users", "id", "integer"),
            col("b", "users", "age", "integer"),
            col("b", "users", "legacy", "text"),
            col("b", "old", "id", "integer"),
        ];
        let sql = migration(Dialect::Postgres, &first, "a", &second, "b");
        assert!(sql.contains("ALTER TABLE \"b\".\"users\" ADD COLUMN \"email\" text;"));
        assert!(sql.contains("ALTER TABLE \"b\".\"users\" ALTER COLUMN \"age\" TYPE bigint;"));
        assert!(sql.contains("CREATE TABLE \"b\".\"orders\" ("));
        // What would lose data is only a comment.
        assert!(sql.contains("-- ALTER TABLE \"b\".\"users\" DROP COLUMN \"legacy\";"));
        assert!(sql.contains("-- DROP TABLE \"b\".\"old\";"));
        assert!(!sql.contains("\nDROP"));
    }

    #[test]
    fn the_same_schema_needs_nothing() {
        let same = [col("a", "t", "id", "integer")];
        let other = [col("b", "t", "id", "integer")];
        assert_eq!(migration(Dialect::Mysql, &same, "a", &other, "b"), "");
    }

    #[test]
    fn a_type_is_changed_the_way_of_each_system() {
        let first = [col("a", "t", "n", "bigint")];
        let second = [col("b", "t", "n", "int")];
        assert!(migration(Dialect::Mysql, &first, "a", &second, "b")
            .contains("MODIFY COLUMN `n` bigint;"));
        assert!(migration(Dialect::Sqlite, &first, "a", &second, "b").starts_with("--"));
    }
}
