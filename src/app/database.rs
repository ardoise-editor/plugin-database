//! The Database panel inside the app: what its messages do.

use std::path::PathBuf;

use super::Task;

use super::{App, Message};
use crate::database::{
    self, grid, protocol, DbMsg, Dialog, Driver, Field, Params, TableTab, TestState, Tree,
};

impl App {
    /// Finds out whether the driver is installed.
    pub(super) fn db_probe_driver(&mut self) -> Task<Message> {
        Task::batch(protocol::Dialect::ALL.map(|dialect| {
            Task::perform(
                database::probe_driver_of(dialect, self.db.driver_path.clone()),
                move |driver| Message::Db(DbMsg::DriverProbed(dialect, driver)),
            )
        }))
    }

    /// The password of a connection that asks every time serves once.
    pub(super) fn db_params(&mut self, id: u64, consume: bool) -> Option<Params> {
        let params = self.db.params(id)?;
        if consume {
            self.db.once.remove(&id);
        }
        Some(params)
    }

    /// Loads the databases, schemas and tables of a connection.
    pub(super) fn db_load_tree(&mut self, id: u64) -> Task<Message> {
        let Some(params) = self.db_params(id, true) else {
            return self.db_ask_password(id);
        };
        self.db.trees.insert(id, Tree::Loading);
        self.db.forget_known();
        Task::perform(database::tree(params), move |result| {
            Message::Db(DbMsg::TreeLoaded(id, result))
        })
    }

    /// Opens the form of a connection whose password is not known yet.
    pub(super) fn db_ask_password(&mut self, id: u64) -> Task<Message> {
        if let Some(connection) = self.db.connection(id).cloned() {
            let mut form = Dialog::edit(&connection, String::new(), self.db.driver_path.clone());
            form.needs_password = true;
            self.db.dialog = Some(form);
        }
        if self.db.driver == Driver::Unknown {
            return self.db_probe_driver();
        }
        Task::none()
    }

    /// Asks whether to trust the key of an SSH server: its fingerprints are
    /// read first.
    fn ask_ssh_trust(&mut self, host: String, port: u16) -> Task<Message> {
        if self.db.ssh_trust.is_some() {
            return Task::none();
        }
        self.db.ssh_trust = Some(database::SshTrust {
            host: host.clone(),
            port,
            keys: Vec::new(),
            scanning: true,
            error: None,
        });
        Task::perform(
            async move { crate::ssh_trust::scan(&host, port).await },
            |r| Message::Db(DbMsg::SshScanned(r)),
        )
    }

    pub(super) fn db_load_rows(&mut self, path: PathBuf, offset: u64) -> Task<Message> {
        let Some(tab) = self.db_tabs.get(&path) else {
            return Task::none();
        };
        let (conn, table, query) = (tab.conn, tab.table_ref(), tab.query(offset));
        // A query that cannot be paged is run again as it is.
        // A query that cannot be paged (it writes, or it is a plan) is not run
        // again: what it gave stays on show, and running it twice could do
        // its work twice.
        if tab.query.is_some() && !tab.paged {
            return Task::none();
        }
        let live = tab.live.then(|| self.db.live.get(&conn).cloned()).flatten();
        let Some(params) = self.db_params(conn, false) else {
            // The tab says why it is empty; the form asks for the password.
            if let Some(tab) = self.db_tabs.get_mut(&path) {
                tab.loading = false;
                tab.error = Some(t!("db.password_needed").to_string());
            }
            return self.db_ask_password(conn);
        };
        let mut no = 0;
        if let Some(tab) = self.db_tabs.get_mut(&path) {
            tab.loading = true;
            tab.error = None;
            tab.load_no += 1;
            no = tab.load_no;
        }
        let (task, handle) = Task::perform(database::rows_with(live, params, table, query), {
            let path = path.clone();
            move |done| Message::Db(DbMsg::RowsFor(path.clone(), no, done))
        })
        .abortable();
        if let Some(tab) = self.db_tabs.get_mut(&path) {
            // The page asked for before is no more wanted.
            if let Some(old) = tab.stop.replace(handle) {
                old.abort();
            }
        }
        task
    }

    /// Puts what the driver ran in the log of the console, and shows the end.
    pub(super) fn db_log(&mut self, conn: u64, executed: &[database::Executed]) -> Task<Message> {
        let label = self
            .db
            .connection(conn)
            .map(database::Connection::label)
            .unwrap_or_default();
        self.db_log_as(&label, executed)
    }

    /// The same, for a connection that is named but may not be kept yet.
    fn db_log_as(&mut self, label: &str, executed: &[database::Executed]) -> Task<Message> {
        if executed.is_empty() {
            return Task::none();
        }
        self.db.console.push(label, executed);
        Task::none()
    }

    /// A tab for the rows a query gave: one that only reads is paged, sorted
    /// and filtered like a table (its rows are asked for again); another one
    /// shows what it gave.
    fn db_open_result(&mut self, conn: u64, result: &database::QueryResult) -> Task<Message> {
        self.assign_db_color(conn);
        // The same query again fills the tab it already has; one with edits
        // that wait is kept as it is (it counts as kept), and a new tab opens.
        let reused = self
            .db_tabs
            .iter()
            .find(|(_, t)| {
                t.conn == conn
                    && t.live
                    && t.query.as_deref() == Some(result.sql.as_str())
                    && t.pending.is_empty()
            })
            .map(|(path, t)| (path.clone(), t.title.clone()));
        let (path, title) = match reused {
            Some(found) => found,
            None => {
                self.db_counter += 1;
                let title = format!("{} {}", t!("db.result"), self.db_counter);
                (
                    PathBuf::from(format!("db://{}/{title}", self.db_counter)),
                    title,
                )
            }
        };
        let dialect = self
            .db
            .connection(conn)
            .map(|c| c.dialect)
            .unwrap_or_default();
        let mut tab = TableTab::new(
            conn,
            self.db.console.database.clone(),
            String::new(),
            String::new(),
            self.db.page_size,
        );
        tab.dialect = dialect;
        tab.title = title;
        tab.query = Some(result.sql.clone());
        tab.paged = result.deferred;
        tab.live = true;
        if !result.deferred {
            tab.set_page(database::result_page(result));
        }
        self.db_tabs.insert(path.clone(), tab);
        let show = self.activate(path.clone());
        if result.deferred {
            Task::batch([show, self.db_load_rows(path, 0)])
        } else {
            show
        }
    }

    /// Runs the statements of the console.
    fn db_run(&mut self, all: bool) -> Task<Message> {
        if self.db.console.running {
            return Task::none();
        }
        let statements = self.db.console.statements(all);
        self.db_run_statements(statements)
    }

