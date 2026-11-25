//! The settings of a data source, in tabs (General, Options, SSH / SSL,
//! Schemas, Advanced, Driver), as in the Data Sources window of a JetBrains
//! IDE; Test, Save, Duplicate, Delete.

use ardoise_plugin::ui::{column, footer, row, separator, text, ButtonStyle, Node, TextStyle};

use super::{choice, picked, Ui};
use crate::database::protocol::{Auth, Dialect, SshAuth, SslMode};
use crate::database::{
    Choice, DbMsg, Dialog, DialogTab, Driver, Field, Flag, Remember, State, TestState, Tint,
};

fn tab_label(tab: DialogTab) -> String {
    match tab {
        DialogTab::General => t!("db.tab_general"),
        DialogTab::Options => t!("db.tab_options"),
        DialogTab::SshSsl => t!("db.tab_ssh"),
        DialogTab::Schemas => t!("db.tab_schemas"),
        DialogTab::Advanced => t!("db.tab_advanced"),
        DialogTab::Driver => t!("db.tab_driver"),
    }
    .to_string()
}

/// A field of the form, by its id; secret: hidden as typed.
fn field(ui: &mut Ui, label: String, value: &str, which: Field, secret: bool) -> Node {
    let id = format!("{which:?}");
    let mut node = ui.input(&id, value, "", move |t| DbMsg::Text(which, t));
    if secret {
        node = node.secret();
    }
    labelled(label, node)
}

/// A control with its label above it (small, muted).
fn labelled(label: String, node: Node) -> Node {
    column([text(label).style(TextStyle::Muted), node]).spacing(4.0)
}

/// A path, typed or picked.
fn file(ui: &mut Ui, label: String, value: &str, which: Field) -> Node {
    let id = format!("{which:?}");
    let node = ui.input(&id, value, "", move |t| DbMsg::Text(which, t));
    let browse = ui.icon_tool(
        "folder-open",
        t!("db.browse").to_string(),
        DbMsg::Browse(which),
    );
    labelled(label, row([node, browse]).spacing(4.0))
}

fn flag(ui: &mut Ui, label: String, on: bool, which: Flag) -> Node {
    ui.checkbox(&format!("{which:?}"), label, on, move |v| {
        DbMsg::Check(which, v)
    })
}

fn hint(said: String) -> Node {
    text(said).style(TextStyle::Muted)
}

fn pick<T: Copy + PartialEq + Send + Sync + 'static>(
    ui: &mut Ui,
    id: &str,
    label: String,
    all: &'static [T],
    value: T,
    name: fn(T) -> String,
    then: fn(T) -> Choice,
) -> Node {
    let (options, chosen) = choice(all, value, name);
    let node = ui.select(id, options, chosen, move |t| match picked(all, &t, name) {
        Some(v) => DbMsg::Choose(then(v)),
        None => DbMsg::CloseDialog,
    });
    labelled(label, node)
}

