# Database — an Ardoise Editor plugin

Databases in [Ardoise Editor](https://github.com/ardoise-editor/ardoise): PostgreSQL, MySQL / MariaDB, SQLite and DuckDB, directly or through an SSH tunnel, with TLS.

- **The tree** of each data source (sidebar): databases, schemas, tables and what they are made of, functions, sequences, types; under it, what can be done to the node picked (open, structure, modify, DDL, import, copy, backup and restore, empty, drop, new table or schema, ER diagram, health).
- **A table in a tab**: its rows by pages, a WHERE condition, a search, filters by column, sorts, hidden columns, the row as a form; cells edited, rows added and deleted, all waiting for Commit (the SQL shown first); a date picker; links to the rows a row points to and those that point to it; export to CSV, JSON, SQL, XLSX and Markdown.
- **A SQL console** (under the editor): queries in tabs, completion of tables and columns, the statement under the cursor or the selection run, Explain, transactions, values asked for `:name` and `?`, a guard before writes without WHERE, and the log of what was run.
- **Popins**: the connection form (a tile for the kind of source, six tabs, test as a picto), the designer, the health, import, copy, backup and restore open over the editor in the frame every popin shares; the diagram stays a tab. The tools of the panel and of the rows are pictos with a tooltip.
- **The table designer**: columns, keys and constraints, indexes, name and comment, and the SQL that comes of it.
- **The ER diagram** of a schema (copied as Mermaid), **the health** of a database (sizes, dead rows, unused indexes), **import** of CSV and JSON, **copy** of a table to another source, **backup and restore**.

- **Also**: queries saved under a name, a confirmation window that shows each query before it runs (an option of the data source), the monitor of sessions and locks, roles and privileges, the comparison of two schemas (the SQL that makes one like the other), a search for a text in every table, test data for a table, a `.env` file that fills the form of a data source, a colour and a group for each data source, and a hex dump (with the picture, when it is one) of binary values.

A DuckDB file is opened by one process at a time (a rule of DuckDB): while the plugin has it open, another program cannot write to it.

Passwords are kept in the system keychain when asked to. The drivers are programs of their own, shipped in the plugin.

## Build

```sh
cargo test
cargo build --release && mkdir -p bin && cp target/release/{database,ardoise-db-postgres,ardoise-db-sqlite,ardoise-db-mysql,ardoise-db-duckdb} bin/
```

## License

MIT
