//! The windows of the tool, each in a tab: the table designer, the health of
//! a database, an import, a copy of a table, a backup or a restore, the ER
//! diagram; and in one tab, what waits for an answer (a drop, an SSH key, a
//! new schema), a text to read whole and the rows a row is linked to.

use ardoise_plugin::ui::{
    column, fill, footer, image, row, scroll, text, ButtonStyle, Node, TextStyle,
};

use super::Ui;
use crate::database::designer::{common_types, Designer, DesignerTab, Load, Problem, ON_DELETE};
use crate::database::import::{delimiter_name, Format, DELIMITERS, ENCODINGS};
use crate::database::protocol::{self, human_size, quote_ident, ConstraintKind, Kind};
use crate::database::{
    BackupFlag, ColField, ConField, ConfirmAction, CopyField, CopyTable, DbMsg, DesignerMsg,
    HealthTab, HealthView, IdxField, Import, State,
};

fn say(msg: DesignerMsg) -> DbMsg {
    DbMsg::Designer(msg)
}

fn heading(label: String) -> Node {
    text(label).style(TextStyle::Heading)
}

fn muted(label: String) -> Node {
    text(label).style(TextStyle::Muted)
}

// ─── The titles of the windows (the frame of the popin draws them) ─────────

pub fn designer_title(d: &Designer) -> String {
    if d.creating {
        t!("db.designer_new_title", schema = d.schema.as_str()).to_string()
    } else {
        let full = format!("{}.{}", d.schema, d.table);
        t!("db.designer_title", name = full.as_str()).to_string()
    }
}

pub fn health_title(view: &HealthView) -> String {
    t!("db.health_title", name = view.database.as_str()).to_string()
}

pub fn import_title(im: &Import) -> String {
    t!("db.import_title", table = im.table.as_str()).to_string()
}

pub fn copy_title(state: &State, copy: &CopyTable) -> String {
    let source = state
        .connections
        .iter()
        .find(|c| c.id == copy.from.0)
        .map(|c| c.label())
        .unwrap_or_default();
    t!(
        "db.copy_title",
        table = copy.from.3.as_str(),
        source = source.as_str()
    )
    .to_string()
}

pub fn backup_title(b: &crate::database::Backup) -> String {
    let scope = if !b.table.is_empty() {
        format!("{}.{}", b.schema, b.table)
    } else if !b.schema.is_empty() {
        b.schema.clone()
    } else {
        b.database.clone()
    };
    if b.restore {
        t!("db.restore_title", database = b.database.as_str()).to_string()
    } else {
        t!("db.backup_title", scope = scope.as_str()).to_string()
    }
}

/// The title of what waits for an answer, in the order `small` shows them.
pub fn small_title(state: &State) -> String {
    if let Some(c) = &state.confirm {
        let name = format!("{}.{}", c.schema, c.table);
        return match c.action {
            ConfirmAction::Drop => t!("db.confirm_drop_title", name = name.as_str()),
            ConfirmAction::Empty => t!("db.confirm_empty_title", name = name.as_str()),
        }
        .to_string();
    }
    if let Some(t) = &state.ssh_trust {
        return t!("db.ssh_trust_title", host = t.host.as_str(), port = t.port).to_string();
    }
    if state.new_schema.is_some() {
        return t!("db.new_schema_title").to_string();
    }
    if state.links.is_some() {
        return t!("db.links_title").to_string();
    }
    match &state.text {
        Some(view) => view.title.clone(),
        None => t!("db.text_title").to_string(),
    }
}

/// A control with its label above it (small, muted).
fn labelled(label: String, node: Node) -> Node {
    column([muted(label), node]).spacing(4.0)
}

fn kind_name(kind: Kind) -> String {
    match kind {
        Kind::Table => t!("db.kind_table"),
        Kind::View => t!("db.kind_view"),
        Kind::Matview => t!("db.kind_matview"),
        Kind::Foreign => t!("db.kind_foreign"),
    }
    .to_string()
}

// ─── Designer ──────────────────────────────────────────────────────────────

fn tab_label(tab: DesignerTab) -> String {
    match tab {
        DesignerTab::Columns => t!("db.dtab_columns"),
        DesignerTab::Constraints => t!("db.dtab_constraints"),
        DesignerTab::Indexes => t!("db.dtab_indexes"),
        DesignerTab::General => t!("db.dtab_general"),
        DesignerTab::Sql => t!("db.dtab_sql"),
    }
    .to_string()
}

