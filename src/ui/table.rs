//! A table (or the result of a query) in a tab: its bar (rows shown, count,
//! pending edits, pages, reload, export), the WHERE and search fields, the
//! grid, and the row as a form, the SQL of the pending edits or the date
//! picker when they are open.

use std::path::Path;

use ardoise_plugin::ui::{
    column, grid, row, scroll, separator, text, GridCell, GridColumn, GridEditing, GridEvent,
    GridRow, GridSelection, MenuItem, Node, RowState, TextStyle,
};

use super::Ui;
use crate::database::grid::{Cell, PickKind, TimePart};
use crate::database::{protocol, DbMsg, TableTab, PAGE_SIZES};

/// What an event of the grid of a table tab stands for.
pub fn grid_message(path: &Path, event: GridEvent) -> Option<DbMsg> {
    Some(DbMsg::Grid(path.to_path_buf(), event))
}

/// The items of the menu of a cell, by action: the message they make for a
/// row and a column of the grid.
pub fn menu_message(path: &Path, action: &str, row: usize, col: usize) -> Option<DbMsg> {
    Some(DbMsg::GridMenu(
        path.to_path_buf(),
        action.to_string(),
        row,
        col,
    ))
}

pub(super) fn menu(tab: &TableTab) -> Vec<MenuItem> {
    let editable = tab.editable();
    let mut items = vec![
        ("view", t!("db.cell_view"), true),
        ("copy", t!("db.cell_copy"), true),
        ("hide", t!("db.hide_column"), true),
        (
            "filter",
            t!("db.filter_by_value"),
            tab.query.is_none() || tab.paged,
        ),
        ("copy_csv", t!("db.copy_csv"), true),
        ("copy_insert", t!("db.copy_insert"), true),
        ("copy_markdown", t!("db.copy_markdown"), true),
        ("copy_tsv", t!("db.copy_tsv"), true),
        ("form", t!("db.form_open"), true),
        ("row_copy", t!("db.row_copy"), true),
        (
            "links",
            t!("db.links_menu"),
            tab.structure.is_some() && tab.query.is_none(),
        ),
    ];
    if editable {
        items.extend([
            ("edit", t!("db.cell_edit"), true),
            ("null", t!("db.cell_null"), true),
            ("default", t!("db.cell_default"), true),
            ("pick", t!("db.pick_date"), true),
            ("add", t!("db.row_add"), true),
            ("delete", t!("db.row_delete"), true),
            ("revert", t!("db.row_revert"), true),
        ]);
    }
    items
        .into_iter()
        .map(|(action, label, enabled)| {
            let item = MenuItem::new(label.to_string(), format!("menu:{action}")).enabled(enabled);
            // F4 opens the form of the row, as it did.
            if action == "form" {
                item.key("F4")
            } else {
                item
            }
        })
        .collect()
}

