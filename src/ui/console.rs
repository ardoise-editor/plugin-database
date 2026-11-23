//! The console under the editor: its queries (in tabs of their own), the
//! connection and database they run on, the transaction, Run, Explain,
//! the values asked for and the guard before writes; and the log of what
//! was run.

use ardoise_plugin::ui::{column, row, scroll, separator, text, Node, TextStyle};

use super::Ui;
use crate::app::App;
use crate::database::console::ConsoleTab;
use crate::database::{protocol, DbMsg, Driver, LogCopy, Tree};

pub fn view(ui: &mut Ui, app: &App) -> Node {
    let state = &app.db;
    let c = &state.console;
    let tab = |ui: &mut Ui, label: String, which: ConsoleTab| {
        let b = ui.button(label, DbMsg::ConsoleTab(which));
        if c.tab == which {
            b.button_style(ardoise_plugin::ui::ButtonStyle::Primary)
        } else {
            b.button_style(ardoise_plugin::ui::ButtonStyle::Ghost)
        }
    };
    let mut header = vec![
        tab(
            ui,
            t!("db.console_queries").to_string(),
            ConsoleTab::Queries,
        ),
        tab(ui, t!("db.console_log").to_string(), ConsoleTab::Log),
    ];
    match c.tab {
        ConsoleTab::Queries => {
            let names: Vec<(u64, String)> = state
                .connections
                .iter()
                .map(|k| (k.id, k.label()))
                .collect();
            let chosen = c
                .connection
                .and_then(|id| names.iter().find(|(i, _)| *i == id).map(|(_, l)| l.clone()));
            let options: Vec<String> = names.iter().map(|(_, l)| l.clone()).collect();
            header.push(ui.select("connection", options, chosen, move |label| {
                let id = names
                    .iter()
                    .find(|(_, l)| *l == label)
                    .map_or(0, |(i, _)| *i);
                DbMsg::ConsoleConnection(id)
            }));
            if let Some(Tree::Loaded(databases)) = c.connection.and_then(|id| state.trees.get(&id))
            {
                if databases.len() > 1 {
                    let names: Vec<String> = databases.iter().map(|d| d.name.clone()).collect();
                    let current = databases.iter().find(|d| d.current).map(|d| d.name.clone());
                    let chosen = if c.database.is_empty() {
                        current.clone()
                    } else {
                        Some(c.database.clone())
                    };
                    header.push(ui.select("database", names, chosen, move |name| {
                        let own = current.as_deref() == Some(name.as_str());
                        DbMsg::ConsoleDatabase(if own { String::new() } else { name })
                    }));
                }
            }
            let ready = !c.running && *state.driver_of(c.dialect) == Driver::Ready;
            if c.in_transaction {
                header.push(text(t!("db.tx_open").to_string()).style(TextStyle::Bold));
                header.push(ui.button_if(
                    t!("db.tx_commit").to_string(),
                    ready.then_some(DbMsg::EndTransaction(true)),
                ));
                header.push(ui.button_if(
                    t!("db.tx_rollback").to_string(),
                    ready.then_some(DbMsg::EndTransaction(false)),
                ));
            }
            if c.running {
                header.push(ui.icon_tool("square", t!("db.cancel").to_string(), DbMsg::Cancel));
            }
            header.push(
                ui.button_if(t!("db.run").to_string(), ready.then_some(DbMsg::Run(false)))
                    .button_style(ardoise_plugin::ui::ButtonStyle::Primary)
                    .icon("play"),
            );
            header.push(ui.tool(t!("db.format").to_string(), DbMsg::FormatSql));
            header.push(ui.button_if(
                t!("db.explain").to_string(),
                ready.then_some(DbMsg::Explain(false)),
            ));
            header.push(ui.button_if(
                t!("db.explain_analyze").to_string(),
                ready.then_some(DbMsg::Explain(true)),
            ));
            header.push(ui.button_if(
                t!("db.run_all").to_string(),
                ready.then_some(DbMsg::Run(true)),
            ));
        }
        ConsoleTab::Log => {
            header.push(ui.input(
                "log_filter",
                &c.log_filter,
                t!("db.log_search"),
                DbMsg::LogFilter,
            ));
            header.push(ui.checkbox(
                "internal",
                t!("db.log_internal"),
                c.show_internal,
                DbMsg::ShowInternal,
            ));
            header.push(ui.button_if(
                t!("db.log_clear").to_string(),
                (!c.log.is_empty()).then_some(DbMsg::ClearLog),
            ));
        }
    }
    let content = match c.tab {
        ConsoleTab::Queries => queries(ui, app),
        ConsoleTab::Log => log(ui, app),
    };
    column([row(header).spacing(4.0), separator(), content])
        .spacing(4.0)
        .padding(6.0)
}