fn problem_text(problem: &Problem) -> String {
    match problem {
        Problem::EmptyTableName => t!("db.dp_empty_table").to_string(),
        Problem::EmptySchema => t!("db.dp_empty_schema").to_string(),
        Problem::NoColumns => t!("db.dp_no_columns").to_string(),
        Problem::EmptyColumnName => t!("db.dp_empty_column").to_string(),
        Problem::DuplicateColumn(name) => t!("db.dp_duplicate", name = name.as_str()).to_string(),
        Problem::EmptyType(name) => t!("db.dp_empty_type", name = name.as_str()).to_string(),
        Problem::NeedsColumns => t!("db.dp_needs_columns").to_string(),
        Problem::NeedsReference => t!("db.dp_needs_reference").to_string(),
        Problem::NeedsExpression => t!("db.dp_needs_expression").to_string(),
        Problem::SeveralKeys => t!("db.dp_several_keys").to_string(),
        Problem::SqliteLimit => t!("db.dp_sqlite_limit").to_string(),
    }
}

fn constraint_label(kind: ConstraintKind) -> String {
    match kind {
        ConstraintKind::Primary => t!("db.dcon_primary"),
        ConstraintKind::Unique => t!("db.dcon_unique"),
        ConstraintKind::Foreign => t!("db.dcon_foreign"),
        ConstraintKind::Check => t!("db.dcon_check"),
        ConstraintKind::Other => t!("db.dcon_other"),
    }
    .to_string()
}

pub fn designer(ui: &mut Ui, d: &Designer) -> Node {
    let tabs = ui.tabs("designer_tab", &DesignerTab::ALL, d.tab, tab_label, |tab| {
        say(DesignerMsg::Tab(tab))
    });
    let content = if d.load == Load::Loading {
        muted(t!("db.designer_loading").to_string())
    } else {
        match d.tab {
            DesignerTab::Columns => columns_tab(ui, d),
            DesignerTab::Constraints => constraints_tab(ui, d),
            DesignerTab::Indexes => indexes_tab(ui, d),
            DesignerTab::General => general_tab(ui, d),
            DesignerTab::Sql => sql_tab(d),
        }
    };
    let problem = d.problem();
    let changes = d.has_changes();
    let status = match (&d.error, &problem) {
        (Some(error), _) => text(error.clone()).style(TextStyle::Error),
        (None, Some(problem)) if changes => text(problem_text(problem)).style(TextStyle::Error),
        _ if d.load == Load::Running => muted(t!("db.confirm_running").to_string()),
        _ => text(""),
    };
    let ready = d.load == Load::Ready && changes && problem.is_none();
    let apply = if d.creating {
        t!("db.dcreate")
    } else {
        t!("db.dapply")
    };
    column([
        tabs,
        scroll(content),
        status,
        footer(
            [],
            [
                ui.button(t!("db.cancel_dialog").to_string(), say(DesignerMsg::Close)),
                ui.button_if(apply.to_string(), ready.then_some(say(DesignerMsg::Apply)))
                    .button_style(ButtonStyle::Primary),
            ],
        ),
    ])
    .spacing(12.0)
}

fn columns_tab(ui: &mut Ui, d: &Designer) -> Node {
    let mut list = vec![row([
        muted(t!("db.dcol_name").to_string()),
        muted(t!("db.dcol_type").to_string()),
        muted(t!("db.dcol_notnull").to_string()),
        muted(t!("db.dcol_default").to_string()),
        muted(t!("db.dcol_comment").to_string()),
    ])
    .spacing(8.0)];
    let types: Vec<String> = common_types(d.dialect)
        .iter()
        .map(|t| t.to_string())
        .collect();
    for (i, c) in d.columns.iter().enumerate() {
        if c.dropped {
            list.push(
                row([
                    muted(c.name.clone()),
                    muted(c.kind.clone()),
                    text(t!("db.ddropped").to_string()).style(TextStyle::Error),
                    ui.icon_tool(
                        "rotate-ccw",
                        t!("db.drestore").to_string(),
                        say(DesignerMsg::ColDrop(i, false)),
                    ),
                ])
                .spacing(8.0),
            );
            continue;
        }
        let special = c
            .original
            .as_ref()
            .is_some_and(|o| o.generated || !o.identity.is_empty());
        let default = if special {
            muted(t!("db.dcol_managed").to_string())
        } else {
            ui.input(&format!("col_default_{i}"), &c.default, "", move |t| {
                say(DesignerMsg::Col(i, ColField::Default, t))
            })
        };
        list.push(
            row([
                ui.input(&format!("col_name_{i}"), &c.name, "", move |t| {
                    say(DesignerMsg::Col(i, ColField::Name, t))
                }),
                ui.input(&format!("col_type_{i}"), &c.kind, "", move |t| {
                    say(DesignerMsg::Col(i, ColField::Type, t))
                }),
                ui.select(&format!("col_types_{i}"), types.clone(), None, move |t| {
                    say(DesignerMsg::Col(i, ColField::Type, t))
                }),
                ui.checkbox(&format!("col_null_{i}"), "", !c.nullable, move |on| {
                    say(DesignerMsg::ColNull(i, !on))
                }),
                default,
                ui.input(&format!("col_comment_{i}"), &c.comment, "", move |t| {
                    say(DesignerMsg::Col(i, ColField::Comment, t))
                }),
                ui.icon_tool(
                    "trash-2",
                    t!("db.ddrop").to_string(),
                    say(DesignerMsg::ColDrop(i, true)),
                ),
            ])
            .spacing(6.0),
        );
        if c.retyped() {
            let hint = format!("\"{}\"::{}", c.name.trim(), c.kind.trim());
            list.push(
                row([
                    muted(t!("db.dcol_using").to_string()),
                    ui.input(&format!("col_using_{i}"), &c.using, hint, move |t| {
                        say(DesignerMsg::Col(i, ColField::Using, t))
                    }),
                ])
                .spacing(6.0),
            );
        }
    }
    list.push(
        ui.tool(t!("db.dcol_add").to_string(), say(DesignerMsg::ColAdd))
            .icon("plus")
            .with_label(),
    );
    list.push(muted(t!("db.dcol_order_note").to_string()));
    column(list).spacing(6.0)
}

