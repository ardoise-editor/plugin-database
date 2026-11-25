//! The DuckDB driver of Ardoise's Database panel: a separate program, as the
//! others, speaking the same protocol. The "database" of a data source is the
//! path of the file.

#[allow(dead_code)]
#[path = "../../src/database/protocol.rs"]
mod protocol;

#[path = "../common/mod.rs"]
mod common;

#[allow(dead_code)]
#[path = "../../src/encoding.rs"]
mod encoding;

#[allow(dead_code)]
#[path = "../../src/database/import.rs"]
mod import;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use common::{Engine, Row, Shape};
use duckdb::types::{TimeUnit, Value};
use duckdb::{AccessMode, Config, Connection};
use protocol::{
    Column, ColumnDef, ConstraintDef, ConstraintKind, Database, ForeignKey, IndexDef, Kind,
    Reference, Request, Schema, Structure, Table, TableColumn,
};

fn main() {
    common::main_loop(|request| Session::open(request).map(|s| Box::new(s) as Box<dyn Engine>));
}

struct Session {
    conn: Connection,
    /// The name of the file, for the tree.
    name: String,
    /// Where the file is, for its size.
    path: String,
    /// A transaction is open. The library cannot be asked (its
    /// `is_autocommit` always says yes): the statements run say it.
    in_tx: bool,
    /// The open transaction was begun by this driver (not one the user ran).
    begun: bool,
    /// The number of the request being answered: a timer that wakes for an
    /// older one has nothing to stop.
    request_no: Arc<AtomicU64>,
    interrupt: Arc<duckdb::InterruptHandle>,
    limit_ms: u64,
}

impl Session {
    fn open(request: &Request) -> Result<Session, String> {
        let target = &request.target;
        let path = target.database.trim();
        if path.is_empty() {
            return Err("no file: give the path of the DuckDB database".to_string());
        }
        if path != ":memory:" && !std::path::Path::new(path).is_file() {
            return Err(format!("{path}: no such file"));
        }
        let mut config = Config::default();
        if target.read_only {
            config = config.access_mode(AccessMode::ReadOnly).map_err(clean)?;
        }
        let conn = if path == ":memory:" {
            Connection::open_in_memory_with_flags(config).map_err(clean)?
        } else {
            Connection::open_with_flags(path, config).map_err(clean)?
        };
        if !target.startup_script.trim().is_empty() {
            conn.execute_batch(&target.startup_script).map_err(clean)?;
        }
        let name = std::path::Path::new(path)
            .file_name()
            .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        let interrupt = conn.interrupt_handle();
        Ok(Session {
            conn,
            name,
            path: path.to_string(),
            in_tx: false,
            begun: false,
            request_no: Arc::new(AtomicU64::new(0)),
            interrupt,
            limit_ms: target.query_timeout * 1000,
        })
    }

    /// Runs a query of the driver's own (a catalog) and logs it as internal.
    fn internal(&mut self, sql: &str) -> Result<Vec<Row>, String> {
        let started = std::time::Instant::now();
        let done = self.read(sql);
        common::record_internal(
            sql,
            started,
            done.as_ref().ok().map(|r| r.len() as u64),
            done.as_ref().err().cloned(),
        );
        done
    }

    fn read(&self, sql: &str) -> Result<Vec<Row>, String> {
        let mut statement = self.conn.prepare(sql).map_err(clean)?;
        let mut rows = statement.query([]).map_err(clean)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(clean)? {
            let mut cells = Vec::new();
            let mut i = 0;
            while let Ok(value) = row.get::<_, Value>(i) {
                cells.push(text_of(&value).0);
                i += 1;
            }
            out.push(cells);
        }
        Ok(out)
    }
}

fn clean(error: duckdb::Error) -> String {
    let text = error.to_string();
    if text.contains("INTERRUPT") {
        return "the query was stopped: it ran past the time limit".to_string();
    }
    text
}

