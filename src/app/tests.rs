//! What the Database tool does, message by message (the tests the editor
//! had for it, moved with it).

use std::path::PathBuf;

use super::{App, Message, Window};
use crate::database::{DbMsg, Done};
use ardoise_plugin::ui::GridEvent;

/// What the grid of a table tab sends.
fn grid(app: &mut App, path: &std::path::Path, event: GridEvent) {
    let _ = app.update(Message::Db(DbMsg::Grid(path.to_path_buf(), event)));
}

/// A move of the cursor of the grid, as the editor sends it.
fn select(app: &mut App, path: &std::path::Path, anchor: (usize, usize), cursor: (usize, usize)) {
    grid(app, path, GridEvent::Select { anchor, cursor });
}

fn app() -> App {
    let mut app = App::new();
    // The connections are read from a file the other tests write too.
    app.db.connections.clear();
    app
}

/// An app with a table open in a tab and a page of it that can be edited.
fn db_app(editable: bool) -> (App, PathBuf) {
    use crate::database::protocol::Column;
    use crate::database::{DbMsg, Dialog, Field, Page};
    let mut app = app();
    // The connections are read from a file the other tests write too.
    app.db.connections.clear();
    let mut form = Dialog::new();
    form.set(Field::User, "dev".into());
    form.set(Field::Password, "secret".into());
    let id = app.db.apply(&form).unwrap();
    let _ = app.update(Message::Db(DbMsg::OpenTable(
        id,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    let path = app.db_tabs.keys().next().unwrap().clone();
    let column = |name: &str, kind: &str, nullable: bool| Column {
        name: name.into(),
        kind: kind.into(),
        nullable,
    };
    let page = Page {
        columns: vec![
            column("id", "integer", false),
            column("label", "text", true),
            column("due", "date", true),
        ],
        rows: (1..=3)
            .map(|i| vec![Some(i.to_string()), Some(format!("row {i}")), None])
            .collect(),
        has_more: false,
        primary_key: vec!["id".into()],
        ctids: Vec::new(),
        truncated: Vec::new(),
        editable,
        source: None,
        offset: 0,
    };
    let _ = app.update(Message::Db(DbMsg::Rows(path.clone(), Done::ok(page))));
    (app, path)
}

#[test]
fn a_table_with_edits_waiting_for_the_commit_is_a_tab_with_changes() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 1, 1)));
    let _ = app.update(Message::Db(DbMsg::EditInput(path.clone(), "x".into())));
    let _ = app.update(Message::Db(DbMsg::EditCommit(path.clone())));
    // What the editor is told of the tab: it asks before quitting.
    assert!(!app.db_tabs[&path].pending.is_empty());
}
#[test]
fn a_cell_is_typed_in_then_waits_for_the_commit() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 1, 1)));
    let tab = &app.db_tabs[&path];
    assert_eq!(tab.editing.as_ref().map(|e| e.text.as_str()), Some("row 2"));
    let _ = app.update(Message::Db(DbMsg::EditInput(
        path.clone(),
        "renamed".into(),
    )));
    let _ = app.update(Message::Db(DbMsg::EditCommit(path.clone())));
    let tab = &app.db_tabs[&path];
    assert!(tab.editing.is_none());
    assert!(tab.pending.is_edited(1, 1));
    assert_eq!(tab.pending.count(), 1);
    // The page itself is not touched until the database says so.
    assert_eq!(
        tab.page.as_ref().unwrap().rows[1][1].as_deref(),
        Some("row 2")
    );
    let _ = app.update(Message::Db(DbMsg::Commit(path.clone())));
    assert!(app.db_tabs[&path].committing);
    // It went well: the page is read again, clean.
    let _ = app.update(Message::Db(DbMsg::Committed(path.clone(), Done::ok(1))));
    let tab = &app.db_tabs[&path];
    assert!(tab.pending.is_empty() && !tab.committing);
}

#[test]
fn a_failed_commit_keeps_the_changes_and_says_why() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::DeleteRow(path.clone(), 0)));
    let _ = app.update(Message::Db(DbMsg::Commit(path.clone())));
    let _ = app.update(Message::Db(DbMsg::Committed(
        path.clone(),
        Done::err("change 1: violates a foreign key"),
    )));
    let tab = &app.db_tabs[&path];
    assert!(!tab.committing);
    assert!(tab.pending.is_deleted(0));
    assert_eq!(
        tab.notice.as_deref(),
        Some("change 1: violates a foreign key")
    );
    // Reverting takes everything back.
    let _ = app.update(Message::Db(DbMsg::RevertAll(path.clone())));
    let tab = &app.db_tabs[&path];
    assert!(tab.pending.is_empty() && tab.notice.is_none());
}

#[test]
fn a_page_cannot_be_left_while_changes_wait() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::DeleteRow(path.clone(), 2)));
    for message in [
        DbMsg::Sort(path.clone(), 1),
        DbMsg::Turn(path.clone(), 1),
        DbMsg::Refresh(path.clone()),
        DbMsg::PageSize(path.clone(), 100),
        DbMsg::ClearFilter(path.clone()),
    ] {
        let _ = app.update(Message::Db(message));
        let tab = &app.db_tabs[&path];
        assert!(tab.pending.is_deleted(2));
        assert!(tab.notice.is_some());
        assert!(tab.order.is_empty() && tab.page_size == 500);
    }
    // With nothing pending, a sort goes through.
    let _ = app.update(Message::Db(DbMsg::RevertAll(path.clone())));
    let _ = app.update(Message::Db(DbMsg::Sort(path.clone(), 1)));
    assert_eq!(app.db_tabs[&path].order, vec![("label".to_string(), false)]);
}

#[test]
fn a_pasted_block_becomes_pending_edits_from_the_selected_cell() {
    let (mut app, path) = db_app(true);
    app.db_tabs.get_mut(&path).unwrap().selected = Some((1, 1));
    let _ = app.update(Message::Db(DbMsg::Paste(
        path.clone(),
        Some("a\t2024-01-02\nb\tc\n".into()),
    )));
    let tab = &app.db_tabs[&path];
    assert_eq!(
        tab.pending.value(tab.page.as_ref().unwrap(), 1, 1),
        Some("a")
    );
    assert_eq!(
        tab.pending.value(tab.page.as_ref().unwrap(), 1, 2),
        Some("2024-01-02")
    );
    assert_eq!(
        tab.pending.value(tab.page.as_ref().unwrap(), 2, 2),
        Some("c")
    );
    assert_eq!(tab.pending.count(), 2);
    // A table that cannot be edited takes nothing.
    let (mut app, path) = db_app(false);
    app.db_tabs.get_mut(&path).unwrap().selected = Some((0, 1));
    let _ = app.update(Message::Db(DbMsg::Paste(path.clone(), Some("a".into()))));
    assert!(app.db_tabs[&path].pending.is_empty());
}

#[test]
fn a_count_is_shown_only_for_the_filter_it_was_made_under() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::Counted(
        path.clone(),
        String::new(),
        Done::ok(42),
    )));
    assert_eq!(app.db_tabs[&path].total, Some((String::new(), 42)));
    let _ = app.update(Message::Db(DbMsg::Counted(
        path.clone(),
        String::new(),
        Done::err("boom"),
    )));
    assert_eq!(app.db_tabs[&path].total, Some((String::new(), 42)));
    assert!(app.db_tabs[&path].notice.is_some());
}

#[test]
fn the_page_size_and_the_filter_are_applied_and_remembered() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::PageSize(path.clone(), 100)));
    assert_eq!(app.db_tabs[&path].page_size, 100);
    assert_eq!(app.db.page_size, 100);
    // The tables opened next start with it.
    let _ = app.update(Message::Db(DbMsg::OpenTable(
        1,
        "shop".into(),
        "public".into(),
        "other".into(),
    )));
    assert!(app
        .db_tabs
        .values()
        .any(|t| t.table == "other" && t.page_size == 100));
    let _ = app.update(Message::Db(DbMsg::Filter(path.clone(), "  id > 1 ".into())));
    // Typed, not yet applied.
    assert_eq!(app.db_tabs[&path].filter, "");
    let _ = app.update(Message::Db(DbMsg::ApplyFilter(path.clone())));
    assert_eq!(app.db_tabs[&path].filter, "id > 1");
    let _ = app.update(Message::Db(DbMsg::ClearFilter(path.clone())));
    assert!(app.db_tabs[&path].filter.is_empty() && app.db_tabs[&path].filter_text.is_empty());
}

#[test]
fn a_cell_gives_the_filter_that_keeps_its_value() {
    let (mut app, path) = db_app(true);
    let name = app.db_tabs[&path].page.as_ref().unwrap().columns[1]
        .name
        .clone();
    let value = {
        let tab = &app.db_tabs[&path];
        tab.pending
            .value(tab.page.as_ref().unwrap(), 0, 1)
            .map(str::to_string)
    };
    let _ = app.update(Message::Db(DbMsg::FilterByCell(path.clone(), 0, 1)));
    let tab = &app.db_tabs[&path];
    let expected = match value {
        Some(v) => format!("= {v}"),
        None => "null".to_string(),
    };
    assert_eq!(tab.column_filters.get(&name), Some(&expected));
    assert!(tab.show_filters);
}

#[test]
fn a_column_is_hidden_and_shown_again_but_never_all_of_them() {
    let (mut app, path) = db_app(true);
    let count = app.db_tabs[&path].page.as_ref().unwrap().columns.len();
    let _ = app.update(Message::Db(DbMsg::HideColumn(path.clone(), 1)));
    assert_eq!(app.db_tabs[&path].hidden.len(), 1);
    for c in 0..count {
        let _ = app.update(Message::Db(DbMsg::HideColumn(path.clone(), c)));
    }
    assert_eq!(app.db_tabs[&path].hidden.len(), count - 1);
    let _ = app.update(Message::Db(DbMsg::ShowColumns(path.clone())));
    assert!(app.db_tabs[&path].hidden.is_empty());
}

#[test]
fn escape_gives_up_the_cell_and_the_date_being_picked() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 0, 1)));
    assert!(app.db_tabs[&path].editing.is_some());
    grid(&mut app, &path, GridEvent::Cancel);
    assert!(app.db_tabs[&path].editing.is_none());
    let _ = app.update(Message::Db(DbMsg::OpenPicker(path.clone(), 0, 2)));
    assert!(app.db_tabs[&path].picker.is_some());
    grid(&mut app, &path, GridEvent::Cancel);
    assert!(app.db_tabs[&path].picker.is_none());
    assert!(app.db_tabs[&path].pending.is_empty());
}

#[test]
fn a_date_is_picked_then_becomes_an_edit_and_null_too() {
    use crate::database::grid::TimePart;
    let (mut app, path) = db_app(true);
    // Not a date column: no picker.
    let _ = app.update(Message::Db(DbMsg::OpenPicker(path.clone(), 0, 1)));
    assert!(app.db_tabs[&path].picker.is_none());
    let _ = app.update(Message::Db(DbMsg::OpenPicker(path.clone(), 0, 2)));
    let _ = app.update(Message::Db(DbMsg::PickerShift(path.clone(), 1)));
    let _ = app.update(Message::Db(DbMsg::PickerDay(path.clone(), 15)));
    // A date has no time to type: this is ignored in the value.
    let _ = app.update(Message::Db(DbMsg::PickerTime(
        path.clone(),
        TimePart::Hour,
        "07".into(),
    )));
    let _ = app.update(Message::Db(DbMsg::PickerApply(path.clone())));
    let tab = &app.db_tabs[&path];
    assert!(tab.picker.is_none());
    let value = tab.pending.value(tab.page.as_ref().unwrap(), 0, 2);
    assert!(value.is_some_and(|v| v.len() == 10 && v.ends_with("-15")));
    // NULL, on a column that takes it and one that does not.
    let _ = app.update(Message::Db(DbMsg::SetNull(path.clone(), 1, 1)));
    assert!(app.db_tabs[&path].pending.is_edited(1, 1));
    let _ = app.update(Message::Db(DbMsg::SetNull(path.clone(), 1, 0)));
    assert!(!app.db_tabs[&path].pending.is_edited(1, 0));
    assert!(app.db_tabs[&path].notice.is_some());
}

