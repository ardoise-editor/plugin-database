//! The plugin as the editor runs it: the panels and tabs drawn as nodes,
//! what is done in them turned into the messages of the Database tool, and
//! what the tool asks of the editor carried out.

pub mod admin;
pub mod console;
pub mod dialog;
mod table;
pub mod tree;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ardoise_plugin::protocol::{
    CompletionParams, Completions, Context, InitializeParams, Settings, TreeAction, UiEvent,
};
use ardoise_plugin::ui::{
    button, checkbox, input, select, tree_item, ButtonStyle, GridEvent, Node, TreeItem,
};
use ardoise_plugin::{run as run_plugin, Host, Plugin};

use crate::app::{App, Effect, Message, Window};
use crate::database::DbMsg;

pub const DIAGRAM: &str = "diagram";
/// The tabs of the windows of the tool (one each, open while it is).
pub const DIALOG: &str = "connection";
pub const DESIGNER: &str = "designer";
pub const HEALTH: &str = "health";
pub const IMPORT: &str = "import";
pub const COPY: &str = "copy";
pub const BACKUP: &str = "backup";
pub const TEXT: &str = "text";

type TextFn = Box<dyn Fn(String) -> DbMsg + Send>;
type CheckFn = Box<dyn Fn(bool) -> DbMsg + Send>;

enum Act {
    Msg(Box<DbMsg>),
    Text(TextFn),
    Check(CheckFn),
    /// The grid of a table tab.
    Grid(PathBuf),
    /// An item of the menu of a cell of that tab's grid.
    Menu(PathBuf),
}

/// What the controls of a panel stand for: the messages their events become.
#[derive(Default)]
pub struct Ui {
    acts: HashMap<String, Act>,
    /// What the chevron of a row of a tree does (its click is in `acts`).
    toggles: HashMap<String, DbMsg>,
    pub dark: bool,
}

/// An id that is the same for the same message, render after render: a
/// click that arrives late can't do something else.
fn id_of(msg: &DbMsg) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{msg:?}").hash(&mut h);
    format!("m{:x}", h.finish())
}

impl Ui {
    pub fn new(dark: bool) -> Self {
        Ui {
            acts: HashMap::new(),
            toggles: HashMap::new(),
            dark,
        }
    }

    /// The id a message has.
    #[cfg(test)]
    fn act_of(&self, msg: &DbMsg) -> String {
        id_of(msg)
    }

    /// An id that sends this message.
    pub fn act(&mut self, msg: DbMsg) -> String {
        let id = id_of(&msg);
        self.acts.insert(id.clone(), Act::Msg(Box::new(msg)));
        id
    }

    pub fn button(&mut self, label: impl Into<String>, msg: DbMsg) -> Node {
        let id = self.act(msg);
        button(label, id)
    }

    /// A button that is there but does nothing when `msg` is none.
    pub fn button_if(&mut self, label: impl Into<String>, msg: Option<DbMsg>) -> Node {
        match msg {
            Some(msg) => self.button(label, msg),
            None => button(label, "none").enabled(false),
        }
    }

    pub fn tool(&mut self, label: impl Into<String>, msg: DbMsg) -> Node {
        self.button(label, msg).button_style(ButtonStyle::Ghost)
    }

    /// A square button that is only a picto (a Lucide icon): `tip` is its
    /// tooltip and its accessible name.
    pub fn icon_tool(&mut self, icon: &str, tip: impl Into<String>, msg: DbMsg) -> Node {
        self.tool(tip, msg).icon(icon)
    }

    /// The same, doing nothing when `msg` is none.
    pub fn icon_tool_if(&mut self, icon: &str, tip: impl Into<String>, msg: Option<DbMsg>) -> Node {
        match msg {
            Some(msg) => self.icon_tool(icon, tip, msg),
            None => button(tip, "none")
                .button_style(ButtonStyle::Ghost)
                .icon(icon)
                .enabled(false),
        }
    }

    pub fn primary(&mut self, label: impl Into<String>, msg: DbMsg) -> Node {
        self.button(label, msg).button_style(ButtonStyle::Primary)
    }