fn constraints_tab(ui: &mut Ui, d: &Designer) -> Node {
    let mut list = Vec::new();
    for (i, c) in d.constraints.iter().enumerate() {
        if let Some(original) = c.original.as_ref().filter(|_| c.dropped || !c.editable()) {
            let style = if c.dropped {
                TextStyle::Muted
            } else {
                TextStyle::Normal
            };
            list.push(
                row([
                    text(constraint_label(c.kind)).style(TextStyle::Bold),
                    text(original.name.clone()).style(style),
                    text(original.definition.clone()).style(TextStyle::Mono),
                    if c.dropped {
                        ui.icon_tool(
                            "rotate-ccw",
                            t!("db.drestore").to_string(),
                            say(DesignerMsg::ConDrop(i, false)),
                        )
                    } else {
                        ui.icon_tool(
                            "trash-2",
                            t!("db.ddrop").to_string(),
                            say(DesignerMsg::ConDrop(i, true)),
                        )
                    },
                ])
                .spacing(8.0),
            );
            continue;
        }
        let mut line = vec![
            text(constraint_label(c.kind)).style(TextStyle::Bold),
            ui.input(
                &format!("con_name_{i}"),
                &c.name,
                t!("db.dcon_name"),
                move |t| say(DesignerMsg::Con(i, ConField::Name, t)),
            ),
        ];
        let columns = |ui: &mut Ui| {
            ui.input(
                &format!("con_cols_{i}"),
                &c.columns,
                t!("db.dcon_columns"),
                move |t| say(DesignerMsg::Con(i, ConField::Columns, t)),
            )
        };
        match c.kind {
            ConstraintKind::Primary | ConstraintKind::Unique | ConstraintKind::Other => {
                line.push(columns(ui));
            }
            ConstraintKind::Foreign => {
                line.push(columns(ui));
                line.push(ui.input(
                    &format!("con_ref_{i}"),
                    &c.reference,
                    t!("db.dcon_reference"),
                    move |t| say(DesignerMsg::Con(i, ConField::Reference, t)),
                ));
                line.push(ui.input(
                    &format!("con_refcols_{i}"),
                    &c.ref_columns,
                    t!("db.dcon_ref_columns"),
                    move |t| say(DesignerMsg::Con(i, ConField::RefColumns, t)),
                ));
                line.push(ui.select(
                    &format!("con_delete_{i}"),
                    ON_DELETE.iter().map(|s| s.to_string()).collect(),
                    Some(c.on_delete.clone()),
                    move |t| say(DesignerMsg::Con(i, ConField::OnDelete, t)),
                ));
            }
            ConstraintKind::Check => {
                line.push(ui.input(
                    &format!("con_expr_{i}"),
                    &c.expr,
                    t!("db.dcon_expr"),
                    move |t| say(DesignerMsg::Con(i, ConField::Expr, t)),
                ));
            }
        }
        let remove = if c.original.is_some() {
            t!("db.ddrop")
        } else {
            t!("db.dremove")
        };
        line.push(ui.icon_tool(
            "trash-2",
            remove.to_string(),
            say(DesignerMsg::ConDrop(i, true)),
        ));
        list.push(row(line).spacing(6.0));
    }
    list.push(
        row([
            ui.tool(
                t!("db.dcon_add_primary").to_string(),
                say(DesignerMsg::ConAdd(ConstraintKind::Primary)),
            ),
            ui.tool(
                t!("db.dcon_add_unique").to_string(),
                say(DesignerMsg::ConAdd(ConstraintKind::Unique)),
            ),
            ui.tool(
                t!("db.dcon_add_foreign").to_string(),
                say(DesignerMsg::ConAdd(ConstraintKind::Foreign)),
            ),
            ui.tool(
                t!("db.dcon_add_check").to_string(),
                say(DesignerMsg::ConAdd(ConstraintKind::Check)),
            ),
        ])
        .spacing(4.0),
    );
    column(list).spacing(6.0)
}

