//! The tools of a database: its health, and what its messages do.

use super::Task;

use super::{App, Message};
use crate::database::import::{self, Format};
use crate::database::{
    self, Backup, BackupFlag, CopyField, CopyTable, DbMsg, HealthTab, HealthView, Import, TableRef,
};

impl App {
    pub(super) fn db_tools_message(&mut self, msg: DbMsg) -> Task<Message> {
        match msg {
            DbMsg::OpenHealth(conn, database) => {
                let dialect = self
                    .db
                    .connection(conn)
                    .map_or_else(Default::default, |c| c.dialect);
                self.db.health = Some(HealthView {
                    conn,
                    dialect,
                    sessions: None,
                    sessions_loading: false,
                    roles: None,
                    database,
                    data: None,
                    loading: true,
                    error: None,
                    tab: HealthTab::Tables,
                    running: None,
                });
                self.db_health_load()
            }
            DbMsg::HealthLoaded(done) => {
                let conn = self.db.health.as_ref().map(|h| h.conn);
                let log = conn.map_or_else(Task::none, |c| self.db_log(c, &done.executed));
                if let Some(view) = self.db.health.as_mut() {
                    view.loading = false;
                    match done.result {
                        Ok(data) => {
                            view.data = Some(data);
                            view.error = None;
                        }
                        Err(error) => view.error = Some(error),
                    }
                }
                log
            }
            DbMsg::HealthTab(tab) => {
                if let Some(view) = self.db.health.as_mut() {
                    view.tab = tab;
                }
                if tab == HealthTab::Sessions {
                    return self.db_sessions_load();
                }
                if tab == HealthTab::Roles {
                    return self.db_roles_load();
                }
                Task::none()
            }
            DbMsg::CompareOpen(conn, database, schema) => {
                let mut choices = Vec::new();
                for connection in &self.db.connections {
                    if let Some(database::Tree::Loaded(databases)) =
                        self.db.trees.get(&connection.id)
                    {
                        for d in databases {
                            for s in &d.schemas {
                                if connection.id == conn && d.name == database && s.name == schema {
                                    continue;
                                }
                                choices.push((connection.id, d.name.clone(), s.name.clone()));
                            }
                        }
                    }
                }
                self.db.compare = Some(database::Compare {
                    conn,
                    database,
                    schema,
                    picked: (!choices.is_empty()).then_some(0),
                    choices,
                    running: false,
                    error: None,
                });
                Task::none()
            }
            DbMsg::ComparePick(at) => {
                if let Some(c) = self.db.compare.as_mut().filter(|c| !c.running) {
                    c.picked = Some(at).filter(|a| *a < c.choices.len());
                }
                Task::none()
            }
            DbMsg::CompareClose => {
                if self.db.compare.as_ref().is_some_and(|c| !c.running) {
                    self.db.compare = None;
                }
                Task::none()
            }
            DbMsg::CompareRun => {
                let Some(c) = self.db.compare.as_ref().filter(|c| !c.running) else {
                    return Task::none();
                };
                let Some((tconn, tdatabase, tschema)) =
                    c.picked.and_then(|at| c.choices.get(at).cloned())
                else {
                    return Task::none();
                };
                let (sconn, sdatabase, sschema) = (c.conn, c.database.clone(), c.schema.clone());
                let (Some(source), Some(target)) =
                    (self.db_params(sconn, false), self.db_params(tconn, false))
                else {
                    return self.db_ask_password(sconn);
                };
                let dialect = self
                    .db
                    .connection(tconn)
                    .map_or_else(Default::default, |k| k.dialect);
                if let Some(c) = self.db.compare.as_mut() {
                    c.running = true;
                    c.error = None;
                }
                Task::perform(
                    async move {
                        let first = database::table_columns(source, sdatabase).await.result?;
                        let second = database::table_columns(target, tdatabase).await.result?;
                        Ok(database::compare::migration(
                            dialect,
                            &first.columns,
                            &sschema,
                            &second.columns,
                            &tschema,
                        ))
                    },
                    |done| Message::Db(DbMsg::Compared(done)),
                )
            }
            DbMsg::Compared(done) => {
                let Some(c) = self.db.compare.as_mut() else {
                    return Task::none();
                };
                c.running = false;
                match done {
                    Ok(sql) if sql.trim().is_empty() => {
                        c.error = Some(t!("db.compare_same").to_string())
                    }
                    Ok(sql) => {
                        let title = t!("db.compare_result").to_string();
                        self.db.compare = None;
                        self.db.text = Some(database::TextView {
                            title,
                            text: sql,
                            sql: true,
                            image: None,
                        });
                    }
                    Err(error) => c.error = Some(error),
                }
                Task::none()
            }
            DbMsg::SearchAllOpen(conn, database, schema) => {
                self.db.search_all = Some(database::SearchAll {
                    conn,
                    database,
                    schema,
                    text: String::new(),
                    running: false,
                    hits: None,
                    error: None,
                });
                Task::none()
            }
            DbMsg::SearchAllText(text) => {
                if let Some(s) = self.db.search_all.as_mut().filter(|s| !s.running) {
                    s.text = text;
                }
                Task::none()
            }
            DbMsg::SearchAllClose => {
                if self.db.search_all.as_ref().is_some_and(|s| !s.running) {
                    self.db.search_all = None;
                }
                Task::none()
            }
            DbMsg::SearchAllRun => {
                let Some(s) = self
                    .db
                    .search_all
                    .as_ref()
                    .filter(|s| !s.running && !s.text.trim().is_empty())
                else {
                    return Task::none();
                };
                let (conn, database, schema, text) =
                    (s.conn, s.database.clone(), s.schema.clone(), s.text.clone());
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                if let Some(s) = self.db.search_all.as_mut() {
                    s.running = true;
                    s.error = None;
                    s.hits = None;
                }
                Task::perform(
                    database::search_all(params, database, schema, text),
                    |done| Message::Db(DbMsg::SearchAllDone(done)),
                )
            }
            DbMsg::SearchAllDone(done) => {
                if let Some(s) = self.db.search_all.as_mut() {
                    s.running = false;
                    match done {
                        Ok(hits) => s.hits = Some(hits),
                        Err(error) => s.error = Some(error),
                    }
                }
                Task::none()
            }
            DbMsg::SearchAllShow(at) => {
                let Some(s) = self.db.search_all.take() else {
                    return Task::none();
                };
                let Some(hit) = s.hits.as_ref().and_then(|h| h.get(at)).cloned() else {
                    self.db.search_all = Some(s);
                    return Task::none();
                };
                self.db_open_table_with(s.conn, s.database, hit.schema, hit.table, Some(hit.filter))
            }
            DbMsg::HealthRefresh => {
                let sessions = self.db_sessions_load();
                let roles = self.db_roles_load();
                Task::batch([self.db_health_load(), sessions, roles])
            }
            DbMsg::RolesLoaded(done) => {
                let conn = self.db.health.as_ref().map(|h| h.conn);
                let log = conn.map_or_else(Task::none, |c| self.db_log(c, &done.executed));
                if let Some(view) = self.db.health.as_mut() {
                    view.loading = false;
                    match done.result {
                        Ok(list) => {
                            view.roles = Some(list);
                            view.error = None;
                        }
                        Err(error) => view.error = Some(error),
                    }
                }
                log
            }
            DbMsg::SessionsLoaded(done) => {
                let conn = self.db.health.as_ref().map(|h| h.conn);
                let log = conn.map_or_else(Task::none, |c| self.db_log(c, &done.executed));
                if let Some(view) = self.db.health.as_mut() {
                    view.sessions_loading = false;
                    match done.result {
                        Ok(list) => {
                            view.sessions = Some(list);
                            view.error = None;
                        }
                        Err(error) => view.error = Some(error),
                    }
                }
                log
            }
            DbMsg::HealthRun(statement) => {
                let Some((conn, database)) = self
                    .db
                    .health
                    .as_ref()
                    .filter(|h| h.running.is_none())
                    .map(|h| (h.conn, h.database.clone()))
                else {
                    return Task::none();
                };
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                if let Some(view) = self.db.health.as_mut() {
                    view.running = Some(statement.clone());
                    view.error = None;
                }
                Task::perform(
                    database::run_statements_in(params, database, vec![statement]),
                    |done| Message::Db(DbMsg::HealthRan(done)),
                )
            }
            DbMsg::HealthRan(done) => {
                let conn = self.db.health.as_ref().map(|h| h.conn);
                let log = conn.map_or_else(Task::none, |c| self.db_log(c, &done.executed));
                let failure = match &done.result {
                    Ok(ran) => ran.failed.as_ref().map(|(_, e)| e.clone()),
                    Err(e) => Some(e.clone()),
                };
                let Some(view) = self.db.health.as_mut() else {
                    return log;
                };
                view.running = None;
                match failure {
                    // The numbers are read again: what was cleaned shows.
                    None => {
                        let sessions = self.db_sessions_load();
                        let roles = self.db_roles_load();
                        Task::batch([log, self.db_health_load(), sessions, roles])
                    }
                    Some(error) => {
                        view.error = Some(error);
                        log
                    }
                }
            }
            DbMsg::HealthToConsole(sql) => {
                self.db.console.append_query(&sql);
                self.db.console.save();
                self.db.console.tab = database::console::ConsoleTab::Queries;
                self.db.console.open = true;
                self.db.health = None;
                Task::none()
            }
            DbMsg::ImportOpen(conn, database, schema, table) => {
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                self.db.import = Some(Import {
                    conn,
                    database: database.clone(),
                    schema: schema.clone(),
                    table: table.clone(),
                    targets: Vec::new(),
                    path: None,
                    format: Format::Csv,
                    delimiter: ',',
                    header: true,
                    empty_null: true,
                    encoding: crate::encoding::Encoding::Utf8,
                    raw: Vec::new(),
                    text: String::new(),
                    sample: import::Sample::default(),
                    mapping: Vec::new(),
                    running: false,
                    error: None,
                    imported: None,
                });
                let window = database::AnswerFor::Import(conn, schema.clone(), table.clone());
                Task::perform(
                    database::structure(
                        params,
                        TableRef {
                            database,
                            schema,
                            table,
                        },
                    ),
                    move |done| {
                        Message::Db(DbMsg::Answer(
                            window.clone(),
                            Box::new(DbMsg::ImportStructure(done)),
                        ))
                    },
                )
            }
            DbMsg::ImportStructure(done) => {
                let conn = self.db.import.as_ref().map(|i| i.conn);
                let log = conn.map_or_else(Task::none, |c| self.db_log(c, &done.executed));
                if let Some(im) = self.db.import.as_mut() {
                    match done.result {
                        Ok(structure) => {
                            // A generated column, and an identity that is
                            // always generated, cannot be filled.
                            im.targets = structure
                                .columns
                                .iter()
                                .filter(|c| !c.generated && c.identity != "a")
                                .map(|c| import::Target {
                                    name: c.name.clone(),
                                    kind: c.kind.clone(),
                                })
                                .collect();
                            im.reread();
                        }
                        Err(error) => im.error = Some(error),
                    }
                }
                log
            }
            DbMsg::ImportPick => Task::perform(
                async {
                    crate::editor::pick_file(String::new(), &["csv", "tsv", "txt", "json"]).await
                },
                |path| Message::Db(DbMsg::ImportPicked(path)),
            ),
            DbMsg::ImportPicked(path) => {
                let (Some(path), Some(_)) = (path, self.db.import.as_ref()) else {
                    return Task::none();
                };
                let format = Format::of(&path.to_string_lossy());
                // A JSON is read whole (up to 64 MiB): not on this thread.
                if cfg!(test) {
                    let read = read_sample(&path, format);
                    return self.db_import_read(path, read);
                }
                Task::perform(
                    async move {
                        let from = path.clone();
                        let read = tokio::task::spawn_blocking(move || read_sample(&from, format))
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r);
                        (path, read)
                    },
                    |(path, read)| Message::Db(DbMsg::ImportRead(path, read)),
                )
            }
            DbMsg::ImportRead(path, read) => self.db_import_read(path, read),
            DbMsg::ImportFormat(format) => {
                if let Some(im) = self.db.import.as_mut() {
                    im.format = format;
                    im.reread();
                }
                Task::none()
            }
            DbMsg::ImportEncoding(encoding) => {
                if let Some(im) = self.db.import.as_mut() {
                    if !im.raw.is_empty() {
                        // The accents are read again from the bytes.
                        let (text, _) = match encoding {
                            crate::encoding::Encoding::Utf16Le
                            | crate::encoding::Encoding::Utf16Be => {
                                let cut = im.raw.len() - im.raw.len() % 2;
                                (
                                    encoding.decode(&im.raw[..cut]).unwrap_or_default(),
                                    encoding,
                                )
                            }
                            crate::encoding::Encoding::Utf8 => {
                                (String::from_utf8_lossy(&im.raw).into_owned(), encoding)
                            }
                            other => (other.decode(&im.raw).unwrap_or_default(), other),
                        };
                        im.encoding = encoding;
                        im.text = text;
                        im.delimiter = import::detect_delimiter(&im.text);
                        im.reread();
                    }
                }
                Task::none()
            }
            DbMsg::ImportDelimiter(delimiter) => {
                if let Some(im) = self.db.import.as_mut() {
                    im.delimiter = delimiter;
                    im.reread();
                }
                Task::none()
            }
            DbMsg::ImportHeader(on) => {
                if let Some(im) = self.db.import.as_mut() {
                    im.header = on;
                    im.reread();
                }
                Task::none()
            }
            DbMsg::ImportEmptyNull(on) => {
                if let Some(im) = self.db.import.as_mut() {
                    im.empty_null = on;
                }
                Task::none()
            }
            DbMsg::ImportMap(at, to) => {
                if let Some(im) = self.db.import.as_mut() {
                    // A column is filled by one field only.
                    if let Some(name) = &to {
                        for other in im.mapping.iter_mut() {
                            if other.as_ref() == Some(name) {
                                *other = None;
                            }
                        }
                    }
                    if let Some(slot) = im.mapping.get_mut(at) {
                        *slot = to;
                    }
                }
                Task::none()
            }
            DbMsg::ImportRun => {
                let Some(im) = self.db.import.as_ref().filter(|i| !i.running) else {
                    return Task::none();
                };
                let (Some(path), true) = (im.path.clone(), im.mapping.iter().any(Option::is_some))
                else {
                    return Task::none();
                };
                let spec = import::spec(
                    im.format,
                    im.encoding,
                    im.delimiter,
                    im.header,
                    im.empty_null,
                    &im.sample.names,
                    &im.mapping,
                    &im.targets,
                );
                let (conn, table) = (
                    im.conn,
                    TableRef {
                        database: im.database.clone(),
                        schema: im.schema.clone(),
                        table: im.table.clone(),
                    },
                );
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                if let Some(im) = self.db.import.as_mut() {
                    im.running = true;
                    im.error = None;
                    im.imported = None;
                }
                Task::perform(
                    database::import_file(params, table, path.to_string_lossy().to_string(), spec),
                    |done| Message::Db(DbMsg::ImportDone(done)),
                )
            }
            DbMsg::ImportDone(done) => {
                let Some(im) = self.db.import.as_mut() else {
                    return Task::none();
                };
                im.running = false;
                let (conn, schema, table) = (im.conn, im.schema.clone(), im.table.clone());
                let log = self.db_log(conn, &done.executed);
                let Some(im) = self.db.import.as_mut() else {
                    return log;
                };
                match done.result {
                    Ok(n) => {
                        im.imported = Some(n);
                        // The tabs of the table show the new rows.
                        let tabs: Vec<_> = self
                            .db_tabs
                            .iter()
                            .filter(|(_, t)| {
                                t.conn == conn
                                    && t.schema == schema
                                    && t.table == table
                                    && t.query.is_none()
                                    && t.pending.is_empty()
                            })
                            .map(|(p, _)| p.clone())
                            .collect();
                        let reloads: Vec<_> = tabs
                            .into_iter()
                            .map(|p| self.db_message(DbMsg::Refresh(p)))
                            .collect();
                        Task::batch(std::iter::once(log).chain(reloads))
                    }
                    Err(error) => {
                        im.error = Some(error);
                        log
                    }
                }
            }
            DbMsg::CopyOpen(conn, database, schema, table) => {
                self.db.copy_table = Some(CopyTable {
                    name: table.clone(),
                    schema: schema.clone(),
                    from: (conn, database, schema, table),
                    dest: Some(conn),
                    database: String::new(),
                    with_data: true,
                    running: false,
                    error: None,
                    copied: None,
                });
                Task::none()
            }
            DbMsg::CopyDest(id) => {
                if let Some(c) = self.db.copy_table.as_mut().filter(|c| !c.running) {
                    c.dest = Some(id);
                    c.error = None;
                    c.copied = None;
                }
                Task::none()
            }
            DbMsg::CopyField(field, text) => {
                if let Some(c) = self.db.copy_table.as_mut().filter(|c| !c.running) {
                    match field {
                        CopyField::Database => c.database = text,
                        CopyField::Schema => c.schema = text,
                        CopyField::Name => c.name = text,
                    }
                    c.error = None;
                    c.copied = None;
                }
                Task::none()
            }
            DbMsg::CopyData(on) => {
                if let Some(c) = self.db.copy_table.as_mut().filter(|c| !c.running) {
                    c.with_data = on;
                }
                Task::none()
            }
            DbMsg::CopyRun => {
                let Some(c) = self.db.copy_table.as_ref().filter(|c| !c.running) else {
                    return Task::none();
                };
                let (name, schema) = (c.name.trim().to_string(), c.schema.trim().to_string());
                let Some(dest) = c.dest else {
                    return Task::none();
                };
                if name.is_empty() || schema.is_empty() {
                    return Task::none();
                }
                let (src_conn, src_db, src_schema, src_table) = c.from.clone();
                let dest_database = if c.database.trim().is_empty() {
                    self.db
                        .connection(dest)
                        .map(|conn| conn.database.clone())
                        .unwrap_or_default()
                } else {
                    c.database.trim().to_string()
                };
                let with_data = c.with_data;
                let Some(source) = self.db_params(src_conn, false) else {
                    return self.db_ask_password(src_conn);
                };
                let Some(target) = self.db_params(dest, false) else {
                    return self.db_ask_password(dest);
                };
                if let Some(c) = self.db.copy_table.as_mut() {
                    c.running = true;
                    c.error = None;
                    c.copied = None;
                }
                let from = TableRef {
                    database: src_db,
                    schema: src_schema,
                    table: src_table,
                };
                let to = TableRef {
                    database: dest_database,
                    schema,
                    table: name,
                };
                Task::perform(
                    database::copy_table(source, target, from, to, with_data),
                    |copied| Message::Db(DbMsg::CopyDone(Box::new(copied))),
                )
            }
            DbMsg::CopyDone(copied) => {
                let copied = *copied;
                let Some((source, dest)) = self
                    .db
                    .copy_table
                    .as_ref()
                    .map(|c| (c.from.0, c.dest.unwrap_or(c.from.0)))
                else {
                    return Task::none();
                };
                let logs = Task::batch([
                    self.db_log(source, &copied.source_log),
                    self.db_log(dest, &copied.dest_log),
                ]);
                let Some(c) = self.db.copy_table.as_mut() else {
                    return logs;
                };
                c.running = false;
                match copied.result {
                    Ok(rows) => {
                        c.copied = Some(rows);
                        // The tree of the data source it went to shows it.
                        let reload = self.db_message(DbMsg::Reload(dest));
                        Task::batch([logs, reload])
                    }
                    Err(error) => {
                        c.error = Some(error);
                        logs
                    }
                }
            }
            DbMsg::CopyClose => {
                if self.db.copy_table.as_ref().is_some_and(|c| !c.running) {
                    self.db.copy_table = None;
                }
                Task::none()
            }
            DbMsg::BackupOpen(conn, database, schema, table, restore) => {
                let dialect = self
                    .db
                    .connection(conn)
                    .map(|c| c.dialect)
                    .unwrap_or_default();
                self.db.backup = Some(Backup {
                    conn,
                    dialect,
                    database,
                    schema,
                    table,
                    restore,
                    spec: database::protocol::ToolSpec {
                        // The others write SQL, and only SQL.
                        format: if dialect == database::protocol::Dialect::Postgres {
                            "custom"
                        } else {
                            "plain"
                        }
                        .into(),
                        no_owner: true,
                        no_privileges: true,
                        single_transaction: restore,
                        ..Default::default()
                    },
                    path: None,
                    running: false,
                    error: None,
                    done: None,
                });
                Task::none()
            }
            DbMsg::BackupFlag(flag, on) => {
                if let Some(b) = self.db.backup.as_mut().filter(|b| !b.running) {
                    match flag {
                        BackupFlag::Plain => {
                            b.spec.format = if on { "plain" } else { "custom" }.to_string()
                        }
                        BackupFlag::SchemaOnly => {
                            b.spec.schema_only = on;
                            b.spec.data_only &= !on;
                        }
                        BackupFlag::DataOnly => {
                            b.spec.data_only = on;
                            b.spec.schema_only &= !on;
                        }
                        BackupFlag::NoOwner => b.spec.no_owner = on,
                        BackupFlag::NoPrivileges => b.spec.no_privileges = on,
                        BackupFlag::Clean => b.spec.clean = on,
                        BackupFlag::SingleTransaction => b.spec.single_transaction = on,
                    }
                    b.error = None;
                    b.done = None;
                }
                Task::none()
            }
            DbMsg::BackupPick => {
                let Some(b) = self.db.backup.as_ref().filter(|b| !b.running) else {
                    return Task::none();
                };
                let restore = b.restore;
                let name = [&b.database, &b.schema, &b.table]
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(".");
                let ext = if b.spec.format == "plain" {
                    "sql"
                } else {
                    "dump"
                };
                Task::perform(
                    async move {
                        if restore {
                            crate::editor::pick_file(String::new(), &[]).await
                        } else {
                            crate::editor::pick_save_path(format!("{name}.{ext}")).await
                        }
                    },
                    |path| Message::Db(DbMsg::BackupPicked(path)),
                )
            }
            DbMsg::BackupPicked(path) => {
                if let (Some(path), Some(b)) = (path, self.db.backup.as_mut()) {
                    b.path = Some(path);
                    b.error = None;
                    b.done = None;
                }
                Task::none()
            }
            DbMsg::BackupRun => {
                let Some(b) = self.db.backup.as_ref().filter(|b| !b.running) else {
                    return Task::none();
                };
                let Some(path) = b.path.clone() else {
                    return Task::none();
                };
                let (conn, restore) = (b.conn, b.restore);
                let (database, schema, table) =
                    (b.database.clone(), b.schema.clone(), b.table.clone());
                let spec = b.spec.clone();
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                if let Some(b) = self.db.backup.as_mut() {
                    b.running = true;
                    b.error = None;
                    b.done = None;
                }
                let path = path.to_string_lossy().to_string();
                if restore {
                    Task::perform(
                        database::restore_backup(params, database, path, spec),
                        |done| Message::Db(DbMsg::BackupDone(done)),
                    )
                } else {
                    Task::perform(
                        database::backup(params, database, schema, table, path, spec),
                        |done| Message::Db(DbMsg::BackupDone(done)),
                    )
                }
            }
            DbMsg::BackupDone(done) => {
                let Some(conn) = self.db.backup.as_ref().map(|b| b.conn) else {
                    return Task::none();
                };
                let log = self.db_log(conn, &done.executed);
                let restored = self.db.backup.as_ref().is_some_and(|b| b.restore);
                if let Some(b) = self.db.backup.as_mut() {
                    b.running = false;
                    match done.result {
                        Ok(said) => b.done = Some(said),
                        Err(error) => b.error = Some(error),
                    }
                }
                // What a restore made is in the tree.
                let ok = self.db.backup.as_ref().is_some_and(|b| b.done.is_some());
                if restored && ok {
                    Task::batch([log, self.db_message(DbMsg::Reload(conn))])
                } else {
                    log
                }
            }
            DbMsg::BackupClose => {
                if self.db.backup.as_ref().is_some_and(|b| !b.running) {
                    self.db.backup = None;
                }
                Task::none()
            }
            DbMsg::ImportClose => {
                if self.db.import.as_ref().is_some_and(|i| !i.running) {
                    self.db.import = None;
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The start of the file to import has been read.
    fn db_import_read(
        &mut self,
        path: std::path::PathBuf,
        read: Result<Vec<u8>, String>,
    ) -> Task<Message> {
        let Some(im) = self.db.import.as_mut() else {
            return Task::none();
        };
        im.format = Format::of(&path.to_string_lossy());
        match read {
            Ok(bytes) => {
                // A JSON is UTF-8 by its standard; a CSV may be anything
                // (Excel writes Windows-1252).
                let (text, encoding) = if im.format == Format::Csv {
                    import::detect_sample(&bytes, bytes.len() >= import::SAMPLE_BYTES)
                } else {
                    (
                        String::from_utf8_lossy(&bytes).into_owned(),
                        crate::encoding::Encoding::Utf8,
                    )
                };
                if im.format == Format::Csv {
                    im.delimiter = import::detect_delimiter(&text);
                    im.raw = bytes;
                } else {
                    im.raw = Vec::new();
                }
                im.encoding = encoding;
                im.text = text;
                im.error = None;
                im.imported = None;
                im.path = Some(path);
                im.reread();
            }
            Err(error) => im.error = Some(error),
        }
        Task::none()
    }

    /// Reads the roles of the server, if their tab is the one on show.
    fn db_roles_load(&mut self) -> Task<Message> {
        let Some((conn, database)) = self
            .db
            .health
            .as_ref()
            .filter(|h| h.tab == HealthTab::Roles)
            .map(|h| (h.conn, h.database.clone()))
        else {
            return Task::none();
        };
        let Some(params) = self.db_params(conn, false) else {
            self.db.health = None;
            return self.db_ask_password(conn);
        };
        if let Some(view) = self.db.health.as_mut() {
            view.loading = true;
        }
        let window = database::AnswerFor::Health(conn, database.clone());
        Task::perform(database::roles(params, database), move |done| {
            Message::Db(DbMsg::Answer(
                window.clone(),
                Box::new(DbMsg::RolesLoaded(done)),
            ))
        })
    }

    /// Reads the sessions of the server, if their tab is the one on show.
    fn db_sessions_load(&mut self) -> Task<Message> {
        let Some((conn, database)) = self
            .db
            .health
            .as_ref()
            .filter(|h| h.tab == HealthTab::Sessions)
            .map(|h| (h.conn, h.database.clone()))
        else {
            return Task::none();
        };
        let Some(params) = self.db_params(conn, false) else {
            self.db.health = None;
            return self.db_ask_password(conn);
        };
        if let Some(view) = self.db.health.as_mut() {
            view.sessions_loading = true;
        }
        let window = database::AnswerFor::Health(conn, database.clone());
        Task::perform(database::sessions(params, database), move |done| {
            Message::Db(DbMsg::Answer(
                window.clone(),
                Box::new(DbMsg::SessionsLoaded(done)),
            ))
        })
    }

    /// Reads the numbers of the database the health window is about.
    fn db_health_load(&mut self) -> Task<Message> {
        let Some((conn, database)) = self
            .db
            .health
            .as_ref()
            .map(|h| (h.conn, h.database.clone()))
        else {
            return Task::none();
        };
        let Some(params) = self.db_params(conn, false) else {
            self.db.health = None;
            return self.db_ask_password(conn);
        };
        if let Some(view) = self.db.health.as_mut() {
            view.loading = true;
        }
        let window = database::AnswerFor::Health(conn, database.clone());
        Task::perform(database::health(params, database), move |done| {
            Message::Db(DbMsg::Answer(
                window.clone(),
                Box::new(DbMsg::HealthLoaded(done)),
            ))
        })
    }
}

/// What is read of a file to show it: the start of a CSV, all of a JSON (its
/// keys are in all of it).
fn read_sample(path: &std::path::Path, format: Format) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let limit = if format == Format::Json {
        64 * 1024 * 1024
    } else {
        import::SAMPLE_BYTES
    };
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
