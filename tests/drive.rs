//! The plugin as the editor runs it, on a SQLite database made for the
//! test: a data source made in its form, a table opened from the tree, a
//! cell typed in the grid and committed, a query run in the console.

#![cfg(unix)]

use std::io::{BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use ardoise_plugin::protocol::{methods, read_message, write_message, Message};
use serde_json::{json, Value};

struct Editor {
    child: Child,
    input: ChildStdin,
    from: Receiver<Message>,
    /// The last render of each panel and tab.
    shown: std::collections::HashMap<String, Value>,
    /// What the editor answers to `panel/editorState`.
    sql: String,
    next: i64,
}

impl Editor {
    fn start(data: &Path) -> Editor {
        let mut child = Command::new(env!("CARGO_BIN_EXE_database"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the plugin starts");
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (to, from) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Ok(Some(message)) = read_message(&mut reader) {
                if to.send(message).is_err() {
                    break;
                }
            }
        });
        let mut editor = Editor {
            child,
            input,
            from,
            shown: Default::default(),
            sql: String::new(),
            next: 1,
        };
        let id = editor.next;
        editor.next += 1;
        editor.send(&Message::request(
            id,
            methods::INITIALIZE,
            json!({
                "api": 1,
                "app_version": "1.0.0",
                "settings": {"locale": "en", "dark": false},
                "workspace": [],
                "plugin_dir": data,
                "data_dir": data,
            }),
        ));
        editor
    }

    fn send(&mut self, message: &Message) {
        write_message(&mut self.input, message).unwrap();
        self.input.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&Message::notification(method, params));
    }

    fn event(&mut self, panel: &str, action: &str, value: Value) {
        let mut params = json!({"panel": panel, "action": action});
        if !value.is_null() {
            params["value"] = value;
        }
        self.notify(methods::UI_EVENT, params);
    }

    /// Reads what the plugin says until `done` holds for the renders kept.
    fn until(&mut self, what: &str, done: impl Fn(&Editor) -> bool) {
        let end = Instant::now() + Duration::from_secs(30);
        while !done(self) {
            let left = end.saturating_duration_since(Instant::now());
            let message = self
                .from
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("waited for {what}; shown: {:?}", self.shown.keys()));
            match message {
                Message::Notification { method, params } if method == methods::RENDER => {
                    let panel = params["panel"].as_str().unwrap_or_default().to_string();
                    self.shown.insert(panel, params["root"].clone());
                }
                Message::Request { id, method, .. } => {
                    let answer = if method == methods::EDITOR_STATE {
                        json!({"text": self.sql, "cursor": {"line": 0, "column": 0}})
                    } else {
                        Value::Null
                    };
                    self.send(&Message::Response {
                        id,
                        result: Ok(answer),
                    });
                }
                _ => {}
            }
        }
    }

    fn shows(&self, panel: &str, needle: &str) -> bool {
        self.shown
            .get(panel)
            .is_some_and(|root| root.to_string().contains(needle))
    }

    /// The action of the button (or the id of the row) labelled so.
    fn action(&self, panel: &str, label: &str) -> String {
        fn find(node: &Value, label: &str) -> Option<String> {
            if node["label"] == label {
                return node["action"]
                    .as_str()
                    .or(node["id"].as_str())
                    .map(str::to_string);
            }
            match node {
                Value::Object(map) => map.values().find_map(|v| find(v, label)),
                Value::Array(list) => list.iter().find_map(|v| find(v, label)),
                _ => None,
            }
        }
        find(&self.shown[panel], label).unwrap_or_else(|| panic!("no {label} in {panel}"))
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn a_sqlite_table_is_opened_edited_and_queried() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let file = dir.path().join("shop.db");
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute_batch(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO items VALUES (1, 'pen'), (2, 'ink');",
    )
    .unwrap();
    drop(db);
    let mut editor = Editor::start(&data);
    editor.until("the tree", |e| e.shows("database", "No database yet."));

    // A data source, in its form.
    let add = editor.action("database", "Add a database");
    editor.event("database", &add, Value::Null);
    editor.until("the form", |e| e.shows("connection", "Database system"));
    editor.event("connection", "dialect", json!("SQLite"));
    editor.until("the file field", |e| e.shows("connection", "\"Database\""));
    editor.event("connection", "Database", json!(file.display().to_string()));
    editor.event("connection", "Name", json!("shop"));
    editor.until("a form that can be saved", |e| {
        e.shows("connection", "shop.db")
            && !e.shows("connection", "\"enabled\":false,\"label\":\"OK\"")
    });
    let save = editor.action("connection", "OK");
    editor.event("connection", &save, Value::Null);
    editor.until("the source in the tree", |e| {
        e.shows("database", "\"shop\"")
    });

    // The source opens on its tables; a click on one opens it.
    let source = editor.action("database", "shop");
    editor.event("database", "tree", json!({"item": source, "kind": "click"}));
    editor.until("the tables", |e| e.shows("database", "\"label\":\"items\""));
    let table = editor.action("database", "items");
    editor.event("database", "tree", json!({"item": table, "kind": "click"}));
    editor.until("the rows", |e| {
        e.shown
            .iter()
            .any(|(id, root)| id.starts_with("table-") && root.to_string().contains("\"pen\""))
    });
    let tab = editor
        .shown
        .keys()
        .find(|id| id.starts_with("table-"))
        .unwrap()
        .clone();

    // A cell typed in, then committed.
    editor.event(&tab, "rows", json!({"kind": "open", "row": 0, "col": 1}));
    editor.until("the cell being typed in", |e| e.shows(&tab, "\"editing\""));
    editor.event(&tab, "rows", json!({"kind": "edit", "value": "pencil"}));
    editor.event(&tab, "rows", json!({"kind": "commit"}));
    editor.until("an edit waiting", |e| e.shows(&tab, "1 pending"));
    let commit = editor.action(&tab, "Commit");
    editor.event(&tab, &commit, Value::Null);
    editor.until("the edit saved", |e| {
        e.shows(&tab, "\"pencil\"") && !e.shows(&tab, "1 pending")
    });
    let db = rusqlite::Connection::open(&file).unwrap();
    let name: String = db
        .query_row("SELECT name FROM items WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "pencil");

    // A query in the console: its rows in a tab of their own.
    let console = editor.action("database", "SQL console");
    editor.event("database", &console, Value::Null);
    editor.until("the console", |e| e.shows("console", "\"sql\""));
    editor.sql = "SELECT name FROM items ORDER BY id".into();
    editor.event("console", "sql", json!(editor.sql.clone()));
    let run = editor.action("console", "Run");
    editor.event("console", &run, Value::Null);
    editor.until("the result", |e| {
        e.shown.iter().any(|(id, root)| {
            id.starts_with("table-") && id != &tab && root.to_string().contains("\"ink\"")
        })
    });
    // The connections are kept (written on a thread of their own).
    let end = Instant::now() + Duration::from_secs(20);
    while !data.join("databases.json").exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    let kept = std::fs::read_to_string(data.join("databases.json")).unwrap();
    assert!(kept.contains("shop.db"));
}
