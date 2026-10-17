mod common;

use std::collections::BTreeSet;

use kitedb::db::{Session, TableRef, make_driver};
use kitedb::models::connection::DatabaseKind;
use kitedb::models::table::{FilterOperator, TableFilter, TableModel};

#[test]
fn staged_changes_are_saved_in_one_transaction() {
    let Some((config, password)) = common::config(DatabaseKind::MySql, "KITELAB_MYSQL") else { return };
    let session = Session::new(make_driver(&config, &password));
    common::block_on(session.run(|driver| {
        driver.connect()?;
        driver.execute(
            "DROP DATABASE IF EXISTS kitelab_model_test; CREATE DATABASE kitelab_model_test;
             USE kitelab_model_test;
             CREATE TABLE items (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(20) NOT NULL, note TEXT NULL);
             INSERT INTO items (name, note) VALUES ('a', 'x'), ('b', NULL), ('c', 'z');",
        )?;
        driver.use_database("kitelab_model_test")
    }))
    .unwrap();

    let model = TableModel::new(TableRef::new(None, "items"), session.clone());
    let names = |model: &TableModel| {
        model.state.borrow().rows.iter().map(|r| r[1].clone().unwrap_or_default()).collect::<Vec<_>>()
    };
    common::block_on(async {
        model.load().await;
        assert_eq!(model.state.borrow().error, None);
        assert!(model.state.borrow().can_edit());
        assert_eq!(model.state.borrow().rows.len(), 3);

        model.set_value(Some("a2".into()), 0, 1);
        model.set_value(None, 0, 2);
        model.set_value(Some("b".into()), 1, 1); // unchanged value: no edit
        model.delete_rows(&BTreeSet::from([2]));
        let row = model.add_row().unwrap();
        model.set_value(Some("d".into()), row, 1);
        assert_eq!(model.state.borrow().change_count(), 3);
        model.save_changes().await;
        assert_eq!(model.state.borrow().error, None);
        assert!(!model.state.borrow().has_changes());
        assert_eq!(names(&model), ["a2", "b", "d"]);
        assert_eq!(model.state.borrow().rows[0][2], None);

        // A failing statement rolls the whole batch back.
        model.set_value(Some("zzz".into()), 1, 1);
        let row = model.add_row().unwrap(); // name is NOT NULL without default -> fails
        model.set_value(None, row, 1);
        model.save_changes().await;
        assert!(model.state.borrow().error.is_some());
        model.reload().await;
        assert_eq!(names(&model), ["a2", "b", "d"]);

        model.state.borrow_mut().filter =
            TableFilter { column: "name".into(), op: FilterOperator::Contains, value: "2".into() };
        model.apply_filter().await;
        assert_eq!(names(&model), ["a2"]);
        model.state.borrow_mut().filter =
            TableFilter { column: String::new(), value: "id >= 2".into(), ..Default::default() };
        model.apply_filter().await;
        model.sort(0).await;
        model.sort(0).await;
        assert_eq!(names(&model), ["d", "b"]);
    });

    common::block_on(session.run(|driver| driver.execute("DROP DATABASE kitelab_model_test"))).unwrap();
}

#[test]
fn postgres_changes_use_schema_and_default_values() {
    let Some((config, password)) = common::config(DatabaseKind::Postgres, "KITELAB_PG") else { return };
    let session = Session::new(make_driver(&config, &password));
    common::block_on(session.run(|driver| {
        driver.connect()?;
        driver.execute(
            "DROP SCHEMA IF EXISTS kitelab_model_test CASCADE; CREATE SCHEMA kitelab_model_test;
             CREATE TABLE kitelab_model_test.items (id serial PRIMARY KEY, name text NOT NULL DEFAULT 'new', note text);
             INSERT INTO kitelab_model_test.items (name, note) VALUES ('a', 'x'), ('b', NULL);",
        )
    }))
    .unwrap();

    let model = TableModel::new(TableRef::new(Some("kitelab_model_test"), "items"), session.clone());
    common::block_on(async {
        model.load().await;
        assert_eq!(model.state.borrow().error, None);
        model.set_value(Some("it's".into()), 0, 2);
        model.add_row(); // all defaults -> INSERT ... DEFAULT VALUES
        model.delete_rows(&BTreeSet::from([1]));
        model.save_changes().await;
        assert_eq!(model.state.borrow().error, None);
        let rows = model.state.borrow().rows.clone();
        let s = |v: &str| Some(v.to_string());
        assert_eq!(rows, vec![vec![s("1"), s("a"), s("it's")], vec![s("3"), s("new"), None]]);

        model.state.borrow_mut().filter =
            TableFilter { column: "name".into(), op: FilterOperator::Contains, value: "NE".into() };
        model.apply_filter().await;
        assert_eq!(model.state.borrow().rows.len(), 1, "ILIKE is case-insensitive");
    });

    common::block_on(session.run(|driver| driver.execute("DROP SCHEMA kitelab_model_test CASCADE"))).unwrap();
}