fn indexes_tab(ui: &mut Ui, d: &Designer) -> Node {
    let mut list = Vec::new();
    for (i, x) in d.indexes.iter().enumerate() {
        if let Some(original) = x.original.as_ref().filter(|_| x.dropped || !x.editable()) {
            list.push(
                row([
                    text(original.name.clone()).style(if x.dropped {
                        TextStyle::Muted
                    } else {
                        TextStyle::Normal
                    }),
                    text(original.definition.clone()).style(TextStyle::Mono),
                    if x.dropped {
                        ui.icon_tool(
                            "rotate-ccw",
                            t!("db.drestore").to_string(),
                            say(DesignerMsg::IdxDrop(i, false)),
                        )
                    } else {
                        ui.icon_tool(
                            "trash-2",
                            t!("db.ddrop").to_string(),
                            say(DesignerMsg::IdxDrop(i, true)),
                        )
                    },
                ])
                .spacing(8.0),
            );
            continue;
        }
        let remove = if x.original.is_some() {
            t!("db.ddrop")
        } else {
            t!("db.dremove")
        };
        list.push(
            row([
                ui.input(
                    &format!("idx_name_{i}"),
                    &x.name,
                    t!("db.dcon_name"),
                    move |t| say(DesignerMsg::Idx(i, IdxField::Name, t)),
                ),
                ui.input(
                    &format!("idx_cols_{i}"),
                    &x.columns,
                    t!("db.dcon_columns"),
                    move |t| say(DesignerMsg::Idx(i, IdxField::Columns, t)),
                ),
                ui.checkbox(
                    &format!("idx_unique_{i}"),
                    t!("db.didx_unique"),
                    x.unique,
                    move |on| say(DesignerMsg::IdxUnique(i, on)),
                ),
                ui.icon_tool(
                    "trash-2",
                    remove.to_string(),
                    say(DesignerMsg::IdxDrop(i, true)),
                ),
            ])
            .spacing(6.0),
        );
    }
    list.push(
        ui.tool(t!("db.didx_add").to_string(), say(DesignerMsg::IdxAdd))
            .icon("plus")
            .with_label(),
    );
    column(list).spacing(6.0)
}

fn general_tab(ui: &mut Ui, d: &Designer) -> Node {
    column([
        row([
            muted(t!("db.dgen_schema").to_string()),
            ui.input("table_schema", &d.schema_to, "", |t| {
                say(DesignerMsg::TableSchema(t))
            }),
        ])
        .spacing(8.0),
        row([
            muted(t!("db.dgen_name").to_string()),
            ui.input("table_name", &d.name, "", |t| {
                say(DesignerMsg::TableName(t))
            }),
        ])
        .spacing(8.0),
        row([
            muted(t!("db.dgen_comment").to_string()),
            ui.input("table_comment", &d.comment, "", |t| {
                say(DesignerMsg::TableComment(t))
            }),
        ])
        .spacing(8.0),
    ])
    .spacing(8.0)
}

fn sql_tab(d: &Designer) -> Node {
    let statements = d.statements();
    if statements.is_empty() {
        return muted(t!("db.dsql_none").to_string());
    }
    let mut list = vec![muted(t!("db.dsql_hint").to_string())];
    for statement in statements {
        list.push(text(format!("{statement};")).style(TextStyle::Mono));
    }
    column(list).spacing(6.0)
}

// ─── Health ────────────────────────────────────────────────────────────────