#[test]
fn a_click_gives_the_focus_to_a_cell_and_moves_it() {
    let (mut app, path) = db_app(true);
    assert_eq!(app.db_tabs[&path].selected, None);
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 1, 2)));
    assert_eq!(app.db_tabs[&path].selected, Some((1, 2)));
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 0, 0)));
    assert_eq!(app.db_tabs[&path].selected, Some((0, 0)));
    // Double click types in it, and the focus follows.
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 2, 1)));
    assert_eq!(app.db_tabs[&path].selected, Some((2, 1)));
    // A click on another cell keeps what was typed and leaves the cell.
    let _ = app.update(Message::Db(DbMsg::EditInput(path.clone(), "typed".into())));
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 0, 1)));
    let tab = &app.db_tabs[&path];
    assert!(tab.editing.is_none() && tab.pending.is_edited(2, 1));
    // The focus is not kept when the page changes.
    let _ = app.update(Message::Db(DbMsg::RevertAll(path.clone())));
    assert_eq!(app.db_tabs[&path].selected, Some((0, 1)));
}

#[test]
fn the_console_opens_on_the_first_data_source_and_closes() {
    let (mut app, _) = db_app(true);
    assert!(!app.db.console.open);
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    assert!(app.db.console.open);
    // The connection to run on is the first one.
    assert_eq!(
        app.db.console.connection,
        app.db.connections.first().map(|c| c.id)
    );
    // It takes the room under the editor.
    assert!(app
        .effects
        .contains(&crate::task::Effect::ShowPanel(crate::CONSOLE.into())));
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    assert!(!app.db.console.open);
}

#[test]
fn what_the_driver_ran_goes_to_the_log_and_the_end_is_shown() {
    use crate::database::{DbMsg, Executed};
    let (mut app, path) = db_app(true);
    let executed = vec![
        Executed {
            sql: "SELECT * FROM \"public\".\"items\" LIMIT 500 OFFSET 0".into(),
            ms: 3,
            rows: Some(3),
            error: None,
            internal: false,
        },
        Executed {
            sql: "DELETE FROM t WHERE id = '1'".into(),
            ms: 1,
            rows: None,
            error: Some("violates a foreign key".into()),
            internal: false,
        },
    ];
    let page = app.db_tabs[&path].page.clone().unwrap();
    let done = Done {
        result: Ok(page),
        executed: executed.clone(),
    };
    let _ = app.update(Message::Db(DbMsg::Rows(path.clone(), done)));
    let log = &app.db.console.log;
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].sql, executed[0].sql);
    assert_eq!(log[1].error.as_deref(), Some("violates a foreign key"));
    assert_eq!(log[0].connection, "localhost");
    // Clearing empties it.
    let _ = app.update(Message::Db(DbMsg::ClearLog));
    assert!(app.db.console.log.is_empty());
}

#[test]
fn a_select_opens_a_tab_and_the_rest_says_what_it_did() {
    use crate::database::{DbMsg, Executed, QueryResult, Ran};
    let (mut app, _) = db_app(true);
    let conn = app.db.connections[0].id;
    let select = QueryResult {
        sql: "SELECT a, b FROM t".into(),
        ms: 2,
        has_rows: true,
        columns: vec!["a".into(), "b".into()],
        rows: vec![vec![Some("1".into()), None]],
        affected: 1,
        truncated: false,
        deferred: false,
    };
    let update = QueryResult {
        sql: "UPDATE t SET b = 'x'".into(),
        ms: 1,
        has_rows: false,
        columns: vec![],
        rows: vec![],
        affected: 3,
        truncated: false,
        deferred: false,
    };
    let executed = vec![
        Executed {
            sql: select.sql.clone(),
            ms: 2,
            rows: Some(1),
            error: None,
            internal: false,
        },
        Executed {
            sql: update.sql.clone(),
            ms: 1,
            rows: Some(3),
            error: None,
            internal: false,
        },
    ];
    let tabs_before = app.db_tabs.len();
    let _ = app.update(Message::Db(DbMsg::Ran(
        conn,
        Done {
            result: Ok(Ran {
                in_transaction: false,
                results: vec![select, update],
                failed: None,
                position: None,
            }),
            executed,
        },
    )));
    // One tab, for the select, that cannot be edited and knows its query.
    assert_eq!(app.db_tabs.len(), tabs_before + 1);
    let tab = app.db_tabs.values().find(|t| t.query.is_some()).unwrap();
    assert_eq!(tab.query.as_deref(), Some("SELECT a, b FROM t"));
    assert!(!tab.editable());
    assert_eq!(tab.page.as_ref().unwrap().rows.len(), 1);
    // Both are in the log; the last run went well.
    assert_eq!(app.db.console.log.len(), 2);
    assert_eq!(app.db.console.last.as_ref().map(|l| l.0), Some(true));
    assert!(!app.db.console.running);
}

#[test]
fn a_result_that_cannot_be_paged_is_never_run_again() {
    use crate::database::{DbMsg, Executed, QueryResult, Ran};
    let (mut app, _) = db_app(true);
    let conn = app.db.connections[0].id;
    let insert = QueryResult {
        sql: "INSERT INTO t VALUES (1) RETURNING id".into(),
        ms: 1,
        has_rows: true,
        columns: vec!["id".into()],
        rows: vec![vec![Some("1".into())]],
        affected: 0,
        truncated: false,
        deferred: false,
    };
    let _ = app.update(Message::Db(DbMsg::Ran(
        conn,
        Done {
            result: Ok(Ran {
                in_transaction: false,
                results: vec![insert],
                failed: None,
                position: None,
            }),
            executed: Vec::<Executed>::new(),
        },
    )));
    let path = app
        .db_tabs
        .iter()
        .find(|(_, t)| t.query.is_some())
        .map(|(p, _)| p.clone())
        .unwrap();
    // Reload does nothing: the write would be made twice.
    let _ = app.update(Message::Db(DbMsg::Refresh(path.clone())));
    let tab = &app.db_tabs[&path];
    assert!(!tab.loading && tab.page.as_ref().unwrap().rows.len() == 1);
}

#[test]
fn an_open_transaction_is_shown_and_cancelling_ends_the_session() {
    use crate::database::{DbMsg, Ran};
    let (mut app, _) = db_app(true);
    let conn = app.db.connections[0].id;
    app.db.console.connection = Some(conn);
    let _ = app.db.live_of(conn);
    let _ = app.update(Message::Db(DbMsg::Ran(
        conn,
        Done {
            result: Ok(Ran {
                in_transaction: true,
                results: vec![],
                failed: None,
                position: None,
            }),
            executed: vec![],
        },
    )));
    assert!(app.db.console.in_transaction);
    // Another database, another session: the transaction is gone.
    let _ = app.update(Message::Db(DbMsg::ConsoleDatabase("other".into())));
    assert!(!app.db.console.in_transaction);
    // A run that is stopped takes its session with it.
    app.db.console.running = true;
    app.db.console.in_transaction = true;
    let _ = app.update(Message::Db(DbMsg::Cancel));
    assert!(!app.db.console.in_transaction && !app.db.live.contains_key(&conn));
    // Removing a data source ends its session too.
    let _ = app.db.live_of(conn);
    app.db.remove(conn);
    assert!(app.db.live.is_empty());
}

#[test]
fn the_console_runs_only_on_the_connection_chosen() {
    use crate::database::{DbMsg, Dialog, Field};
    let (mut app, _) = db_app(true);
    let first = app.db.connections[0].id;
    let mut form = Dialog::new();
    form.set(Field::User, "staging".into());
    let second = app.db.apply(&form).unwrap();
    let _ = app.update(Message::Db(DbMsg::ConsoleConnection(second)));
    // A transaction open there: the console stays on it.
    app.db.console.in_transaction = true;
    let _ = app.update(Message::Db(DbMsg::ConsoleConnection(first)));
    assert_eq!(app.db.console.connection, Some(second));
    assert_eq!(
        app.db.console.last,
        Some((false, t!("db.console_busy_transaction").to_string()))
    );
    // Nor while a query runs.
    app.db.console.in_transaction = false;
    app.db.console.running = true;
    let _ = app.update(Message::Db(DbMsg::ConsoleConnection(first)));
    assert_eq!(app.db.console.connection, Some(second));
    app.db.console.running = false;
    // Its connection removed, the console runs on none, not on the first.
    app.db.remove(second);
    assert_eq!(app.db.console.connection, None);
    app.db.console.append_query("DELETE FROM t WHERE id = 1");
    let _ = app.update(Message::Db(DbMsg::Run(true)));
    assert!(!app.db.console.running);
    assert_eq!(
        app.db.console.last,
        Some((false, t!("db.console_choose_connection").to_string()))
    );
}

#[test]
fn a_failed_query_shows_the_log_and_says_which_statement() {
    use crate::database::console::ConsoleTab;
    use crate::database::{DbMsg, Executed, Ran};
    let (mut app, _) = db_app(true);
    let conn = app.db.connections[0].id;
    app.db.console.tab = ConsoleTab::Queries;
    let _ = app.update(Message::Db(DbMsg::Ran(
        conn,
        Done {
            result: Ok(Ran {
                in_transaction: false,
                results: vec![],
                failed: Some((1, "syntax error".into())),
                position: None,
            }),
            executed: vec![Executed {
                sql: "SELEKT 1".into(),
                ms: 0,
                rows: None,
                error: Some("syntax error".into()),
                internal: false,
            }],
        },
    )));
    let (ok, message) = app.db.console.last.clone().unwrap();
    assert!(!ok && message.contains('2') && message.contains("syntax error"));
    assert_eq!(app.db.console.tab, ConsoleTab::Log);
    assert_eq!(app.db.console.log.len(), 1);
    // No tab came of it.
    assert!(app.db_tabs.values().all(|t| t.query.is_none()));
}

#[test]
fn running_without_a_connection_or_a_query_says_so() {
    let mut app = app();
    app.db.connections.clear();
    let _ = app.update(Message::Db(DbMsg::Run(true)));
    assert_eq!(app.db.console.last.as_ref().map(|l| l.0), Some(false));
    assert!(!app.db.console.running);
    let (mut app, _) = db_app(true);
    // Opened, the console runs on the first data source.
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    app.db.console.editor = crate::text_editor::Content::with_text("  \n ");
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    assert_eq!(app.db.console.last.as_ref().map(|l| l.0), Some(false));
    assert!(!app.db.console.running);
    // With something written, it runs.
    app.db.console.editor = crate::text_editor::Content::with_text("SELECT 1");
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    assert!(app.db.console.running);
}

#[test]
fn a_risky_statement_waits_for_a_yes_before_it_runs() {
    let (mut app, _) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    app.db.console.editor = crate::text_editor::Content::with_text("DELETE FROM t");
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    assert!(!app.db.console.running);
    assert!(app.db.console.guard.is_some());
    // Giving up leaves nothing waiting.
    let _ = app.update(Message::Db(DbMsg::GuardCancel));
    assert!(app.db.console.guard.is_none() && !app.db.console.running);
    // A yes runs it.
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    let _ = app.update(Message::Db(DbMsg::GuardRun));
    assert!(app.db.console.running && app.db.console.guard.is_none());
}

#[test]
fn on_a_production_source_a_change_with_a_where_waits_too() {
    let (mut app, _) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    app.db.console.editor =
        crate::text_editor::Content::with_text("UPDATE t SET a = 1 WHERE id = 2");
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    assert!(app.db.console.running, "not production: it runs");
    app.db.console.running = false;
    let id = app.db.connections[0].id;
    app.db.connections[0].production = true;
    app.db.console.connection = Some(id);
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    assert!(app.db.console.guard.is_some() && !app.db.console.running);
}