    /// A strip of tabs, one per value of `all`, the open one underlined.
    pub fn tabs<T: Copy + PartialEq + Send + Sync + 'static>(
        &mut self,
        id: &str,
        all: &'static [T],
        open: T,
        label: fn(T) -> String,
        then: fn(T) -> DbMsg,
    ) -> Node {
        let (items, at) = Self::choices(all, open, label, |_| None);
        self.pick_index(id, all, open, then);
        ardoise_plugin::ui::tabs(id, items).selected(at)
    }

    /// Tiles, one per value of `all`, a picto above each label.
    pub fn tiles<T: Copy + PartialEq + Send + Sync + 'static>(
        &mut self,
        id: &str,
        all: &'static [T],
        chosen: T,
        label: fn(T) -> String,
        icon: fn(T) -> Option<&'static str>,
        then: fn(T) -> DbMsg,
    ) -> Node {
        let (items, at) = Self::choices(all, chosen, label, icon);
        self.pick_index(id, all, chosen, then);
        ardoise_plugin::ui::tiles(id, items).selected(at)
    }

    fn choices<T: Copy + PartialEq>(
        all: &[T],
        selected: T,
        label: fn(T) -> String,
        icon: impl Fn(T) -> Option<&'static str>,
    ) -> (Vec<ardoise_plugin::ui::Item>, String) {
        let items = all
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let item = ardoise_plugin::ui::item(i.to_string(), label(*v));
                match icon(*v) {
                    Some(icon) => item.icon(icon),
                    None => item,
                }
            })
            .collect();
        let at = all.iter().position(|v| *v == selected).unwrap_or(0);
        (items, at.to_string())
    }

    /// The event of tabs or tiles carries the place of the one picked; a
    /// stale one (nothing there any more) picks what is already chosen.
    fn pick_index<T: Copy + Send + Sync + 'static>(
        &mut self,
        id: &str,
        all: &'static [T],
        current: T,
        then: fn(T) -> DbMsg,
    ) {
        self.acts.insert(
            id.to_string(),
            Act::Text(Box::new(move |at| {
                let value = at
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| all.get(i).copied())
                    .unwrap_or(current);
                then(value)
            })),
        );
    }

    /// A field with a fixed id (the editor keeps what is typed in it).
    pub fn input(
        &mut self,
        id: &str,
        value: &str,
        placeholder: impl Into<String>,
        on: impl Fn(String) -> DbMsg + Send + 'static,
    ) -> Node {
        self.acts.insert(id.to_string(), Act::Text(Box::new(on)));
        input(id, value, placeholder)
    }

    pub fn submit(&mut self, node: Node, msg: DbMsg) -> Node {
        let id = self.act(msg);
        node.on_submit(id)
    }

    pub fn checkbox(
        &mut self,
        id: &str,
        label: impl Into<String>,
        on: bool,
        then: impl Fn(bool) -> DbMsg + Send + 'static,
    ) -> Node {
        self.acts.insert(id.to_string(), Act::Check(Box::new(then)));
        checkbox(id, label, on)
    }

    /// A choice among labels; the one picked is given to `then`.
    pub fn select(
        &mut self,
        id: &str,
        options: Vec<String>,
        selected: Option<String>,
        then: impl Fn(String) -> DbMsg + Send + 'static,
    ) -> Node {
        self.acts.insert(id.to_string(), Act::Text(Box::new(then)));
        select(id, options, selected)
    }
    pub fn tree_item(&mut self, label: impl Into<String>, msg: DbMsg) -> TreeItem {
        let id = self.act(msg);
        tree_item(id, label)
    }

    /// An editor of text whose changes go to `then`.
    pub fn editor(
        &mut self,
        id: &str,
        value: &str,
        language: &str,
        then: impl Fn(String) -> DbMsg + Send + 'static,
    ) -> Node {
        self.acts.insert(id.to_string(), Act::Text(Box::new(then)));
        ardoise_plugin::ui::editor(id, value, language)
    }

    /// What the chevron of a row of a tree does.
    pub fn on_toggle(&mut self, row: &TreeItem, msg: DbMsg) {
        self.toggles.insert(row.id.clone(), msg);
    }

    pub fn menu(&mut self, action: &str, path: &Path) {
        self.acts
            .insert(action.to_string(), Act::Menu(path.to_path_buf()));
    }

    pub fn grid(&mut self, id: &str, path: &Path) {
        self.acts
            .insert(id.to_string(), Act::Grid(path.to_path_buf()));
    }

    /// The message an event of the panel stands for.
    fn message(&self, event: &UiEvent) -> Option<DbMsg> {
        match self.acts.get(&event.action)? {
            Act::Msg(msg) => Some(*msg.clone()),
            Act::Text(f) => Some(f(event.text().unwrap_or_default().to_string())),
            Act::Check(f) => Some(f(event.checked().unwrap_or(false))),
            Act::Menu(path) => {
                let value = event.value.as_ref()?;
                let at = |k: &str| value.get(k).and_then(|v| v.as_u64()).map(|n| n as usize);
                table::menu_message(path, &event.action, at("row")?, at("col")?)
            }
            Act::Grid(path) => {
                let grid = GridEvent::from_event(event)?;
                table::grid_message(path, grid)
            }
        }
    }

    /// The message of a row of a list or a tree.
    fn row_message(&self, event: &UiEvent) -> Option<DbMsg> {
        let id = match event.tree_item() {
            Some((id, TreeAction::Toggle)) => {
                if let Some(msg) = self.toggles.get(id) {
                    return Some(msg.clone());
                }
                id.to_string()
            }
            Some((id, TreeAction::Click)) => id.to_string(),
            None => event.text()?.to_string(),
        };
        match self.acts.get(&id)? {
            Act::Msg(msg) => Some(*msg.clone()),
            _ => None,
        }
    }
}

