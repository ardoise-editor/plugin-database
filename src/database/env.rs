//! A `.env` file read for the data source it names: `DATABASE_URL=…`, or the
//! `DB_HOST`, `DB_PORT`, `DB_DATABASE`… of the frameworks.

use super::protocol::{parse_url, Dialect, DEFAULT_PORT};

/// What a `.env` file says of a data source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imported {
    pub dialect: Dialect,
    pub host: String,
    pub port: u16,
    /// For SQLite, the path of the file.
    pub database: String,
    pub user: String,
    pub password: String,
}

/// The lines of a `.env` file as pairs (comments, `export` and quotes gone).
fn pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let mut value = value.trim();
            if let Some(quote) = ['"', '\''].into_iter().find(|q| value.starts_with(*q)) {
                // A quoted value ends with its quote; what follows is a comment.
                let inner = &value[1..];
                value = inner.split_once(quote).map_or(inner, |(v, _)| v);
            } else if let Some((before, _)) = value.split_once(" #") {
                value = before.trim_end();
            }
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

/// A URL of any of the three systems: which, and where it points to.
pub fn parse_any_url(url: &str) -> Option<Imported> {
    let url = url.trim();
    let files = [
        (Dialect::Duckdb, "duckdb:///"),
        (Dialect::Duckdb, "duckdb://"),
        (Dialect::Duckdb, "duckdb:"),
        (Dialect::Duckdb, "jdbc:duckdb:"),
        (Dialect::Sqlite, "sqlite:///"),
        (Dialect::Sqlite, "sqlite://"),
        (Dialect::Sqlite, "sqlite:"),
        (Dialect::Sqlite, "jdbc:sqlite:"),
    ];
    for (dialect, scheme) in files {
        if let Some(path) = url.strip_prefix(scheme) {
            // `sqlite:///data/a.db` is the absolute path `/data/a.db`.
            let path = if scheme.ends_with(":///") {
                format!("/{path}")
            } else {
                path.to_string()
            };
            return Some(Imported {
                dialect,
                database: path,
                ..Imported::default()
            });
        }
    }
    let (dialect, as_postgres) = if let Some(rest) = url
        .strip_prefix("jdbc:mysql://")
        .or_else(|| url.strip_prefix("jdbc:mariadb://"))
        .or_else(|| url.strip_prefix("mysql://"))
        .or_else(|| url.strip_prefix("mariadb://"))
    {
        (Dialect::Mysql, format!("postgres://{rest}"))
    } else {
        (Dialect::Postgres, url.to_string())
    };
    let parsed = parse_url(&as_postgres)?;
    let explicit = as_postgres
        .split('?')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .is_some_and(|rest| {
            rest.trim_start_matches("postgres://")
                .split('/')
                .next()
                .is_some_and(|authority| authority.rsplit_once(':').is_some())
        });
    let port = if dialect == Dialect::Mysql && !explicit && parsed.port == DEFAULT_PORT {
        Dialect::Mysql.default_port()
    } else {
        parsed.port
    };
    Some(Imported {
        dialect,
        host: parsed.host,
        port,
        database: parsed.database,
        user: parsed.user.unwrap_or_default(),
        password: parsed.password.unwrap_or_default(),
    })
}