#[test]
fn a_source_that_confirms_shows_every_query_in_a_window_before_it_runs() {
    let (mut app, _) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    let id = app.db.connections[0].id;
    app.db.connections[0].confirm_runs = true;
    app.db.console.connection = Some(id);
    app.db.console.editor = crate::text_editor::Content::with_text("SELECT 1");
    let _ = app.update(Message::Db(DbMsg::Run(false)));
    assert!(!app.db.console.running);
    let guard = app.db.console.guard.clone().expect("the query waits");
    assert_eq!(guard.risk, crate::database::protocol::Risk::Confirm);
    assert_eq!(guard.statements, vec!["SELECT 1".to_string()]);
    assert!(crate::ui::window_titles(&app)
        .iter()
        .any(|(id, _, wanted)| *id == crate::ui::TEXT && *wanted));
    let _ = app.update(Message::Db(DbMsg::GuardRun));
    assert!(app.db.console.running && app.db.console.guard.is_none());
}

#[test]
fn a_query_is_kept_under_a_name_put_back_and_forgotten() {
    let (mut app, _) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    app.db.console.saved.clear();
    app.db.console.editor = crate::text_editor::Content::with_text("SELECT 1");
    let _ = app.update(Message::Db(DbMsg::SavedName("one".into())));
    let _ = app.update(Message::Db(DbMsg::SavedKeep));
    assert_eq!(app.db.console.saved.len(), 1);
    assert_eq!(app.db.console.saved[0].name, "one");
    assert!(app.db.console.save_name.is_empty());
    // The same name is the same query, changed.
    app.db.console.editor = crate::text_editor::Content::with_text("SELECT 2");
    let _ = app.update(Message::Db(DbMsg::SavedName("one".into())));
    let _ = app.update(Message::Db(DbMsg::SavedKeep));
    assert_eq!(app.db.console.saved.len(), 1);
    assert!(app.db.console.saved[0].sql.contains("SELECT 2"));
    // What is kept is on disk for the next time.
    assert_eq!(crate::database::console::load_saved(), app.db.console.saved);
    // Without a name, its first words name it.
    app.db.console.editor = crate::text_editor::Content::with_text("SELECT * FROM t");
    let _ = app.update(Message::Db(DbMsg::SavedKeep));
    assert_eq!(app.db.console.saved[1].name, "SELECT * FROM t");
    // Put back, it is written in the console.
    app.db.console.editor = crate::text_editor::Content::new();
    let _ = app.update(Message::Db(DbMsg::SavedUse(0)));
    assert!(app.db.console.editor.text().contains("SELECT 2"));
    let _ = app.update(Message::Db(DbMsg::SavedDelete(0)));
    assert_eq!(app.db.console.saved.len(), 1);
}

#[test]
fn the_health_window_reads_the_sessions_when_their_tab_is_shown() {
    use crate::database::HealthTab;
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenHealth(id, String::new())));
    assert!(app.db.health.as_ref().unwrap().sessions.is_none());
    let _ = app.update(Message::Db(DbMsg::HealthTab(HealthTab::Sessions)));
    assert_eq!(app.db.health.as_ref().unwrap().tab, HealthTab::Sessions);
    // SQLite has no other session: its window has no such tab.
    assert_eq!(
        HealthTab::of(crate::database::protocol::Dialect::Sqlite),
        &[HealthTab::Tables, HealthTab::Indexes]
    );
    assert!(
        crate::ui::admin::new_role_statement(crate::database::protocol::Dialect::Postgres)
            .starts_with("CREATE ROLE")
    );
    assert_eq!(
        crate::ui::admin::drop_role_statement(crate::database::protocol::Dialect::Postgres, "a b"),
        "DROP ROLE \"a b\";"
    );
    let _ = app.update(Message::Db(DbMsg::HealthTab(HealthTab::Roles)));
    assert_eq!(app.db.health.as_ref().unwrap().tab, HealthTab::Roles);
    assert_eq!(
        crate::ui::admin::stop_statement(crate::database::protocol::Dialect::Mysql, 7, false),
        "KILL QUERY 7"
    );
    assert_eq!(
        crate::ui::admin::stop_statement(crate::database::protocol::Dialect::Postgres, 7, true),
        "SELECT pg_terminate_backend(7)"
    );
}

#[test]
fn a_definition_is_taken_to_the_console_to_be_changed() {
    let (mut app, _) = db_app(true);
    app.db.console.editor = crate::text_editor::Content::new();
    app.db.text = Some(crate::database::TextView {
        title: "public.v".into(),
        text: "CREATE OR REPLACE VIEW v AS SELECT 1;".into(),
        sql: true,
        image: None,
    });
    let _ = app.update(Message::Db(DbMsg::TextToConsole));
    assert!(app.db.text.is_none());
    assert!(app.db.console.open);
    assert!(app
        .db
        .console
        .editor
        .text()
        .contains("CREATE OR REPLACE VIEW v"));
    // A value that is not SQL stays where it is.
    app.db.text = Some(crate::database::TextView {
        title: "note".into(),
        text: "hello".into(),
        sql: false,
        image: None,
    });
    let _ = app.update(Message::Db(DbMsg::TextToConsole));
    assert!(app.db.text.is_some());
}

#[test]
fn a_schema_is_compared_with_another_one_of_the_tree() {
    use crate::database::protocol::{Database, Schema};
    use crate::database::Tree;
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let schema = |name: &str| Schema {
        name: name.into(),
        tables: Vec::new(),
        routines: Vec::new(),
        sequences: Vec::new(),
        types: Vec::new(),
    };
    app.db.trees.insert(
        id,
        Tree::Loaded(vec![Database {
            name: "shop".into(),
            current: true,
            schemas: vec![schema("public"), schema("sales")],
        }]),
    );
    let _ = app.update(Message::Db(DbMsg::CompareOpen(
        id,
        "shop".into(),
        "public".into(),
    )));
    let compare = app.db.compare.as_ref().expect("the question is asked");
    // The schema itself is not offered to be compared with itself.
    assert_eq!(
        compare.choices,
        vec![(id, "shop".to_string(), "sales".to_string())]
    );
    assert_eq!(compare.picked, Some(0));
    assert!(crate::ui::window_titles(&app)
        .iter()
        .any(|(w, _, wanted)| *w == crate::ui::TEXT && *wanted));
    // Two schemas alike: it is said, nothing else shows.
    let _ = app.update(Message::Db(DbMsg::Compared(Ok(String::new()))));
    assert!(app.db.compare.as_ref().unwrap().error.is_some());
    let _ = app.update(Message::Db(DbMsg::Compared(Ok(
        "ALTER TABLE t ADD COLUMN a int;".into(),
    ))));
    assert!(app.db.compare.is_none());
    assert!(app.db.text.as_ref().unwrap().sql);
}

#[test]
fn a_data_source_has_a_colour_and_a_group_that_are_kept() {
    use crate::database::{Choice, Field, Tint};
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenDialog(Some(id))));
    let _ = app.update(Message::Db(DbMsg::Choose(Choice::Tint(Tint::Red))));
    let _ = app.update(Message::Db(DbMsg::Text(Field::Group, " Clients ".into())));
    let draft = &app.db.dialog.as_ref().unwrap().draft;
    assert_eq!((draft.color, draft.group.as_str()), (Tint::Red, "Clients"));
    // The colour shows before the name, and both are written to the file.
    assert!(draft.label().starts_with("🔴 "));
    let text = serde_json::to_string(draft).unwrap();
    let back: crate::database::Connection = serde_json::from_str(&text).unwrap();
    assert_eq!((back.color, back.group.as_str()), (Tint::Red, "Clients"));
    // A file written before has neither.
    let old: crate::database::Connection = serde_json::from_str(r#"{"id": 1}"#).unwrap();
    assert_eq!((old.color, old.group.as_str()), (Tint::None, ""));
}

#[test]
fn a_env_file_fills_the_form_of_a_data_source() {
    let (mut app, _) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::OpenDialog(None)));
    let form = app.db.dialog.as_mut().unwrap();
    assert!(!form.apply_env("APP_ENV=local"));
    assert!(form.apply_env("DATABASE_URL=mysql://root:pw@db.local:3307/shop"));
    assert_eq!(
        form.draft.dialect,
        crate::database::protocol::Dialect::Mysql
    );
    assert_eq!(
        (
            form.draft.host.as_str(),
            form.draft.port,
            form.draft.database.as_str()
        ),
        ("db.local", 3307, "shop")
    );
    assert_eq!(
        (form.draft.user.as_str(), form.password.as_str()),
        ("root", "pw")
    );
    assert_eq!(form.port, "3307");
}

#[test]
fn an_sql_file_runs_on_the_data_source_of_the_console() {
    let (mut app, _) = db_app(true);
    let file = |language: &str| {
        Message::Db(DbMsg::RunFile(Box::new(
            ardoise_plugin::protocol::ActiveFile {
                path: None,
                text: "SELECT 1;\nSELECT 2".into(),
                selection: None,
                language: Some(language.into()),
                lines: None,
            },
        )))
    };
    // Not an SQL file: nothing runs.
    let _ = app.update(file("md"));
    assert!(!app.db.console.running);
    // No data source chosen for the console: none is guessed.
    let _ = app.update(file("sql"));
    assert!(!app.db.console.running && app.db.console.open);
    app.db.console.connection = Some(app.db.connections[0].id);
    let _ = app.update(file("sql"));
    assert!(app.db.console.running && app.db.console.open);
    assert_eq!(app.db.console.running_sql, vec!["SELECT 1", "SELECT 2"]);
}

#[test]
fn shift_extends_a_block_of_cells_that_is_copied_as_a_whole() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 0, 0)));
    assert!(app.db_tabs[&path].block().is_none());
    // Shift and an arrow grow the block from where it started.
    select(&mut app, &path, (0, 0), (1, 0));
    select(&mut app, &path, (0, 0), (1, 1));
    assert_eq!(app.db_tabs[&path].block(), Some(((0, 0), (1, 1))));
    // A move without Shift leaves it.
    select(&mut app, &path, (2, 1), (2, 1));
    assert!(app.db_tabs[&path].block().is_none());
    // Shift and a click do the same.
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 0, 0)));
    app.mod_shift = true;
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 1, 1)));
    assert_eq!(app.db_tabs[&path].block(), Some(((0, 0), (1, 1))));
    app.mod_shift = false;
    let _ = app.update(Message::Db(DbMsg::Select(path.clone(), 1, 1)));
    assert!(app.db_tabs[&path].block().is_none());
}

#[test]
fn a_row_is_edited_in_a_form_that_moves_from_row_to_row() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::OpenForm(path.clone(), 1)));
    assert_eq!(app.db_tabs[&path].form, Some(1));
    // A letter typed, then the field emptied: the cell was not NULL.
    let _ = app.update(Message::Db(DbMsg::FormInput(path.clone(), 1, "new".into())));
    assert!(app.db_tabs[&path].pending.is_edited(1, 1));
    let _ = app.update(Message::Db(DbMsg::FormNull(path.clone(), 1)));
    assert_eq!(
        app.db_tabs[&path]
            .pending
            .cell(app.db_tabs[&path].page.as_ref().unwrap(), 1, 1),
        crate::database::grid::Cell::Null
    );
    // The row before and after, and no further than the rows there are.
    let _ = app.update(Message::Db(DbMsg::FormMove(path.clone(), -1)));
    assert_eq!(app.db_tabs[&path].form, Some(0));
    let _ = app.update(Message::Db(DbMsg::FormMove(path.clone(), -1)));
    assert_eq!(app.db_tabs[&path].form, Some(0));
    let _ = app.update(Message::Db(DbMsg::FormMove(path.clone(), 9)));
    assert_eq!(app.db_tabs[&path].form, Some(2));
    // The lists of values arrive; a failure is no list, not an error.
    let _ = app.update(Message::Db(DbMsg::ChoicesLoaded(
        path.clone(),
        Done::err("nope"),
    )));
    assert_eq!(app.db_tabs[&path].choices, Some(Vec::new()));
    // Escape closes it.
    grid(&mut app, &path, GridEvent::Cancel);
    assert_eq!(app.db_tabs[&path].form, None);
}

