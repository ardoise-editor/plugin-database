//! The Database panel of the sidebar: the data sources, their databases,
//! schemas, tables (and what a table is made of), functions, sequences and
//! types; under the tree, what can be done to the node picked.

use ardoise_plugin::ui::{column, row, scroll, separator, text, tree, Node, TextStyle, TreeItem};

use super::Ui;
use crate::app::App;
use crate::database::protocol::Kind;
use crate::database::{DbMsg, Picked, Tree};

fn glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Table => "▦",
        Kind::View => "◫",
        Kind::Matview => "◨",
        Kind::Foreign => "▤",
    }
}

pub fn view(ui: &mut Ui, app: &App) -> Node {
    let state = &app.db;
    let mut header = vec![
        text(t!("sidebar.database").to_uppercase()).style(TextStyle::Muted),
        ui.tool(t!("db.console").to_string(), DbMsg::ToggleConsole),
        ui.tool(format!("+ {}", t!("db.add")), DbMsg::OpenDialog(None)),
    ];
    if state.connections.is_empty() {
        header.push(text(""));
        return column([
            row(header).spacing(4.0),
            text(t!("db.empty").to_string()).style(TextStyle::Muted),
            ui.primary(t!("db.add").to_string(), DbMsg::OpenDialog(None)),
        ])
        .spacing(8.0)
        .padding(8.0);
    }
    let needle = state.tree_filter.trim().to_lowercase();
    let filtering = !needle.is_empty();
    let hit = |name: &str| !filtering || name.to_lowercase().contains(&needle);
    let is_open = |key: &str| filtering || state.open.contains(key);
    let picked = state.tree_picked.clone();
    let mut roots: Vec<TreeItem> = Vec::new();
    for connection in &state.connections {
        let id = connection.id;
        let key = id.to_string();
        let open = state.open.contains(&key)
            || (filtering && matches!(state.trees.get(&id), Some(Tree::Loaded(_))));
        let mut children: Vec<TreeItem> = Vec::new();
        let note = |ui: &mut Ui, label: String, msg: DbMsg| ui.tree_item(label, msg);
        match state.trees.get(&id) {
            None | Some(Tree::Loading) if open => {
                children.push(note(ui, t!("db.loading").to_string(), DbMsg::Reload(id)))
            }
            None | Some(Tree::Loading) => {}
            Some(Tree::Failed(error)) => {
                children.push(note(ui, error.clone(), DbMsg::OpenDialog(Some(id))).icon("⚠"))
            }
            Some(Tree::Loaded(databases)) if databases.iter().all(|d| d.schemas.is_empty()) => {
                children.push(note(ui, t!("db.no_tables").to_string(), DbMsg::Reload(id)))
            }
            Some(Tree::Loaded(databases)) => {
                let several = databases.len() > 1;
                for database in databases {
                    let database_key = format!("{id}/{}", database.name);
                    let mut schemas: Vec<TreeItem> = Vec::new();
                    for schema in &database.schemas {
                        let tables: Vec<_> =
                            schema.tables.iter().filter(|t| hit(&t.name)).collect();
                        let routines: Vec<_> =
                            schema.routines.iter().filter(|r| hit(&r.name)).collect();
                        let sequences: Vec<_> =
                            schema.sequences.iter().filter(|n| hit(n)).collect();
                        let types: Vec<_> = schema.types.iter().filter(|n| hit(n)).collect();
                        if filtering
                            && tables.is_empty()
                            && routines.is_empty()
                            && sequences.is_empty()
                            && types.is_empty()
                        {
                            continue;
                        }
                        let schema_key = format!("{database_key}/{}", schema.name);
                        let mut items: Vec<TreeItem> = Vec::new();
                        for table in tables {
                            let table_key = (
                                id,
                                database.name.clone(),
                                schema.name.clone(),
                                table.name.clone(),
                            );
                            // What the table is made of, read when it opens.
                            let shown: Vec<TreeItem> = match state.structures.get(&table_key) {
                                Some(structure) => structure_lines(structure)
                                    .into_iter()
                                    .map(|(g, label)| {
                                        ui.tree_item(
                                            label,
                                            DbMsg::Picked(Picked::Table(
                                                id,
                                                database.name.clone(),
                                                schema.name.clone(),
                                                table.name.clone(),
                                                table.kind,
                                            )),
                                        )
                                        .icon(g)
                                    })
                                    .collect(),
                                None => Vec::new(),
                            };
                            let line = ui
                                .tree_item(
                                    table.name.clone(),
                                    DbMsg::Picked(Picked::Table(
                                        id,
                                        database.name.clone(),
                                        schema.name.clone(),
                                        table.name.clone(),
                                        table.kind,
                                    )),
                                )
                                .icon(glyph(table.kind));
                            let toggle = DbMsg::ToggleTable(
                                id,
                                database.name.clone(),
                                schema.name.clone(),
                                table.name.clone(),
                            );
                            ui.on_toggle(&line, toggle);
                            let line = line
                                .expandable()
                                .expanded(
                                    state.open.contains(&format!("{schema_key}/{}", table.name)),
                                )
                                .children(shown);
                            items.push(line);
                        }
                        // Functions, sequences and types, each in a folder.
                        let folder =
                            |ui: &mut Ui, name: &str, label: String, rows: Vec<TreeItem>| {
                                let key = format!("{schema_key}/{name}");
                                ui.tree_item(
                                    format!("{label} ({})", rows.len()),
                                    DbMsg::Toggle(key.clone()),
                                )
                                .expandable()
                                .expanded(is_open(&key))
                                .children(rows)
                            };
                        if !routines.is_empty() {
                            let rows = routines
                                .iter()
                                .map(|r| {
                                    ui.tree_item(
                                        format!("{}({})", r.name, r.args),
                                        DbMsg::ShowFunction(
                                            id,
                                            database.name.clone(),
                                            schema.name.clone(),
                                            r.name.clone(),
                                            r.args.clone(),
                                        ),
                                    )
                                    .icon("ƒ")
                                })
                                .collect();
                            items.push(folder(
                                ui,
                                "routines",
                                t!("db.tree_routines").to_string(),
                                rows,
                            ));
                        }
                        if !sequences.is_empty() {
                            let rows = sequences
                                .iter()
                                .map(|n| {
                                    ui.tree_item(n.to_string(), DbMsg::Toggle(schema_key.clone()))
                                        .icon("#")
                                })
                                .collect();
                            items.push(folder(
                                ui,
                                "sequences",
                                t!("db.tree_sequences").to_string(),
                                rows,
                            ));
                        }
                        if !types.is_empty() {
                            let rows = types
                                .iter()
                                .map(|n| {
                                    ui.tree_item(n.to_string(), DbMsg::Toggle(schema_key.clone()))
                                        .icon("τ")
                                })
                                .collect();
                            items.push(folder(ui, "types", t!("db.tree_types").to_string(), rows));
                        }
                        let line = ui
                            .tree_item(
                                schema.name.clone(),
                                DbMsg::Picked(Picked::Schema(
                                    id,
                                    database.name.clone(),
                                    schema.name.clone(),
                                )),
                            )
                            .icon("◇");
                        ui.on_toggle(&line, DbMsg::Toggle(schema_key.clone()));
                        schemas.push(
                            line.expandable()
                                .expanded(is_open(&schema_key))
                                .children(items),
                        );
                    }
                    if several {
                        let line = ui
                            .tree_item(
                                database.name.clone(),
                                DbMsg::Picked(Picked::Database(id, database.name.clone())),
                            )
                            .icon("⛁");
                        ui.on_toggle(&line, DbMsg::Toggle(database_key.clone()));
                        children.push(
                            line.expandable()
                                .expanded(is_open(&database_key))
                                .children(schemas),
                        );
                    } else {
                        children.extend(schemas);
                    }
                }
            }
        }
        let line = ui
            .tree_item(connection.label(), DbMsg::Picked(Picked::Connection(id)))
            .icon("⛃");
        ui.on_toggle(&line, DbMsg::Toggle(key.clone()));
        roots.push(line.expandable().expanded(open).children(children));
    }
    let filter = ui.input(
        "tree_filter",
        &state.tree_filter,
        t!("db.tree_filter"),
        DbMsg::TreeFilter,
    );
    let mut body = vec![
        row(header).spacing(4.0),
        filter,
        scroll(tree("tree", roots)),
    ];
    if let Some(picked) = &picked {
        body.push(separator());
        body.push(actions(ui, app, picked));
    }
    column(body).spacing(6.0).padding(8.0)
}