struct State {
    app: App,
    /// What the controls of each panel and tab stand for.
    panels: HashMap<String, Ui>,
    /// The tabs of the plugin open in the editor.
    open_tabs: std::collections::HashSet<String>,
    /// The title each window was opened with: a new one reopens it.
    titles: HashMap<String, String>,
    dark: bool,
}

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    host: Host,
    runtime: Arc<tokio::runtime::Runtime>,
}

fn lock(state: &Arc<Mutex<State>>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// Handles a message, and what follows from it; then draws again.
    fn dispatch(&self, message: Message) {
        let mut effects = Vec::new();
        {
            let task = {
                let mut state = lock(&self.state);
                let task = state.app.update(message);
                effects.append(&mut state.app.effects);
                task
            };
            let mut futures = Vec::new();
            task.split(&mut effects, &mut futures);
            for future in futures {
                let me = self.clone();
                self.runtime.spawn(async move {
                    if let Some(message) = future.await {
                        let again = me.clone();
                        // Off the runtime: the editor may be asked things.
                        let _ = tokio::task::spawn_blocking(move || again.dispatch(message)).await;
                    }
                });
            }
        }
        for effect in effects {
            self.effect(effect);
        }
        self.render();
    }

    fn effect(&self, effect: Effect) {
        match effect {
            Effect::OpenTab(id, title) => {
                lock(&self.state).open_tabs.insert(id.clone());
                let _ = self.host.open_tab(&id, title);
            }
            Effect::CloseTab(id) => {
                lock(&self.state).open_tabs.remove(&id);
                self.host.close_tab(&id);
            }
            Effect::Copy(text) => self.host.copy(text),
            Effect::Say(text, true) => self.host.error(text),
            Effect::Say(text, false) => self.host.notify(text),
            Effect::ShowPanel(panel) => self.host.show_panel(&panel),
        }
    }

    /// Draws the tree, the console and every tab of the plugin that is open.
    fn render(&self) {
        let mut state = lock(&self.state);
        let dark = state.dark;
        // The windows of the tool are popins (the diagram, a tab), open while
        // they are wanted.
        let wanted = windows(&state.app);
        let mut opens = Vec::new();
        let mut closes = Vec::new();
        for (id, title, on) in &wanted {
            let open = state.open_tabs.contains(*id);
            if *on && !open {
                opens.push((id.to_string(), title.clone(), window_head(&state.app, id)));
            } else if *on && state.titles.get(*id) != Some(title) {
                // Open, but with another title (the next of the questions):
                // the editor renames it and brings it in front.
                opens.push((id.to_string(), title.clone(), window_head(&state.app, id)));
            } else if !*on && open {
                closes.push(id.to_string());
            }
        }
        let mut drawn: Vec<(String, Node)> = Vec::new();
        let mut panels = HashMap::new();
        let mut ui = Ui::new(dark);
        drawn.push((crate::TREE.into(), tree::view(&mut ui, &state.app)));
        panels.insert(crate::TREE.to_string(), ui);
        let mut ui = Ui::new(dark);
        drawn.push((crate::CONSOLE.into(), console::view(&mut ui, &state.app)));
        panels.insert(crate::CONSOLE.to_string(), ui);
        let tabs: Vec<(PathBuf, String)> = state
            .app
            .tab_ids
            .iter()
            .map(|(p, id)| (p.clone(), id.clone()))
            .collect();
        let mut dirty = Vec::new();
        for (path, id) in tabs {
            if let Some(tab) = state.app.db_tabs.get(&path) {
                let mut ui = Ui::new(dark);
                drawn.push((id.clone(), table::view(&mut ui, &path, tab)));
                dirty.push((id.clone(), !tab.pending.is_empty()));
                panels.insert(id, ui);
            }
        }
        for (id, _, on) in &wanted {
            if *on {
                let mut ui = Ui::new(dark);
                if let Some(node) = window(&mut ui, &state.app, id) {
                    drawn.push((id.to_string(), node));
                    panels.insert(id.to_string(), ui);
                }
            }
        }
        state.panels = panels;
        for (id, title, _) in &opens {
            state.open_tabs.insert(id.clone());
            state.titles.insert(id.clone(), title.clone());
        }
        for id in &closes {
            state.open_tabs.remove(id);
            state.titles.remove(id);
        }
        drop(state);
        for (id, node) in drawn {
            self.host.render(&id, node);
        }
        for (id, dirty) in dirty {
            self.host.tab_dirty(&id, dirty);
        }
        // The windows are popins over the editor, but the diagram, which is
        // read and compared with the tables, stays a tab.
        for (id, title, (icon, subtitle)) in opens {
            let _ = if id == DIAGRAM {
                self.host.open_tab(&id, title)
            } else {
                self.host.open_dialog_with(&id, title, icon, subtitle)
            };
        }
        for id in closes {
            self.host.close_tab(&id);
        }
    }
}