pub fn view(ui: &mut Ui, state: &State, dialog: &Dialog) -> Node {
    let tabs = ui.tabs(
        "dialog_tab",
        &DialogTab::ALL,
        dialog.tab,
        tab_label,
        DbMsg::Tab,
    );
    let body = match dialog.tab {
        DialogTab::General => general(ui, state, dialog),
        DialogTab::Options => options(ui, dialog),
        DialogTab::SshSsl => ssh_ssl(ui, dialog),
        DialogTab::Schemas => schemas(ui, dialog),
        DialogTab::Advanced => advanced(ui, dialog),
        DialogTab::Driver => driver(ui, state, dialog),
    };
    let test = match &dialog.test {
        TestState::Idle => text(""),
        TestState::Running => text(t!("db.testing").to_string()).style(TextStyle::Muted),
        TestState::Ok(version) => text(t!("db.test_ok", version = version.as_str()).to_string()),
        TestState::Failed(error) => text(error.clone()).style(TextStyle::Error),
    };
    let can_test = dialog.valid() && dialog.test != TestState::Running;
    // At the left, the pictos (test, duplicate); at the right Cancel, Delete
    // (when editing) and Save.
    let mut start = vec![ui
        .icon_tool_if(
            "zap",
            t!("db.test").to_string(),
            can_test.then_some(DbMsg::Test),
        )
        .button_style(ButtonStyle::Outline)];
    let mut end = vec![ui.button(t!("db.cancel_dialog").to_string(), DbMsg::CloseDialog)];
    if let Some(id) = dialog.editing {
        start.push(ui.icon_tool("copy", t!("db.duplicate").to_string(), DbMsg::Duplicate(id)));
        let label = if dialog.confirm_delete {
            t!("db.delete_confirm")
        } else {
            t!("db.delete")
        };
        end.push(
            ui.button(label.to_string(), DbMsg::Delete)
                .button_style(ButtonStyle::Danger),
        );
    }
    end.push(
        ui.button_if(
            t!("db.save").to_string(),
            dialog.valid().then_some(DbMsg::Save),
        )
        .button_style(ButtonStyle::Primary),
    );
    column([tabs, body, test, footer(start, end)]).spacing(16.0)
}