#[test]
fn a_form_changes_nothing_in_a_table_that_is_read_only() {
    let (mut app, path) = db_app(false);
    let _ = app.update(Message::Db(DbMsg::OpenForm(path.clone(), 0)));
    let _ = app.update(Message::Db(DbMsg::FormInput(path.clone(), 1, "x".into())));
    let _ = app.update(Message::Db(DbMsg::FormNull(path.clone(), 1)));
    assert!(app.db_tabs[&path].pending.is_empty());
}

#[test]
fn the_health_of_a_database_is_read_and_cleaned_from_its_window() {
    use crate::database::protocol::{Health, IndexHealth};
    use crate::database::{DbMsg, HealthTab};
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenHealth(id, "shop".into())));
    let view = app.db.health.as_ref().expect("the window opens");
    assert!(view.loading && view.data.is_none());
    let data = Health {
        database_size: 1 << 20,
        tables: Vec::new(),
        indexes: vec![IndexHealth {
            schema: "public".into(),
            table: "items".into(),
            name: "items_label_idx".into(),
            size: 8192,
            scans: 0,
            unique: false,
            primary: false,
        }],
    };
    let _ = app.update(Message::Db(DbMsg::HealthLoaded(Done::ok(data))));
    let view = app.db.health.as_ref().unwrap();
    assert!(!view.loading && view.data.is_some() && view.error.is_none());
    let _ = app.update(Message::Db(DbMsg::HealthTab(HealthTab::Indexes)));
    assert_eq!(app.db.health.as_ref().unwrap().tab, HealthTab::Indexes);
    // A clean-up runs one at a time.
    let _ = app.update(Message::Db(DbMsg::HealthRun(
        "VACUUM \"public\".\"items\"".into(),
    )));
    assert!(app.db.health.as_ref().unwrap().running.is_some());
    let _ = app.update(Message::Db(DbMsg::HealthRun(
        "ANALYZE \"public\".\"items\"".into(),
    )));
    assert_eq!(
        app.db.health.as_ref().unwrap().running.as_deref(),
        Some("VACUUM \"public\".\"items\"")
    );
    // A failure is said in the window.
    let _ = app.update(Message::Db(DbMsg::HealthRan(Done::err(
        "permission denied",
    ))));
    let view = app.db.health.as_ref().unwrap();
    assert!(view.running.is_none());
    assert_eq!(view.error.as_deref(), Some("permission denied"));
    // A statement for the console goes there, and the window closes.
    app.db.console.editor = crate::text_editor::Content::new();
    let _ = app.update(Message::Db(DbMsg::HealthToConsole(
        "DROP INDEX \"public\".\"items_label_idx\";".into(),
    )));
    assert!(app.db.health.is_none() && app.db.console.open);
    assert!(app.db.console.editor.text().contains("DROP INDEX"));
    // Escape closes the window.
    let _ = app.update(Message::Db(DbMsg::OpenHealth(id, "shop".into())));
    assert!(app.close_window(Window::Health) && app.db.health.is_none());
}

#[test]
fn a_file_is_read_mapped_and_imported_into_a_table() {
    use crate::database::protocol::{ColumnDef, Structure};
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::ImportOpen(
        id,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    let column = |name: &str, kind: &str, generated: bool| ColumnDef {
        name: name.into(),
        kind: kind.into(),
        nullable: true,
        default: None,
        identity: String::new(),
        generated,
        comment: String::new(),
    };
    let structure = Structure {
        columns: vec![
            column("id", "integer", false),
            column("label", "text", false),
            column("total", "integer", true),
        ],
        ..Structure::default()
    };
    let _ = app.update(Message::Db(DbMsg::ImportStructure(Done::ok(structure))));
    // A generated column cannot be filled.
    assert_eq!(app.db.import.as_ref().unwrap().targets.len(), 2);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rows.csv");
    std::fs::write(&file, "ID;Label;other\n1;a;x\n2;b;y\n").unwrap();
    let _ = app.update(Message::Db(DbMsg::ImportPicked(Some(file))));
    let im = app.db.import.as_ref().unwrap();
    assert_eq!(im.delimiter, ';');
    assert_eq!(im.sample.names, vec!["ID", "Label", "other"]);
    assert_eq!(
        im.mapping,
        vec![Some("id".to_string()), Some("label".to_string()), None]
    );
    // A column is filled by one field only.
    let _ = app.update(Message::Db(DbMsg::ImportMap(2, Some("id".into()))));
    let im = app.db.import.as_ref().unwrap();
    assert_eq!(im.mapping[0], None);
    assert_eq!(im.mapping[2].as_deref(), Some("id"));
    // Without a header, the fields are numbered and go by place.
    let _ = app.update(Message::Db(DbMsg::ImportHeader(false)));
    let im = app.db.import.as_ref().unwrap();
    assert_eq!(im.sample.names, vec!["1", "2", "3"]);
    assert_eq!(im.sample.rows.len(), 3);
    let _ = app.update(Message::Db(DbMsg::ImportHeader(true)));
    // Going: one import at a time, and what comes back is said.
    let _ = app.update(Message::Db(DbMsg::ImportRun));
    assert!(app.db.import.as_ref().unwrap().running);
    assert!(
        !app.close_window(Window::Import),
        "the window stays while it runs"
    );
    let _ = app.update(Message::Db(DbMsg::ImportDone(Done::err("bad value"))));
    let im = app.db.import.as_ref().unwrap();
    assert!(!im.running && im.error.as_deref() == Some("bad value"));
    let _ = app.update(Message::Db(DbMsg::ImportRun));
    let _ = app.update(Message::Db(DbMsg::ImportDone(Done::ok(2))));
    assert_eq!(app.db.import.as_ref().unwrap().imported, Some(2));
    assert!(app.close_window(Window::Import) && app.db.import.is_none());
}

#[test]
fn a_table_is_copied_to_another_source_from_its_window() {
    use crate::database::{Copied, CopyField, DbMsg};
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::CopyOpen(
        id,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    let copy = app.db.copy_table.as_ref().expect("the window opens");
    assert_eq!(
        (copy.name.as_str(), copy.schema.as_str()),
        ("items", "public")
    );
    assert!(copy.with_data && copy.dest == Some(id));
    // Nothing goes without a name.
    let _ = app.update(Message::Db(DbMsg::CopyField(CopyField::Name, "  ".into())));
    let _ = app.update(Message::Db(DbMsg::CopyRun));
    assert!(!app.db.copy_table.as_ref().unwrap().running);
    let _ = app.update(Message::Db(DbMsg::CopyField(
        CopyField::Name,
        "items2".into(),
    )));
    let _ = app.update(Message::Db(DbMsg::CopyData(false)));
    let _ = app.update(Message::Db(DbMsg::CopyRun));
    assert!(app.db.copy_table.as_ref().unwrap().running);
    // It cannot be closed, or changed, while it runs.
    assert!(!app.close_window(Window::Copy));
    let _ = app.update(Message::Db(DbMsg::CopyField(
        CopyField::Name,
        "other".into(),
    )));
    assert_eq!(app.db.copy_table.as_ref().unwrap().name, "items2");
    let failed = Copied {
        result: Err("relation already exists".into()),
        source_log: Vec::new(),
        dest_log: Vec::new(),
    };
    let _ = app.update(Message::Db(DbMsg::CopyDone(Box::new(failed))));
    let copy = app.db.copy_table.as_ref().unwrap();
    assert!(!copy.running && copy.error.as_deref() == Some("relation already exists"));
    let _ = app.update(Message::Db(DbMsg::CopyRun));
    let done = Copied {
        result: Ok(0),
        source_log: Vec::new(),
        dest_log: Vec::new(),
    };
    let _ = app.update(Message::Db(DbMsg::CopyDone(Box::new(done))));
    assert_eq!(app.db.copy_table.as_ref().unwrap().copied, Some(0));
    assert!(app.close_window(Window::Copy) && app.db.copy_table.is_none());
}

#[test]
fn a_backup_and_a_restore_are_asked_for_in_one_window() {
    use crate::database::{BackupFlag, DbMsg};
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::BackupOpen(
        id,
        "shop".into(),
        "public".into(),
        "items".into(),
        false,
    )));
    let b = app.db.backup.as_ref().expect("the window opens");
    assert!(!b.restore && b.spec.format == "custom" && b.spec.no_owner);
    // Nothing goes before a file is chosen.
    let _ = app.update(Message::Db(DbMsg::BackupRun));
    assert!(!app.db.backup.as_ref().unwrap().running);
    // Structure only and data only exclude each other.
    let _ = app.update(Message::Db(DbMsg::BackupFlag(BackupFlag::SchemaOnly, true)));
    let _ = app.update(Message::Db(DbMsg::BackupFlag(BackupFlag::DataOnly, true)));
    let spec = &app.db.backup.as_ref().unwrap().spec;
    assert!(spec.data_only && !spec.schema_only);
    let _ = app.update(Message::Db(DbMsg::BackupFlag(BackupFlag::Plain, true)));
    assert_eq!(app.db.backup.as_ref().unwrap().spec.format, "plain");
    let _ = app.update(Message::Db(DbMsg::BackupPicked(Some("/tmp/x.sql".into()))));
    let _ = app.update(Message::Db(DbMsg::BackupRun));
    assert!(app.db.backup.as_ref().unwrap().running);
    assert!(
        !app.close_window(Window::Backup),
        "the window stays while it works"
    );
    let _ = app.update(Message::Db(DbMsg::BackupDone(Done::err(
        "pg_dump was not found",
    ))));
    let b = app.db.backup.as_ref().unwrap();
    assert!(!b.running && b.error.as_deref() == Some("pg_dump was not found"));
    let _ = app.update(Message::Db(DbMsg::BackupRun));
    let _ = app.update(Message::Db(DbMsg::BackupDone(Done::ok("4.2 KB".into()))));
    assert_eq!(
        app.db.backup.as_ref().unwrap().done.as_deref(),
        Some("4.2 KB")
    );
    assert!(app.close_window(Window::Backup) && app.db.backup.is_none());
    // A restore reads its backup in one transaction by default.
    let _ = app.update(Message::Db(DbMsg::BackupOpen(
        id,
        "shop".into(),
        String::new(),
        String::new(),
        true,
    )));
    let b = app.db.backup.as_ref().unwrap();
    assert!(b.restore && b.spec.single_transaction && !b.spec.clean);
}

#[test]
fn the_linked_rows_of_a_row_are_listed_in_a_window_that_closes_when_one_is_opened() {
    use crate::database::ddl::Link;
    use crate::database::{DbMsg, LinksView};
    let (mut app, _) = db_app(true);
    let id = app.db.connections[0].id;
    let view = LinksView {
        conn: id,
        database: "shop".into(),
        schema: "public".into(),
        links: vec![Link {
            outgoing: false,
            schema: "public".into(),
            table: "orders".into(),
            filter: "\"customer_id\" = '1'".into(),
            label: "\"customer_id\" = '1'".into(),
        }],
    };
    let _ = app.update(Message::Db(DbMsg::ShowLinks(Box::new(view.clone()))));
    assert_eq!(app.db.links.as_ref().map(|v| v.links.len()), Some(1));
    // Escape closes it.
    assert!(app.close_window(Window::Text) && app.db.links.is_none());
    // Opening one of the tables closes it too.
    let _ = app.update(Message::Db(DbMsg::ShowLinks(Box::new(view))));
    let _ = app.update(Message::Db(DbMsg::OpenRelated(
        id,
        "shop".into(),
        "public".into(),
        "orders".into(),
        "\"customer_id\" = '1'".into(),
    )));
    assert!(app.db.links.is_none());
}