/// The picto and the line under the title of a window, as its header shows
/// them.
pub fn window_head(app: &App, id: &str) -> (&'static str, String) {
    let db = &app.db;
    match id {
        DIALOG => ("database", t!("db.sub_dialog").to_string()),
        DESIGNER => ("table-2", t!("db.sub_designer").to_string()),
        HEALTH => ("heart-pulse", t!("db.sub_health").to_string()),
        IMPORT => ("file-input", t!("db.sub_import").to_string()),
        COPY => ("copy-plus", t!("db.sub_copy").to_string()),
        BACKUP if db.backup.as_ref().is_some_and(|b| b.restore) => {
            ("archive-restore", t!("db.sub_restore").to_string())
        }
        BACKUP => ("hard-drive-download", t!("db.sub_backup").to_string()),
        TEXT if db.confirm.is_some() => ("trash-2", t!("db.sub_confirm").to_string()),
        TEXT if db.ssh_trust.is_some() => ("key-round", t!("db.sub_ssh").to_string()),
        TEXT if db.new_schema.is_some() => ("folder-plus", t!("db.sub_schema").to_string()),
        TEXT if db.links.is_some() => ("link", t!("db.sub_links").to_string()),
        _ => ("file", t!("db.sub_text").to_string()),
    }
}

/// The windows of the tool, their titles and whether each is wanted now.
#[cfg(test)]
pub fn window_titles(app: &App) -> Vec<(&'static str, String, bool)> {
    windows(app)
}