/// What can be done to the node picked (the right-click menus of the tree).
fn actions(ui: &mut Ui, app: &App, picked: &Picked) -> Node {
    let mut out = Vec::new();
    match picked.clone() {
        Picked::Connection(id) => {
            out.push(ui.tool(t!("db.refresh").to_string(), DbMsg::Reload(id)));
            out.push(ui.tool(t!("db.edit").to_string(), DbMsg::OpenDialog(Some(id))));
            out.push(ui.tool(t!("db.duplicate").to_string(), DbMsg::Duplicate(id)));
            out.push(ui.tool(t!("db.console").to_string(), DbMsg::ConsoleConnection(id)));
        }
        Picked::Database(id, database) => {
            out.push(ui.tool(
                t!("db.new_schema").to_string(),
                DbMsg::NewSchema(id, database.clone()),
            ));
            out.push(ui.tool(
                t!("db.health").to_string(),
                DbMsg::OpenHealth(id, database.clone()),
            ));
            out.push(ui.tool(
                t!("db.backup").to_string(),
                DbMsg::BackupOpen(id, database.clone(), String::new(), String::new(), false),
            ));
            out.push(ui.tool(
                t!("db.restore").to_string(),
                DbMsg::BackupOpen(id, database, String::new(), String::new(), true),
            ));
        }
        Picked::Schema(id, database, schema) => {
            out.push(ui.tool(
                t!("db.new_table").to_string(),
                DbMsg::NewTable(id, database.clone(), schema.clone()),
            ));
            out.push(ui.tool(
                t!("db.diagram").to_string(),
                DbMsg::ShowDiagram(id, database.clone(), schema.clone()),
            ));
            out.push(ui.tool(
                t!("db.new_schema").to_string(),
                DbMsg::NewSchema(id, database.clone()),
            ));
            out.push(ui.tool(
                t!("db.health").to_string(),
                DbMsg::OpenHealth(id, database.clone()),
            ));
            out.push(ui.tool(
                t!("db.backup").to_string(),
                DbMsg::BackupOpen(id, database.clone(), schema.clone(), String::new(), false),
            ));
            out.push(ui.tool(
                t!("db.restore").to_string(),
                DbMsg::BackupOpen(id, database, schema, String::new(), true),
            ));
        }
        Picked::Table(id, database, schema, table, kind) => {
            let (a, b, c, d) = (id, database.clone(), schema.clone(), table.clone());
            out.push(ui.primary(
                t!("db.table_open").to_string(),
                DbMsg::OpenTable(a, b.clone(), c.clone(), d.clone()),
            ));
            out.push(ui.tool(
                t!("db.table_structure").to_string(),
                DbMsg::ToggleTable(a, b.clone(), c.clone(), d.clone()),
            ));
            if kind == Kind::Table {
                out.push(ui.tool(
                    t!("db.table_modify").to_string(),
                    DbMsg::OpenDesigner(a, b.clone(), c.clone(), d.clone()),
                ));
                out.push(ui.tool(
                    t!("db.table_ddl").to_string(),
                    DbMsg::ShowDdl(a, b.clone(), c.clone(), d.clone()),
                ));
                out.push(ui.tool(
                    t!("db.import").to_string(),
                    DbMsg::ImportOpen(a, b.clone(), c.clone(), d.clone()),
                ));
                out.push(ui.tool(
                    t!("db.copy_table").to_string(),
                    DbMsg::CopyOpen(a, b.clone(), c.clone(), d.clone()),
                ));
                out.push(ui.tool(
                    t!("db.backup").to_string(),
                    DbMsg::BackupOpen(a, b.clone(), c.clone(), d.clone(), false),
                ));
                out.push(ui.tool(
                    t!("db.table_empty").to_string(),
                    DbMsg::AskEmpty(a, b.clone(), c.clone(), d.clone()),
                ));
            } else {
                out.push(ui.tool(
                    t!("db.table_ddl").to_string(),
                    DbMsg::ShowDefinition(a, b.clone(), c.clone(), d.clone(), kind),
                ));
            }
            let drop = match kind {
                Kind::Table => t!("db.table_drop"),
                Kind::View => t!("db.view_drop"),
                Kind::Matview => t!("db.matview_drop"),
                Kind::Foreign => t!("db.foreign_drop"),
            };
            out.push(
                ui.button(drop.to_string(), DbMsg::AskDrop(a, b, c, d, kind))
                    .button_style(ardoise_plugin::ui::ButtonStyle::Danger),
            );
        }
    }
    let _ = app;
    // Several lines of buttons: the panel is narrow.
    let mut lines = Vec::new();
    let mut it = out.into_iter().peekable();
    while it.peek().is_some() {
        lines.push(row(it.by_ref().take(2)).spacing(4.0));
    }
    column(lines).spacing(4.0)
}