#[test]
fn the_cursor_moves_from_cell_to_cell_and_a_key_types_in_a_cell_or_deletes_its_row() {
    let (mut app, path) = db_app(true);
    select(&mut app, &path, (1, 2), (1, 2));
    assert_eq!(app.db_tabs[&path].selected, Some((1, 2)));
    // F2 types in the cell.
    grid(&mut app, &path, GridEvent::Open { row: 0, col: 0 });
    assert!(app.db_tabs[&path].editing.is_some());
    grid(&mut app, &path, GridEvent::Cancel);
    // Delete marks the row of the cursor.
    select(&mut app, &path, (0, 0), (0, 0));
    grid(&mut app, &path, GridEvent::Delete);
    assert!(app.db_tabs[&path].pending.is_deleted(0));
}
#[test]
fn a_copy_takes_the_cell_of_the_cursor_or_the_block() {
    let (mut app, path) = db_app(true);
    select(&mut app, &path, (1, 0), (1, 0));
    let task = app.update(Message::Db(DbMsg::Grid(path.clone(), GridEvent::Copy)));
    let (mut effects, mut futures) = (Vec::new(), Vec::new());
    task.split(&mut effects, &mut futures);
    assert_eq!(effects, vec![crate::task::Effect::Copy("2".into())]);
    select(&mut app, &path, (0, 0), (1, 1));
    let task = app.update(Message::Db(DbMsg::Grid(path.clone(), GridEvent::Copy)));
    let (mut effects, mut futures) = (Vec::new(), Vec::new());
    task.split(&mut effects, &mut futures);
    assert_eq!(
        effects,
        vec![crate::task::Effect::Copy("1\trow 1\n2\trow 2".into())]
    );
}

#[test]
fn delete_goes_down_the_table_and_needs_a_selected_cell() {
    let (mut app, path) = db_app(true);
    // Nothing selected: nothing is deleted.
    grid(&mut app, &path, GridEvent::Delete);
    assert!(app.db_tabs[&path].pending.is_empty());
    // Each Delete marks the selected row and moves to the next one.
    select(&mut app, &path, (1, 1), (1, 1));
    grid(&mut app, &path, GridEvent::Delete);
    assert!(app.db_tabs[&path].pending.is_deleted(1));
    assert_eq!(app.db_tabs[&path].selected, Some((2, 1)));
    grid(&mut app, &path, GridEvent::Delete);
    assert!(app.db_tabs[&path].pending.is_deleted(2));
    // The last row stays selected; row 0 is never touched.
    assert_eq!(app.db_tabs[&path].selected, Some((2, 1)));
    assert!(!app.db_tabs[&path].pending.is_deleted(0));
}
#[test]
fn a_long_cell_is_edited_from_its_whole_value() {
    let (mut app, path) = db_app(true);
    let long = "x".repeat(10_000);
    app.db_tabs
        .get_mut(&path)
        .unwrap()
        .page
        .as_mut()
        .unwrap()
        .rows[0][1] = Some(long.clone());
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 0, 1)));
    let tab = &app.db_tabs[&path];
    assert_eq!(
        tab.editing.as_ref().map(|e| e.text.as_str()),
        Some(long.as_str())
    );
}

#[test]
fn a_reading_query_opens_a_paged_tab_and_one_from_a_table_can_be_edited() {
    use crate::database::protocol::Source;
    use crate::database::{DbMsg, Executed, QueryResult, Ran};
    let (mut app, _) = db_app(true);
    let conn = app.db.connections[0].id;
    let select = QueryResult {
        sql: "SELECT id, label FROM items".into(),
        ms: 0,
        has_rows: true,
        columns: vec!["id".into(), "label".into()],
        rows: vec![],
        affected: 0,
        truncated: false,
        deferred: true,
    };
    let _ = app.update(Message::Db(DbMsg::Ran(
        conn,
        Done {
            result: Ok(Ran {
                in_transaction: false,
                results: vec![select],
                failed: None,
                position: None,
            }),
            executed: Vec::<Executed>::new(),
        },
    )));
    let path = app
        .db_tabs
        .iter()
        .find(|(_, t)| t.query.is_some())
        .map(|(p, _)| p.clone())
        .unwrap();
    let tab = &app.db_tabs[&path];
    assert!(tab.paged && tab.loading);
    // Its page asks for the query, sorted and filtered like a table.
    let q = tab.query(0);
    assert_eq!(q.sql.as_deref(), Some("SELECT id, label FROM items"));
    assert!(!tab.is_static_result());
    // The page that comes tells where the rows are from: they can be edited there.
    let mut page = app
        .db_tabs
        .values()
        .find(|t| t.table == "items")
        .unwrap()
        .page
        .clone()
        .unwrap();
    page.source = Some(Source {
        schema: "public".into(),
        table: "items".into(),
    });
    let _ = app.update(Message::Db(DbMsg::Rows(path.clone(), Done::ok(page))));
    let tab = &app.db_tabs[&path];
    assert!(tab.editable());
    assert_eq!(tab.table_ref().table, "items");
    assert!(tab.title.starts_with(&t!("db.result").to_string()));
    // A sort by two columns, stacked with Alt.
    app.mod_alt = true;
    let _ = app.update(Message::Db(DbMsg::Sort(path.clone(), 1)));
    let _ = app.update(Message::Db(DbMsg::Sort(path.clone(), 2)));
    assert_eq!(app.db_tabs[&path].order.len(), 2);
}

#[test]
fn the_console_proposes_tables_then_columns() {
    use crate::database::protocol::{Database, Kind, Schema, Table, TableColumn};
    use crate::database::Tree;
    let (mut app, _) = db_app(true);
    let conn = app.db.connections[0].id;
    app.db.console.connection = Some(conn);
    app.db.trees.insert(
        conn,
        Tree::Loaded(vec![Database {
            name: "shop".into(),
            current: true,
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "items".into(),
                    kind: Kind::Table,
                }],
                ..Default::default()
            }],
        }]),
    );
    let column = |name: &str, kind: &str| TableColumn {
        schema: "public".into(),
        table: "items".into(),
        name: name.into(),
        kind: kind.into(),
    };
    app.db
        .columns
        .insert(conn, vec![column("id", "integer"), column("label", "text")]);
    // After FROM: the table comes first, in place of what was typed.
    let (from, items) = app.console_completions("SELECT * FROM ite", 17);
    assert_eq!(items[0].label, "items");
    assert_eq!(items[0].insert.as_deref(), Some("items"));
    assert_eq!(from, 14);
    // Then the columns of the table, after a dot and an alias.
    let text = "SELECT i.la FROM items i";
    let (from, items) = app.console_completions(text, 11);
    assert_eq!(items[0].label, "label");
    assert_eq!(from, 9);
    // Asked with nothing typed, there are proposals too.
    let (_, items) = app.console_completions("SELECT ", 7);
    assert!(!items.is_empty());
    // No data source for the console: nothing is proposed.
    app.db.console.connection = None;
    assert!(app.console_completions("SELECT ", 7).1.is_empty());
}

#[test]
fn a_row_is_added_filled_in_and_waits_for_the_commit() {
    use crate::database::grid::Cell;
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::AddRow(path.clone())));
    let tab = &app.db_tabs[&path];
    // It comes after the three rows, and the first column that is not the
    // key is typed in.
    assert_eq!(tab.pending.inserts.len(), 1);
    assert_eq!(tab.selected, Some((3, 1)));
    assert_eq!(tab.editing.as_ref().map(|e| (e.row, e.col)), Some((3, 1)));
    let _ = app.update(Message::Db(DbMsg::EditInput(path.clone(), "fresh".into())));
    let _ = app.update(Message::Db(DbMsg::EditCommit(path.clone())));
    let tab = &app.db_tabs[&path];
    let page = tab.page.as_ref().unwrap();
    assert_eq!(tab.pending.cell(page, 3, 1), Cell::Value("fresh"));
    assert_eq!(tab.pending.cell(page, 3, 0), Cell::Default);
    // A second one, then NULL in a cell of it and a date by the picker.
    let _ = app.update(Message::Db(DbMsg::AddRow(path.clone())));
    let _ = app.update(Message::Db(DbMsg::SetNull(path.clone(), 4, 1)));
    let _ = app.update(Message::Db(DbMsg::OpenPicker(path.clone(), 4, 2)));
    let _ = app.update(Message::Db(DbMsg::PickerDay(path.clone(), 9)));
    let _ = app.update(Message::Db(DbMsg::PickerApply(path.clone())));
    let tab = &app.db_tabs[&path];
    let page = tab.page.as_ref().unwrap();
    assert_eq!(tab.pending.cell(page, 4, 1), Cell::Null);
    assert!(tab
        .pending
        .value(page, 4, 2)
        .is_some_and(|d| d.ends_with("-09")));
    assert_eq!(tab.pending.count(), 2);
    // The commit sends both, as inserts.
    let changes = tab.pending.changes(page);
    assert_eq!(changes.len(), 2);
    assert!(changes
        .iter()
        .all(|c| c.kind == crate::database::protocol::ChangeKind::Insert));
    let _ = app.update(Message::Db(DbMsg::Commit(path.clone())));
    assert!(app.db_tabs[&path].committing);
    // A page cannot be left meanwhile.
    let _ = app.update(Message::Db(DbMsg::Committed(
        path.clone(),
        Done::err("change 2: null value in column violates not-null"),
    )));
    let tab = &app.db_tabs[&path];
    assert_eq!(tab.pending.inserts.len(), 2);
    assert!(tab.notice.is_some());
    let _ = app.update(Message::Db(DbMsg::Turn(path.clone(), 1)));
    assert_eq!(app.db_tabs[&path].offset(), 0);
}

#[test]
fn a_new_row_can_be_removed_and_reverted_and_walked_to_with_the_keys() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::AddRow(path.clone())));
    grid(&mut app, &path, GridEvent::Cancel);
    // The cursor goes down onto the new row.
    select(&mut app, &path, (3, 0), (3, 0));
    assert_eq!(app.db_tabs[&path].selected.map(|s| s.0), Some(3));
    // Delete takes the new row out: nothing waits.
    grid(&mut app, &path, GridEvent::Delete);
    assert!(app.db_tabs[&path].pending.is_empty());
    // With a table that cannot be edited, no row is added.
    let (mut app, path) = db_app(false);
    let _ = app.update(Message::Db(DbMsg::AddRow(path.clone())));
    assert!(app.db_tabs[&path].pending.is_empty());
}

#[test]
fn an_entry_of_the_log_is_copied_whole_or_in_parts() {
    use crate::database::console::ConsoleTab;
    use crate::database::{DbMsg, Executed, LogCopy};
    let mut app = app();
    app.db.console.log.clear();
    app.db.console.push(
        "shop",
        &[
            Executed {
                sql: "SELECT 1".into(),
                ms: 1,
                rows: Some(1),
                error: None,
                internal: false,
            },
            Executed {
                sql: "DELETE FROM t".into(),
                ms: 2,
                rows: None,
                error: Some("boom".into()),
                internal: false,
            },
        ],
    );
    app.db.console.open = true;
    app.db.console.tab = ConsoleTab::Log;
    // Each part of an entry is copied by its button.
    for (what, said) in [(LogCopy::Sql, "DELETE FROM t"), (LogCopy::Error, "boom")] {
        let task = app.update(Message::Db(DbMsg::CopyLog(1, what)));
        let (mut effects, mut futures) = (Vec::new(), Vec::new());
        task.split(&mut effects, &mut futures);
        assert_eq!(effects, vec![crate::task::Effect::Copy(said.into())]);
    }
    let _ = app.update(Message::Db(DbMsg::CopyLog(1, LogCopy::Entry)));
    // Clearing forgets the log.
    let _ = app.update(Message::Db(DbMsg::ClearLog));
    assert!(app.db.console.log.is_empty());
}