/// The windows of the tool and whether each is wanted now.
fn windows(app: &App) -> Vec<(&'static str, String, bool)> {
    let db = &app.db;
    vec![
        (
            DIALOG,
            match &db.dialog {
                Some(d) if d.editing.is_some() => t!("db.dialog_edit").to_string(),
                _ => t!("db.dialog_new").to_string(),
            },
            db.dialog.is_some(),
        ),
        (
            DESIGNER,
            db.designer
                .as_ref()
                .map_or_else(|| t!("db.table_modify").to_string(), admin::designer_title),
            db.designer.is_some(),
        ),
        (
            HEALTH,
            db.health
                .as_ref()
                .map_or_else(|| t!("db.health").to_string(), admin::health_title),
            db.health.is_some(),
        ),
        (
            IMPORT,
            db.import
                .as_ref()
                .map_or_else(|| t!("db.import").to_string(), admin::import_title),
            db.import.is_some(),
        ),
        (
            COPY,
            db.copy_table.as_ref().map_or_else(
                || t!("db.copy_table").to_string(),
                |c| admin::copy_title(db, c),
            ),
            db.copy_table.is_some(),
        ),
        (
            BACKUP,
            db.backup
                .as_ref()
                .map_or_else(|| t!("db.backup").to_string(), admin::backup_title),
            db.backup.is_some(),
        ),
        (
            TEXT,
            admin::small_title(db),
            db.text.is_some()
                || db.links.is_some()
                || db.new_schema.is_some()
                || db.confirm.is_some()
                || db.ssh_trust.is_some(),
        ),
        (DIAGRAM, t!("db.diagram").to_string(), app.diagram.is_some()),
    ]
}

fn window(ui: &mut Ui, app: &App, id: &str) -> Option<Node> {
    match id {
        DIALOG => Some(dialog::view(ui, &app.db, app.db.dialog.as_ref()?)),
        DESIGNER => Some(admin::designer(ui, app.db.designer.as_ref()?)),
        HEALTH => Some(admin::health(ui, app.db.health.as_ref()?)),
        IMPORT => Some(admin::import(ui, app.db.import.as_ref()?)),
        COPY => Some(admin::copy(ui, &app.db, app.db.copy_table.as_ref()?)),
        BACKUP => Some(admin::backup(ui, app.db.backup.as_ref()?)),
        TEXT => Some(admin::small(ui, &app.db)),
        DIAGRAM => Some(admin::diagram(ui, app.diagram.as_ref()?)),
        _ => None,
    }
}

/// The window a tab stands for.
fn window_of(id: &str) -> Option<Window> {
    Some(match id {
        DIALOG => Window::Dialog,
        DESIGNER => Window::Designer,
        HEALTH => Window::Health,
        IMPORT => Window::Import,
        COPY => Window::Copy,
        BACKUP => Window::Backup,
        TEXT => Window::Text,
        DIAGRAM => Window::Diagram,
        _ => return None,
    })
}

struct DbPlugin {
    shared: Option<Shared>,
}

impl DbPlugin {
    fn shared(&self) -> Shared {
        self.shared.clone().expect("initialized")
    }
}

impl Plugin for DbPlugin {
    fn initialize(&mut self, host: &Host, params: &InitializeParams) {
        rust_i18n::set_locale(&params.settings.locale);
        crate::fsutil::set_data_dir(params.data_dir.clone());
        crate::editor::set_host(host.clone());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");
        let shared = Shared {
            state: Arc::new(Mutex::new(State {
                app: App::new(),
                panels: HashMap::new(),
                open_tabs: Default::default(),
                titles: Default::default(),
                dark: params.settings.dark,
            })),
            host: host.clone(),
            runtime: Arc::new(runtime),
        };
        self.shared = Some(shared.clone());
        shared.render();
    }

    fn command(&mut self, host: &Host, command: &str, _: &Context) {
        let shared = self.shared();
        match command {
            "database.show" => host.show_panel(crate::TREE),
            "database.console" => {
                let open = lock(&shared.state).app.db.console.open;
                if !open {
                    shared.dispatch(Message::Db(DbMsg::ToggleConsole));
                }
                host.show_panel(crate::CONSOLE);
            }
            "database.add" => shared.dispatch(Message::Db(DbMsg::OpenDialog(None))),
            "database.run_file" => {
                let me = shared.clone();
                let host = host.clone();
                std::thread::spawn(move || {
                    if let Ok(file) = host.active_file() {
                        me.dispatch(Message::Db(DbMsg::RunFile(Box::new(file))));
                    }
                });
            }
            _ => {}
        }
    }

