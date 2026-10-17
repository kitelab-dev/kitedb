//! Live-server helpers. Configure with environment variables; a test is
//! skipped when its server variable is missing:
//!   KITELAB_MYSQL=host:port:user:password
//!   KITELAB_PG=host:port:user:password

use kitedb::db::{Driver, make_driver};
use kitedb::models::connection::{ConnectionConfig, DatabaseKind, SslMode};

pub fn config(kind: DatabaseKind, variable: &str) -> Option<(ConnectionConfig, String)> {
    let Ok(spec) = std::env::var(variable) else {
        eprintln!("skipped: {variable} not set");
        return None;
    };
    let parts: Vec<&str> = spec.split(':').collect();
    let config = ConnectionConfig {
        kind,
        host: parts[0].to_string(),
        port: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(kind.default_port()),
        user: parts.get(2).unwrap_or(&kind.default_user()).to_string(),
        ssl_mode: SslMode::Prefer,
        ..ConnectionConfig::default()
    };
    Some((config, parts.get(3).unwrap_or(&"").to_string()))
}

#[allow(dead_code)] // not every test file uses it
pub fn driver(kind: DatabaseKind, variable: &str) -> Option<Box<dyn Driver>> {
    let (config, password) = config(kind, variable)?;
    let mut driver = make_driver(&config, &password);
    driver.connect().expect("connect");
    Some(driver)
}

pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    gtk::glib::MainContext::new().block_on(future)
}