/// What a successful operation on a table gave.
fn altered(
    what: crate::database::AlterKind,
    conn: u64,
    result: Result<(), String>,
) -> crate::database::Altered {
    use crate::database::Ran;
    let done = match result {
        Ok(()) => Done::ok(Ran {
            in_transaction: false,
            results: vec![],
            failed: None,
            position: None,
        }),
        Err(e) => Done::ok(Ran {
            in_transaction: false,
            results: vec![],
            failed: Some((0, e)),
            position: None,
        }),
    };
    crate::database::Altered {
        what,
        conn,
        database: "shop".into(),
        schema: "public".into(),
        table: "items".into(),
        done,
    }
}

#[test]
fn a_drop_asks_first_then_closes_the_tabs_of_the_table_and_says_when_it_fails() {
    use crate::database::{AlterKind, ConfirmAction, DbMsg, Kind};
    let (mut app, path) = db_app(true);
    let conn = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::AskDrop(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
        Kind::Table,
    )));
    let confirm = app.db.confirm.as_ref().unwrap();
    assert_eq!(confirm.action, ConfirmAction::Drop);
    assert_eq!(confirm.statement(), "DROP TABLE \"public\".\"items\"");
    // Nothing is run before the yes; a view is dropped as a view.
    assert!(!confirm.running);
    let _ = app.update(Message::Db(DbMsg::ConfirmCascade(true)));
    assert!(app
        .db
        .confirm
        .as_ref()
        .unwrap()
        .statement()
        .ends_with("CASCADE"));
    let _ = app.update(Message::Db(DbMsg::ConfirmRun));
    assert!(app.db.confirm.as_ref().unwrap().running);
    // A refusal of the database stays on the question.
    let _ = app.update(Message::Db(DbMsg::Altered(Box::new(altered(
        AlterKind::Drop,
        conn,
        Err("cannot drop: other objects depend on it".into()),
    )))));
    let confirm = app.db.confirm.as_ref().unwrap();
    assert!(!confirm.running);
    assert_eq!(
        confirm.error.as_deref(),
        Some("cannot drop: other objects depend on it")
    );
    assert!(app.db_tabs.contains_key(&path));
    // It went: the question goes, and so do the tabs of the table.
    let _ = app.update(Message::Db(DbMsg::ConfirmRun));
    let _ = app.update(Message::Db(DbMsg::Altered(Box::new(altered(
        AlterKind::Drop,
        conn,
        Ok(()),
    )))));
    assert!(app.db.confirm.is_none());
    assert!(!app.db_tabs.contains_key(&path));
    let view = Kind::View;
    let _ = app.update(Message::Db(DbMsg::AskDrop(
        conn,
        "shop".into(),
        "public".into(),
        "v".into(),
        view,
    )));
    assert!(app
        .db
        .confirm
        .as_ref()
        .unwrap()
        .statement()
        .starts_with("DROP VIEW"));
    // Closing its tab gives the question up.
    assert!(app.close_window(Window::Text));
    assert!(app.db.confirm.is_none());
}

#[test]
fn deleting_all_the_rows_asks_first_and_keeps_the_tabs_which_read_again() {
    use crate::database::{AlterKind, DbMsg};
    let (mut app, path) = db_app(true);
    let conn = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::AskEmpty(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    assert_eq!(
        app.db.confirm.as_ref().unwrap().statement(),
        "DELETE FROM \"public\".\"items\""
    );
    let _ = app.update(Message::Db(DbMsg::ConfirmRun));
    let _ = app.update(Message::Db(DbMsg::Altered(Box::new(altered(
        AlterKind::Empty,
        conn,
        Ok(()),
    )))));
    assert!(app.db.confirm.is_none());
    // The table is still there, being read again.
    assert!(app.db_tabs[&path].loading);
}

#[test]
fn a_data_source_is_deleted_on_the_second_click() {
    let (mut app, _path) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenDialog(Some(id))));
    let _ = app.update(Message::Db(DbMsg::Delete));
    assert!(app.db.connection(id).is_some());
    assert!(app.db.dialog.as_ref().unwrap().confirm_delete);
    let _ = app.update(Message::Db(DbMsg::Delete));
    assert!(app.db.connection(id).is_none());
    assert!(app.db.dialog.is_none());
}

/// The nodes of a tree of JSON whose `type` is `kind`.
fn nodes<'a>(value: &'a serde_json::Value, kind: &str, out: &mut Vec<&'a serde_json::Value>) {
    if value["type"] == kind {
        out.push(value);
    }
    match value {
        serde_json::Value::Object(map) => map.values().for_each(|v| nodes(v, kind, out)),
        serde_json::Value::Array(list) => list.iter().for_each(|v| nodes(v, kind, out)),
        _ => {}
    }
}

/// The pictos of the buttons of a render, as `(icon, tooltip)`.
fn pictos(node: &ardoise_plugin::ui::Node) -> Vec<(String, String)> {
    let json = serde_json::to_value(node).unwrap();
    let mut buttons = Vec::new();
    nodes(&json, "button", &mut buttons);
    buttons
        .iter()
        .filter_map(|b| {
            Some((
                b["icon"].as_str()?.to_string(),
                b["label"].as_str()?.to_string(),
            ))
        })
        .collect()
}

#[test]
fn the_header_of_the_panel_has_four_pictos_and_the_tree_folds_and_reloads() {
    use crate::ui::{tree, Ui};
    let (mut app, _path) = db_app(true);
    let id = app.db.connections[0].id;
    let mut ui = Ui::new(true);
    let node = tree::view(&mut ui, &app);
    let found = pictos(&node);
    let header: Vec<&str> = found.iter().take(4).map(|(i, _)| i.as_str()).collect();
    assert_eq!(
        header,
        ["plus", "terminal", "refresh-cw", "chevrons-up-down"]
    );
    // The tooltips are the labels they replaced (and the new one).
    assert_eq!(found[0].1, t!("db.add"));
    assert_eq!(found[1].1, t!("db.console"));
    assert_eq!(found[2].1, t!("db.refresh"));
    assert_eq!(found[3].1, t!("db.collapse_all"));
    // The add button is the one of the accent.
    let json = serde_json::to_value(&node).unwrap();
    let mut buttons = Vec::new();
    nodes(&json, "button", &mut buttons);
    assert_eq!(buttons[0]["style"], "primary");
    // Fold: every node closes.
    app.db.open.insert(id.to_string());
    app.db.open.insert(format!("{id}/shop"));
    let _ = app.update(Message::Db(DbMsg::CollapseAll));
    assert!(app.db.open.is_empty());
    // Reload: only what is open.
    let _ = app.update(Message::Db(DbMsg::ReloadAll));
    assert!(app.db.open.is_empty(), "a closed source is left alone");
    app.db.open.insert(id.to_string());
    let _ = app.update(Message::Db(DbMsg::ReloadAll));
    assert!(matches!(
        app.db.trees.get(&id),
        Some(crate::database::Tree::Loading)
    ));
}

#[test]
fn the_empty_panel_keeps_its_big_button_with_the_text() {
    use crate::ui::{tree, Ui};
    let app = app();
    let mut ui = Ui::new(true);
    let json = serde_json::to_value(tree::view(&mut ui, &app)).unwrap();
    let mut buttons = Vec::new();
    nodes(&json, "button", &mut buttons);
    let big = buttons
        .iter()
        .find(|b| b["style"] == "primary" && b.get("icon").is_none())
        .expect("the main button, with its text");
    assert_eq!(big["label"], t!("db.add").to_string());
}

#[test]
fn the_connection_form_has_tabs_tiles_and_a_footer_with_save_last() {
    use crate::database::{Dialog, DialogTab};
    use crate::ui::{dialog, Ui};
    let mut app = app();
    app.db.dialog = Some(Dialog::new());
    let mut ui = Ui::new(true);
    let node = dialog::view(&mut ui, &app.db, app.db.dialog.as_ref().unwrap());
    let json = serde_json::to_value(&node).unwrap();
    let children = json["children"].as_array().unwrap();
    // The tabs first (all six), the footer last.
    assert_eq!(children[0]["type"], "tabs");
    assert_eq!(
        children[0]["items"].as_array().unwrap().len(),
        DialogTab::ALL.len()
    );
    assert_eq!(children.last().unwrap()["type"], "footer");
    // A tile for each kind of data source.
    let mut tiles = Vec::new();
    nodes(&json, "tiles", &mut tiles);
    assert_eq!(tiles[0]["items"].as_array().unwrap().len(), 4);
    // The footer: the test as a picto at the left; Cancel then Save at the
    // right; no Delete for a new source.
    let footer = children.last().unwrap();
    assert_eq!(footer["start"][0]["icon"], "zap");
    assert_eq!(footer["start"][0]["label"], t!("db.test").to_string());
    let end: Vec<&str> = footer["end"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        end,
        [t!("db.cancel_dialog").as_ref(), t!("db.save").as_ref()]
    );
    assert_eq!(footer["end"][1]["style"], "primary");
}

#[test]
fn the_form_of_a_source_that_exists_has_delete_in_red_and_duplicate_as_a_picto() {
    use crate::ui::{dialog, Ui};
    let (mut app, _path) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenDialog(Some(id))));
    let mut ui = Ui::new(true);
    let node = dialog::view(&mut ui, &app.db, app.db.dialog.as_ref().unwrap());
    let json = serde_json::to_value(&node).unwrap();
    let footer = json["children"].as_array().unwrap().last().unwrap().clone();
    let icons: Vec<&str> = footer["start"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["icon"].as_str().unwrap())
        .collect();
    assert_eq!(icons, ["zap", "copy"]);
    let delete = &footer["end"][1];
    assert_eq!(delete["style"], "danger");
    assert_eq!(delete["label"], t!("db.delete").to_string());
}