fn the_grid(ui: &mut Ui, path: &Path, tab: &TableTab) -> Node {
    let Some(page) = &tab.page else {
        return text(t!("db.loading").to_string()).style(TextStyle::Muted);
    };
    let shown = tab.shown_columns();
    let place = |c: usize| shown.iter().position(|s| *s == c);
    let columns = shown.iter().map(|&c| {
        let column = &page.columns[c];
        GridColumn {
            name: column.name.clone(),
            detail: column.kind.clone(),
            width: tab.widths.get(c).copied(),
            sort: tab
                .order
                .iter()
                .find(|(name, _)| *name == column.name)
                .map(|(_, desc)| !desc),
            filter: tab
                .column_text
                .get(&column.name)
                .cloned()
                .unwrap_or_default(),
        }
    });
    let count = tab.pending.rows(page);
    let rows = (0..count).map(|r| {
        let state = if tab.pending.is_new(page, r) {
            RowState::New
        } else if tab.pending.is_deleted(r) {
            RowState::Deleted
        } else if tab.pending.edits.keys().any(|(row, _)| *row == r) {
            RowState::Edited
        } else {
            RowState::Saved
        };
        let cells = shown
            .iter()
            .map(|&c| match tab.pending.cell(page, r, c) {
                Cell::Value(v) => GridCell {
                    text: if page.truncated.contains(&(r, c)) {
                        format!("{} …", crate::database::grid::shown(v))
                    } else {
                        crate::database::grid::shown(v).to_string()
                    },
                    null: false,
                    edited: tab.pending.is_edited(r, c),
                },
                Cell::Null => GridCell {
                    text: String::new(),
                    null: true,
                    edited: tab.pending.is_edited(r, c),
                },
                Cell::Default => GridCell {
                    text: "DEFAULT".into(),
                    null: false,
                    edited: true,
                },
            })
            .collect();
        GridRow { cells, state }
    });
    let mut g = grid("rows", columns, rows);
    g.selection = tab.selected.and_then(|(r, c)| {
        let cursor = (r, place(c)?);
        let anchor = tab
            .anchor
            .and_then(|(ar, ac)| Some((ar, place(ac)?)))
            .unwrap_or(cursor);
        Some(GridSelection { anchor, cursor })
    });
    g.editing = tab.editing.as_ref().and_then(|e| {
        Some(GridEditing {
            row: e.row,
            col: place(e.col)?,
            value: e.text.clone(),
        })
    });
    g.filters = tab.show_filters;
    g.menu = menu(tab);
    for item in &g.menu {
        ui.menu(&item.action, path);
    }
    ui.grid("rows", path);
    g.into()
}