pub fn health(ui: &mut Ui, view: &HealthView) -> Node {
    let mut head = Vec::new();
    if let Some(data) = &view.data {
        head.push(muted(human_size(data.database_size)));
    }
    head.push(fill());
    head.push(
        ui.icon_tool_if(
            "refresh-cw",
            t!("db.refresh").to_string(),
            (!view.loading && view.running.is_none())
                .then(|| DbMsg::OpenHealth(view.conn, view.database.clone())),
        ),
    );
    let tabs = ui.tabs(
        "health_tab",
        &HealthTab::ALL,
        view.tab,
        |tab| match tab {
            HealthTab::Tables => t!("db.health_tables").to_string(),
            HealthTab::Indexes => t!("db.health_indexes").to_string(),
        },
        DbMsg::HealthTab,
    );
    let mut list = Vec::new();
    match (&view.data, view.tab) {
        (None, _) => list.push(muted(if view.loading {
            t!("db.loading").to_string()
        } else {
            String::new()
        })),
        (Some(data), HealthTab::Tables) => {
            list.push(
                row([
                    muted(t!("db.health_table").to_string()),
                    muted(t!("db.health_rows").to_string()),
                    muted(t!("db.health_size").to_string()),
                    muted(t!("db.health_idx_size").to_string()),
                    muted(t!("db.health_dead").to_string()),
                    muted(t!("db.health_vacuum").to_string()),
                    muted(t!("db.health_analyze").to_string()),
                ])
                .spacing(10.0),
            );
            for t in &data.tables {
                let qualified = format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.name));
                let dirty = t.dead_rows > 1000 && t.dead_rows * 5 > t.rows.max(1);
                let free = view.running.is_none();
                list.push(
                    row([
                        text(format!("{}.{}", t.schema, t.name)),
                        text(t.rows.to_string()),
                        text(human_size(t.total_size)),
                        text(human_size(t.indexes_size)),
                        text(t.dead_rows.to_string()).style(if dirty {
                            TextStyle::Error
                        } else {
                            TextStyle::Normal
                        }),
                        muted(t.last_vacuum.clone()),
                        muted(t.last_analyze.clone()),
                        ui.button_if(
                            "VACUUM",
                            free.then(|| DbMsg::HealthRun(format!("VACUUM {qualified}"))),
                        ),
                        ui.button_if(
                            "ANALYZE",
                            free.then(|| DbMsg::HealthRun(format!("ANALYZE {qualified}"))),
                        ),
                    ])
                    .spacing(10.0),
                );
            }
        }
        (Some(data), HealthTab::Indexes) => {
            list.push(
                row([
                    muted(t!("db.health_index").to_string()),
                    muted(t!("db.health_table").to_string()),
                    muted(t!("db.health_size").to_string()),
                    muted(t!("db.health_scans").to_string()),
                ])
                .spacing(10.0),
            );
            for i in &data.indexes {
                let mut line = vec![
                    text(i.name.clone()).style(if i.unused() {
                        TextStyle::Error
                    } else {
                        TextStyle::Normal
                    }),
                    text(format!("{}.{}", i.schema, i.table)),
                    text(human_size(i.size)),
                    text(i.scans.to_string()),
                ];
                if i.unused() {
                    line.push(text(t!("db.health_unused").to_string()).style(TextStyle::Error));
                    line.push(ui.tool(
                        t!("db.health_drop_in_console").to_string(),
                        DbMsg::HealthToConsole(format!(
                            "DROP INDEX {}.{};",
                            quote_ident(&i.schema),
                            quote_ident(&i.name)
                        )),
                    ));
                }
                list.push(row(line).spacing(10.0));
            }
        }
    }
    let mut body = vec![
        tabs,
        row(head).spacing(8.0),
        scroll(column(list).spacing(4.0)),
    ];
    if let Some(running) = &view.running {
        body.push(muted(running.clone()));
    }
    if let Some(error) = &view.error {
        body.push(text(error.clone()).style(TextStyle::Error));
    }
    column(body).spacing(12.0)
}

// ─── Import ────────────────────────────────────────────────────────────────