    fn panel_opened(&mut self, _: &Host, panel: &str) {
        let shared = self.shared();
        {
            let mut state = lock(&shared.state);
            if panel == crate::CONSOLE && !state.app.db.console.open {
                drop(state);
                shared.dispatch(Message::Db(DbMsg::ToggleConsole));
                return;
            }
            // A tab the session kept, of a table no longer open: it goes.
            let known = panel == crate::TREE
                || panel == crate::CONSOLE
                || state.app.path_of(panel).is_some()
                || windows(&state.app)
                    .iter()
                    .any(|(id, _, on)| *id == panel && *on);
            if !known {
                state.open_tabs.remove(panel);
                drop(state);
                shared.host.close_tab(panel);
                return;
            }
            if panel != crate::TREE && panel != crate::CONSOLE {
                state.open_tabs.insert(panel.to_string());
                if let Some(path) = state.app.path_of(panel) {
                    state.app.current_file = Some(path);
                }
            }
        }
        shared.render();
    }

    fn panel_closed(&mut self, _: &Host, panel: &str) {
        let shared = self.shared();
        if panel == crate::CONSOLE && lock(&shared.state).app.db.console.open {
            shared.dispatch(Message::Db(DbMsg::ToggleConsole));
        }
    }

    fn tab_closed(&mut self, _: &Host, tab: &str) {
        let shared = self.shared();
        let (path, was_open) = {
            let mut state = lock(&shared.state);
            let was_open = state.open_tabs.remove(tab);
            (state.app.path_of(tab), was_open)
        };
        if let Some(path) = path {
            {
                let mut state = lock(&shared.state);
                state.app.db_tabs.remove(&path);
                state.app.tab_ids.remove(&path);
            }
            shared.render();
            return;
        }
        // A window closed with its tab: it goes, unless it is working
        // (its tab comes back at the next drawing).
        if was_open {
            if let Some(window) = window_of(tab) {
                lock(&shared.state).app.close_window(window);
                shared.render();
            }
        }
    }

    fn ui_event(&mut self, _: &Host, event: &UiEvent) {
        let shared = self.shared();
        let msg = {
            let state = lock(&shared.state);
            let Some(ui) = state.panels.get(&event.panel) else {
                return;
            };
            ui.message(event).or_else(|| ui.row_message(event))
        };
        let Some(msg) = msg else {
            return;
        };
        // A run takes the statement at the cursor of the console's editor,
        // or what is selected there: the editor says where they are.
        if matches!(msg, DbMsg::Run(_) | DbMsg::Explain(_) | DbMsg::FormatSql) {
            let me = shared.clone();
            std::thread::spawn(move || {
                if let Ok(found) = me.host.editor_state(crate::CONSOLE, "sql") {
                    use crate::text_editor::{Cursor, Position};
                    let at = |p: ardoise_plugin::protocol::TextPosition| Position {
                        line: p.line,
                        column: p.column,
                    };
                    me.dispatch(Message::Db(DbMsg::SqlText(found.text)));
                    me.dispatch(Message::Db(DbMsg::SqlCursor(Cursor {
                        position: at(found.cursor),
                        selection: found.anchor.map(at),
                    })));
                }
                me.dispatch(Message::Db(msg));
            });
            return;
        }
        shared.dispatch(Message::Db(msg));
    }

    fn settings_changed(&mut self, _: &Host, settings: &Settings) {
        rust_i18n::set_locale(&settings.locale);
        let shared = self.shared();
        lock(&shared.state).dark = settings.dark;
        shared.render();
    }

    fn completion(&mut self, _: &Host, params: &CompletionParams) -> Result<Completions, String> {
        if params.panel != crate::CONSOLE {
            return Ok(Completions::default());
        }
        let shared = self.shared();
        let mut state = lock(&shared.state);
        let (from, items) = state.app.console_completions(&params.text, params.offset);
        Ok(Completions {
            items,
            from: Some(from),
        })
    }

    fn forget_data(&mut self, _: &Host) -> Result<(), String> {
        if let Some(shared) = &self.shared {
            lock(&shared.state).app.db.forget_secrets();
        }
        Ok(())
    }