fn general(ui: &mut Ui, state: &State, dialog: &Dialog) -> Node {
    let d = &dialog.draft;
    let mut form = Vec::new();
    if dialog.needs_password {
        form.push(text(t!("db.password_needed").to_string()).style(TextStyle::Error));
    }
    form.push(ui.tool(
        t!("db.env_import").to_string(),
        DbMsg::Browse(Field::EnvFile),
    ));
    // The kind of the data source: three tiles.
    form.push(ui.tiles(
        "dialect",
        &Dialect::ALL,
        d.dialect,
        |v| v.label().to_string(),
        |v| {
            Some(match v {
                Dialect::Sqlite | Dialect::Duckdb => "file",
                _ => "database",
            })
        },
        |v| DbMsg::Choose(Choice::Dialect(v)),
    ));
    form.push(field(
        ui,
        t!("db.field_name").to_string(),
        &d.name,
        Field::Name,
        false,
    ));
    form.push(field(
        ui,
        t!("db.field_comment").to_string(),
        &d.comment,
        Field::Comment,
        false,
    ));
    form.push(field(
        ui,
        t!("db.field_group").to_string(),
        &d.group,
        Field::Group,
        false,
    ));
    form.push(pick(
        ui,
        "color",
        t!("db.field_color").to_string(),
        &Tint::ALL,
        d.color,
        |v| {
            let name = match v {
                Tint::None => t!("db.color_none"),
                Tint::Red => t!("db.color_red"),
                Tint::Orange => t!("db.color_orange"),
                Tint::Yellow => t!("db.color_yellow"),
                Tint::Green => t!("db.color_green"),
                Tint::Blue => t!("db.color_blue"),
                Tint::Purple => t!("db.color_purple"),
            };
            format!("{} {name}", v.dot()).trim().to_string()
        },
        Choice::Tint,
    ));
    // A SQLite database is a file: nothing else is asked.
    if matches!(d.dialect, Dialect::Sqlite | Dialect::Duckdb) {
        form.push(file(
            ui,
            t!("db.field_file").to_string(),
            &d.database,
            Field::Database,
        ));
        form.push(hint(t!("db.file_hint").to_string()));
        form.push(driver_line(ui, state, d.dialect));
        return column(form).spacing(8.0);
    }
    if d.dialect == Dialect::Postgres {
        const KINDS: [bool; 2] = [false, true];
        form.push(pick(
            ui,
            "url_only",
            t!("db.conn_type").to_string(),
            &KINDS,
            d.url_only,
            |v| {
                if v {
                    t!("db.conn_url").to_string()
                } else {
                    t!("db.conn_default").to_string()
                }
            },
            Choice::UrlOnly,
        ));
    }
    if d.url_only {
        form.push(field(
            ui,
            t!("db.field_url").to_string(),
            &d.url,
            Field::Url,
            false,
        ));
    } else {
        let host = field(
            ui,
            t!("db.field_host").to_string(),
            &d.host,
            Field::Host,
            false,
        );
        let port = field(
            ui,
            t!("db.field_port").to_string(),
            &dialog.port,
            Field::Port,
            false,
        )
        .width(110.0);
        form.push(row([host, port]).spacing(12.0));
        form.push(field(
            ui,
            t!("db.field_database").to_string(),
            &d.database,
            Field::Database,
            false,
        ));
    }
    const AUTH_PG: [Auth; 3] = [Auth::UserPassword, Auth::Pgpass, Auth::NoAuth];
    const AUTH: [Auth; 2] = [Auth::UserPassword, Auth::NoAuth];
    let auth_name = |a: Auth| match a {
        Auth::UserPassword => t!("db.auth_password").to_string(),
        Auth::Pgpass => t!("db.auth_pgpass").to_string(),
        Auth::NoAuth => t!("db.auth_none").to_string(),
    };
    form.push(pick(
        ui,
        "auth",
        t!("db.auth").to_string(),
        if d.dialect == Dialect::Postgres {
            &AUTH_PG
        } else {
            &AUTH
        },
        d.auth,
        auth_name,
        Choice::Auth,
    ));
    if d.auth != Auth::NoAuth {
        let user = field(
            ui,
            t!("db.field_user").to_string(),
            &d.user,
            Field::User,
            false,
        );
        if d.auth == Auth::UserPassword {
            let password = field(
                ui,
                t!("db.field_password").to_string(),
                &dialog.password,
                Field::Password,
                true,
            );
            form.push(row([user, password]).spacing(12.0));
        } else {
            form.push(user);
        }
    }
    if d.auth == Auth::UserPassword {
        const REMEMBER: [Remember; 3] =
            [Remember::Never, Remember::UntilRestart, Remember::Forever];
        form.push(pick(
            ui,
            "remember",
            t!("db.remember").to_string(),
            &REMEMBER,
            d.remember,
            |r| match r {
                Remember::Never => t!("db.remember_never").to_string(),
                Remember::UntilRestart => t!("db.remember_session").to_string(),
                Remember::Forever => t!("db.remember_forever").to_string(),
            },
            Choice::Remember,
        ));
        if d.remember == Remember::Forever {
            form.push(hint(t!("db.remember_forever_hint").to_string()));
        }
    }
    if !d.url_only {
        form.push(
            row([
                text(t!("db.field_url").to_string()),
                text(d.jdbc_url()).style(TextStyle::Mono),
            ])
            .spacing(8.0),
        );
    }
    form.push(driver_line(ui, state, d.dialect));
    column(form).spacing(8.0)
}

/// Whether the driver of the system is there.
fn driver_line(ui: &mut Ui, state: &State, dialect: Dialect) -> Node {
    let _ = ui;
    match state.driver_of(dialect) {
        Driver::Ready => text(t!("db.driver_ready").to_string()).style(TextStyle::Muted),
        Driver::Missing => text(t!("db.driver_missing_plugin").to_string()).style(TextStyle::Error),
        _ => text(t!("db.driver_checking").to_string()).style(TextStyle::Muted),
    }
}