pub fn import(ui: &mut Ui, im: &Import) -> Node {
    let mut body = vec![row([
        ui.button_if(
            t!("db.import_choose").to_string(),
            (!im.running).then_some(DbMsg::ImportPick),
        ),
        muted(
            im.path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
    ])
    .spacing(8.0)];
    if im.path.is_some() {
        let format = |ui: &mut Ui, label: &str, which: Format| {
            let b = ui.button(label, DbMsg::ImportFormat(which));
            if im.format == which {
                b.button_style(ButtonStyle::Primary)
            } else {
                b.button_style(ButtonStyle::Ghost)
            }
        };
        let mut line = vec![
            format(ui, "CSV", Format::Csv),
            format(ui, "JSON", Format::Json),
        ];
        if im.format == Format::Csv {
            for delimiter in DELIMITERS {
                let b = ui.button(delimiter_name(delimiter), DbMsg::ImportDelimiter(delimiter));
                line.push(if im.delimiter == delimiter {
                    b.button_style(ButtonStyle::Primary)
                } else {
                    b.button_style(ButtonStyle::Ghost)
                });
            }
            line.push(ui.checkbox(
                "header",
                t!("db.import_header"),
                im.header,
                DbMsg::ImportHeader,
            ));
        }
        line.push(ui.checkbox(
            "empty_null",
            t!("db.import_empty_null"),
            im.empty_null,
            DbMsg::ImportEmptyNull,
        ));
        body.push(row(line).spacing(6.0));
        if im.format == Format::Csv {
            let mut encodings = vec![muted(t!("db.import_encoding").to_string())];
            for encoding in ENCODINGS {
                let b = ui.button(encoding.label(), DbMsg::ImportEncoding(encoding));
                let on = im.encoding == encoding
                    || (encoding == crate::encoding::Encoding::Utf8
                        && im.encoding == crate::encoding::Encoding::Utf8Bom);
                encodings.push(if on {
                    b.button_style(ButtonStyle::Primary)
                } else {
                    b.button_style(ButtonStyle::Ghost)
                });
            }
            body.push(row(encodings).spacing(4.0));
        }
        // Each field of the file goes to a column, or is left out.
        let skip = t!("db.import_skip").to_string();
        let mut choices = vec![skip.clone()];
        choices.extend(im.targets.iter().map(|t| t.name.clone()));
        let mut map = vec![row([
            muted(t!("db.import_field").to_string()),
            muted(t!("db.import_example").to_string()),
            muted(t!("db.import_column").to_string()),
        ])
        .spacing(10.0)];
        for (i, name) in im.sample.names.iter().enumerate() {
            let example = im
                .sample
                .rows
                .first()
                .and_then(|r| r.get(i))
                .cloned()
                .unwrap_or_default();
            let chosen = im
                .mapping
                .get(i)
                .cloned()
                .flatten()
                .unwrap_or_else(|| skip.clone());
            let skip_label = skip.clone();
            map.push(
                row([
                    text(name.clone()).style(TextStyle::Bold),
                    muted(example),
                    ui.select(
                        &format!("map_{i}"),
                        choices.clone(),
                        Some(chosen),
                        move |c| DbMsg::ImportMap(i, (c != skip_label).then_some(c)),
                    ),
                ])
                .spacing(10.0),
            );
        }
        body.push(scroll(column(map).spacing(4.0)).height(260.0));
    }
    if let Some(error) = &im.error {
        body.push(text(error.clone()).style(TextStyle::Error));
    }
    if let Some(n) = im.imported {
        body.push(text(t!("db.import_done", n = n).to_string()));
    }
    let ready = im.path.is_some() && im.mapping.iter().any(Option::is_some) && !im.running;
    if im.running {
        body.push(muted(t!("db.import_running").to_string()));
    }
    body.push(footer(
        [],
        [
            ui.button(t!("db.cancel_dialog").to_string(), DbMsg::ImportClose),
            ui.button_if(
                t!("db.import_run").to_string(),
                ready.then_some(DbMsg::ImportRun),
            )
            .button_style(ButtonStyle::Primary),
        ],
    ));
    column(body).spacing(12.0)
}

// ─── Copy of a table ───────────────────────────────────────────────────────

pub fn copy(ui: &mut Ui, state: &State, copy: &CopyTable) -> Node {
    let names: Vec<(u64, String)> = state
        .connections
        .iter()
        .map(|c| (c.id, c.label()))
        .collect();
    let chosen = copy
        .dest
        .and_then(|id| names.iter().find(|(i, _)| *i == id).map(|(_, l)| l.clone()));
    let options: Vec<String> = names.iter().map(|(_, l)| l.clone()).collect();
    let field = |ui: &mut Ui, label: String, value: &str, hint: String, which: CopyField| {
        labelled(
            label,
            ui.input(&format!("copy_{which:?}"), value, hint, move |t| {
                DbMsg::CopyField(which, t)
            }),
        )
    };
    let ready = copy.dest.is_some() && !copy.name.trim().is_empty() && !copy.running;
    let mut body = vec![
        labelled(
            t!("db.copy_to").to_string(),
            ui.select("copy_dest", options, chosen, move |label| {
                DbMsg::CopyDest(
                    names
                        .iter()
                        .find(|(_, l)| *l == label)
                        .map_or(0, |(i, _)| *i),
                )
            }),
        ),
        field(
            ui,
            t!("db.copy_database").to_string(),
            &copy.database,
            t!("db.copy_database_hint").to_string(),
            CopyField::Database,
        ),
        field(
            ui,
            t!("db.copy_schema").to_string(),
            &copy.schema,
            String::new(),
            CopyField::Schema,
        ),
        field(
            ui,
            t!("db.copy_name").to_string(),
            &copy.name,
            String::new(),
            CopyField::Name,
        ),
        ui.checkbox(
            "copy_data",
            t!("db.copy_with_data"),
            copy.with_data,
            DbMsg::CopyData,
        ),
        muted(t!("db.copy_hint").to_string()),
    ];
    if let Some(error) = &copy.error {
        body.push(text(error.clone()).style(TextStyle::Error));
    }
    if let Some(rows) = copy.copied {
        body.push(text(t!("db.copy_done", n = rows).to_string()));
    }
    if copy.running {
        body.push(muted(t!("db.copy_running").to_string()));
    }
    body.push(footer(
        [],
        [
            ui.button(t!("db.cancel_dialog").to_string(), DbMsg::CopyClose),
            ui.button_if(
                t!("db.copy_run").to_string(),
                ready.then_some(DbMsg::CopyRun),
            )
            .button_style(ButtonStyle::Primary),
        ],
    ));
    column(body).spacing(12.0)
}

// ─── Backup and restore ────────────────────────────────────────────────────

pub fn backup(ui: &mut Ui, b: &crate::database::Backup) -> Node {
    let flag = |ui: &mut Ui, label: String, on: bool, which: BackupFlag| {
        ui.checkbox(&format!("backup_{which:?}"), label, on, move |v| {
            DbMsg::BackupFlag(which, v)
        })
    };
    let postgres = b.dialect == protocol::Dialect::Postgres;
    let mut options = Vec::new();
    if !b.restore && postgres {
        options.push(flag(
            ui,
            t!("db.backup_plain").to_string(),
            b.spec.format == "plain",
            BackupFlag::Plain,
        ));
        options.push(flag(
            ui,
            t!("db.backup_schema_only").to_string(),
            b.spec.schema_only,
            BackupFlag::SchemaOnly,
        ));
        options.push(flag(
            ui,
            t!("db.backup_data_only").to_string(),
            b.spec.data_only,
            BackupFlag::DataOnly,
        ));
    }
    if postgres {
        options.push(flag(
            ui,
            t!("db.backup_no_owner").to_string(),
            b.spec.no_owner,
            BackupFlag::NoOwner,
        ));
        options.push(flag(
            ui,
            t!("db.backup_no_privileges").to_string(),
            b.spec.no_privileges,
            BackupFlag::NoPrivileges,
        ));
    }
    if b.restore && postgres {
        options.push(flag(
            ui,
            t!("db.restore_clean").to_string(),
            b.spec.clean,
            BackupFlag::Clean,
        ));
        options.push(flag(
            ui,
            t!("db.restore_single").to_string(),
            b.spec.single_transaction,
            BackupFlag::SingleTransaction,
        ));
    }
    let mut body = vec![
        column(options).spacing(6.0),
        row([
            ui.button_if(
                t!("db.import_choose").to_string(),
                (!b.running).then_some(DbMsg::BackupPick),
            ),
            muted(
                b.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
        ])
        .spacing(8.0),
    ];
    if b.restore && b.spec.clean {
        body.push(text(t!("db.restore_warning_clean").to_string()).style(TextStyle::Error));
    }
    if let Some(error) = &b.error {
        body.push(text(error.clone()).style(TextStyle::Error));
    }
    if let Some(said) = &b.done {
        body.push(text(if b.restore {
            t!("db.restore_done", kind = said.as_str()).to_string()
        } else {
            t!("db.backup_done", size = said.as_str()).to_string()
        }));
    }
    let run = if b.restore {
        t!("db.restore_run")
    } else {
        t!("db.backup_run")
    };
    if b.running {
        body.push(muted(t!("db.backup_running").to_string()));
    }
    body.push(footer(
        [],
        [
            ui.button(t!("db.cancel_dialog").to_string(), DbMsg::BackupClose),
            ui.button_if(
                run.to_string(),
                (b.path.is_some() && !b.running).then_some(DbMsg::BackupRun),
            )
            .button_style(ButtonStyle::Primary),
        ],
    ));
    column(body).spacing(12.0)
}

// ─── What waits for an answer, a text, the linked rows ─────────────────────

/// What waits for an answer, one at a time (the next shows when this one is
/// answered): a drop, an SSH key, a new schema, the rows linked, a text.
pub fn small(ui: &mut Ui, state: &State) -> Node {
    if let Some(c) = &state.confirm {
        let name = format!("{}.{}", c.schema, c.table);
        let (message, action) = match c.action {
            ConfirmAction::Drop => (
                t!("db.confirm_drop_text", kind = kind_name(c.kind).as_str()).to_string(),
                t!("db.confirm_drop_button").to_string(),
            ),
            ConfirmAction::Empty => (
                t!("db.confirm_empty_text").to_string(),
                t!("db.confirm_empty_button").to_string(),
            ),
        };
        let _ = name;
        let mut out = vec![muted(message), text(c.statement()).style(TextStyle::Mono)];
        if c.action == ConfirmAction::Drop && c.dialect == protocol::Dialect::Postgres {
            out.push(ui.checkbox(
                "cascade",
                t!("db.confirm_cascade"),
                c.cascade,
                DbMsg::ConfirmCascade,
            ));
        }
        if let Some(error) = &c.error {
            out.push(text(error.clone()).style(TextStyle::Error));
        }
        if c.running {
            out.push(muted(t!("db.confirm_running").to_string()));
        }
        out.push(footer(
            [],
            [
                ui.button(t!("db.guard_cancel").to_string(), DbMsg::ConfirmClose),
                ui.button_if(action, (!c.running).then_some(DbMsg::ConfirmRun))
                    .button_style(ButtonStyle::Danger),
            ],
        ));
        return column(out).spacing(12.0);
    }
    if let Some(t) = &state.ssh_trust {
        let mut out = vec![muted(t!("db.ssh_trust_text").to_string())];
        for key in &t.keys {
            out.push(text(key.fingerprint.clone()).style(TextStyle::Mono));
        }
        if t.scanning {
            out.push(muted(t!("db.ssh_scanning").to_string()));
        }
        if let Some(error) = &t.error {
            out.push(text(error.clone()).style(TextStyle::Error));
        }
        out.push(footer(
            [],
            [
                ui.button(t!("db.guard_cancel").to_string(), DbMsg::SshTrustNo),
                ui.button_if(
                    t!("db.ssh_trust_yes").to_string(),
                    (!t.keys.is_empty()).then_some(DbMsg::SshTrustYes),
                )
                .button_style(ButtonStyle::Primary),
            ],
        ));
        return column(out).spacing(12.0);
    }
    if let Some(n) = &state.new_schema {
        let field = ui.input(
            "schema_name",
            &n.name,
            t!("db.new_schema_name"),
            DbMsg::SchemaName,
        );
        let mut out = vec![ui.submit(field, DbMsg::SchemaCreate)];
        if let Some(error) = &n.error {
            out.push(text(error.clone()).style(TextStyle::Error));
        }
        out.push(footer(
            [],
            [
                ui.button(t!("db.guard_cancel").to_string(), DbMsg::SchemaClose),
                ui.button_if(
                    t!("db.dcreate").to_string(),
                    (!n.running && !n.name.trim().is_empty()).then_some(DbMsg::SchemaCreate),
                )
                .button_style(ButtonStyle::Primary),
            ],
        ));
        return column(out).spacing(12.0);
    }
    let mut out = Vec::new();
    if let Some(view) = &state.links {
        for (outgoing, title, hint) in [
            (
                true,
                t!("db.links_out").to_string(),
                t!("db.links_out_hint").to_string(),
            ),
            (
                false,
                t!("db.links_in").to_string(),
                t!("db.links_in_hint").to_string(),
            ),
        ] {
            let group: Vec<_> = view
                .links
                .iter()
                .filter(|l| l.outgoing == outgoing)
                .collect();
            if group.is_empty() {
                continue;
            }
            out.push(text(title).style(TextStyle::Bold));
            out.push(muted(hint));
            for link in group {
                let name = if link.schema == "public" || link.schema == view.schema {
                    link.table.clone()
                } else {
                    format!("{}.{}", link.schema, link.table)
                };
                out.push(ui.tool(
                    format!("{name}  {}", crate::database::ddl::short_label(&link.label)),
                    DbMsg::OpenRelated(
                        view.conn,
                        view.database.clone(),
                        link.schema.clone(),
                        link.table.clone(),
                        link.filter.clone(),
                    ),
                ));
            }
        }
        return column(out).spacing(8.0);
    }
    if let Some(view) = &state.text {
        out.push(scroll(text(view.text.clone()).style(TextStyle::Mono)).height(320.0));
        out.push(muted(
            t!("db.text_size", n = view.text.chars().count()).to_string(),
        ));
        out.push(footer(
            [
                ui.icon_tool("copy", t!("db.text_copy").to_string(), DbMsg::TextCopy)
                    .button_style(ButtonStyle::Outline),
            ],
            [],
        ));
    }
    column(out).spacing(12.0)
}

// ─── Diagram ───────────────────────────────────────────────────────────────

pub fn diagram(ui: &mut Ui, shown: &crate::database::diagram::Shown) -> Node {
    column([
        row([
            heading(t!("db.diagram_title", schema = shown.title.as_str()).to_string()),
            ui.tool(t!("db.diagram_mermaid").to_string(), DbMsg::CopyDiagram),
        ])
        .spacing(8.0),
        scroll(image(crate::database::diagram::svg(
            &shown.diagram,
            ui.dark,
        ))),
    ])
    .spacing(8.0)
    .padding(12.0)
}