/// Whether a transaction is open once `sql` has run.
fn tx_after(open: bool, sql: &str) -> bool {
    match protocol::sql_words(sql, protocol::Dialect::Duckdb)
        .first()
        .map(String::as_str)
    {
        Some("BEGIN" | "START") => true,
        Some("COMMIT" | "END" | "ROLLBACK" | "ABORT") => false,
        _ => open,
    }
}

fn quote(name: &str) -> String {
    protocol::quote_ident(name)
}

fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn cell(row: &Row, i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

/// The values of a list the catalog gave as text (`a,b`).
fn names(text: &str) -> Vec<String> {
    text.split(',')
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

fn micros(unit: &TimeUnit, n: i64) -> i64 {
    match unit {
        TimeUnit::Second => n.saturating_mul(1_000_000),
        TimeUnit::Millisecond => n.saturating_mul(1000),
        TimeUnit::Microsecond => n,
        TimeUnit::Nanosecond => n / 1000,
    }
}

fn time_text(us: i64) -> String {
    let (s, frac) = (us.div_euclid(1_000_000), us.rem_euclid(1_000_000));
    let base = format!("{:02}:{:02}:{:02}", s / 3600 % 24, s / 60 % 60, s % 60);
    if frac == 0 {
        base
    } else {
        format!("{base}.{frac:06}")
    }
}

/// A value as the driver says it: text, with NULL apart, and whether it is a
/// number.
fn text_of(value: &Value) -> (Option<String>, bool) {
    let plain = |s: String| (Some(s), false);
    let number = |s: String| (Some(s), true);
    match value {
        Value::Null => (None, false),
        Value::Boolean(b) => plain(b.to_string()),
        Value::TinyInt(n) => number(n.to_string()),
        Value::SmallInt(n) => number(n.to_string()),
        Value::Int(n) => number(n.to_string()),
        Value::BigInt(n) => number(n.to_string()),
        Value::HugeInt(n) => number(n.to_string()),
        Value::UHugeInt(n) => number(n.to_string()),
        Value::UTinyInt(n) => number(n.to_string()),
        Value::USmallInt(n) => number(n.to_string()),
        Value::UInt(n) => number(n.to_string()),
        Value::UBigInt(n) => number(n.to_string()),
        Value::Float(n) => number(n.to_string()),
        Value::Double(n) => number(n.to_string()),
        Value::Decimal(d) => number(d.to_string()),
        Value::Text(s) | Value::Enum(s) => plain(s.clone()),
        Value::Blob(bytes) | Value::Geometry(bytes) => plain(format!(
            "\\x{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )),
        Value::Date32(days) => plain(
            chrono::NaiveDate::from_num_days_from_ce_opt(days.saturating_add(719_163))
                .map_or_else(|| days.to_string(), |d| d.format("%Y-%m-%d").to_string()),
        ),
        Value::Timestamp(unit, n) => {
            let us = micros(unit, *n);
            plain(chrono::DateTime::from_timestamp_micros(us).map_or_else(
                || n.to_string(),
                |t| {
                    let t = t.naive_utc();
                    if us.rem_euclid(1_000_000) == 0 {
                        t.format("%Y-%m-%d %H:%M:%S").to_string()
                    } else {
                        t.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
                    }
                },
            ))
        }
        Value::Time64(unit, n) => plain(time_text(micros(unit, *n))),
        Value::Interval {
            months,
            days,
            nanos,
        } => plain(format!(
            "{months} months {days} days {}",
            time_text(nanos / 1000)
        )),
        Value::List(items) | Value::Array(items) => plain(format!(
            "[{}]",
            items
                .iter()
                .map(|v| text_of(v).0.unwrap_or_else(|| "NULL".into()))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Value::Struct(map) => plain(format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{k}: {}", text_of(v).0.unwrap_or_else(|| "NULL".into())))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Value::Map(map) => plain(format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!(
                    "{}={}",
                    text_of(k).0.unwrap_or_else(|| "NULL".into()),
                    text_of(v).0.unwrap_or_else(|| "NULL".into())
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Value::Union(inner) => text_of(inner),
        #[allow(unreachable_patterns)]
        other => plain(format!("{other:?}")),
    }
}

impl Engine for Session {
    fn start_request(&mut self) {
        let no = self.request_no.fetch_add(1, Ordering::SeqCst) + 1;
        if self.limit_ms == 0 {
            return;
        }
        // DuckDB has no clock of its own: a timer stops what still runs.
        let (seen, handle, limit) = (
            self.request_no.clone(),
            self.interrupt.clone(),
            self.limit_ms,
        );
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(limit));
            if seen.load(Ordering::SeqCst) == no {
                handle.interrupt();
            }
        });
    }

    fn quote(&self, name: &str) -> String {
        quote(name)
    }

    fn same(&self, column: &str, literal: &str) -> String {
        format!("{column} IS NOT DISTINCT FROM {literal}")
    }

    fn key_equals(&self, column: &str, literal: &str) -> String {
        self.same(column, literal)
    }

    fn version(&mut self) -> Result<String, String> {
        let rows = self.internal("SELECT version()")?;
        Ok(format!(
            "DuckDB {}",
            cell(&rows[0], 0).trim_start_matches('v')
        ))
    }

    fn tree(&mut self, request: &Request) -> Result<Vec<Database>, String> {
        let system = if request.scope.system_schemas {
            ""
        } else {
            "WHERE schema_name NOT IN ('information_schema', 'pg_catalog')"
        };
        let rows = self.internal(&format!(
            "SELECT schema_name, name, kind FROM ( \
               SELECT schema_name, table_name AS name, 'table' AS kind FROM duckdb_tables() \
               UNION ALL \
               SELECT schema_name, view_name, 'view' FROM duckdb_views() WHERE NOT internal \
             ) {system} ORDER BY schema_name, name"
        ))?;
        let mut schemas: Vec<Schema> = Vec::new();
        for r in &rows {
            let schema = cell(r, 0);
            if !schemas.iter().any(|s| s.name == schema) {
                schemas.push(Schema {
                    name: schema.clone(),
                    ..Schema::default()
                });
            }
            let at = schemas.iter().position(|s| s.name == schema).unwrap_or(0);
            schemas[at].tables.push(Table {
                name: cell(r, 1),
                kind: if cell(r, 2) == "view" {
                    Kind::View
                } else {
                    Kind::Table
                },
            });
        }
        if schemas.is_empty() {
            schemas.push(Schema {
                name: "main".to_string(),
                ..Schema::default()
            });
        }
        Ok(vec![Database {
            name: self.name.clone(),
            current: true,
            schemas,
        }])
    }

    fn shape(&mut self, schema: &str, table: &str) -> Result<Shape, String> {
        let (s, t) = (literal(schema), literal(table));
        let columns = self.internal(&format!(
            "SELECT column_name, data_type, is_nullable FROM duckdb_columns() \
             WHERE schema_name = {s} AND table_name = {t} ORDER BY column_index"
        ))?;
        if columns.is_empty() {
            return Err(format!("{table}: no such table"));
        }
        let keys = self.internal(&format!(
            "SELECT array_to_string(constraint_column_names, ',') FROM duckdb_constraints() \
             WHERE schema_name = {s} AND table_name = {t} AND constraint_type = 'PRIMARY KEY'"
        ))?;
        let is_table = !self
            .internal(&format!(
                "SELECT 1 FROM duckdb_tables() WHERE schema_name = {s} AND table_name = {t}"
            ))?
            .is_empty();
        Ok(Shape {
            primary_key: keys.first().map(|k| names(&cell(k, 0))).unwrap_or_default(),
            columns: columns
                .iter()
                .map(|r| Column {
                    name: cell(r, 0),
                    kind: cell(r, 1),
                    nullable: cell(r, 2) != "false",
                })
                .collect(),
            is_table,
            row_id: Some("rowid"),
        })
    }

    fn fetch(&mut self, sql: &str, limit: usize) -> Result<(Vec<String>, Vec<Row>, bool), String> {
        let mut statement = self.conn.prepare(sql).map_err(clean)?;
        let mut rows = statement.query([]).map_err(clean)?;
        let mut data: Vec<Row> = Vec::new();
        let mut more = false;
        while let Some(row) = rows.next().map_err(clean)? {
            if data.len() > limit {
                more = true;
                break;
            }
            let mut cells = Vec::new();
            let mut i = 0;
            while let Ok(value) = row.get::<_, Value>(i) {
                cells.push(text_of(&value).0);
                i += 1;
            }
            data.push(cells);
        }
        let names = rows.as_ref().map(|s| s.column_names()).unwrap_or_default();
        Ok((names, data, more))
    }

    fn each(&mut self, sql: &str, sink: &mut common::Sink<'_>) -> Result<(), String> {
        let mut statement = self.conn.prepare(sql).map_err(clean)?;
        let mut rows = statement.query([]).map_err(clean)?;
        let mut names: Vec<String> = Vec::new();
        while let Some(row) = rows.next().map_err(clean)? {
            if names.is_empty() {
                names = row.as_ref().column_names();
            }
            let mut cells: Vec<(Option<String>, bool)> = Vec::new();
            let mut i = 0;
            while let Ok(value) = row.get::<_, Value>(i) {
                cells.push(text_of(&value));
                i += 1;
            }
            let numeric: Vec<bool> = cells.iter().map(|c| c.1).collect();
            sink(&names, &numeric, cells.into_iter().map(|c| c.0).collect())?;
        }
        Ok(())
    }

    fn execute(&mut self, sql: &str) -> Result<u64, String> {
        let done = self.conn.execute(sql, []).map_err(clean);
        // A COMMIT that fails ends the transaction too.
        if done.is_ok() || !tx_after(true, sql) {
            self.in_tx = tx_after(self.in_tx, sql);
        }
        if !self.in_tx {
            self.begun = false;
        }
        done.map(|changed| changed as u64)
    }

    fn columns_of(&mut self, sql: &str) -> Result<Option<Vec<String>>, String> {
        // Only a statement that gives rows can be wrapped to ask for none: a
        // failed attempt on another would abort the transaction that is open.
        if !protocol::wrappable(sql, protocol::Dialect::Duckdb) {
            return Ok(None);
        }
        let probe = format!(
            "SELECT * FROM ({}) AS probe LIMIT 0",
            sql.trim().trim_end_matches(';')
        );
        let Ok(mut statement) = self.conn.prepare(&probe) else {
            return Ok(None);
        };
        let Ok(rows) = statement.query([]) else {
            return Ok(None);
        };
        let names = rows.as_ref().map(|s| s.column_names()).unwrap_or_default();
        Ok((!names.is_empty()).then_some(names))
    }

    fn begin(&mut self) -> Result<(), String> {
        if !self.in_tx {
            self.conn.execute_batch("BEGIN").map_err(clean)?;
            self.in_tx = true;
            self.begun = true;
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<(), String> {
        if self.in_tx && self.begun {
            // Kept or not, the transaction is over.
            self.in_tx = false;
            self.begun = false;
            self.conn.execute_batch("COMMIT").map_err(clean)?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), String> {
        self.begun = false;
        if std::mem::take(&mut self.in_tx) {
            self.conn.execute_batch("ROLLBACK").map_err(clean)?;
        }
        Ok(())
    }

    fn in_transaction(&mut self) -> bool {
        self.in_tx
    }

    fn structure(&mut self, schema: &str, table: &str) -> Result<Structure, String> {
        let (s, t) = (literal(schema), literal(table));
        let columns: Vec<ColumnDef> = self
            .internal(&format!(
                "SELECT column_name, data_type, is_nullable, column_default FROM duckdb_columns() \
                 WHERE schema_name = {s} AND table_name = {t} ORDER BY column_index"
            ))?
            .iter()
            .map(|r| ColumnDef {
                name: cell(r, 0),
                kind: cell(r, 1),
                nullable: cell(r, 2) != "false",
                default: r.get(3).cloned().flatten(),
                identity: String::new(),
                generated: false,
                comment: String::new(),
            })
            .collect();
        if columns.is_empty() {
            return Err(format!("{table}: no such table"));
        }
        let mut constraints = Vec::new();
        let found = self.internal(&format!(
            "SELECT constraint_index, constraint_type, array_to_string(constraint_column_names, ','), \
                    COALESCE(expression, ''), COALESCE(referenced_table, ''), \
                    COALESCE(array_to_string(referenced_column_names, ','), '') \
             FROM duckdb_constraints() WHERE schema_name = {s} AND table_name = {t} \
             ORDER BY constraint_index"
        ))?;
        let list = |text: &str| {
            names(text)
                .iter()
                .map(|n| quote(n))
                .collect::<Vec<_>>()
                .join(", ")
        };
        for r in &found {
            let (kind, columns_text) = (cell(r, 1), cell(r, 2));
            let (id, definition, kind) = match kind.as_str() {
                "PRIMARY KEY" => (
                    "PRIMARY".to_string(),
                    format!("PRIMARY KEY ({})", list(&columns_text)),
                    ConstraintKind::Primary,
                ),
                "UNIQUE" => (
                    format!("unique_{}", cell(r, 0)),
                    format!("UNIQUE ({})", list(&columns_text)),
                    ConstraintKind::Unique,
                ),
                "FOREIGN KEY" => (
                    format!("fk_{}", cell(r, 0)),
                    format!(
                        "FOREIGN KEY ({}) REFERENCES {}({})",
                        list(&columns_text),
                        quote(&cell(r, 4)),
                        list(&cell(r, 5))
                    ),
                    ConstraintKind::Foreign,
                ),
                "CHECK" => (
                    format!("check_{}", cell(r, 0)),
                    format!("CHECK ({})", cell(r, 3)),
                    ConstraintKind::Check,
                ),
                _ => continue,
            };
            constraints.push(ConstraintDef {
                name: id,
                kind,
                definition,
            });
        }
        let indexes: Vec<IndexDef> = self
            .internal(&format!(
                "SELECT index_name, COALESCE(sql, ''), is_unique FROM duckdb_indexes() \
                 WHERE schema_name = {s} AND table_name = {t} ORDER BY index_name"
            ))?
            .iter()
            .map(|r| IndexDef {
                name: cell(r, 0),
                definition: cell(r, 1),
                unique: cell(r, 2) == "true",
            })
            .collect();
        let referenced_by: Vec<Reference> = self
            .internal(&format!(
                "SELECT schema_name, table_name, constraint_index, \
                        array_to_string(constraint_column_names, ','), \
                        array_to_string(referenced_column_names, ',') \
                 FROM duckdb_constraints() \
                 WHERE constraint_type = 'FOREIGN KEY' AND referenced_table = {t}"
            ))?
            .iter()
            .map(|r| Reference {
                name: format!("fk_{}", cell(r, 2)),
                schema: cell(r, 0),
                table: cell(r, 1),
                definition: format!(
                    "FOREIGN KEY ({}) REFERENCES {}({})",
                    list(&cell(r, 3)),
                    quote(table),
                    list(&cell(r, 4))
                ),
            })
            .collect();
        Ok(Structure {
            columns,
            constraints,
            indexes,
            comment: String::new(),
            referenced_by,
            extra: Box::default(),
        })
    }

    fn definition(&mut self, request: &Request) -> Result<String, String> {
        let (s, t) = (literal(&request.schema), literal(&request.table));
        let rows = self.internal(&format!(
            "SELECT sql FROM duckdb_views() WHERE schema_name = {s} AND view_name = {t} \
             UNION ALL \
             SELECT sql FROM duckdb_tables() WHERE schema_name = {s} AND table_name = {t}"
        ))?;
        let sql = rows
            .first()
            .map(|r| cell(r, 0))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{}: no such table or view", request.table))?;
        Ok(format!("{};\n", sql.trim_end().trim_end_matches(';')))
    }

    fn catalog(
        &mut self,
        request: &Request,
    ) -> Result<(Vec<TableColumn>, Vec<ForeignKey>), String> {
        let system = if request.scope.system_schemas {
            "1 = 1"
        } else {
            "schema_name NOT IN ('information_schema', 'pg_catalog')"
        };
        let columns = self
            .internal(&format!(
                "SELECT schema_name, table_name, column_name, data_type FROM duckdb_columns() \
                 WHERE NOT internal AND {system} \
                 ORDER BY schema_name, table_name, column_index LIMIT {}",
                protocol::MAX_COMPLETION_COLUMNS
            ))?
            .iter()
            .map(|r| TableColumn {
                schema: cell(r, 0),
                table: cell(r, 1),
                name: cell(r, 2),
                kind: cell(r, 3),
            })
            .collect();
        let keys = self
            .internal(
                "SELECT schema_name, table_name, array_to_string(constraint_column_names, ','), \
                        COALESCE(referenced_table, ''), \
                        COALESCE(array_to_string(referenced_column_names, ','), '') \
                 FROM duckdb_constraints() WHERE constraint_type = 'FOREIGN KEY' LIMIT 20000",
            )?
            .iter()
            .map(|r| ForeignKey {
                schema: cell(r, 0),
                table: cell(r, 1),
                columns: names(&cell(r, 2)),
                ref_schema: cell(r, 0),
                ref_table: cell(r, 3),
                ref_columns: names(&cell(r, 4)),
            })
            .collect();
        Ok((columns, keys))
    }

    fn health(&mut self, _request: &Request) -> Result<protocol::Health, String> {
        let rows = self.internal(
            "SELECT schema_name, table_name, estimated_size FROM duckdb_tables() \
             ORDER BY estimated_size DESC, table_name LIMIT 500",
        )?;
        let tables = rows
            .iter()
            .map(|r| protocol::TableHealth {
                schema: cell(r, 0),
                name: cell(r, 1),
                rows: cell(r, 2).parse().unwrap_or(0),
                table_size: 0,
                indexes_size: 0,
                total_size: 0,
                dead_rows: 0,
                last_vacuum: String::new(),
                last_analyze: String::new(),
            })
            .collect();
        let indexes = self
            .internal(
                "SELECT schema_name, table_name, index_name, is_unique FROM duckdb_indexes() \
                 ORDER BY index_name LIMIT 500",
            )?
            .iter()
            .map(|r| protocol::IndexHealth {
                schema: cell(r, 0),
                table: cell(r, 1),
                name: cell(r, 2),
                size: 0,
                scans: -1,
                unique: cell(r, 3) == "true",
                primary: false,
            })
            .collect();
        Ok(protocol::Health {
            database_size: std::fs::metadata(&self.path).map_or(0, |m| m.len() as i64),
            tables,
            indexes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value as Json};

    fn database() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("shop.duckdb")
            .to_string_lossy()
            .into_owned();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, name VARCHAR NOT NULL, born DATE, price DECIMAL(8,2), tags VARCHAR[]);
             CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), placed TIMESTAMP);
             CREATE VIEW names AS SELECT name FROM customers;
             CREATE INDEX by_name ON customers(name);
             INSERT INTO customers VALUES (1, 'Ann', DATE '2024-01-02', 1.5, ['a','b']), (2, 'Bob', NULL, NULL, NULL);
             INSERT INTO orders VALUES (1, 1, TIMESTAMP '2024-03-04 05:06:07');",
        )
        .unwrap();
        (dir, path)
    }

    fn call(path: &str, request: Json) -> Result<Json, String> {
        let mut request = request;
        request["target"] = json!({ "database": path, "dialect": "duckdb" });
        let open = |r: &Request| Session::open(r).map(|s| Box::new(s) as Box<dyn Engine>);
        common::run(&request.to_string(), &mut None, &open, false)
    }

    #[test]
    fn the_version_and_the_tree_are_read() {
        let (_dir, path) = database();
        let version = call(&path, json!({"op": "test"})).unwrap();
        assert!(version["version"].as_str().unwrap().starts_with("DuckDB"));
        let tree = call(&path, json!({"op": "tree"})).unwrap();
        let databases: Vec<Database> = serde_json::from_value(tree["databases"].clone()).unwrap();
        assert_eq!(databases[0].name, "shop.duckdb");
        let tables = &databases[0].schemas[0].tables;
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["customers", "names", "orders"]);
        assert_eq!(tables[1].kind, Kind::View);
        assert!(call("/no/such/file.duckdb", json!({"op": "test"}))
            .unwrap_err()
            .contains("no such file"));
    }

    #[test]
    fn rows_come_as_text_of_every_type() {
        let (_dir, path) = database();
        let page = call(
            &path,
            json!({"op": "rows", "schema": "main", "table": "customers", "limit": 10,
                   "order": [{"column": "id", "desc": false}]}),
        )
        .unwrap();
        assert_eq!(
            page["rows"][0],
            json!(["1", "Ann", "2024-01-02", "1.50", "[a, b]"])
        );
        assert_eq!(page["rows"][1], json!(["2", "Bob", null, null, null]));
        assert_eq!(page["primary_key"], json!(["id"]));
        assert_eq!(page["editable"], true);
        let orders = call(
            &path,
            json!({"op": "rows", "schema": "main", "table": "orders", "limit": 10}),
        )
        .unwrap();
        assert_eq!(orders["rows"][0][2], "2024-03-04 05:06:07");
        let view = call(
            &path,
            json!({"op": "rows", "schema": "main", "table": "names", "limit": 10}),
        )
        .unwrap();
        assert_eq!(view["editable"], false);
    }

    #[test]
    fn a_change_is_applied_and_a_filter_reads() {
        let (_dir, path) = database();
        let done = call(
            &path,
            json!({"op": "apply", "schema": "main", "table": "customers", "changes": [
                {"kind": "update", "key": [["id", "2"]], "set": [["name", "Bobby"]]},
                {"kind": "insert", "key": [], "set": [["id", "3"], ["name", "Cy"]]},
                {"kind": "delete", "key": [["id", "3"]], "set": []},
            ]}),
        )
        .unwrap();
        assert_eq!(done["applied"], 3, "{done}");
        let found = call(
            &path,
            json!({"op": "rows", "schema": "main", "table": "customers", "limit": 10,
                   "filter": "name ILIKE 'b%'"}),
        )
        .unwrap();
        assert_eq!(found["rows"].as_array().unwrap().len(), 1);
        assert_eq!(found["rows"][0][1], "Bobby");
        let total = call(
            &path,
            json!({"op": "count", "schema": "main", "table": "customers"}),
        )
        .unwrap();
        assert_eq!(total["total"], 2);
    }

    #[test]
    fn the_structure_the_definition_and_the_catalog_are_read() {
        let (_dir, path) = database();
        let s = call(
            &path,
            json!({"op": "structure", "schema": "main", "table": "customers"}),
        )
        .unwrap();
        let structure: Structure = serde_json::from_value(s["structure"].clone()).unwrap();
        assert_eq!(structure.columns[1].name, "name");
        assert!(!structure.columns[1].nullable);
        assert!(structure
            .constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::Primary && c.definition.contains("\"id\"")));
        assert!(structure.indexes.iter().any(|i| i.name == "by_name"));
        assert!(structure.referenced_by.iter().any(|r| r.table == "orders"));
        let def = call(
            &path,
            json!({"op": "definition", "schema": "main", "table": "names", "object": "view"}),
        )
        .unwrap();
        assert!(def["definition"].as_str().unwrap().contains("SELECT"));
        let catalog = call(&path, json!({"op": "columns"})).unwrap();
        let columns: Vec<TableColumn> = serde_json::from_value(catalog["columns"].clone()).unwrap();
        assert!(columns
            .iter()
            .any(|c| c.table == "orders" && c.name == "placed"));
        assert!(catalog["foreign_keys"][0]["ref_table"] == "customers");
    }

    #[test]
    fn statements_run_in_a_transaction_and_a_failure_says_where() {
        let (_dir, path) = database();
        let ran = call(
            &path,
            json!({"op": "query", "transaction": true, "statements": [
                "INSERT INTO customers (id, name) VALUES (9, 'Zed')",
                "INSERT INTO customers (id, name) VALUES (9, 'Dup')",
            ]}),
        )
        .unwrap();
        assert_eq!(ran["failed"]["index"], 1, "{ran}");
        let none = call(
            &path,
            json!({"op": "count", "schema": "main", "table": "customers", "filter": "id = 9"}),
        )
        .unwrap();
        assert_eq!(none["total"], 0, "all or nothing");
    }

    #[test]
    fn a_transaction_of_ones_own_lasts_until_it_is_ended() {
        let (_dir, path) = database();
        let open = |r: &Request| Session::open(r).map(|s| Box::new(s) as Box<dyn Engine>);
        let mut slot = None;
        let mut ask = |statements: Json| {
            let request = json!({"op": "query", "statements": statements,
                "target": {"database": path, "dialect": "duckdb"}});
            common::run(&request.to_string(), &mut slot, &open, true).unwrap()
        };
        let begun = ask(json!([
            "BEGIN",
            "INSERT INTO customers (id, name) VALUES (7, 'Tx')"
        ]));
        assert_eq!(begun["in_transaction"], true, "{begun}");
        let undone = ask(json!(["ROLLBACK"]));
        assert_eq!(undone["in_transaction"], false, "{undone}");
        ask(json!([
            "BEGIN",
            "INSERT INTO customers (id, name) VALUES (8, 'Kept')"
        ]));
        let kept = ask(json!(["COMMIT"]));
        assert_eq!(kept["in_transaction"], false, "{kept}");
        drop(slot);
        // Read again from the file: only what was committed is there.
        let total = call(
            &path,
            json!({"op": "count", "schema": "main", "table": "customers", "filter": "id IN (7, 8)"}),
        )
        .unwrap();
        assert_eq!(total["total"], 1);
    }

    #[test]
    fn a_statement_past_the_time_limit_is_stopped() {
        let (_dir, path) = database();
        let request = json!({"op": "query",
            "statements": ["CREATE TABLE big AS SELECT SUM(a.range * b.range) FROM range(1000000) a, range(1000000) b"],
            "target": {"database": path, "dialect": "duckdb", "query_timeout": 1}});
        let open = |r: &Request| Session::open(r).map(|s| Box::new(s) as Box<dyn Engine>);
        let started = std::time::Instant::now();
        let ran = common::run(&request.to_string(), &mut None, &open, false).unwrap();
        assert!(
            ran["failed"]["error"]
                .as_str()
                .unwrap()
                .contains("time limit"),
            "{ran}"
        );
        assert!(started.elapsed().as_secs() < 30);
    }

    #[test]
    fn a_read_only_file_is_not_changed() {
        let (_dir, path) = database();
        let request = json!({"op": "query",
            "statements": ["DELETE FROM orders"],
            "target": {"database": path, "dialect": "duckdb", "read_only": true}});
        let open = |r: &Request| Session::open(r).map(|s| Box::new(s) as Box<dyn Engine>);
        let ran = common::run(&request.to_string(), &mut None, &open, false);
        assert!(ran.map_or(true, |a| !a["failed"].is_null()));
        let total = call(
            &path,
            json!({"op": "count", "schema": "main", "table": "orders"}),
        )
        .unwrap();
        assert_eq!(total["total"], 1);
    }

    #[test]
    fn the_health_lists_the_tables_and_the_other_sessions_are_not_there() {
        let (_dir, path) = database();
        let health = call(&path, json!({"op": "health"})).unwrap();
        assert!(health["health"]["database_size"].as_i64().unwrap() > 0);
        assert!(health["health"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "customers"));
        assert!(call(&path, json!({"op": "sessions"}))
            .unwrap_err()
            .contains("not available"));
    }
}
