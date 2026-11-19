# Database — an Ardoise Editor plugin

Databases in [Ardoise Editor](https://github.com/ardoise-editor/ardoise): PostgreSQL, MySQL / MariaDB and SQLite, directly or through an SSH tunnel, with TLS.

- **The tree** of each data source (sidebar): databases, schemas, tables and what they are made of, functions, sequences, types; under it, what can be done to the node picked (open, structure, modify, DDL, import, copy, backup and restore, empty, drop, new table or schema, ER diagram, health).
- **A table in a tab**: its rows by pages, a WHERE condition, a search, filters by column, sorts, hidden columns, the row as a form; cells edited, rows added and deleted, all waiting for Commit (the SQL shown first); a date picker; links to the rows a row points to and those that point to it; export to CSV, JSON, SQL, XLSX and Markdown.
- **A SQL console** (under the editor): queries in tabs, completion of tables and columns, the statement under the cursor or the selection run, Explain, transactions, values asked for `:name` and `?`, a guard before writes without WHERE, and the log of what was run.
- **The table designer**: columns, keys and constraints, indexes, name and comment, and the SQL that comes of it.
- **The ER diagram** of a schema (copied as Mermaid), **the health** of a database (sizes, dead rows, unused indexes), **import** of CSV and JSON, **copy** of a table to another source, **backup and restore**.

Passwords are kept in the system keychain when asked to. The drivers are programs of their own, shipped in the plugin.

## Build

```sh
cargo test
cargo build --release && mkdir -p bin && cp target/release/{database,ardoise-db-postgres,ardoise-db-sqlite,ardoise-db-mysql} bin/
```

## License

MIT