/// The data source a `.env` file names, if it names one.
pub fn from_env(text: &str) -> Option<Imported> {
    let all = pairs(text);
    let get = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            all.iter()
                .find(|(key, value)| key == k && !value.is_empty())
                .map(|(_, v)| v.clone())
        })
    };
    if let Some(url) = get(&[
        "DATABASE_URL",
        "DB_URL",
        "DATABASE_URI",
        "POSTGRES_URL",
        "POSTGRESQL_URL",
        "MYSQL_URL",
        "MARIADB_URL",
        "SQLITE_URL",
        "DUCKDB_URL",
    ]) {
        if let Some(imported) = parse_any_url(&url) {
            return Some(imported);
        }
    }
    let dialect = match get(&["DB_CONNECTION", "DB_DRIVER", "DATABASE_TYPE"])
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "mysql" | "mariadb" => Some(Dialect::Mysql),
        "pgsql" | "postgres" | "postgresql" => Some(Dialect::Postgres),
        "sqlite" | "sqlite3" => Some(Dialect::Sqlite),
        "duckdb" => Some(Dialect::Duckdb),
        _ => None,
    };
    if let Some(file @ (Dialect::Sqlite | Dialect::Duckdb)) = dialect {
        return Some(Imported {
            dialect: file,
            database: get(&["DB_DATABASE", "DB_NAME", "DB_PATH"])?,
            ..Imported::default()
        });
    }
    let host = get(&[
        "DB_HOST",
        "DATABASE_HOST",
        "POSTGRES_HOST",
        "MYSQL_HOST",
        "PGHOST",
    ])?;
    let port: Option<u16> = get(&[
        "DB_PORT",
        "DATABASE_PORT",
        "POSTGRES_PORT",
        "MYSQL_PORT",
        "PGPORT",
    ])
    .and_then(|p| p.parse().ok());
    let dialect = dialect.unwrap_or(
        if port == Some(3306) || all.iter().any(|(k, _)| k.starts_with("MYSQL_")) {
            Dialect::Mysql
        } else {
            Dialect::Postgres
        },
    );
    Some(Imported {
        dialect,
        host,
        port: port.unwrap_or_else(|| dialect.default_port()),
        database: get(&[
            "DB_DATABASE",
            "DB_NAME",
            "DATABASE_NAME",
            "POSTGRES_DB",
            "MYSQL_DATABASE",
            "PGDATABASE",
        ])
        .unwrap_or_default(),
        user: get(&[
            "DB_USERNAME",
            "DB_USER",
            "DATABASE_USER",
            "POSTGRES_USER",
            "MYSQL_USER",
            "PGUSER",
        ])
        .unwrap_or_default(),
        password: get(&[
            "DB_PASSWORD",
            "DB_PASS",
            "DATABASE_PASSWORD",
            "POSTGRES_PASSWORD",
            "MYSQL_PASSWORD",
            "PGPASSWORD",
        ])
        .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_names_the_system_and_the_place() {
        let pg = from_env("# db\nexport DATABASE_URL=\"postgres://me:p%40ss@db.example.com:6543/shop?sslmode=require\"\n").unwrap();
        assert_eq!(pg.dialect, Dialect::Postgres);
        assert_eq!(
            (pg.host.as_str(), pg.port, pg.database.as_str()),
            ("db.example.com", 6543, "shop")
        );
        assert_eq!((pg.user.as_str(), pg.password.as_str()), ("me", "p@ss"));
        let my = from_env("DATABASE_URL=mysql://root:x@127.0.0.1/app").unwrap();
        assert_eq!((my.dialect, my.port), (Dialect::Mysql, 3306));
        let lite = from_env("DATABASE_URL=sqlite:///var/data/a.db").unwrap();
        assert_eq!(
            (lite.dialect, lite.database.as_str()),
            (Dialect::Sqlite, "/var/data/a.db")
        );
    }

    #[test]
    fn the_parts_of_a_framework_are_put_together() {
        let text = "DB_CONNECTION=mysql\nDB_HOST=db\nDB_PORT=3307\nDB_DATABASE=blog\nDB_USERNAME=u # who\nDB_PASSWORD='s e c'\n";
        let found = from_env(text).unwrap();
        assert_eq!(found.dialect, Dialect::Mysql);
        assert_eq!((found.host.as_str(), found.port), ("db", 3307));
        assert_eq!(
            (
                found.database.as_str(),
                found.user.as_str(),
                found.password.as_str()
            ),
            ("blog", "u", "s e c")
        );
        let pg = from_env("DB_HOST=localhost\nDB_DATABASE=x").unwrap();
        assert_eq!((pg.dialect, pg.port), (Dialect::Postgres, 5432));
    }

    #[test]
    fn a_duckdb_file_is_named_too() {
        let url = from_env("DATABASE_URL=duckdb:///data/a.duckdb").unwrap();
        assert_eq!(
            (url.dialect, url.database.as_str()),
            (Dialect::Duckdb, "/data/a.duckdb")
        );
        let parts = from_env("DB_CONNECTION=duckdb\nDB_DATABASE=local.duckdb").unwrap();
        assert_eq!(parts.dialect, Dialect::Duckdb);
    }

    #[test]
    fn a_file_without_a_data_source_gives_none() {
        assert!(from_env("APP_ENV=local\n# DATABASE_URL=postgres://a/b\n").is_none());
    }
}