    fn shutdown(&mut self, _: &Host) {
        if let Some(shared) = &self.shared {
            lock(&shared.state).app.db.console.save();
        }
    }
}

pub fn run() {
    if let Err(error) = run_plugin(DbPlugin { shared: None }) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// Labels of the options of a choice, and the one that stands for `value`.
pub fn choice<T: PartialEq + Copy>(
    all: &[T],
    value: T,
    label: impl Fn(T) -> String,
) -> (Vec<String>, Option<String>) {
    (
        all.iter().map(|v| label(*v)).collect(),
        all.iter().find(|v| **v == value).map(|v| label(*v)),
    )
}

/// The value a label picked stands for.
pub fn picked<T: Copy>(all: &[T], text: &str, label: impl Fn(T) -> String) -> Option<T> {
    all.iter().copied().find(|v| label(*v) == text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_keeps_its_id_from_one_render_to_the_next() {
        let mut a = Ui::new(false);
        let mut b = Ui::new(false);
        assert_eq!(a.act(DbMsg::ToggleConsole), b.act(DbMsg::ToggleConsole));
        assert_ne!(a.act(DbMsg::Reload(1)), a.act(DbMsg::Reload(2)));
        let field = a.input("f", "", "", DbMsg::TreeFilter);
        assert!(matches!(field, Node::Input { .. }));
        let event = UiEvent {
            panel: "p".into(),
            action: "f".into(),
            value: Some(serde_json::json!("users")),
        };
        assert!(matches!(a.message(&event), Some(DbMsg::TreeFilter(t)) if t == "users"));
    }

    /// The buttons and menu items that carry a key, as `(label, key)`.
    fn keyed(node: &serde_json::Value, out: &mut Vec<(String, String)>) {
        if let (Some(label), Some(key)) = (node["label"].as_str(), node["key"].as_str()) {
            out.push((label.to_string(), key.to_string()));
        }
        match node {
            serde_json::Value::Object(map) => map.values().for_each(|v| keyed(v, out)),
            serde_json::Value::Array(list) => list.iter().for_each(|v| keyed(v, out)),
            _ => {}
        }
    }

    #[test]
    fn f4_opens_the_form_and_ctrl_c_copies_the_sql_of_the_entry_clicked() {
        use crate::app::Message;
        use crate::database::console::{ConsoleTab, LogEntry};
        let tab =
            crate::database::TableTab::new(1, "shop".into(), "public".into(), "t".into(), 100);
        let menu = serde_json::to_value(table::menu(&tab)).unwrap();
        let mut found = Vec::new();
        keyed(&menu, &mut found);
        assert_eq!(
            found,
            vec![(t!("db.form_open").to_string(), "F4".to_string())]
        );

        let mut app = App::new();
        app.db.console.open = true;
        app.db.console.tab = ConsoleTab::Log;
        for sql in ["SELECT 1", "SELECT 2"] {
            app.db.console.log.push(LogEntry {
                time: "10:00:00".into(),
                connection: "shop".into(),
                sql: sql.into(),
                ms: 1,
                rows: None,
                error: None,
                internal: false,
            });
        }
        let shown = |app: &App| {
            let mut ui = Ui::new(false);
            let node = console::view(&mut ui, app);
            let mut found = Vec::new();
            keyed(&serde_json::to_value(&node).unwrap(), &mut found);
            (ui, node, found)
        };
        let (ui, node, found) = shown(&app);
        assert!(found.is_empty(), "nothing clicked: no key");
        // A click on the SQL of the second entry.
        let json = serde_json::to_value(&node).unwrap().to_string();
        let id = ui.act_of(&DbMsg::SelectLog(1));
        assert!(json.contains(&id));
        let event = UiEvent {
            panel: "console".into(),
            action: "log_entry".into(),
            value: Some(serde_json::json!(id)),
        };
        let msg = ui.row_message(&event).unwrap();
        assert!(matches!(msg, DbMsg::SelectLog(1)));
        let _ = app.update(Message::Db(msg));
        let (_, _, found) = shown(&app);
        assert_eq!(
            found,
            vec![(t!("db.log_copy_sql").to_string(), "Ctrl+C".to_string())]
        );
    }
}
