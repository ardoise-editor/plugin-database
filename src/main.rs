//! The Database plugin of Ardoise: data sources (PostgreSQL, MySQL /
//! MariaDB, SQLite, through SSH and TLS), their tree, the rows of a table in
//! a tab to read, filter, sort and edit, a SQL console with its log and its
//! completion, the designer of a table, the ER diagram of a schema, import,
//! export, copy and backup of tables, the health of a database.

#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en");

mod app;
mod database;
mod editor;
// Shared with the drivers, which encode what they import.
#[allow(dead_code)]
mod encoding;
mod fsutil;
mod ssh_trust;
mod task;
mod text_editor;
mod ui;

pub const TREE: &str = "database";
pub const CONSOLE: &str = "console";

fn main() {
    ui::run();
}
