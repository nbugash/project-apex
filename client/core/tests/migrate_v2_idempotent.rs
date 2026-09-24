//! Migrating a database that is already current must do nothing at all.

use apex_shell::adapters::outbound::sqlite::migrate::{migrate, read_version};
use apex_shell::adapters::outbound::sqlite::schema;
use rusqlite::Connection;

fn current() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open");
    schema::apply_pragmas(&conn).expect("pragmas");
    let mut noop = |_| {};
    migrate(&mut conn, schema::CURRENT_VERSION, &mut noop).expect("migrate");
    conn
}

#[test]
fn a_second_run_is_a_no_op() {
    let mut conn = current();
    let before: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master ORDER BY name")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };

    let mut phases = Vec::new();
    migrate(&mut conn, schema::CURRENT_VERSION, &mut |p| phases.push(p)).expect("migrate again");

    assert!(
        phases.is_empty(),
        "nothing to do means nothing reported: {phases:?}"
    );
    assert_eq!(
        read_version(&conn).expect("version"),
        schema::CURRENT_VERSION
    );
    let after: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master ORDER BY name")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(before, after, "the schema must be untouched");
}

#[test]
fn a_database_from_the_future_is_refused_rather_than_downgraded() {
    let mut conn = current();
    conn.pragma_update(None, "user_version", 99i64)
        .expect("bump");
    let mut noop = |_| {};
    let outcome = migrate(&mut conn, schema::CURRENT_VERSION, &mut noop);
    assert!(
        outcome.is_err(),
        "a newer projection must not be silently rewritten"
    );
}
