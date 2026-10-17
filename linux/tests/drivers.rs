mod common;

use std::time::{Duration, Instant};

use kitedb::db::{Session, TableKind, TableRef, make_driver};
use kitedb::models::connection::DatabaseKind;

#[test]
fn mysql() {
    let Some(mut driver) = common::driver(DatabaseKind::MySql, "KITELAB_MYSQL") else { return };
    assert!(!driver.server_version().is_empty());
    driver.execute("DROP DATABASE IF EXISTS kitelab_test; CREATE DATABASE kitelab_test").unwrap();
    driver.use_database("kitelab_test").unwrap();
    assert!(driver.databases().unwrap().contains(&"kitelab_test".to_string()));
    driver
        .execute(
            "CREATE TABLE users (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(50) NOT NULL, \
               bio TEXT NULL, avatar VARBINARY(4) NULL, score DECIMAL(6,2) DEFAULT 1.5, KEY idx_name (name));
             CREATE VIEW user_names AS SELECT name FROM users;",
        )
        .unwrap();
    let results = driver
        .execute(
            "INSERT INTO users (name, bio, avatar) VALUES ('Бат', NULL, 0x0102), ('O''Neil', 'a\\\\b', NULL);
             SELECT * FROM users ORDER BY id",
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].affected_rows, Some(2));
    let rows = &results[1];
    assert_eq!(rows.columns, ["id", "name", "bio", "avatar", "score"]);
    let s = |v: &str| Some(v.to_string());
    assert_eq!(rows.rows[0], [s("1"), s("Бат"), None, s("0x0102"), s("1.50")]);
    assert_eq!(rows.rows[1][1], s("O'Neil"));
    assert_eq!(rows.rows[1][2], s("a\\b"));

    assert_eq!(driver.quote_literal("it's \\ ok"), "'it\\'s \\\\ ok'");
    let quoted = driver.query(&format!("SELECT {}", driver.quote_literal("it's \\ ok"))).unwrap();
    assert_eq!(quoted.rows[0][0], s("it's \\ ok"));

    // The order depends on the server's collation (MySQL and MariaDB differ).
    let mut tables = driver.tables(None).unwrap();
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["user_names", "users"]);
    assert_eq!(tables.iter().map(|t| t.kind).collect::<Vec<_>>(), [TableKind::View, TableKind::Table]);
    let users = TableRef::new(None, "users");
    let columns = driver.columns(&users).unwrap();
    assert_eq!(columns.iter().filter(|c| c.is_primary_key).map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id"]);
    assert_eq!(columns[4].default_value, s("1.50"));
    let indexes = driver.indexes(&users).unwrap();
    assert_eq!(indexes.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["PRIMARY", "idx_name"]);

    assert!(driver.execute("SELEC 1").is_err());
    // The connection is still usable after an error.
    assert_eq!(driver.query("SELECT 1 + 1").unwrap().rows[0][0], s("2"));
    driver.execute("DROP DATABASE kitelab_test").unwrap();
}

#[test]
fn postgres() {
    let Some(mut driver) = common::driver(DatabaseKind::Postgres, "KITELAB_PG") else { return };
    assert!(!driver.server_version().is_empty());
    driver.execute("DROP SCHEMA IF EXISTS kitelab_test CASCADE; CREATE SCHEMA kitelab_test").unwrap();
    driver
        .execute(
            "CREATE TABLE kitelab_test.users (id serial PRIMARY KEY, name text NOT NULL, tags text[], \
               meta jsonb, created timestamptz DEFAULT now(), raw bytea);
             CREATE INDEX idx_name ON kitelab_test.users (name);
             CREATE VIEW kitelab_test.user_names AS SELECT name FROM kitelab_test.users;",
        )
        .unwrap();
    let results = driver
        .execute(
            "INSERT INTO kitelab_test.users (name, tags, meta, raw) VALUES ('Бат', '{a,b}', '{\"k\": 1}', '\\x0102'), ('O''Neil', NULL, NULL, NULL);
             SELECT id, name, tags, meta, raw FROM kitelab_test.users ORDER BY id",
        )
        .unwrap();
    let s = |v: &str| Some(v.to_string());
    assert_eq!(results[0].affected_rows, Some(2));
    assert_eq!(results[1].rows[0], [s("1"), s("Бат"), s("{a,b}"), s("{\"k\": 1}"), s("\\x0102")]);
    assert_eq!(results[1].rows[1][2], None);

    // A query without rows still reports its columns.
    let empty = driver.query("SELECT id, name FROM kitelab_test.users WHERE false").unwrap();
    assert_eq!(empty.columns, ["id", "name"]);
    assert!(empty.rows.is_empty());

    assert!(driver.schemas().unwrap().contains(&"kitelab_test".to_string()));
    let tables = driver.tables(Some("kitelab_test")).unwrap();
    assert_eq!(tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["user_names", "users"]);
    let users = TableRef::new(Some("kitelab_test"), "users");
    let columns = driver.columns(&users).unwrap();
    assert_eq!(columns.iter().filter(|c| c.is_primary_key).map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id"]);
    assert_eq!(columns[1].data_type, "text");
    let indexes = driver.indexes(&users).unwrap();
    assert_eq!(indexes.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["users_pkey", "idx_name"]);

    for value in ["it's", "back\\slash 'q'"] {
        assert_eq!(driver.query(&format!("SELECT {}", driver.quote_literal(value))).unwrap().rows[0][0], s(value));
    }
    assert!(driver.execute("SELEC 1").is_err());
    assert_eq!(driver.query("SELECT 1 + 1").unwrap().rows[0][0], s("2"));
    driver.execute("DROP SCHEMA kitelab_test CASCADE").unwrap();
}

fn assert_cancels(kind: DatabaseKind, variable: &str, sleep: &'static str) {
    let Some((config, password)) = common::config(kind, variable) else { return };
    let session = Session::new(make_driver(&config, &password));
    common::block_on(session.run(|driver| driver.connect())).unwrap();
    let started = Instant::now();
    let running = session.run(move |driver| driver.execute(sleep));
    let canceller = session.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        canceller.cancel();
    });
    let result = common::block_on(running);
    assert!(started.elapsed() < Duration::from_secs(4), "cancel took {:?}", started.elapsed());
    // MySQL's KILL QUERY makes SLEEP() return 1 instead of failing.
    if let Ok(results) = result {
        assert_eq!(results[0].rows[0][0].as_deref(), Some("1"));
    }
    let after = common::block_on(session.run(|driver| driver.query("SELECT 1"))).unwrap();
    assert_eq!(after.rows[0][0].as_deref(), Some("1"));
}

#[test]
fn mysql_cancel() {
    assert_cancels(DatabaseKind::MySql, "KITELAB_MYSQL", "SELECT SLEEP(5)");
}

#[test]
fn postgres_cancel() {
    assert_cancels(DatabaseKind::Postgres, "KITELAB_PG", "SELECT pg_sleep(5)");
}