pub fn view(ui: &mut Ui, path: &Path, tab: &TableTab) -> Node {
    let owned = || path.to_path_buf();
    let shown = tab.page.as_ref();
    let is_query = tab.is_static_result();
    let range = match shown {
        Some(page) if !page.rows.is_empty() => t!(
            "db.rows_range",
            from = page.offset + 1,
            to = page.offset + page.rows.len() as u64
        )
        .to_string(),
        Some(_) => t!("db.no_rows").to_string(),
        None => String::new(),
    };
    let title = if tab.query.is_some() {
        tab.title.clone()
    } else {
        format!("{}.{}", tab.schema, tab.table)
    };
    let mut bar = vec![
        text(title).style(TextStyle::Bold),
        text(range).style(TextStyle::Muted),
    ];
    if shown.is_some() && !is_query {
        if let Some((filter, total)) = &tab.total {
            if *filter == tab.where_clause() {
                bar.push(text(t!("db.rows_total", n = total).to_string()).style(TextStyle::Muted));
            }
        }
        bar.push(ui.tool("Σ", DbMsg::Count(owned())));
    }
    if tab.editable() && shown.is_some_and(|page| page.primary_key.is_empty()) && !is_query {
        bar.push(text(t!("db.keyless_table").to_string()).style(TextStyle::Muted));
    }
    if shown.is_some() && !tab.editable() && !is_query {
        bar.push(text(t!("db.read_only_table").to_string()).style(TextStyle::Muted));
    }
    let pending = tab.pending.count();
    if pending > 0 || tab.editing.is_some() {
        bar.push(text(t!("db.pending", n = pending).to_string()).style(TextStyle::Bold));
        bar.push(ui.tool(t!("db.revert").to_string(), DbMsg::RevertAll(owned())));
        bar.push(ui.tool(t!("db.preview").to_string(), DbMsg::Preview(owned(), true)));
        bar.push(ui.button_if(
            t!("db.commit").to_string(),
            (!tab.committing).then(|| DbMsg::Commit(owned())),
        ));
    }
    if tab.editable() {
        bar.push(
            ui.tool(t!("db.row_add").to_string(), DbMsg::AddRow(owned()))
                .icon("plus")
                .with_label(),
        );
    }
    if is_query {
        if let Some(sql) = &tab.query {
            let line = sql.split_whitespace().collect::<Vec<_>>().join(" ");
            bar.push(text(line).style(TextStyle::Mono));
        }
        if shown.is_some_and(|page| page.has_more) {
            bar.push(
                text(t!("db.result_truncated", n = protocol::MAX_RESULT_ROWS).to_string())
                    .style(TextStyle::Muted),
            );
        }
    } else {
        let mut sizes = PAGE_SIZES.to_vec();
        if !sizes.contains(&tab.page_size) {
            sizes.push(tab.page_size);
            sizes.sort_unstable();
        }
        let p = owned();
        bar.push(ui.select(
            "page_size",
            sizes.iter().map(|n| n.to_string()).collect(),
            Some(tab.page_size.to_string()),
            move |n| DbMsg::PageSize(p.clone(), n.parse().unwrap_or(500)),
        ));
        bar.push(ui.button_if("‹", (tab.offset() > 0).then(|| DbMsg::Turn(owned(), -1))));
        bar.push(
            ui.button_if(
                "›",
                shown
                    .is_some_and(|page| page.has_more)
                    .then(|| DbMsg::Turn(owned(), 1)),
            ),
        );
    }
    bar.push(ui.button_if("⟳", (!tab.loading).then(|| DbMsg::Refresh(owned()))));
    if !tab.hidden.is_empty() {
        bar.push(ui.tool(
            t!("db.show_columns", n = tab.hidden.len()).to_string(),
            DbMsg::ShowColumns(owned()),
        ));
    }
    if tab.loading && tab.stop.is_some() {
        bar.push(
            ui.tool(t!("db.stop_load").to_string(), DbMsg::StopLoad(owned()))
                .icon("square")
                .with_label(),
        );
    }
    if tab.query.is_none() || tab.paged {
        let ready = !tab.loading && tab.error.is_none();
        for (label, format) in [
            ("CSV", "csv"),
            ("JSON", "json"),
            ("SQL", "sql"),
            ("XLSX", "xlsx"),
            ("MD", "md"),
        ] {
            bar.push(ui.button_if(label, ready.then(|| DbMsg::Export(owned(), format))));
        }
    }
    let mut page = vec![row(bar).spacing(6.0)];
    if !is_query {
        let p = owned();
        let filter = ui.input("where", &tab.filter_text, t!("db.where_hint"), move |t| {
            DbMsg::Filter(p.clone(), t)
        });
        let filter = ui.submit(filter, DbMsg::ApplyFilter(owned()));
        let p = owned();
        let search = ui.input("search", &tab.search_text, t!("db.search_hint"), move |t| {
            DbMsg::SearchText(p.clone(), t)
        });
        let search = ui.submit(search, DbMsg::ApplyFilter(owned()));
        let mut line = vec![
            text("WHERE").style(TextStyle::Mono),
            filter,
            ui.tool("✓", DbMsg::ApplyFilter(owned())),
            search,
            ui.tool(
                format!("▽ {}", t!("db.column_filters")),
                DbMsg::ToggleColumnFilters(owned()),
            ),
        ];
        if tab.is_filtered() {
            line.push(ui.icon_tool("x", t!("db.close").to_string(), DbMsg::ClearFilter(owned())));
        }
        page.push(row(line).spacing(6.0));
    }
    page.push(separator());
    if let Some(notice) = &tab.notice {
        page.push(
            row([
                text(notice.clone()).style(TextStyle::Error),
                ui.icon_tool(
                    "x",
                    t!("db.close").to_string(),
                    DbMsg::DismissNotice(owned()),
                ),
            ])
            .spacing(6.0),
        );
    }
    // What opens over the grid in the editor: here, above it.
    if let Some(picker) = &tab.picker {
        page.push(date_picker(ui, path, picker));
        page.push(separator());
    } else if tab.preview {
        page.push(preview(ui, path, tab));
        page.push(separator());
    } else if let Some(r) = tab.form {
        page.push(form(ui, path, tab, r));
        page.push(separator());
    }
    page.push(match &tab.error {
        Some(error) => text(error.clone()).style(TextStyle::Error),
        None => the_grid(ui, path, tab),
    });
    column(page).spacing(6.0).padding(8.0)
}