fn options(ui: &mut Ui, dialog: &Dialog) -> Node {
    let d = &dialog.draft;
    let timeout = if d.query_timeout == 0 {
        String::new()
    } else {
        d.query_timeout.to_string()
    };
    let script = ui
        .editor("script", &dialog.script.text(), "sql", DbMsg::ScriptText)
        .height(120.0);
    column([
        flag(
            ui,
            t!("db.opt_read_only").to_string(),
            d.read_only,
            Flag::ReadOnly,
        ),
        hint(t!("db.opt_read_only_hint").to_string()),
        flag(
            ui,
            t!("db.opt_production").to_string(),
            d.production,
            Flag::Production,
        ),
        hint(t!("db.opt_production_hint").to_string()),
        flag(
            ui,
            t!("db.opt_confirm").to_string(),
            d.confirm_runs,
            Flag::ConfirmRuns,
        ),
        hint(t!("db.opt_confirm_hint").to_string()),
        flag(
            ui,
            t!("db.opt_single").to_string(),
            d.scope.single_database,
            Flag::SingleDatabase,
        ),
        hint(t!("db.opt_single_hint").to_string()),
        field(
            ui,
            t!("db.opt_time_zone").to_string(),
            &d.time_zone,
            Field::TimeZone,
            false,
        ),
        hint(t!("db.opt_time_zone_hint").to_string()),
        field(
            ui,
            t!("db.opt_timeout").to_string(),
            &timeout,
            Field::QueryTimeout,
            false,
        ),
        hint(t!("db.opt_timeout_hint").to_string()),
        text(t!("db.opt_script").to_string()),
        script,
        hint(t!("db.opt_script_hint").to_string()),
    ])
    .spacing(8.0)
}

fn ssh_ssl(ui: &mut Ui, dialog: &Dialog) -> Node {
    let d = &dialog.draft;
    let mut form = vec![
        text(t!("db.ssh").to_string()).style(TextStyle::Heading),
        flag(
            ui,
            t!("db.ssh_use").to_string(),
            d.ssh.enabled,
            Flag::SshEnabled,
        ),
    ];
    if d.ssh.enabled {
        form.push(field(
            ui,
            t!("db.ssh_host").to_string(),
            &d.ssh.host,
            Field::SshHost,
            false,
        ));
        form.push(field(
            ui,
            t!("db.ssh_port").to_string(),
            &dialog.ssh_port,
            Field::SshPort,
            false,
        ));
        form.push(field(
            ui,
            t!("db.ssh_user").to_string(),
            &d.ssh.user,
            Field::SshUser,
            false,
        ));
        const SSH: [SshAuth; 3] = [SshAuth::Password, SshAuth::KeyPair, SshAuth::Agent];
        form.push(pick(
            ui,
            "ssh_auth",
            t!("db.ssh_auth").to_string(),
            &SSH,
            d.ssh.auth,
            |a| match a {
                SshAuth::Password => t!("db.ssh_auth_password").to_string(),
                SshAuth::KeyPair => t!("db.ssh_auth_key").to_string(),
                SshAuth::Agent => t!("db.ssh_auth_agent").to_string(),
            },
            Choice::SshAuth,
        ));
        if d.ssh.auth == SshAuth::KeyPair {
            form.push(file(
                ui,
                t!("db.ssh_key").to_string(),
                &d.ssh.key_file,
                Field::SshKey,
            ));
        }
        if d.ssh.auth != SshAuth::Agent {
            let label = if d.ssh.auth == SshAuth::Password {
                t!("db.ssh_secret_password")
            } else {
                t!("db.ssh_secret_passphrase")
            };
            form.push(field(
                ui,
                label.to_string(),
                &d.ssh_secret,
                Field::SshSecret,
                true,
            ));
        }
        form.push(field(
            ui,
            t!("db.ssh_local_port").to_string(),
            &d.ssh.local_port,
            Field::SshLocalPort,
            false,
        ));
        form.push(hint(t!("db.ssh_hint").to_string()));
    }
    if !matches!(d.dialect, Dialect::Sqlite | Dialect::Duckdb) {
        form.push(separator());
        form.push(text(t!("db.ssl").to_string()).style(TextStyle::Heading));
        const MODES: [SslMode; 5] = [
            SslMode::Disable,
            SslMode::Prefer,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ];
        form.push(pick(
            ui,
            "ssl_mode",
            t!("db.ssl_mode").to_string(),
            &MODES,
            d.ssl.mode,
            |m| match m {
                SslMode::Disable => t!("db.ssl_disable").to_string(),
                SslMode::Prefer => t!("db.ssl_prefer").to_string(),
                SslMode::Require => t!("db.ssl_require").to_string(),
                SslMode::VerifyCa => t!("db.ssl_verify_ca").to_string(),
                SslMode::VerifyFull => t!("db.ssl_verify_full").to_string(),
            },
            Choice::SslMode,
        ));
        if d.ssl.mode != SslMode::Disable {
            form.push(file(
                ui,
                t!("db.ssl_ca").to_string(),
                &d.ssl.ca_file,
                Field::SslCa,
            ));
            form.push(file(
                ui,
                t!("db.ssl_cert").to_string(),
                &d.ssl.client_cert,
                Field::SslCert,
            ));
            form.push(file(
                ui,
                t!("db.ssl_key").to_string(),
                &d.ssl.client_key,
                Field::SslKey,
            ));
        }
        form.push(hint(t!("db.ssl_hint").to_string()));
    }
    column(form).spacing(8.0)
}