fn queries(ui: &mut Ui, app: &App) -> Node {
    let state = &app.db;
    let c = &state.console;
    // The queries in tabs of their own.
    let mut tabs = Vec::new();
    for (at, doc) in c.docs.iter().enumerate() {
        let b = ui.button(doc.title.clone(), DbMsg::SwitchConsole(at));
        tabs.push(if at == c.active {
            b.button_style(ardoise_plugin::ui::ButtonStyle::Primary)
        } else {
            b.button_style(ardoise_plugin::ui::ButtonStyle::Ghost)
        });
        if c.docs.len() > 1 {
            tabs.push(ui.icon_tool("x", t!("db.close").to_string(), DbMsg::CloseConsole(at)));
        }
    }
    tabs.push(ui.icon_tool("plus", t!("db.console_new").to_string(), DbMsg::NewConsole));
    let mut editor = ui
        .editor("sql", &c.editor.text(), "sql", DbMsg::SqlText)
        .with_completion()
        .height(220.0)
        // The statement "Run" would run is framed by the editor.
        .regions(c.statement_regions());
    // The word the server blamed, underlined.
    if let Some(mark) = &c.error_mark {
        editor = editor.mark(ardoise_plugin::ui::EditorMark {
            line: mark.line,
            column: mark.column,
            len: mark.len,
        });
    }
    let status: Node = if let Some(binds) = &c.binds {
        let mut line = vec![text(t!("db.bind_title").to_string()).style(TextStyle::Bold)];
        for (at, name) in binds.names.iter().enumerate() {
            line.push(text(name.clone()).style(TextStyle::Mono));
            let value = binds.values.get(at).cloned().unwrap_or_default();
            let field = ui.input(&format!("bind_{at}"), &value, "", move |t| {
                DbMsg::BindValue(at, t)
            });
            line.push(ui.submit(field, DbMsg::BindRun));
        }
        line.push(ui.primary(t!("db.bind_run").to_string(), DbMsg::BindRun));
        line.push(ui.tool(t!("db.guard_cancel").to_string(), DbMsg::BindCancel));
        row(line).spacing(6.0)
    } else if let Some(guard) = &c.guard {
        let said = match guard.risk {
            protocol::Risk::NoWhere => t!("db.guard_no_where").to_string(),
            _ => t!("db.guard_writes").to_string(),
        };
        row([
            text(said).style(TextStyle::Error),
            ui.button(t!("db.guard_run").to_string(), DbMsg::GuardRun)
                .button_style(ardoise_plugin::ui::ButtonStyle::Danger),
            ui.tool(t!("db.guard_cancel").to_string(), DbMsg::GuardCancel),
        ])
        .spacing(6.0)
    } else if c.running {
        text(t!("db.console_running").to_string()).style(TextStyle::Muted)
    } else if state.connections.is_empty() {
        text(t!("db.console_no_connection").to_string()).style(TextStyle::Muted)
    } else {
        match &c.last {
            Some((true, said)) => text(said.clone()).style(TextStyle::Muted),
            Some((false, said)) => text(said.clone()).style(TextStyle::Error),
            None => text(t!("db.console_shortcuts").to_string()).style(TextStyle::Muted),
        }
    };
    let mut out = vec![row(tabs).spacing(2.0), editor, status];
    if let Some(mark) = &c.error_mark {
        out.push(
            text(
                t!(
                    "db.error_at",
                    line = mark.line + 1,
                    column = mark.column + 1
                )
                .to_string(),
            )
            .style(TextStyle::Error),
        );
    }
    column(out).spacing(4.0)
}

fn log(ui: &mut Ui, app: &App) -> Node {
    let c = &app.db.console;
    if c.log.is_empty() {
        return text(t!("db.log_empty").to_string()).style(TextStyle::Muted);
    }
    let kept: Vec<_> = c
        .log
        .iter()
        .enumerate()
        .filter(|(_, e)| c.show_internal || !e.internal)
        .filter(|(_, e)| c.matches(e))
        .collect();
    let older = kept.len().saturating_sub(c.log_limit);
    let mut list = Vec::new();
    if older > 0 {
        list.push(ui.tool(t!("db.log_older", n = older).to_string(), DbMsg::LogMore));
    }
    for (at, entry) in kept.into_iter().skip(older) {
        let ok = entry.error.is_none();
        let mut head = vec![
            text(entry.time.clone()).style(TextStyle::Muted),
            text(if ok { "✓" } else { "✗" }).style(if ok {
                TextStyle::Bold
            } else {
                TextStyle::Error
            }),
            text(entry.connection.clone()).style(TextStyle::Muted),
            text(format!("{} ms", entry.ms)).style(TextStyle::Muted),
        ];
        if let Some(rows) = entry.rows {
            head.push(
                text(if rows == 1 {
                    t!("db.log_row_one").to_string()
                } else {
                    t!("db.log_rows", n = rows).to_string()
                })
                .style(TextStyle::Muted),
            );
        }
        head.push(ui.tool(t!("db.log_reuse").to_string(), DbMsg::LogReuse(at)));
        let picked = c.selected_log == Some(at);
        let copy_sql = ui.icon_tool(
            "copy",
            t!("db.log_copy_sql").to_string(),
            DbMsg::CopyLog(at, LogCopy::Sql),
        );
        // The entry clicked: Cmd/Ctrl+C copies its SQL.
        head.push(if picked {
            copy_sql.key("Ctrl+C")
        } else {
            copy_sql
        });
        head.push(ui.icon_tool(
            "copy",
            t!("db.log_copy_entry").to_string(),
            DbMsg::CopyLog(at, LogCopy::Entry),
        ));
        if !ok {
            head.push(ui.icon_tool(
                "copy",
                t!("db.log_copy_error").to_string(),
                DbMsg::CopyLog(at, LogCopy::Error),
            ));
        }
        list.push(row(head).spacing(6.0));
        // A click on the SQL picks the entry.
        let mut sql = ardoise_plugin::ui::list(
            "log_entry",
            [ardoise_plugin::ui::item(
                ui.act(DbMsg::SelectLog(at)),
                entry.sql.clone(),
            )],
        );
        if picked {
            if let Node::List {
                selected, items, ..
            } = &mut sql
            {
                *selected = items.first().map(|i| i.id.clone());
            }
        }
        list.push(sql);
        if let Some(error) = &entry.error {
            list.push(text(error.clone()).style(TextStyle::Error));
        }
        list.push(separator());
    }
    scroll(column(list).spacing(2.0)).stick_to_bottom()
}