/// A row as a form: a field for each column, typed by what it holds.
fn form(ui: &mut Ui, path: &Path, tab: &TableTab, r: usize) -> Node {
    let Some(page) = &tab.page else {
        return text("");
    };
    let owned = || path.to_path_buf();
    let count = tab.pending.rows(page);
    let editable = page.editable && !tab.pending.is_deleted(r);
    let mut fields = Vec::new();
    for (c, col) in page.columns.iter().enumerate() {
        let cell = tab.pending.cell(page, r, c);
        let current = match cell {
            Cell::Value(v) => v.to_string(),
            _ => String::new(),
        };
        let placeholder = match cell {
            Cell::Null => "NULL",
            Cell::Default => "DEFAULT",
            Cell::Value(_) => "",
        };
        let kind = col.kind.to_lowercase();
        let listed = tab
            .choices
            .as_ref()
            .and_then(|all| all.iter().find(|ch| ch.column == col.name));
        let id = format!("form_{c}");
        let p = owned();
        let on = move |t: String| DbMsg::FormInput(p.clone(), c, t);
        let mut control = vec![];
        if !editable {
            control.push(text(if current.is_empty() {
                placeholder.to_string()
            } else {
                current.clone()
            }));
        } else if kind == "boolean" || kind == "bool" {
            let chosen = match current.to_lowercase().as_str() {
                "t" | "true" => Some("true".to_string()),
                "f" | "false" => Some("false".to_string()),
                _ => None,
            };
            control.push(ui.select(&id, vec!["true".into(), "false".into()], chosen, on));
        } else if let Some(list) = listed {
            let mut values = list.values.clone();
            if !current.is_empty() && !values.contains(&current) {
                values.insert(0, current.clone());
            }
            let chosen = (!current.is_empty()).then_some(current.clone());
            control.push(ui.select(&id, values, chosen, on));
            if list.more {
                control.push(
                    text(t!("db.choices_more", n = protocol::MAX_CHOICES).to_string())
                        .style(TextStyle::Muted),
                );
            }
        } else {
            control.push(ui.input(&id, &current, placeholder, on));
        }
        if editable && (kind == "json" || kind == "jsonb") && !current.is_empty() {
            let valid = serde_json::from_str::<serde_json::Value>(&current).is_ok();
            control.push(
                text(if valid { "✓ JSON" } else { "✗ JSON" }).style(if valid {
                    TextStyle::Muted
                } else {
                    TextStyle::Error
                }),
            );
            control.push(ui.button_if(
                t!("db.form_format").to_string(),
                valid.then(|| DbMsg::FormJson(owned(), c)),
            ));
        }
        if editable && col.nullable && !matches!(cell, Cell::Null) {
            control.push(ui.tool("NULL", DbMsg::FormNull(owned(), c)));
        }
        let mut line = vec![
            text(col.name.clone()).style(TextStyle::Bold),
            text(col.kind.clone()).style(TextStyle::Muted),
        ];
        line.extend(control);
        fields.push(row(line).spacing(8.0));
    }
    column([
        row([
            text(t!("db.form_title", n = r + 1, total = count).to_string())
                .style(TextStyle::Heading),
            ui.button_if("‹", (r > 0).then(|| DbMsg::FormMove(owned(), -1))),
            ui.button_if("›", (r + 1 < count).then(|| DbMsg::FormMove(owned(), 1))),
            ui.icon_tool("x", t!("db.close").to_string(), DbMsg::CloseForm(owned())),
        ])
        .spacing(6.0),
        scroll(column(fields).spacing(6.0)).height(320.0),
    ])
    .spacing(6.0)
}