fn schemas(ui: &mut Ui, dialog: &Dialog) -> Node {
    let d = &dialog.draft;
    column([
        field(
            ui,
            t!("db.schema_pattern").to_string(),
            &d.scope.pattern,
            Field::SchemaPattern,
            false,
        ),
        hint(t!("db.schema_pattern_hint").to_string()),
        field(
            ui,
            t!("db.object_filter").to_string(),
            &d.scope.object_filter,
            Field::ObjectFilter,
            false,
        ),
        hint(t!("db.object_filter_hint").to_string()),
        flag(
            ui,
            t!("db.show_system").to_string(),
            d.scope.system_schemas,
            Flag::SystemSchemas,
        ),
        flag(
            ui,
            t!("db.show_templates").to_string(),
            d.scope.template_databases,
            Flag::TemplateDatabases,
        ),
    ])
    .spacing(8.0)
}

fn advanced(ui: &mut Ui, dialog: &Dialog) -> Node {
    let mut form = vec![row([
        text(t!("db.prop_key").to_string()).style(TextStyle::Bold),
        text(t!("db.prop_value").to_string()).style(TextStyle::Bold),
    ])
    .spacing(8.0)];
    for (i, (key, value)) in dialog.draft.props.iter().enumerate() {
        form.push(
            row([
                ui.input(&format!("prop_key_{i}"), key, "", move |t| {
                    DbMsg::PropKey(i, t)
                }),
                ui.input(&format!("prop_value_{i}"), value, "", move |t| {
                    DbMsg::PropValue(i, t)
                }),
                ui.icon_tool(
                    "trash-2",
                    t!("db.dremove").to_string(),
                    DbMsg::PropRemove(i),
                ),
            ])
            .spacing(6.0),
        );
    }
    form.push(ui.tool(t!("db.prop_add").to_string(), DbMsg::PropAdd));
    form.push(hint(t!("db.props_hint").to_string()));
    column(form).spacing(8.0)
}

fn driver(ui: &mut Ui, state: &State, dialog: &Dialog) -> Node {
    let dialect = dialog.draft.dialect;
    column([
        driver_line(ui, state, dialect),
        row([
            text(t!("db.driver_protocol").to_string()),
            text(crate::database::DRIVER_PROTOCOL.to_string()),
        ])
        .spacing(8.0),
        file(
            ui,
            t!("db.driver_file").to_string(),
            &dialog.driver_file,
            Field::DriverFile,
        ),
        hint(t!("db.driver_file_hint").to_string()),
        ui.tool(
            t!("db.driver_use_bundled").to_string(),
            DbMsg::UseDownloadedDriver,
        ),
    ])
    .spacing(8.0)
}
