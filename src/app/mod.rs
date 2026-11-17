//! The Database tool: its state (the connections, the tables open in tabs,
//! the console) and what its messages do. The logic is the editor's own,
//! moved here: what it asked of the editor's window (a tab, the
//! clipboard, the status bar) is now asked of the editor through effects.

mod database;
mod db_tools;
mod db_tree;
mod table_admin;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::database::{DbMsg, State, TableTab};
pub use crate::task::{Effect, Task};

#[derive(Debug, Clone)]
pub enum Message {
    Db(DbMsg),
}

pub struct App {
    pub db: State,
    pub db_tabs: HashMap<PathBuf, TableTab>,
    pub db_counter: usize,
    /// The id of the tab of the editor each table tab is.
    pub tab_ids: HashMap<PathBuf, String>,
    /// The tab of the plugin in front, if one is.
    pub current_file: Option<PathBuf>,
    /// Keys held with the last click.
    pub mod_shift: bool,
    pub mod_alt: bool,
    /// What is asked of the editor once a message is handled.
    pub effects: Vec<Effect>,
    /// The text the diagram of a schema is copied as (Mermaid), and its
    /// picture, while its tab is open.
    pub diagram: Option<crate::database::diagram::Shown>,
}

impl App {
    pub fn new() -> Self {
        App {
            db: State::load(),
            db_tabs: HashMap::new(),
            db_counter: 0,
            tab_ids: HashMap::new(),
            current_file: None,
            mod_shift: false,
            mod_alt: false,
            effects: Vec::new(),
            diagram: None,
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.effects.push(Effect::Say(text.into(), error));
    }

    /// The id of the tab of a table (made the first time).
    pub fn tab_id(&mut self, path: &std::path::Path) -> String {
        let next = self.tab_ids.len() + 1;
        self.tab_ids
            .entry(path.to_path_buf())
            .or_insert_with(|| format!("table-{next}"))
            .clone()
    }

    /// The table tab an id of the editor stands for.
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        self.tab_ids
            .iter()
            .find(|(_, tab)| *tab == id)
            .map(|(path, _)| path.clone())
    }

    /// Brings a tab to the front (it is opened if it was not).
    pub fn activate(&mut self, path: PathBuf) -> Task<Message> {
        let id = self.tab_id(&path);
        let title = self
            .db_tabs
            .get(&path)
            .map(|t| {
                if t.title.is_empty() {
                    t.table.clone()
                } else {
                    t.title.clone()
                }
            })
            .unwrap_or_default();
        self.current_file = Some(path);
        Task::effect(Effect::OpenTab(id, title))
    }

    pub fn close_path(&mut self, path: PathBuf) -> Task<Message> {
        self.db_tabs.remove(&path);
        match self.tab_ids.remove(&path) {
            Some(id) => Task::effect(Effect::CloseTab(id)),
            None => Task::none(),
        }
    }

    /// The tabs of a data source had a color in the editor's tab bar: the
    /// tabs of a plugin are grouped under the plugin.
    pub fn assign_db_color(&mut self, _conn: u64) {}

    /// What may come at a place (in characters) of the console's text: where
    /// the replaced fragment starts (in characters), and the items.
    pub fn console_completions(
        &mut self,
        text: &str,
        offset: usize,
    ) -> (usize, Vec<simpleedit_plugin::protocol::CompletionItem>) {
        let Some(conn) = self.db.console.connection else {
            return (offset, Vec::new());
        };
        let known = self.db.known(conn);
        let caret = text
            .char_indices()
            .nth(offset)
            .map_or(text.len(), |(i, _)| i);
        match crate::database::completion::suggest(text, caret, &known, true) {
            Some(found) => (
                text[..found.start].chars().count(),
                found
                    .items
                    .into_iter()
                    .map(|item| simpleedit_plugin::protocol::CompletionItem {
                        label: item.label,
                        insert: Some(item.insert),
                        detail: item.detail,
                    })
                    .collect(),
            ),
            None => (offset, Vec::new()),
        }
    }

    /// The console takes the room under the editor.
    pub fn plugin_bottom_yield(&mut self) {
        self.effects.push(Effect::ShowPanel(crate::CONSOLE.into()));
    }
}