/// The SQL that Commit would run.
fn preview(ui: &mut Ui, path: &Path, tab: &TableTab) -> Node {
    let table = tab.table_ref();
    let changes = tab
        .page
        .as_ref()
        .map(|page| tab.pending.changes(page))
        .unwrap_or_default();
    let mut lines = vec![
        row([
            text(t!("db.preview_title").to_string()).style(TextStyle::Heading),
            ui.icon_tool(
                "x",
                t!("db.close").to_string(),
                DbMsg::Preview(path.to_path_buf(), false),
            ),
        ])
        .spacing(6.0),
        text(t!("db.preview_hint").to_string()).style(TextStyle::Muted),
    ];
    if changes.is_empty() {
        lines.push(text(t!("db.no_changes").to_string()).style(TextStyle::Muted));
    }
    for change in &changes {
        lines.push(
            text(protocol::display_change(
                change,
                &table.schema,
                &table.table,
            ))
            .style(TextStyle::Mono),
        );
    }
    scroll(column(lines).spacing(6.0)).height(280.0)
}

/// A month to pick a day in, and a time to type, as the column asks.
fn date_picker(ui: &mut Ui, path: &Path, picker: &crate::database::grid::Picker) -> Node {
    let owned = || path.to_path_buf();
    let mut out = Vec::new();
    if picker.kind.has_date() {
        let months = t!("db.months").to_string();
        let name = months
            .split(',')
            .nth(picker.month as usize - 1)
            .unwrap_or_default();
        let month = format!("{name} {}", picker.year);
        out.push(
            row([
                ui.tool("‹", DbMsg::PickerShift(owned(), -1)),
                text(month).style(TextStyle::Bold),
                ui.tool("›", DbMsg::PickerShift(owned(), 1)),
                ui.tool(t!("db.today").to_string(), DbMsg::PickerToday(owned())),
            ])
            .spacing(6.0),
        );
        // The weeks of the month, Monday first, under the names of the days.
        let weekdays = t!("db.weekdays").to_string();
        out.push(
            row(weekdays
                .split(',')
                .map(|d| text(format!("{d:>2}")).style(TextStyle::Muted)))
            .spacing(8.0),
        );
        let first = picker.first_weekday() as usize;
        let days = picker.days_in_month() as usize;
        let mut cells: Vec<Node> = (0..first).map(|_| text("   ")).collect();
        for day in 1..=days {
            let label = if picker.day == Some(day as u32) {
                format!("[{day:>2}]")
            } else {
                format!("{day:>2}")
            };
            cells.push(ui.tool(label, DbMsg::PickerDay(owned(), day as u32)));
        }
        let mut weeks = Vec::new();
        let mut it = cells.into_iter().peekable();
        while it.peek().is_some() {
            weeks.push(row(it.by_ref().take(7)).spacing(2.0));
        }
        out.push(column(weeks).spacing(2.0));
    }
    if picker.kind.has_time() {
        let field = |ui: &mut Ui, id: &str, value: &str, part: TimePart| {
            let p = owned();
            ui.input(id, value, "", move |t| {
                DbMsg::PickerTime(p.clone(), part, t)
            })
        };
        out.push(
            row([
                field(ui, "hour", &picker.hour, TimePart::Hour),
                text(":"),
                field(ui, "minute", &picker.minute, TimePart::Minute),
                text(":"),
                field(ui, "second", &picker.second, TimePart::Second),
            ])
            .spacing(4.0),
        );
    }
    let _ = PickKind::Date;
    out.push(
        row([
            ui.primary(t!("db.apply").to_string(), DbMsg::PickerApply(owned())),
            ui.tool(
                t!("db.cancel_dialog").to_string(),
                DbMsg::PickerClose(owned()),
            ),
        ])
        .spacing(6.0),
    );
    column(out).spacing(6.0)
}