#[test]
fn closing_the_text_window_answers_one_question_at_a_time() {
    use crate::app::Window;
    let (mut app, _path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::AskEmpty(
        app.db.connections[0].id,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    assert!(app.db.confirm.is_some());
    app.db.new_schema = Some(crate::database::NewSchema {
        conn: app.db.connections[0].id,
        database: "shop".into(),
        name: String::new(),
        running: false,
        error: None,
    });
    // The first (the question) closes; the next one shows.
    assert!(app.close_window(Window::Text));
    assert!(app.db.confirm.is_none() && app.db.new_schema.is_some());
    assert!(app.close_window(Window::Text));
    assert!(app.db.new_schema.is_none());
    assert!(!app.close_window(Window::Text));
}

/// The buttons of the console that are on, by label, in what it shows.
fn console_buttons_on(app: &App) -> Vec<String> {
    use crate::ui::{console, Ui};
    let json = serde_json::to_value(console::view(&mut Ui::new(true), app)).unwrap();
    let mut buttons = Vec::new();
    nodes(&json, "button", &mut buttons);
    buttons
        .iter()
        .filter(|b| b.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true))
        .filter_map(|b| b["label"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn the_buttons_of_the_console_wake_up_with_a_query_to_run() {
    let (mut app, _path) = db_app(true);
    app.db.set_driver(
        crate::database::protocol::Dialect::Postgres,
        crate::database::Driver::Ready,
    );
    let _ = app.update(Message::Db(DbMsg::ToggleConsole));
    assert!(app.db.console.connection.is_some());
    let runs = [
        t!("db.run").to_string(),
        t!("db.format").to_string(),
        t!("db.explain").to_string(),
        t!("db.explain_analyze").to_string(),
        t!("db.run_all").to_string(),
    ];
    // An empty editor (what other tests saved is not read back here):
    // nothing to run, format or explain.
    let _ = app.update(Message::Db(DbMsg::SqlText(String::new())));
    let on = console_buttons_on(&app);
    for label in &runs {
        assert!(!on.contains(label), "{label} is off with nothing typed");
    }
    // Something typed: they are on.
    let _ = app.update(Message::Db(DbMsg::SqlText("select 1".into())));
    let on = console_buttons_on(&app);
    for label in &runs {
        assert!(on.contains(label), "{label} is on with a query");
    }
    // Blanks are nothing.
    let _ = app.update(Message::Db(DbMsg::SqlText("  \n ".into())));
    assert!(!console_buttons_on(&app).contains(&runs[0]));
}

#[test]
fn each_window_has_the_picto_and_the_line_of_its_header() {
    let (mut app, _path) = db_app(true);
    let id = app.db.connections[0].id;
    let head = |app: &App, window: &str| crate::ui::window_head(app, window);
    assert_eq!(head(&app, crate::ui::DIALOG).0, "database");
    assert_eq!(head(&app, crate::ui::DESIGNER).0, "table-2");
    assert_eq!(head(&app, crate::ui::HEALTH).0, "heart-pulse");
    assert_eq!(head(&app, crate::ui::IMPORT).0, "file-input");
    assert_eq!(head(&app, crate::ui::COPY).0, "copy-plus");
    assert_eq!(head(&app, crate::ui::BACKUP).0, "hard-drive-download");
    // A question that removes something has the red picto of a loss.
    let _ = app.update(Message::Db(DbMsg::AskEmpty(
        id,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    let (icon, line) = head(&app, crate::ui::TEXT);
    assert_eq!(icon, "trash-2");
    assert_eq!(line, t!("db.sub_confirm").to_string());
    // Every line says something, in both languages.
    for window in [
        crate::ui::DIALOG,
        crate::ui::DESIGNER,
        crate::ui::HEALTH,
        crate::ui::IMPORT,
        crate::ui::COPY,
        crate::ui::BACKUP,
    ] {
        assert!(!head(&app, window).1.starts_with("db."));
    }
}

#[test]
fn every_window_but_the_diagram_is_a_popin_with_a_title_of_its_own() {
    let (mut app, _path) = db_app(true);
    let id = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenDialog(Some(id))));
    let titles = crate::ui::window_titles(&app);
    let dialog = titles
        .iter()
        .find(|(w, _, _)| *w == crate::ui::DIALOG)
        .unwrap();
    assert!(dialog.2, "wanted");
    assert_eq!(dialog.1, t!("db.dialog_edit").to_string());
}

/// Writes what the plugin renders (the tree, the form of a new source and of
/// one that exists, a question) as JSON, to look at it in the editor without
/// a screen: `UI_DUMP=/tmp/ui cargo test dump_the_renders -- --ignored`, then
/// `PLUGIN_UI=/tmp/ui SNAPSHOT=/tmp/look.png SCENE=form cargo test look_tests
/// -- --ignored` in the editor.
#[test]
#[ignore]
fn dump_the_renders() {
    use crate::database::Dialog;
    use crate::ui::{admin, dialog, tree, Ui};
    let Ok(dir) = std::env::var("UI_DUMP") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let save = |name: &str, node: &ardoise_plugin::ui::Node| {
        std::fs::write(
            std::path::Path::new(&dir).join(name),
            serde_json::to_string_pretty(node).unwrap(),
        )
        .unwrap();
    };
    let empty = app();
    let (mut app, _path) = db_app(true);
    let id = app.db.connections[0].id;
    app.db.connections[0].name = "prod-postgres".into();
    app.db.open.insert(id.to_string());
    save("tree.json", &tree::view(&mut Ui::new(true), &app));
    app.db.dialog = Some(Dialog::new());
    save(
        "form.json",
        &dialog::view(&mut Ui::new(true), &app.db, app.db.dialog.as_ref().unwrap()),
    );
    let _ = app.update(Message::Db(DbMsg::OpenDialog(Some(id))));
    save(
        "form_edit.json",
        &dialog::view(&mut Ui::new(true), &app.db, app.db.dialog.as_ref().unwrap()),
    );
    app.db.dialog = None;
    let _ = app.update(Message::Db(DbMsg::AskDrop(
        id,
        "shop".into(),
        "public".into(),
        "items".into(),
        crate::database::protocol::Kind::Table,
    )));
    save("confirm.json", &admin::small(&mut Ui::new(true), &app.db));
    std::fs::write(
        std::path::Path::new(&dir).join("titles.json"),
        serde_json::to_string(&crate::ui::window_titles(&app)).unwrap(),
    )
    .unwrap();
    save("tree_empty.json", &tree::view(&mut Ui::new(true), &empty));
}

fn orders_structure() -> crate::database::protocol::Structure {
    use crate::database::protocol::{
        ColumnDef, ConstraintDef, ConstraintKind, Reference, Structure,
    };
    let column = |name: &str, kind: &str| ColumnDef {
        name: name.into(),
        kind: kind.into(),
        nullable: true,
        default: None,
        identity: String::new(),
        generated: false,
        comment: String::new(),
    };
    Structure {
        columns: vec![column("id", "integer"), column("label", "text")],
        constraints: vec![ConstraintDef {
            name: "items_pkey".into(),
            kind: ConstraintKind::Primary,
            definition: "PRIMARY KEY (id)".into(),
        }],
        referenced_by: vec![Reference {
            name: "lines_item_fkey".into(),
            schema: "public".into(),
            table: "lines".into(),
            definition: "FOREIGN KEY (item_id) REFERENCES items(id)".into(),
        }],
        ..Default::default()
    }
}

#[test]
fn a_table_of_the_tree_opens_to_its_structure_and_shows_its_ddl() {
    use crate::database::{DbMsg, Done};
    let (mut app, path) = db_app(true);
    let conn = app.db.connections[0].id;
    let key = (
        conn,
        "shop".to_string(),
        "public".to_string(),
        "items".to_string(),
    );
    // Opening the node reads the structure; it is kept.
    let _ = app.update(Message::Db(DbMsg::ToggleTable(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    assert!(app.db.open.contains(&format!("{conn}/shop/public/items")));
    let _ = app.update(Message::Db(DbMsg::StructureRead(
        key.clone(),
        false,
        Done::ok(orders_structure()),
    )));
    assert!(app.db.structures.contains_key(&key));
    // The tab of the table learns it too, for the links of its rows.
    assert!(app.db_tabs[&path].structure.is_some());
    // The DDL comes from what is kept.
    let _ = app.update(Message::Db(DbMsg::ShowDdl(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    let text = app.db.text.as_ref().unwrap();
    assert_eq!(text.title, "items");
    assert!(text.text.starts_with("CREATE TABLE \"public\".\"items\" ("));
    assert!(text
        .text
        .contains("CONSTRAINT \"items_pkey\" PRIMARY KEY (id)"));
    let _ = app.close_window(crate::app::Window::Text);
    assert!(app.db.text.is_none());
    // Folding it again.
    let _ = app.update(Message::Db(DbMsg::ToggleTable(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    assert!(!app.db.open.contains(&format!("{conn}/shop/public/items")));
    // A reload forgets what the tables were made of.
    let _ = app.update(Message::Db(DbMsg::Reload(conn)));
    assert!(app.db.structures.is_empty());
    // The filter of the tree is kept as it is typed.
    let _ = app.update(Message::Db(DbMsg::TreeFilter("it".into())));
    assert_eq!(app.db.tree_filter, "it");
}

#[test]
fn a_value_is_shown_whole_and_json_is_indented() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::ViewValue(path.clone(), 0, 1)));
    let view = app.db.text.as_ref().unwrap();
    assert_eq!(view.title, "label");
    assert_eq!(
        view.text,
        app.db_tabs[&path].page.as_ref().unwrap().rows[0][1]
            .clone()
            .unwrap()
    );
    let _ = app.close_window(crate::app::Window::Text);
    let tab = app.db_tabs.get_mut(&path).unwrap();
    let page = tab.page.clone().unwrap();
    tab.pending
        .edits
        .insert((0, 1), Some("{\"a\":1}".to_string()));
    let _ = page;
    let _ = app.update(Message::Db(DbMsg::ViewValue(path, 0, 1)));
    assert_eq!(app.db.text.as_ref().unwrap().text, "{\n  \"a\": 1\n}");
}

#[test]
fn bytes_are_shown_as_a_dump_and_an_image_as_a_picture() {
    let (mut app, path) = db_app(true);
    let tab = app.db_tabs.get_mut(&path).unwrap();
    tab.pending
        .edits
        .insert((0, 1), Some("\\x89504e470d0a1a0a00".to_string()));
    let _ = app.update(Message::Db(DbMsg::ViewValue(path.clone(), 0, 1)));
    let view = app.db.text.as_ref().unwrap();
    assert!(view.title.starts_with("label (9 B, PNG)"), "{}", view.title);
    assert!(view.text.starts_with("00000000  89 50 4e 47"));
    assert!(view
        .image
        .as_ref()
        .unwrap()
        .contains("data:image/png;base64,"));
    // A text stays a text.
    let _ = app.close_window(crate::app::Window::Text);
    let tab = app.db_tabs.get_mut(&path).unwrap();
    tab.pending.edits.insert((0, 1), Some("hello".to_string()));
    let _ = app.update(Message::Db(DbMsg::ViewValue(path, 0, 1)));
    let view = app.db.text.as_ref().unwrap();
    assert_eq!((view.text.as_str(), view.image.is_none()), ("hello", true));
}

#[test]
fn a_column_is_dragged_to_a_width_that_a_new_page_keeps() {
    let (mut app, path) = db_app(true);
    let before = app.db_tabs[&path].widths[1];
    // The editor drags the edge, and says the width it ends at.
    grid(
        &mut app,
        &path,
        GridEvent::Resize {
            col: 1,
            width: before + 50.0,
        },
    );
    assert!((app.db_tabs[&path].widths[1] - (before + 50.0)).abs() < 0.01);
    grid(
        &mut app,
        &path,
        GridEvent::Resize {
            col: 1,
            width: 40.0,
        },
    );
    assert_eq!(app.db_tabs[&path].widths[1], 40.0);
    // Another page of the table keeps it.
    let tab = app.db_tabs.get_mut(&path).unwrap();
    let page = tab.page.clone().unwrap();
    tab.set_page(page);
    assert_eq!(tab.widths[1], 40.0);
}

#[test]
fn the_rows_a_row_is_linked_to_open_with_a_condition() {
    let (mut app, path) = db_app(true);
    let conn = app.db.connections[0].id;
    // Another table, a condition on it.
    let _ = app.update(Message::Db(DbMsg::OpenRelated(
        conn,
        "shop".into(),
        "public".into(),
        "lines".into(),
        "\"item_id\" = '7'".into(),
    )));
    let opened = app
        .db_tabs
        .iter()
        .find(|(_, t)| t.table == "lines")
        .map(|(_, t)| t)
        .unwrap();
    assert_eq!(opened.filter, "\"item_id\" = '7'");
    assert_eq!(opened.filter_text, "\"item_id\" = '7'");
    // The table that is already open gets the condition.
    let _ = app.update(Message::Db(DbMsg::OpenRelated(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
        "\"id\" = '3'".into(),
    )));
    assert_eq!(app.db_tabs[&path].filter, "\"id\" = '3'");
    assert_eq!(app.db_tabs.len(), 2);
}

#[test]
fn a_schema_is_named_then_created_or_refused() {
    use crate::database::{AlterKind, Altered, DbMsg, Done, Ran};
    let (mut app, _path) = db_app(true);
    let conn = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::NewSchema(conn, "shop".into())));
    // Nothing is run without a name.
    let _ = app.update(Message::Db(DbMsg::SchemaCreate));
    assert!(!app.db.new_schema.as_ref().unwrap().running);
    let _ = app.update(Message::Db(DbMsg::SchemaName("sales".into())));
    let _ = app.update(Message::Db(DbMsg::SchemaCreate));
    assert!(app.db.new_schema.as_ref().unwrap().running);
    let altered = |done| {
        Message::Db(DbMsg::Altered(Box::new(Altered {
            what: AlterKind::Schema,
            conn,
            database: "shop".into(),
            schema: "sales".into(),
            table: String::new(),
            done,
        })))
    };
    let _ = app.update(altered(Done::failed("already exists".to_string())));
    let n = app.db.new_schema.as_ref().unwrap();
    assert!(!n.running);
    assert_eq!(n.error.as_deref(), Some("already exists"));
    let _ = app.update(Message::Db(DbMsg::SchemaCreate));
    let _ = app.update(altered(Done::ok(Ran {
        in_transaction: false,
        results: vec![],
        failed: None,
        position: None,
    })));
    assert!(app.db.new_schema.is_none());
}

#[test]
fn a_new_table_starts_with_an_id_and_is_made_by_the_designer() {
    use crate::database::{DbMsg, DesignerMsg};
    let (mut app, _path) = db_app(true);
    let conn = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::NewTable(
        conn,
        "shop".into(),
        "sales".into(),
    )));
    let d = app.db.designer.as_ref().unwrap();
    assert!(d.creating);
    assert_eq!(d.schema_to, "sales");
    // It needs a name first.
    assert!(d.problem().is_some());
    let _ = app.update(Message::Db(DbMsg::Designer(DesignerMsg::TableName(
        "orders".into(),
    ))));
    let d = app.db.designer.as_ref().unwrap();
    assert_eq!(d.problem(), None);
    assert!(d.statements()[0].starts_with("CREATE TABLE \"sales\".\"orders\""));
    let _ = app.update(Message::Db(DbMsg::Designer(DesignerMsg::Apply)));
    assert_eq!(
        app.db.designer.as_ref().unwrap().load,
        crate::database::designer::Load::Running
    );
}

#[test]
fn a_running_query_is_stopped() {
    let (mut app, _path) = db_app(true);
    app.db.console.running = true;
    let _ = app.update(Message::Db(DbMsg::Cancel));
    assert!(!app.db.console.running);
    assert_eq!(app.db.console.last.as_ref().map(|l| l.0), Some(false));
}

#[test]
fn the_designer_opens_loads_is_edited_and_applied_in_one_transaction() {
    use crate::database::designer::{DesignerTab, Load};
    use crate::database::protocol::{ColumnDef, ConstraintDef, ConstraintKind, Structure};
    use crate::database::{AlterKind, ColField, DbMsg, DesignerMsg};
    let (mut app, path) = db_app(true);
    let conn = app.db.connections[0].id;
    let _ = app.update(Message::Db(DbMsg::OpenDesigner(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    assert_eq!(app.db.designer.as_ref().unwrap().load, Load::Loading);
    let column = |name: &str, kind: &str| ColumnDef {
        name: name.into(),
        kind: kind.into(),
        nullable: true,
        default: None,
        identity: String::new(),
        generated: false,
        comment: String::new(),
    };
    let structure = Structure {
        columns: vec![column("id", "integer"), column("label", "text")],
        constraints: vec![ConstraintDef {
            name: "items_pkey".into(),
            kind: ConstraintKind::Primary,
            definition: "PRIMARY KEY (id)".into(),
        }],
        indexes: vec![],
        comment: String::new(),
        ..Default::default()
    };
    let _ = app.update(Message::Db(DbMsg::Designer(DesignerMsg::Loaded(Done::ok(
        structure,
    )))));
    assert_eq!(app.db.designer.as_ref().unwrap().load, Load::Ready);
    // Nothing changed: nothing to apply.
    let _ = app.update(Message::Db(DbMsg::Designer(DesignerMsg::Apply)));
    assert_eq!(app.db.designer.as_ref().unwrap().load, Load::Ready);
    // Edits: a rename, a new column, a dropped one, a new table name.
    let say = |app: &mut App, m: DesignerMsg| {
        let _ = app.update(Message::Db(DbMsg::Designer(m)));
    };
    say(
        &mut app,
        DesignerMsg::Col(1, ColField::Name, "title".into()),
    );
    say(&mut app, DesignerMsg::ColAdd);
    say(&mut app, DesignerMsg::Col(2, ColField::Name, "note".into()));
    say(&mut app, DesignerMsg::ColDrop(0, true));
    say(&mut app, DesignerMsg::ColDrop(0, false));
    say(&mut app, DesignerMsg::TableName("things".into()));
    say(&mut app, DesignerMsg::Tab(DesignerTab::Sql));
    let d = app.db.designer.as_ref().unwrap();
    assert_eq!(d.tab, DesignerTab::Sql);
    assert_eq!(d.statements().len(), 3);
    assert_eq!(d.problem(), None);
    // A column that is new is removed, not marked.
    say(&mut app, DesignerMsg::ColAdd);
    assert_eq!(app.db.designer.as_ref().unwrap().columns.len(), 4);
    say(&mut app, DesignerMsg::ColDrop(3, true));
    assert_eq!(app.db.designer.as_ref().unwrap().columns.len(), 3);
    // A problem stops the apply.
    say(&mut app, DesignerMsg::Col(1, ColField::Name, String::new()));
    say(&mut app, DesignerMsg::Apply);
    assert_eq!(app.db.designer.as_ref().unwrap().load, Load::Ready);
    say(
        &mut app,
        DesignerMsg::Col(1, ColField::Name, "title".into()),
    );
    // Applied: it runs, and a refusal stays on the designer with its reason.
    say(&mut app, DesignerMsg::Apply);
    assert_eq!(app.db.designer.as_ref().unwrap().load, Load::Running);
    let _ = app.update(Message::Db(DbMsg::Altered(Box::new(altered(
        AlterKind::Modify(Some("things".into())),
        conn,
        Err("column \"note\" already exists".into()),
    )))));
    let d = app.db.designer.as_ref().unwrap();
    assert_eq!(d.load, Load::Ready);
    assert_eq!(d.error.as_deref(), Some("column \"note\" already exists"));
    assert!(app.db_tabs.contains_key(&path));
    // Then it goes: the designer closes, the tabs of the renamed table too.
    say(&mut app, DesignerMsg::Apply);
    let _ = app.update(Message::Db(DbMsg::Altered(Box::new(altered(
        AlterKind::Modify(Some("things".into())),
        conn,
        Ok(()),
    )))));
    assert!(app.db.designer.is_none());
    assert!(!app.db_tabs.contains_key(&path));
    // Its tab closes a designer that is not running.
    let _ = app.update(Message::Db(DbMsg::OpenDesigner(
        conn,
        "shop".into(),
        "public".into(),
        "items".into(),
    )));
    assert!(app.close_window(Window::Designer));
    assert!(app.db.designer.is_none());
}

#[test]
fn a_table_that_cannot_be_edited_refuses_everything() {
    let (mut app, path) = db_app(false);
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 0, 1)));
    let _ = app.update(Message::Db(DbMsg::DeleteRow(path.clone(), 0)));
    let _ = app.update(Message::Db(DbMsg::SetNull(path.clone(), 0, 1)));
    let _ = app.update(Message::Db(DbMsg::OpenPicker(path.clone(), 0, 2)));
    let tab = &app.db_tabs[&path];
    assert!(tab.editing.is_none() && tab.picker.is_none() && tab.pending.is_empty());
}

#[test]
fn a_deleted_row_cannot_be_typed_in() {
    let (mut app, path) = db_app(true);
    let _ = app.update(Message::Db(DbMsg::DeleteRow(path.clone(), 1)));
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 1, 1)));
    assert!(app.db_tabs[&path].editing.is_none());
    let _ = app.update(Message::Db(DbMsg::RevertRow(path.clone(), 1)));
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 1, 1)));
    assert!(app.db_tabs[&path].editing.is_some());
}

#[test]
fn the_diagram_of_a_schema_opens_and_is_copied_as_mermaid() {
    use crate::database::protocol::{Catalog, TableColumn};
    let mut app = App::new();
    let catalog = Catalog {
        columns: vec![TableColumn {
            schema: "s".into(),
            table: "t".into(),
            name: "id".into(),
            kind: "integer".into(),
        }],
        foreign_keys: Vec::new(),
    };
    let task = app.update(Message::Db(DbMsg::DiagramLoaded(
        "s".into(),
        Done::ok(catalog),
    )));
    let (mut effects, mut futures) = (Vec::new(), Vec::new());
    task.split(&mut effects, &mut futures);
    // Its tab opens, with the picture and the Mermaid text.
    assert!(matches!(&effects[0], crate::task::Effect::OpenTab(id, _) if id == crate::ui::DIAGRAM));
    let shown = app.diagram.as_ref().expect("the diagram");
    assert_eq!(shown.title, "s");
    assert_eq!(shown.diagram.tables.len(), 1);
    assert!(shown.mermaid.contains("erDiagram"));
    assert!(crate::database::diagram::svg(&shown.diagram, true).starts_with("<svg"));
    let task = app.update(Message::Db(DbMsg::CopyDiagram));
    let (mut effects, mut futures) = (Vec::new(), Vec::new());
    task.split(&mut effects, &mut futures);
    assert!(matches!(&effects[0], crate::task::Effect::Copy(text) if text.contains("erDiagram")));
    // A schema without tables says so and opens nothing.
    let mut app = App::new();
    let _ = app.update(Message::Db(DbMsg::DiagramLoaded(
        "empty".into(),
        Done::ok(Catalog::default()),
    )));
    assert!(app.diagram.is_none());
}

#[test]
fn an_answer_for_a_window_that_is_not_the_open_one_is_dropped() {
    use crate::database::AnswerFor;
    let mut app = App::new();
    app.db.designer = Some(crate::database::designer::Designer::new(
        1,
        "d".into(),
        "public".into(),
        "b".into(),
    ));
    // The structure of "a" arrives while the designer of "b" is open.
    let late = DbMsg::Answer(
        AnswerFor::Designer(1, "public".into(), "a".into()),
        Box::new(DbMsg::Designer(crate::database::DesignerMsg::Close)),
    );
    let _ = app.update(Message::Db(late));
    assert!(app.db.designer.is_some(), "not taken for the open one");
    let on_time = DbMsg::Answer(
        AnswerFor::Designer(1, "public".into(), "b".into()),
        Box::new(DbMsg::Designer(crate::database::DesignerMsg::Close)),
    );
    let _ = app.update(Message::Db(on_time));
    assert!(app.db.designer.is_none());
}

#[test]
fn removing_the_plugin_with_its_data_takes_the_passwords_out_of_the_keychain() {
    use crate::database::secrets;
    let mut app = App::new();
    app.db.connections.clear();
    for id in [9101, 9102] {
        app.db.connections.push(crate::database::Connection {
            id,
            ..Default::default()
        });
        assert!(secrets::save(id, secrets::PASSWORD, "hunter2"));
        assert!(secrets::save(id, secrets::SSH, "ssh-pass"));
    }
    app.db.forget_secrets();
    for id in [9101, 9102] {
        assert_eq!(secrets::load(id, secrets::PASSWORD), None);
        assert_eq!(secrets::load(id, secrets::SSH), None);
    }
}

#[test]
fn the_pending_changes_are_counted_in_words_that_agree() {
    use crate::database::DbMsg;
    let (mut app, path) = db_app(true);
    let shown = |app: &App| {
        let mut ui = crate::ui::Ui::new(false);
        let node = crate::ui::table::view(&mut ui, &path, &app.db_tabs[&path]);
        serde_json::to_string(&node).unwrap()
    };
    let one = t!("db.pending_one").to_string();
    let none = t!("db.pending", n = 0).to_string();
    // A cell being typed in: nothing waits yet, so nothing is counted.
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 1, 1)));
    let typing = shown(&app);
    assert!(!typing.contains(&none) && !typing.contains(&one));
    assert!(typing.contains(t!("db.revert").as_ref()));
    let _ = app.update(Message::Db(DbMsg::EditInput(path.clone(), "x".into())));
    let _ = app.update(Message::Db(DbMsg::EditCommit(path.clone())));
    assert!(shown(&app).contains(&one));
    let _ = app.update(Message::Db(DbMsg::Edit(path.clone(), 2, 1)));
    let _ = app.update(Message::Db(DbMsg::EditInput(path.clone(), "y".into())));
    let _ = app.update(Message::Db(DbMsg::EditCommit(path.clone())));
    assert!(shown(&app).contains(t!("db.pending", n = 2).as_ref()));
}