/// What a table is made of, as lines of the tree.
fn structure_lines(s: &crate::database::protocol::Structure) -> Vec<(&'static str, String)> {
    use crate::database::designer::{list, parse_constraint};
    use crate::database::protocol::ConstraintKind;
    let columns_of = |kind: ConstraintKind| -> Vec<String> {
        s.constraints
            .iter()
            .filter(|c| c.kind == kind)
            .filter_map(|c| parse_constraint(kind, &c.definition))
            .flat_map(|p| list(&p.columns))
            .collect()
    };
    let (keys, foreign) = (
        columns_of(ConstraintKind::Primary),
        columns_of(ConstraintKind::Foreign),
    );
    let mut out = Vec::new();
    for c in &s.columns {
        let glyph = if keys.contains(&c.name) {
            "⚷"
        } else if foreign.contains(&c.name) {
            "→"
        } else {
            "·"
        };
        out.push((
            glyph,
            format!(
                "{}  {}{}",
                c.name,
                c.kind,
                if c.nullable { "" } else { "  NOT NULL" }
            ),
        ));
    }
    for i in &s.indexes {
        out.push((if i.unique { "◆" } else { "◇" }, i.name.clone()));
    }
    for c in s
        .constraints
        .iter()
        .filter(|c| c.kind != ConstraintKind::Primary)
    {
        let glyph = match c.kind {
            ConstraintKind::Unique => "◆",
            ConstraintKind::Foreign => "→",
            ConstraintKind::Check => "✓",
            _ => "·",
        };
        out.push((glyph, c.name.clone()));
    }
    out
}