    /// What is done in the grid of a table tab.
    fn db_grid(&mut self, path: PathBuf, event: simpleedit_plugin::ui::GridEvent) -> Task<Message> {
        use simpleedit_plugin::ui::GridEvent;
        let Some(tab) = self.db_tabs.get(&path) else {
            return Task::none();
        };
        let shown = tab.shown_columns();
        let real = |c: usize| shown.get(c).copied().unwrap_or(c);
        let name_of = |tab: &TableTab, c: usize| {
            tab.page
                .as_ref()
                .and_then(|p| p.columns.get(c))
                .map(|c| c.name.clone())
        };
        match event {
            GridEvent::Select { anchor, cursor } => {
                self.mod_shift = false;
                let task = self.db_message(DbMsg::Select(path.clone(), cursor.0, real(cursor.1)));
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.anchor = (anchor != cursor).then(|| (anchor.0, real(anchor.1)));
                }
                task
            }
            GridEvent::Open { row, col } => self.db_message(DbMsg::Edit(path, row, real(col))),
            GridEvent::Edit { value } => self.db_message(DbMsg::EditInput(path, value)),
            GridEvent::Commit => self.db_message(DbMsg::EditCommit(path)),
            // Escape: the date picker, the cell being typed in, or the form
            // goes, in that order.
            GridEvent::Cancel => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    let _ = tab.picker.take().is_some()
                        || tab.editing.take().is_some()
                        || tab.form.take().is_some();
                }
                Task::none()
            }
            GridEvent::Sort { col } => self.db_message(DbMsg::Sort(path, real(col))),
            GridEvent::Resize { col, width } => {
                let col = real(col);
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let Some(w) = tab.widths.get_mut(col) {
                        *w = width;
                    }
                    if let Some(name) = name_of(tab, col) {
                        tab.custom_widths.insert(name, width);
                    }
                }
                Task::none()
            }
            GridEvent::Filter { col, value, submit } => {
                let Some(name) = name_of(tab, real(col)) else {
                    return Task::none();
                };
                let typed = self.db_message(DbMsg::ColumnFilter(path.clone(), name, value));
                if submit {
                    return Task::batch([typed, self.db_message(DbMsg::ApplyFilter(path))]);
                }
                typed
            }
            GridEvent::End => Task::none(),
            GridEvent::Copy => match (tab.block(), tab.selected) {
                (Some(_), _) => self.db_message(DbMsg::CopySelection(path, grid::CopyFormat::Tsv)),
                (None, Some((row, col))) => self.db_message(DbMsg::CopyCell(path, row, col)),
                (None, None) => Task::none(),
            },
            GridEvent::Paste { text } => self.db_message(DbMsg::Paste(path, Some(text))),
            // The row of the cursor is marked deleted (a new one goes), and
            // the cursor moves on to the next row.
            GridEvent::Delete => match tab.selected {
                Some((row, col)) if tab.editable() => {
                    let task = self.db_message(DbMsg::DeleteRow(path.clone(), row));
                    if let Some(tab) = self.db_tabs.get_mut(&path) {
                        let rows = tab.page.as_ref().map_or(0, |p| tab.pending.rows(p));
                        if row + 1 < rows {
                            tab.selected = Some((row + 1, col));
                            tab.anchor = None;
                        }
                    }
                    task
                }
                _ => Task::none(),
            },
        }
    }

    /// An item of the menu of a cell: the cell is picked, then the item does
    /// its part.
    fn db_grid_menu(
        &mut self,
        path: PathBuf,
        action: &str,
        row: usize,
        col: usize,
    ) -> Task<Message> {
        let Some(tab) = self.db_tabs.get(&path) else {
            return Task::none();
        };
        let c = tab.shown_columns().get(col).copied().unwrap_or(col);
        let picked = self.db_message(DbMsg::Select(path.clone(), row, c));
        let p = path.clone();
        let then = match action.trim_start_matches("menu:") {
            "view" => self.db_message(DbMsg::ViewValue(p, row, c)),
            "copy" => self.db_message(DbMsg::CopyCell(p, row, c)),
            "hide" => self.db_message(DbMsg::HideColumn(p, c)),
            "filter" => self.db_message(DbMsg::FilterByCell(p, row, c)),
            "copy_csv" => self.db_message(DbMsg::CopySelection(p, grid::CopyFormat::Csv)),
            "copy_insert" => self.db_message(DbMsg::CopySelection(p, grid::CopyFormat::Insert)),
            "copy_markdown" => self.db_message(DbMsg::CopySelection(p, grid::CopyFormat::Markdown)),
            "copy_tsv" => self.db_message(DbMsg::CopySelection(p, grid::CopyFormat::Tsv)),
            "form" => self.db_message(DbMsg::OpenForm(p, row)),
            "row_copy" => self.db_message(DbMsg::CopyRow(p, row)),
            "links" => {
                let Some(tab) = self.db_tabs.get(&path) else {
                    return picked;
                };
                let links = match (&tab.structure, &tab.page) {
                    (Some(structure), Some(page)) if !tab.pending.is_new(page, row) => page
                        .rows
                        .get(row)
                        .map(|values| {
                            database::ddl::links(
                                &tab.schema,
                                structure,
                                &page.columns,
                                values,
                                tab.dialect,
                            )
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                if links.is_empty() {
                    self.set_status(t!("db.no_links"), false);
                    return picked;
                }
                let view = database::LinksView {
                    conn: tab.conn,
                    database: tab.database.clone(),
                    schema: tab.schema.clone(),
                    links,
                };
                self.db_message(DbMsg::ShowLinks(Box::new(view)))
            }
            "edit" => self.db_message(DbMsg::Edit(p, row, c)),
            "null" => self.db_message(DbMsg::SetNull(p, row, c)),
            "default" => self.db_message(DbMsg::SetDefault(p, row, c)),
            "pick" => self.db_message(DbMsg::OpenPicker(p, row, c)),
            "add" => self.db_message(DbMsg::AddRow(p)),
            "delete" => self.db_message(DbMsg::DeleteRow(p, row)),
            "revert" => self.db_message(DbMsg::RevertRow(p, row)),
            _ => Task::none(),
        };
        Task::batch([picked, then])
    }

    /// Runs what the SQL file being edited says (its selection, or all of
    /// it), on the data source of the console, as the console would.
    fn db_run_file(&mut self, file: simpleedit_plugin::protocol::ActiveFile) -> Task<Message> {
        let sql = file.language.as_deref() == Some("sql")
            || file
                .path
                .as_ref()
                .and_then(|p| p.extension())
                .is_some_and(|e| e.eq_ignore_ascii_case("sql"));
        if !sql {
            self.set_status(t!("db.file_not_sql"), false);
            return Task::none();
        }
        let text = file
            .selection
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(file.text);
        let statements: Vec<String> =
            database::console::split_statements(&text, self.db.console.dialect)
                .into_iter()
                .map(|span| text[span.start..span.end].to_string())
                .collect();
        // The result and the log are shown where the console shows them.
        self.db.console.open = true;
        self.db_run_statements(statements)
    }

    /// Why the console can't move to another connection or database now:
    /// a query is running, or a transaction is open (its locks would stay).
    fn db_console_busy(&self) -> Option<String> {
        if self.db.console.running {
            Some(t!("db.console_busy_running").to_string())
        } else if self.db.console.in_transaction {
            Some(t!("db.console_busy_transaction").to_string())
        } else {
            None
        }
    }

    fn db_run_statements(&mut self, statements: Vec<String>) -> Task<Message> {
        if self.db.console.running {
            return Task::none();
        }
        // Only on the connection chosen: never on another one in its place.
        let Some(conn) = self
            .db
            .console
            .connection
            .filter(|id| self.db.connection(*id).is_some())
        else {
            let message = if self.db.connections.is_empty() {
                t!("db.console_no_connection")
            } else {
                t!("db.console_choose_connection")
            };
            self.db.console.last = Some((false, message.to_string()));
            return Task::none();
        };
        self.db.use_console_connection(Some(conn));
        self.db.console.save();
        if statements.is_empty() {
            self.db.console.last = Some((false, t!("db.console_nothing").to_string()));
            return Task::none();
        }
        // Parameters (`:name`, `?`) are asked for before anything runs.
        let names = database::console::placeholders(&statements, self.db.console.dialect);
        if !names.is_empty() {
            self.db.console.last = None;
            self.db.console.binds = Some(database::console::Binds {
                conn,
                statements,
                values: vec![String::new(); names.len()],
                names,
            });
            return Task::none();
        }
        self.db_run_checked(conn, statements)
    }

    /// Runs statements whose parameters are known: after the question about
    /// the risk of them, if they have one.
    fn db_run_checked(&mut self, conn: u64, statements: Vec<String>) -> Task<Message> {
        let production = self.db.connection(conn).is_some_and(|c| c.production);
        let risk = statements
            .iter()
            .find_map(|sql| database::protocol::risk_of(sql, production, self.db.console.dialect));
        if let Some(risk) = risk {
            self.db.console.last = None;
            self.db.console.guard = Some(database::console::Guard {
                conn,
                statements,
                risk,
            });
            return Task::none();
        }
        self.db_exec(conn, statements)
    }

    /// Runs statements in the session of the console.
    fn db_exec(&mut self, conn: u64, statements: Vec<String>) -> Task<Message> {
        let Some(params) = self.db_params(conn, false) else {
            return self.db_ask_password(conn);
        };
        self.db.console.running = true;
        self.db.console.last = None;
        self.db.console.error_mark = None;
        self.db.console.running_sql = statements.clone();
        let live = self.db.live_of(conn);
        let database = self.db.console.database.clone();
        let (task, handle) = Task::perform(
            database::run_with(Some(live), params, database, statements, false),
            move |done| Message::Db(DbMsg::Ran(conn, done)),
        )
        .abortable();
        self.db.console.abort = Some(handle);
        task
    }

    /// Whether the tab has changes that wait: it then says so, and what was
    /// asked (another page, a sort…) is not done.
    fn db_blocked(&mut self, path: &PathBuf) -> bool {
        match self.db_tabs.get_mut(path) {
            Some(tab) if !tab.pending.is_empty() => {
                tab.notice = Some(t!("db.pending_block").to_string());
                true
            }
            _ => false,
        }
    }

    /// What was typed in the cell becomes a pending edit.
    fn db_commit_edit(&mut self, path: &PathBuf) {
        let Some(tab) = self.db_tabs.get_mut(path) else {
            return;
        };
        let (Some(editing), Some(page)) = (tab.editing.take(), tab.page.as_ref()) else {
            return;
        };
        // Nothing typed in a cell that is NULL leaves it NULL.
        let was_null = matches!(
            tab.pending.cell(page, editing.row, editing.col),
            grid::Cell::Null
        );
        let value = if editing.text.is_empty() && was_null {
            None
        } else {
            Some(editing.text)
        };
        let page = page.clone();
        // Nothing typed in a cell of an added row that holds no text leaves
        // the column's default; an emptied text is an empty text.
        if tab.pending.is_new(&page, editing.row)
            && value.as_deref() == Some("")
            && !matches!(
                tab.pending.cell(&page, editing.row, editing.col),
                grid::Cell::Value(_)
            )
        {
            return;
        }
        tab.pending.set(&page, editing.row, editing.col, value);
    }

    /// The row the cursor moved to: the grid of the editor keeps it in view.
    fn db_reveal(_tab: &mut TableTab, _row: usize) -> Task<Message> {
        Task::none()
    }

    /// Opens a table in a tab, or goes to the one that shows it.
    pub(super) fn db_open_table(
        &mut self,
        conn: u64,
        database: String,
        schema: String,
        table: String,
    ) -> Task<Message> {
        self.db_open_table_with(conn, database, schema, table, None)
    }

    /// Opens a table, with a condition on its rows if there is one.
    pub(super) fn db_open_table_with(
        &mut self,
        conn: u64,
        database: String,
        schema: String,
        table: String,
        filter: Option<String>,
    ) -> Task<Message> {
        let title = if schema == "public" {
            table.clone()
        } else {
            format!("{schema}.{table}")
        };
        let existing = self
            .db_tabs
            .iter()
            .find(|(_, t)| {
                t.conn == conn && t.database == database && t.schema == schema && t.table == table
            })
            .map(|(p, _)| p.clone());
        if let Some(path) = existing {
            let show = self.activate(path.clone());
            let Some(filter) = filter else {
                return show;
            };
            // Rows with edits waiting are not read again under them.
            if self.db_blocked(&path) {
                return show;
            }
            if let Some(tab) = self.db_tabs.get_mut(&path) {
                tab.filter_text = filter.clone();
                tab.filter = filter;
                tab.notice = None;
            }
            return Task::batch([show, self.db_load_rows(path, 0)]);
        }
        self.assign_db_color(conn);
        self.db_counter += 1;
        let path = PathBuf::from(format!("db://{}/{}", self.db_counter, title));
        let key = (conn, database.clone(), schema.clone(), table.clone());
        let mut tab = TableTab::new(conn, database, schema, table, self.db.page_size);
        tab.dialect = self
            .db
            .connection(conn)
            .map(|c| c.dialect)
            .unwrap_or_default();
        tab.structure = self.db.structures.get(&key).cloned();
        if let Some(filter) = filter {
            tab.filter_text = filter.clone();
            tab.filter = filter;
        }
        self.db_tabs.insert(path.clone(), tab);
        let show = self.activate(path.clone());
        let mut tasks = vec![show, self.db_load_rows(path, 0)];
        // What the table is made of: for the links of its rows.
        if !self.db.structures.contains_key(&key) {
            tasks.push(self.db_read_structure(key, false));
        }
        Task::batch(tasks)
    }

    pub(crate) fn db_message(&mut self, msg: DbMsg) -> Task<Message> {
        match msg {
            DbMsg::Answer(window, inner) => {
                let open = match &window {
                    database::AnswerFor::Designer(c, s, t) => self
                        .db
                        .designer
                        .as_ref()
                        .is_some_and(|d| d.conn == *c && d.schema == *s && d.table == *t),
                    database::AnswerFor::Health(c, d) => self
                        .db
                        .health
                        .as_ref()
                        .is_some_and(|h| h.conn == *c && h.database == *d),
                    database::AnswerFor::Import(c, s, t) => self
                        .db
                        .import
                        .as_ref()
                        .is_some_and(|i| i.conn == *c && i.schema == *s && i.table == *t),
                };
                if open {
                    self.db_message(*inner)
                } else {
                    Task::none()
                }
            }
            DbMsg::OpenDialog(id) => {
                let driver_file = self.db.driver_path.clone();
                self.db.dialog = Some(match id.and_then(|id| self.db.connection(id).cloned()) {
                    Some(connection) => {
                        let password = self
                            .db
                            .secrets(&connection)
                            .map(|(password, _)| password)
                            .unwrap_or_default();
                        Dialog::edit(&connection, password, driver_file)
                    }
                    None => {
                        let mut form = Dialog::new();
                        form.driver_file = driver_file;
                        form
                    }
                });
                if self.db.driver == Driver::Unknown {
                    return self.db_probe_driver();
                }
                Task::none()
            }
            DbMsg::CloseDialog => {
                self.db.dialog = None;
                Task::none()
            }
            DbMsg::Tab(tab) => {
                if let Some(form) = &mut self.db.dialog {
                    form.tab = tab;
                }
                Task::none()
            }
            DbMsg::Text(field, value) => {
                if let Some(form) = &mut self.db.dialog {
                    form.set(field, value);
                }
                Task::none()
            }
            DbMsg::Check(flag, on) => {
                if let Some(form) = &mut self.db.dialog {
                    form.check(flag, on);
                }
                Task::none()
            }
            DbMsg::Choose(choice) => {
                if let Some(form) = &mut self.db.dialog {
                    form.choose(choice);
                }
                Task::none()
            }
            DbMsg::ScriptText(text) => {
                if let Some(form) = &mut self.db.dialog {
                    if form.script.text() != text {
                        form.script = crate::text_editor::Content::with_text(&text);
                        form.test = TestState::Idle;
                    }
                }
                Task::none()
            }
            DbMsg::PropKey(i, value) => {
                if let Some(prop) = self
                    .db
                    .dialog
                    .as_mut()
                    .and_then(|f| f.draft.props.get_mut(i))
                {
                    prop.0 = value;
                }
                Task::none()
            }
            DbMsg::PropValue(i, value) => {
                if let Some(prop) = self
                    .db
                    .dialog
                    .as_mut()
                    .and_then(|f| f.draft.props.get_mut(i))
                {
                    prop.1 = value;
                }
                Task::none()
            }
            DbMsg::PropAdd => {
                if let Some(form) = &mut self.db.dialog {
                    form.draft.props.push((String::new(), String::new()));
                }
                Task::none()
            }
            DbMsg::PropRemove(i) => {
                if let Some(form) = &mut self.db.dialog {
                    if i < form.draft.props.len() {
                        form.draft.props.remove(i);
                    }
                }
                Task::none()
            }
            DbMsg::Browse(field) => {
                Task::perform(crate::editor::pick_file(String::new(), &[]), move |path| {
                    Message::Db(DbMsg::Browsed(field, path))
                })
            }
            DbMsg::Browsed(field, path) => {
                let Some(path) = path else {
                    return Task::none();
                };
                if let Some(form) = &mut self.db.dialog {
                    form.set(field, path.display().to_string());
                }
                if field == Field::DriverFile {
                    return self.db_use_driver_file(path.display().to_string());
                }
                Task::none()
            }
            DbMsg::Test => {
                let Some(form) = &mut self.db.dialog else {
                    return Task::none();
                };
                if !form.valid() {
                    return Task::none();
                }
                let params = Params {
                    target: form.target(),
                    scope: form.draft.scope.clone(),
                    driver: self.db.driver_path.clone(),
                    browse: None,
                };
                form.test = TestState::Running;
                Task::perform(database::test(params), |result| {
                    Message::Db(DbMsg::Tested(result))
                })
            }
            DbMsg::Tested(done) => {
                let mut label = String::new();
                // An SSH server whose key is not known: its fingerprint is asked about.
                let unknown = done
                    .result
                    .as_ref()
                    .err()
                    .filter(|e| crate::ssh_trust::is_unknown_host(e))
                    .and(self.db.dialog.as_ref())
                    .map(|f| (f.draft.ssh.host.clone(), f.draft.ssh.port));
                if let Some((host, port)) = unknown.filter(|(h, _)| !h.is_empty()) {
                    let log = self.db_log_as(&label, &done.executed);
                    return Task::batch([log, self.ask_ssh_trust(host, port)]);
                }
                if let Some(form) = &mut self.db.dialog {
                    label = form.draft.label();
                    form.test = match done.result {
                        Ok(version) => TestState::Ok(version),
                        Err(error) => TestState::Failed(error),
                    };
                }
                self.db_log_as(&label, &done.executed)
            }
            DbMsg::Save => {
                let Some(form) = self.db.dialog.take().filter(Dialog::valid) else {
                    return Task::none();
                };
                // A driver file typed by hand.
                let typed = form.driver_file.trim().to_string();
                let mut tasks = Vec::new();
                if typed != self.db.driver_path {
                    tasks.push(self.db_use_driver_file(typed));
                }
                let Some(id) = self.db.apply(&form) else {
                    self.db.dialog = Some(form);
                    return Task::none();
                };
                // The tables open for it that failed read again, then the
                // tree (the one that reads last uses a password that asks
                // every time).
                let paths: Vec<PathBuf> = self
                    .db_tabs
                    .iter()
                    .filter(|(_, t)| t.conn == id && t.error.is_some())
                    .map(|(p, _)| p.clone())
                    .collect();
                for path in paths {
                    let offset = self.db_tabs.get(&path).map_or(0, TableTab::offset);
                    tasks.push(self.db_load_rows(path, offset));
                }
                self.db.open.insert(id.to_string());
                tasks.push(self.db_load_tree(id));
                Task::batch(tasks)
            }
            DbMsg::Duplicate(id) => {
                let Some(copy) = self.db.duplicate(id) else {
                    return Task::none();
                };
                self.db.dialog = None;
                self.db_message(DbMsg::OpenDialog(Some(copy)))
            }
            DbMsg::Delete => {
                // The first click asks; the second deletes.
                if let Some(dialog) = self.db.dialog.as_mut().filter(|d| !d.confirm_delete) {
                    dialog.confirm_delete = true;
                    return Task::none();
                }
                if let Some(id) = self.db.dialog.as_ref().and_then(|d| d.editing) {
                    self.db.remove(id);
                }
                self.db.dialog = None;
                Task::none()
            }
            DbMsg::DriverProbed(dialect, driver) => {
                // A download in progress is not interrupted by a look.
                if *self.db.driver_of(dialect) != Driver::Downloading {
                    self.db.set_driver(dialect, driver);
                }
                Task::none()
            }
            DbMsg::UseDownloadedDriver => {
                if let Some(form) = &mut self.db.dialog {
                    form.driver_file.clear();
                }
                self.db_use_driver_file(String::new())
            }
            DbMsg::Picked(picked) => {
                self.db.tree_picked = Some(picked.clone());
                match picked {
                    database::Picked::Table(id, database, schema, table, _) => {
                        self.db_message(DbMsg::OpenTable(id, database, schema, table))
                    }
                    database::Picked::Connection(id) => {
                        self.db_message(DbMsg::Toggle(id.to_string()))
                    }
                    database::Picked::Database(id, database) => {
                        self.db_message(DbMsg::Toggle(format!("{id}/{database}")))
                    }
                    database::Picked::Schema(id, database, schema) => {
                        self.db_message(DbMsg::Toggle(format!("{id}/{database}/{schema}")))
                    }
                }
            }
            DbMsg::Toggle(key) => {
                if self.db.open.remove(&key) {
                    return Task::none();
                }
                self.db.open.insert(key.clone());
                // A connection that opens loads its tables the first time.
                if let Ok(id) = key.parse::<u64>() {
                    if !matches!(self.db.trees.get(&id), Some(Tree::Loaded(_))) {
                        return self.db_load_tree(id);
                    }
                }
                Task::none()
            }
            DbMsg::Reload(id) => {
                self.db.open.insert(id.to_string());
                self.db.structures.retain(|k, _| k.0 != id);
                self.db_load_tree(id)
            }
            DbMsg::TreeLoaded(id, done) => {
                let log = self.db_log(id, &done.executed);
                let unknown = done
                    .result
                    .as_ref()
                    .err()
                    .filter(|e| crate::ssh_trust::is_unknown_host(e))
                    .and(self.db.connection(id))
                    .map(|c| (c.ssh.host.clone(), c.ssh.port));
                let ask = match unknown.filter(|(h, _)| !h.is_empty()) {
                    Some((host, port)) => self.ask_ssh_trust(host, port),
                    None => Task::none(),
                };
                let log = Task::batch([log, ask]);
                let read_columns = done.result.is_ok();
                self.db.forget_known();
                if self.db.connection(id).is_some() {
                    self.db.trees.insert(
                        id,
                        match done.result {
                            Ok(databases) => {
                                // What holds the tables is shown open.
                                for database in &databases {
                                    if database.current {
                                        self.db.open.insert(format!("{id}/{}", database.name));
                                    }
                                    if let Some(schema) =
                                        database.schemas.iter().find(|s| !s.tables.is_empty())
                                    {
                                        self.db.open.insert(format!(
                                            "{id}/{}/{}",
                                            database.name, schema.name
                                        ));
                                    }
                                }
                                Tree::Loaded(databases)
                            }
                            Err(error) => Tree::Failed(error),
                        },
                    );
                }
                // The columns of the tables come for the completion of queries.
                let columns = match (read_columns, self.db.params(id)) {
                    (true, Some(params)) => Task::perform(
                        database::table_columns(params, String::new()),
                        move |done| Message::Db(DbMsg::ColumnsLoaded(id, done)),
                    ),
                    _ => Task::none(),
                };
                Task::batch([log, columns])
            }
            DbMsg::OpenTable(conn, database, schema, table) => {
                self.db_open_table(conn, database, schema, table)
            }
            DbMsg::RowsFor(path, no, done) => {
                // Answers can come in any order: only the last asked counts.
                let current = self.db_tabs.get(&path).is_some_and(|t| t.load_no == no);
                if current {
                    if let Some(tab) = self.db_tabs.get_mut(&path) {
                        tab.stop = None;
                    }
                    return self.db_message(DbMsg::Rows(path, done));
                }
                match self.db_tabs.get(&path).map(|t| t.conn) {
                    Some(conn) => self.db_log(conn, &done.executed),
                    None => Task::none(),
                }
            }
            DbMsg::Rows(path, done) => {
                let conn = self.db_tabs.get(&path).map(|t| t.conn);
                let log = match conn {
                    Some(conn) => self.db_log(conn, &done.executed),
                    None => Task::none(),
                };
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    match done.result {
                        // Typing began while the page was on its way: the
                        // page would wipe it, so the edits win.
                        Ok(_) if !tab.pending.is_empty() || tab.editing.is_some() => {
                            tab.loading = false;
                            tab.notice = Some(t!("db.page_kept_edits").to_string());
                        }
                        Ok(page) => tab.set_page(page),
                        Err(error) => {
                            tab.loading = false;
                            tab.error = Some(error);
                        }
                    }
                }
                log
            }
            DbMsg::Paste(path, text) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let (Some(text), Some(page), Some(at)) = (
                        text,
                        tab.page.as_ref(),
                        tab.block().map(|b| b.0).or(tab.selected),
                    ) {
                        if tab.editable() && tab.editing.is_none() {
                            grid::paste_block(&mut tab.pending, page, at, &text);
                        }
                    }
                }
                Task::none()
            }
            DbMsg::Count(path) => {
                let Some(tab) = self.db_tabs.get(&path) else {
                    return Task::none();
                };
                let (conn, table, query) = (tab.conn, tab.table_ref(), tab.query(0));
                let live = tab.live.then(|| self.db.live.get(&conn).cloned()).flatten();
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                let filter = query.filter.clone();
                Task::perform(database::count(live, params, table, query), move |done| {
                    Message::Db(DbMsg::Counted(path.clone(), filter.clone(), done))
                })
            }
            DbMsg::Counted(path, filter, done) => {
                let conn = self.db_tabs.get(&path).map(|t| t.conn);
                let log = match conn {
                    Some(conn) => self.db_log(conn, &done.executed),
                    None => Task::none(),
                };
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    match done.result {
                        Ok(total) => tab.total = Some((filter, total)),
                        Err(error) => tab.notice = Some(error),
                    }
                }
                log
            }
            DbMsg::Turn(path, pages) => {
                if self.db_blocked(&path) {
                    return Task::none();
                }
                let Some(tab) = self.db_tabs.get(&path) else {
                    return Task::none();
                };
                let offset = (tab.offset() as i64 + pages * tab.page_size as i64).max(0);
                self.db_load_rows(path, offset as u64)
            }
            DbMsg::Refresh(path) => {
                if self.db_blocked(&path) {
                    return Task::none();
                }
                let offset = self.db_tabs.get(&path).map_or(0, TableTab::offset);
                self.db_load_rows(path, offset)
            }
            DbMsg::Filter(path, text) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.filter_text = text;
                }
                Task::none()
            }
            DbMsg::OpenForm(path, row) => {
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return Task::none();
                };
                if tab.page.as_ref().is_none_or(|p| row >= tab.pending.rows(p)) {
                    return Task::none();
                }
                tab.form = Some(row);
                tab.selected = Some((row, tab.selected.map_or(0, |s| s.1)));
                tab.anchor = None;
                if tab.choices.is_some() {
                    return Task::none();
                }
                let (conn, table) = (tab.conn, tab.table_ref());
                match self.db_params(conn, false) {
                    Some(params) => Task::perform(database::table_choices(params, table), {
                        let path = path.clone();
                        move |done| Message::Db(DbMsg::ChoicesLoaded(path.clone(), done))
                    }),
                    None => Task::none(),
                }
            }
            DbMsg::ChoicesLoaded(path, done) => {
                let conn = self.db_tabs.get(&path).map(|t| t.conn);
                let log = conn.map_or_else(Task::none, |c| self.db_log(c, &done.executed));
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    // No list is not an error: the field is typed in.
                    tab.choices = Some(done.result.unwrap_or_default());
                }
                log
            }
            DbMsg::CloseForm(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.form = None;
                }
                Task::none()
            }
            DbMsg::FormMove(path, by) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    let count = tab.page.as_ref().map_or(0, |p| tab.pending.rows(p));
                    if let (Some(row), true) = (tab.form, count > 0) {
                        let next = (row as i64 + by as i64).clamp(0, count as i64 - 1) as usize;
                        tab.form = Some(next);
                        tab.selected = Some((next, tab.selected.map_or(0, |s| s.1)));
                        return Self::db_reveal(tab, next);
                    }
                }
                Task::none()
            }
            DbMsg::FormInput(path, col, text) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let (Some(row), Some(page)) = (tab.form, tab.page.clone()) {
                        if page.editable && !tab.pending.is_deleted(row) {
                            tab.pending.type_into(&page, row, col, text);
                        }
                    }
                }
                Task::none()
            }
            DbMsg::FormNull(path, col) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let (Some(row), Some(page)) = (tab.form, tab.page.clone()) {
                        if page.editable && !tab.pending.is_deleted(row) {
                            tab.pending.set(&page, row, col, None);
                        }
                    }
                }
                Task::none()
            }
            DbMsg::FormJson(path, col) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let (Some(row), Some(page)) = (tab.form, tab.page.clone()) {
                        let laid = tab
                            .pending
                            .value(&page, row, col)
                            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                            .and_then(|v| serde_json::to_string_pretty(&v).ok());
                        if let (Some(text), true) = (laid, page.editable) {
                            tab.pending.type_into(&page, row, col, text);
                        }
                    }
                }
                Task::none()
            }
            msg @ (DbMsg::OpenHealth(..)
            | DbMsg::HealthLoaded(_)
            | DbMsg::HealthTab(_)
            | DbMsg::HealthRun(_)
            | DbMsg::HealthRan(_)
            | DbMsg::HealthToConsole(_)
            | DbMsg::HealthClose
            | DbMsg::ImportOpen(..)
            | DbMsg::ImportStructure(_)
            | DbMsg::ImportPick
            | DbMsg::ImportPicked(_)
            | DbMsg::ImportRead(..)
            | DbMsg::ImportFormat(_)
            | DbMsg::ImportDelimiter(_)
            | DbMsg::ImportEncoding(_)
            | DbMsg::ImportHeader(_)
            | DbMsg::ImportEmptyNull(_)
            | DbMsg::ImportMap(..)
            | DbMsg::ImportRun
            | DbMsg::ImportDone(_)
            | DbMsg::ImportClose
            | DbMsg::CopyOpen(..)
            | DbMsg::CopyDest(_)
            | DbMsg::CopyField(..)
            | DbMsg::CopyData(_)
            | DbMsg::CopyRun
            | DbMsg::CopyDone(_)
            | DbMsg::CopyClose
            | DbMsg::BackupOpen(..)
            | DbMsg::BackupFlag(..)
            | DbMsg::BackupPick
            | DbMsg::BackupPicked(_)
            | DbMsg::BackupRun
            | DbMsg::BackupDone(_)
            | DbMsg::BackupClose) => self.db_tools_message(msg),
            DbMsg::SearchText(path, text) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.search_text = text;
                }
                Task::none()
            }
            DbMsg::ColumnFilter(path, column, text) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.column_text.insert(column, text);
                }
                Task::none()
            }
            DbMsg::HideColumn(path, col) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    let name = tab
                        .page
                        .as_ref()
                        .and_then(|p| p.columns.get(col))
                        .map(|c| c.name.clone());
                    let count = tab.page.as_ref().map_or(0, |p| p.columns.len());
                    // One column at least stays.
                    if let Some(name) = name.filter(|_| tab.hidden.len() + 1 < count) {
                        tab.hidden.insert(name);
                    }
                }
                Task::none()
            }
            DbMsg::ShowColumns(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.hidden.clear();
                }
                Task::none()
            }
            DbMsg::SshScanned(result) => {
                if let Some(trust) = self.db.ssh_trust.as_mut() {
                    trust.scanning = false;
                    match result {
                        Ok(keys) => trust.keys = keys,
                        Err(e) => trust.error = Some(e),
                    }
                }
                Task::none()
            }
            DbMsg::SshTrustNo => {
                self.db.ssh_trust = None;
                Task::none()
            }
            DbMsg::SshTrustYes => {
                let Some(trust) = self.db.ssh_trust.take() else {
                    return Task::none();
                };
                match crate::ssh_trust::trust(&trust.keys) {
                    Ok(()) => {
                        self.set_status(t!("db.ssh_trusted", host = trust.host.as_str()), false)
                    }
                    Err(e) => self.set_status(e, true),
                }
                Task::none()
            }
            DbMsg::FilterByCell(path, row, col) => {
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return Task::none();
                };
                let found = tab.page.as_ref().and_then(|page| {
                    let name = page.columns.get(col)?.name.clone();
                    let value = tab.pending.value(page, row, col).map(str::to_string);
                    Some((name, value))
                });
                let Some((column, value)) = found else {
                    return Task::none();
                };
                // An empty cell is NULL here: the same filter finds it.
                let text = match value {
                    Some(v) => format!("= {v}"),
                    None => "null".to_string(),
                };
                tab.column_text.insert(column, text);
                tab.show_filters = true;
                self.db_message(DbMsg::ApplyFilter(path))
            }
            DbMsg::ToggleColumnFilters(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.show_filters = !tab.show_filters;
                }
                Task::none()
            }
            DbMsg::ApplyFilter(path) => {
                if self.db_blocked(&path) {
                    return Task::none();
                }
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.filter = tab.filter_text.trim().to_string();
                    tab.search = tab.search_text.trim().to_string();
                    tab.column_filters = tab
                        .column_text
                        .iter()
                        .filter(|(_, text)| !text.trim().is_empty())
                        .map(|(c, t)| (c.clone(), t.clone()))
                        .collect();
                    tab.notice = None;
                }
                self.db_load_rows(path, 0)
            }
            DbMsg::ClearFilter(path) => {
                if self.db_blocked(&path) {
                    return Task::none();
                }
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.filter_text.clear();
                    tab.filter.clear();
                    tab.search_text.clear();
                    tab.search.clear();
                    tab.column_text.clear();
                    tab.column_filters.clear();
                    tab.notice = None;
                }
                self.db_load_rows(path, 0)
            }
            DbMsg::PageSize(path, size) => {
                if self.db_blocked(&path) {
                    return Task::none();
                }
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.page_size = size;
                }
                // The next tables open with it.
                self.db.page_size = size;
                self.db.persist();
                self.db_load_rows(path, 0)
            }
            DbMsg::Sort(path, column) => {
                if self.db_blocked(&path) {
                    return Task::none();
                }
                // Alt+click stacks the sort on another column.
                let add = self.mod_alt;
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return Task::none();
                };
                let Some(name) = tab
                    .page
                    .as_ref()
                    .and_then(|p| p.columns.get(column))
                    .map(|c| c.name.clone())
                else {
                    return Task::none();
                };
                tab.order = tab.sorted_by(&name, add);
                tab.notice = None;
                self.db_load_rows(path, 0)
            }
            DbMsg::Edit(path, row, col) => {
                // What was being typed elsewhere is kept first.
                self.db_commit_edit(&path);
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return Task::none();
                };
                let Some(page) = tab.page.as_ref().filter(|p| p.editable) else {
                    return Task::none();
                };
                if tab.pending.is_deleted(row) || row >= tab.pending.rows(page) {
                    return Task::none();
                }
                // A value that was cut would be written cut.
                if page.truncated.contains(&(row, col)) {
                    tab.notice = Some(t!("db.cell_cut").to_string());
                    return Task::none();
                }
                let text = tab
                    .pending
                    .value(page, row, col)
                    .unwrap_or_default()
                    .to_string();
                tab.picker = None;
                tab.notice = None;
                tab.selected = Some((row, col));
                tab.editing = Some(grid::Editing { row, col, text });
                Task::none()
            }
            DbMsg::EditInput(path, text) => {
                if let Some(editing) = self.db_tabs.get_mut(&path).and_then(|t| t.editing.as_mut())
                {
                    editing.text = text;
                }
                Task::none()
            }
            DbMsg::EditCommit(path) => {
                self.db_commit_edit(&path);
                Task::none()
            }
            DbMsg::SetNull(path, row, col) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    let Some(page) = tab.page.clone().filter(|p| p.editable) else {
                        return Task::none();
                    };
                    if page.columns.get(col).is_some_and(|c| !c.nullable) {
                        tab.notice = Some(t!("db.not_nullable").to_string());
                    } else {
                        tab.pending.set(&page, row, col, None);
                        tab.notice = None;
                    }
                }
                Task::none()
            }
            DbMsg::DeleteRow(path, row) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let (true, Some(page)) = (tab.editable(), tab.page.clone()) {
                        tab.editing = None;
                        let before = tab.pending.rows(&page);
                        tab.pending.delete(&page, row);
                        tab.notice = None;
                        // The next row takes the selection, so that Alt+Delete
                        // again goes down the table instead of jumping to a
                        // row out of view. A new row is taken out, and the
                        // next one moves up into its place.
                        let rows = tab.pending.rows(&page);
                        let col = tab.selected.map_or(0, |(_, c)| c);
                        let next = if rows < before { row } else { row + 1 };
                        tab.anchor = None;
                        tab.selected = (rows > 0).then(|| (next.min(rows - 1), col));
                        if let Some((next, _)) = tab.selected {
                            return Self::db_reveal(tab, next);
                        }
                    }
                }
                Task::none()
            }
            DbMsg::RevertRow(path, row) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let Some(page) = tab.page.clone() {
                        tab.pending.revert_row(&page, row);
                    }
                }
                Task::none()
            }
            DbMsg::AddRow(path) => {
                self.db_commit_edit(&path);
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return Task::none();
                };
                let Some(page) = tab.page.clone().filter(|p| p.editable) else {
                    return Task::none();
                };
                let row = tab.pending.add_row(&page);
                // The first column that is not the key: the key is most often
                // given by the database.
                let col = page
                    .columns
                    .iter()
                    .position(|c| !page.primary_key.contains(&c.name))
                    .unwrap_or(0);
                tab.selected = Some((row, col));
                tab.notice = None;
                let reveal = Self::db_reveal(tab, row);
                Task::batch([reveal, self.db_message(DbMsg::Edit(path, row, col))])
            }
            DbMsg::SetDefault(path, row, col) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let Some(page) = tab.page.clone() {
                        tab.pending.set_default(&page, row, col);
                    }
                }
                Task::none()
            }
            DbMsg::RevertAll(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.pending = grid::Pending::default();
                    tab.editing = None;
                    tab.notice = None;
                }
                Task::none()
            }
            DbMsg::Commit(path) => {
                self.db_commit_edit(&path);
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return Task::none();
                };
                let Some(page) = tab.page.as_ref() else {
                    return Task::none();
                };
                if tab.committing || tab.pending.is_empty() {
                    return Task::none();
                }
                let changes = tab.pending.changes(page);
                if changes.is_empty() {
                    tab.notice = Some(t!("db.no_changes").to_string());
                    return Task::none();
                }
                let (conn, table) = (tab.conn, tab.table_ref());
                let live = tab.live.then(|| self.db.live.get(&conn).cloned()).flatten();
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.committing = true;
                    tab.notice = None;
                }
                Task::perform(
                    database::apply(live, params, table, changes),
                    move |result| Message::Db(DbMsg::Committed(path.clone(), result)),
                )
            }
            DbMsg::Preview(path, on) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.preview = on;
                }
                Task::none()
            }
            DbMsg::Committed(path, done) => {
                let Some(conn) = self.db_tabs.get(&path).map(|t| t.conn) else {
                    return Task::none();
                };
                let log = self.db_log(conn, &done.executed);
                let Some(tab) = self.db_tabs.get_mut(&path) else {
                    return log;
                };
                tab.committing = false;
                match done.result {
                    Ok(_) => {
                        tab.pending = grid::Pending::default();
                        tab.notice = None;
                        // What the table is now.
                        let offset = tab.offset();
                        Task::batch([log, self.db_load_rows(path, offset)])
                    }
                    Err(error) => {
                        tab.notice = Some(error);
                        log
                    }
                }
            }
            DbMsg::Grid(path, event) => self.db_grid(path, event),
            DbMsg::GridMenu(path, action, row, col) => self.db_grid_menu(path, &action, row, col),
            DbMsg::Select(path, row, col) => {
                self.db.console.editor_focused = false;
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    // A click elsewhere ends what was typed.
                    if tab
                        .editing
                        .as_ref()
                        .is_some_and(|e| (e.row, e.col) != (row, col))
                    {
                        self.db_commit_edit(&path);
                    }
                }
                let extend = self.mod_shift;
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    // Shift extends the block from where it started.
                    if extend {
                        tab.anchor = tab.anchor.or(tab.selected);
                    } else {
                        tab.anchor = None;
                    }
                    tab.selected = Some((row, col));
                }
                Task::none()
            }
            DbMsg::ToggleConsole => {
                let console = &mut self.db.console;
                console.open = !console.open;
                if console.open {
                    // One tool panel at the bottom.
                    self.plugin_bottom_yield();
                    if self.db.console.connection.is_none() {
                        let first = self.db.connections.first().map(|c| c.id);
                        self.db.use_console_connection(first);
                    }
                    if self.db.driver == Driver::Unknown {
                        return self.db_probe_driver();
                    }
                } else {
                    console.save();
                }
                Task::none()
            }
            DbMsg::ConsoleTab(tab) => {
                self.db.console.tab = tab;
                Task::none()
            }
            DbMsg::ConsoleConnection(id) => {
                if self.db.console.connection != Some(id) {
                    // The query running, or the transaction open, stays on
                    // the session of the connection it started on.
                    if let Some(why) = self.db_console_busy() {
                        self.db.console.last = Some((false, why));
                        return Task::none();
                    }
                    // Another connection: its session, not this one's.
                    self.db.console.database.clear();
                    self.db.console.in_transaction = false;
                }
                self.db.use_console_connection(Some(id));
                Task::none()
            }
            DbMsg::ConsoleDatabase(name) => {
                if self.db.console.database != name {
                    // What runs answers for the database it started in.
                    if self.db.console.running {
                        let why = t!("db.console_busy_running").to_string();
                        self.db.console.last = Some((false, why));
                        return Task::none();
                    }
                    // The session moves to another database: a transaction
                    // of the old one is rolled back.
                    self.db.console.database = name;
                    self.db.console.in_transaction = false;
                }
                Task::none()
            }
            DbMsg::Explain(analyze) => {
                if self.db.console.running {
                    return Task::none();
                }
                let statements = self.db.console.statements(false);
                let Some(conn) = self.db.console.connection else {
                    self.db.console.last =
                        Some((false, t!("db.console_no_connection").to_string()));
                    return Task::none();
                };
                if statements.is_empty() {
                    self.db.console.last = Some((false, t!("db.console_nothing").to_string()));
                    return Task::none();
                }
                // Only a statement that reads can be run to be measured.
                if analyze
                    && statements
                        .iter()
                        .any(|s| !database::protocol::wrappable(s, self.db.console.dialect))
                {
                    self.db.console.last = Some((false, t!("db.explain_not_read").to_string()));
                    return Task::none();
                }
                let dialect = self.db.console.dialect;
                let prefix = match (dialect, analyze) {
                    (database::protocol::Dialect::Postgres, true) => "EXPLAIN (ANALYZE, BUFFERS) ",
                    (database::protocol::Dialect::Mysql, true) => "EXPLAIN ANALYZE ",
                    // SQLite measures nothing: its plan is all there is.
                    (database::protocol::Dialect::Sqlite, _) => "EXPLAIN QUERY PLAN ",
                    (_, false) => "EXPLAIN ",
                };
                let statements = statements
                    .into_iter()
                    .map(|s| format!("{prefix}{s}"))
                    .collect();
                self.db_exec(conn, statements)
            }
            DbMsg::EndTransaction(commit) => {
                let Some(conn) = self.db.console.connection else {
                    return Task::none();
                };
                if self.db.console.running {
                    return Task::none();
                }
                let end = if commit { "COMMIT" } else { "ROLLBACK" };
                self.db_exec(conn, vec![end.to_string()])
            }
            DbMsg::Export(path, format) => {
                let Some(tab) = self.db_tabs.get(&path) else {
                    return Task::none();
                };
                let (conn, table, query) = (tab.conn, tab.table_ref(), tab.query(0));
                let live = tab.live.then(|| self.db.live.get(&conn).cloned()).flatten();
                let name = if tab.query.is_some() || tab.table.is_empty() {
                    "result".to_string()
                } else {
                    tab.table.clone()
                };
                let Some(params) = self.db_params(conn, false) else {
                    return self.db_ask_password(conn);
                };
                let ext = format.to_string();
                Task::perform(
                    async move {
                        let file = crate::editor::pick_save_path(format!("{name}.{ext}")).await?;
                        let target = file.to_string_lossy().to_string();
                        Some(match format {
                            "csv" | "json" => {
                                database::export(live, params, table, query, target, format).await
                            }
                            other => {
                                database::export_converted(
                                    live, params, table, query, target, other,
                                )
                                .await
                            }
                        })
                    },
                    move |done| Message::Db(DbMsg::Exported(path.clone(), done)),
                )
            }
            DbMsg::Exported(path, done) => {
                let Some(done) = done else {
                    return Task::none();
                };
                let log = match self.db_tabs.get(&path).map(|t| t.conn) {
                    Some(conn) => self.db_log(conn, &done.executed),
                    None => Task::none(),
                };
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.notice = Some(match done.result {
                        Ok(file) => t!("db.exported", path = file.as_str()).to_string(),
                        Err(error) => error,
                    });
                }
                log
            }
            DbMsg::ShowInternal(on) => {
                self.db.console.show_internal = on;
                Task::none()
            }
            DbMsg::CopyLog(at, what) => {
                let Some(entry) = self.db.console.log.get(at) else {
                    return Task::none();
                };
                let text = match what {
                    database::LogCopy::Sql => entry.sql.clone(),
                    database::LogCopy::Error => entry.error.clone().unwrap_or_default(),
                    database::LogCopy::Entry => entry.text(),
                };
                crate::task::copy(text)
            }
            DbMsg::AskDrop(conn, database, schema, table, kind) => self.admin_ask(
                (conn, database, schema, table),
                kind,
                database::ConfirmAction::Drop,
            ),
            DbMsg::AskEmpty(conn, database, schema, table) => self.admin_ask(
                (conn, database, schema, table),
                database::Kind::Table,
                database::ConfirmAction::Empty,
            ),
            DbMsg::ConfirmCascade(on) => {
                if let Some(c) = self.db.confirm.as_mut() {
                    c.cascade = on;
                }
                Task::none()
            }
            DbMsg::ConfirmRun => self.admin_confirm_run(),
            DbMsg::ConfirmClose => {
                if self.db.confirm.as_ref().is_some_and(|c| !c.running) {
                    self.db.confirm = None;
                }
                Task::none()
            }
            DbMsg::OpenDesigner(conn, database, schema, table) => {
                self.admin_open_designer((conn, database, schema, table))
            }
            DbMsg::Designer(msg) => self.admin_designer(msg),
            msg @ (DbMsg::NewTable(..)
            | DbMsg::NewSchema(..)
            | DbMsg::SchemaName(_)
            | DbMsg::SchemaCreate
            | DbMsg::SchemaClose
            | DbMsg::ToggleTable(..)
            | DbMsg::StructureRead(..)
            | DbMsg::ShowDefinition(..)
            | DbMsg::ShowFunction(..)
            | DbMsg::Definition(..)
            | DbMsg::ShowDdl(..)
            | DbMsg::TreeFilter(_)
            | DbMsg::ViewValue(..)
            | DbMsg::CopyRow(..)
            | DbMsg::TextCopy
            | DbMsg::CopyCell(..)
            | DbMsg::CopySelection(..)
            | DbMsg::ShowLinks(_)
            | DbMsg::CloseLinks
            | DbMsg::ShowDiagram(..)
            | DbMsg::DiagramLoaded(..)
            | DbMsg::CopyDiagram
            | DbMsg::TextClose
            | DbMsg::OpenRelated(..)) => self.db_tree_message(msg),
            DbMsg::Altered(altered) => self.admin_altered(*altered),
            DbMsg::ClearLog => {
                self.db.console.log.clear();
                self.db.console.selected_log = None;
                Task::none()
            }
            DbMsg::SqlText(text) => {
                let console = &mut self.db.console;
                if console.editor.text() != text {
                    let cursor = console.editor.cursor();
                    console.editor = crate::text_editor::Content::with_text(&text);
                    console.editor.move_to(cursor);
                    console.error_mark = None;
                }
                self.db.console.editor_focused = true;
                Task::none()
            }
            DbMsg::SqlCursor(cursor) => {
                self.db.console.editor.move_to(cursor);
                Task::none()
            }
            DbMsg::Stopped(done) => {
                // What the stop ran is in the log; nothing else to say.
                match self.db.console.connection {
                    Some(conn) => self.db_log(conn, &done.executed),
                    None => Task::none(),
                }
            }
            DbMsg::ColumnsLoaded(id, done) => {
                let log = self.db_log(id, &done.executed);
                if let Ok(catalog) = done.result {
                    self.db.forget_known();
                    self.db.columns.insert(id, catalog.columns);
                    self.db.foreign_keys.insert(id, catalog.foreign_keys);
                }
                log
            }
            DbMsg::Run(all) => {
                self.db.console.guard = None;
                self.db.console.binds = None;
                self.db_run(all)
            }
            DbMsg::GuardRun => match self.db.console.guard.take() {
                Some(guard) if !self.db.console.running => {
                    self.db_exec(guard.conn, guard.statements)
                }
                _ => Task::none(),
            },
            DbMsg::NewConsole => {
                self.db.console.new_doc();
                Task::none()
            }
            DbMsg::SwitchConsole(at) => {
                self.db.console.switch_doc(at);
                Task::none()
            }
            DbMsg::CloseConsole(at) => {
                self.db.console.close_doc(at);
                self.db.console.save();
                Task::none()
            }
            DbMsg::RunFile(file) => self.db_run_file(*file),
            DbMsg::FormatSql => {
                self.db.console.format();
                self.db.console.completion = None;
                self.db.console.save();
                Task::none()
            }
            DbMsg::LogFilter(text) => {
                self.db.console.log_filter = text;
                self.db.console.selected_log = None;
                Task::none()
            }
            DbMsg::LogReuse(at) => {
                if let Some(sql) = self.db.console.log.get(at).map(|e| e.sql.clone()) {
                    self.db.console.append_query(&sql);
                    self.db.console.tab = database::console::ConsoleTab::Queries;
                    self.db.console.save();
                }
                Task::none()
            }
            DbMsg::LogMore => {
                self.db.console.log_limit += 200;
                Task::none()
            }
            DbMsg::GuardCancel => {
                self.db.console.guard = None;
                Task::none()
            }
            DbMsg::StopLoad(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    if let Some(handle) = tab.stop.take() {
                        handle.abort();
                    }
                    // Whatever answer was on its way is not wanted.
                    tab.load_no += 1;
                    tab.loading = false;
                    tab.error = Some(t!("db.load_stopped").to_string());
                }
                Task::none()
            }
            DbMsg::BindValue(at, text) => {
                if let Some(value) = self
                    .db
                    .console
                    .binds
                    .as_mut()
                    .and_then(|b| b.values.get_mut(at))
                {
                    *value = text;
                }
                Task::none()
            }
            DbMsg::BindCancel => {
                self.db.console.binds = None;
                Task::none()
            }
            DbMsg::BindRun => match self.db.console.binds.take() {
                Some(binds) if !self.db.console.running => {
                    let statements = database::console::bind(
                        &binds.statements,
                        self.db.console.dialect,
                        &binds.names,
                        &binds.values,
                    );
                    self.db_run_checked(binds.conn, statements)
                }
                _ => Task::none(),
            },
            DbMsg::Cancel => {
                // The task is dropped, so is the driver: the server sees the
                // connection go and stops the query.
                if let Some(handle) = self.db.console.abort.take() {
                    handle.abort();
                }
                if self.db.console.running {
                    self.db.console.running = false;
                    // The driver is stopped with the session it held, and a
                    // transaction of it is rolled back by the server.
                    let mut stop = Task::none();
                    if let Some(conn) = self.db.console.connection {
                        // The server is told to stop it: before PostgreSQL 14 it
                        // would not notice that the driver went away.
                        let pid = self.db.live.get(&conn).map_or(0, database::Live::pid);
                        if pid > 0 {
                            if let Some(params) = self.db_params(conn, false) {
                                stop = Task::perform(database::cancel(params, pid), |done| {
                                    Message::Db(DbMsg::Stopped(done))
                                });
                            }
                        }
                        self.db.end_live(conn);
                    }
                    self.db.console.in_transaction = false;
                    self.db.console.last = Some((false, t!("db.console_cancelled").to_string()));
                    return stop;
                }
                Task::none()
            }
            DbMsg::Ran(conn, done) => {
                self.db.console.running = false;
                self.db.console.abort = None;
                let log = self.db_log(conn, &done.executed);
                let mut tasks = vec![log];
                match done.result {
                    Err(error) => {
                        // The session is gone with its transaction.
                        self.db.console.in_transaction = false;
                        self.db.console.last = Some((false, error));
                    }
                    Ok(ran) => {
                        if self.db.console.connection == Some(conn) {
                            self.db.console.in_transaction = ran.in_transaction;
                        }
                        let ms: u64 = ran.results.iter().map(|r| r.ms).sum();
                        self.db.console.last = Some(match &ran.failed {
                            Some((index, error)) => (
                                false,
                                t!("db.console_failed", n = index + 1, error = error.as_str())
                                    .to_string(),
                            ),
                            None => (
                                true,
                                t!("db.console_done", n = ran.results.len(), ms = ms).to_string(),
                            ),
                        });
                        // A SELECT shows its rows in a tab, like a table does;
                        // the rest says what it did in the log.
                        for result in ran.results.iter().filter(|r| r.has_rows).take(10) {
                            tasks.push(self.db_open_result(conn, result));
                        }
                        if let (Some((index, _)), Some(position)) = (&ran.failed, ran.position) {
                            if let Some(sql) = self.db.console.running_sql.get(*index).cloned() {
                                self.db.console.mark_error(&sql, position);
                            }
                        }
                        if ran.failed.is_some() {
                            self.db.console.tab = database::console::ConsoleTab::Log;
                        }
                    }
                }
                Task::batch(tasks)
            }
            DbMsg::OpenPicker(path, row, col) => {
                self.db_commit_edit(&path);
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    let Some(page) = tab.page.as_ref().filter(|p| p.editable) else {
                        return Task::none();
                    };
                    let Some(kind) = page
                        .columns
                        .get(col)
                        .and_then(|c| grid::PickKind::of(&c.kind))
                    else {
                        return Task::none();
                    };
                    let current = tab.pending.value(page, row, col);
                    tab.picker = Some(grid::Picker::new(row, col, kind, current));
                    tab.notice = None;
                }
                Task::none()
            }
            DbMsg::PickerShift(path, months) => {
                if let Some(picker) = self.db_tabs.get_mut(&path).and_then(|t| t.picker.as_mut()) {
                    picker.shift(months);
                }
                Task::none()
            }
            DbMsg::PickerDay(path, day) => {
                if let Some(picker) = self.db_tabs.get_mut(&path).and_then(|t| t.picker.as_mut()) {
                    picker.day = Some(day);
                }
                Task::none()
            }
            DbMsg::PickerTime(path, part, text) => {
                if let Some(picker) = self.db_tabs.get_mut(&path).and_then(|t| t.picker.as_mut()) {
                    picker.set_time(part, text);
                }
                Task::none()
            }
            DbMsg::PickerToday(path) => {
                if let Some(picker) = self.db_tabs.get_mut(&path).and_then(|t| t.picker.as_mut()) {
                    picker.today();
                }
                Task::none()
            }
            DbMsg::PickerApply(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    let value = tab.picker.as_ref().and_then(grid::Picker::value);
                    if let (Some(value), Some(picker), Some(page)) =
                        (value, tab.picker.take(), tab.page.clone())
                    {
                        tab.pending.set(&page, picker.row, picker.col, Some(value));
                    }
                }
                Task::none()
            }
            DbMsg::PickerClose(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.picker = None;
                }
                Task::none()
            }
            DbMsg::DismissNotice(path) => {
                if let Some(tab) = self.db_tabs.get_mut(&path) {
                    tab.notice = None;
                }
                Task::none()
            }
        }
    }

    /// The driver is this file (or the downloaded one, for an empty path).
    fn db_use_driver_file(&mut self, path: String) -> Task<Message> {
        self.db.driver_path = path.trim().to_string();
        self.db.persist();
        self.db.driver = Driver::Unknown;
        self.db_probe_driver()
    }
}
