//! Database access: a blocking, single-connection `Driver` per server kind
//! and a `Session` that runs it on its own thread.

mod mysql;
mod postgres;
mod tls;

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::models::connection::{ConnectionConfig, DatabaseKind};

pub use self::mysql::MySqlDriver;
pub use self::postgres::PostgresDriver;

/// Rows kept per result; the rest is dropped and `truncated` is set.
pub const MAX_ROWS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    pub message: String,
    /// Native error code (MySQL errno, unused for Postgres).
    pub code: u32,
}

impl DbError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_code(message, 0)
    }

    pub fn with_code(message: impl Into<String>, code: u32) -> Self {
        Self { message: message.into().trim().to_string(), code }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DbError {}

pub type DbResult<T> = Result<T, DbError>;

/// One result of a statement. Values are kept in the server's text
/// representation, which is exactly what a client UI needs to display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows affected by INSERT/UPDATE/DELETE, when the server reports it.
    pub affected_rows: Option<u64>,
    /// True when more rows were returned than `MAX_ROWS`.
    pub truncated: bool,
}

impl QueryResult {
    pub fn has_result_set(&self) -> bool {
        !self.columns.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
}

impl TableRef {
    pub fn new(schema: Option<&str>, name: &str) -> Self {
        Self { schema: schema.map(str::to_string), name: name.to_string(), kind: TableKind::Table }
    }

    pub fn id(&self) -> String {
        format!("{}.{}", self.schema.as_deref().unwrap_or(""), self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub extra: String,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    pub name: String,
    pub columns: String,
    pub unique: bool,
    pub primary: bool,
    pub definition: String,
}

/// Stops the statement a driver is running. Safe to call from any thread.
pub trait Canceller: Send + Sync {
    fn cancel(&self);
}

/// A blocking, single-connection database driver. Every call happens on the
/// owning `Session` thread; only the `Canceller` is shared.
pub trait Driver: Send {
    fn kind(&self) -> DatabaseKind;
    fn server_version(&self) -> String;
    /// The database the connection currently points to.
    fn current_database(&self) -> Option<String>;

    fn connect(&mut self) -> DbResult<()>;
    fn close(&mut self);
    /// Runs one or more statements and returns every result produced.
    fn execute(&mut self, sql: &str) -> DbResult<Vec<QueryResult>>;
    fn quote_literal(&self, value: &str) -> String;
    fn canceller(&self) -> Arc<dyn Canceller>;

    fn databases(&mut self) -> DbResult<Vec<String>>;
    fn use_database(&mut self, name: &str) -> DbResult<()>;
    fn schemas(&mut self) -> DbResult<Vec<String>>;
    fn tables(&mut self, schema: Option<&str>) -> DbResult<Vec<TableRef>>;
    fn columns(&mut self, table: &TableRef) -> DbResult<Vec<ColumnInfo>>;
    fn indexes(&mut self, table: &TableRef) -> DbResult<Vec<IndexInfo>>;

    /// Runs a query and returns the last result that carried rows (or the
    /// last result overall).
    fn query(&mut self, sql: &str) -> DbResult<QueryResult> {
        let mut results = self.execute(sql)?;
        let index = results.iter().rposition(QueryResult::has_result_set);
        Ok(match index {
            Some(index) => results.swap_remove(index),
            None => results.pop().unwrap_or_default(),
        })
    }
}

impl DatabaseKind {
    pub fn quote_identifier(self, name: &str) -> String {
        match self {
            DatabaseKind::MySql => format!("`{}`", name.replace('`', "``")),
            DatabaseKind::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    pub fn qualified_name(self, table: &TableRef) -> String {
        match (&table.schema, self) {
            (Some(schema), DatabaseKind::Postgres) => {
                format!("{}.{}", self.quote_identifier(schema), self.quote_identifier(&table.name))
            }
            _ => self.quote_identifier(&table.name),
        }
    }
}

pub fn make_driver(config: &ConnectionConfig, password: &str) -> Box<dyn Driver> {
    match config.kind {
        DatabaseKind::MySql => Box::new(MySqlDriver::new(config, password)),
        DatabaseKind::Postgres => Box::new(PostgresDriver::new(config, password)),
    }
}

type Job = Box<dyn FnOnce(&mut dyn Driver) + Send>;

/// Owns a driver on a private thread and runs work on it one job at a time,
/// so blocking network calls never run on the GTK main thread.
pub struct Session {
    kind: DatabaseKind,
    jobs: mpsc::Sender<Job>,
    canceller: Arc<dyn Canceller>,
}

impl Session {
    pub fn new(driver: Box<dyn Driver>) -> Arc<Self> {
        let kind = driver.kind();
        let canceller = driver.canceller();
        let (jobs, receiver) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("kitedb-session".into())
            .spawn(move || {
                let mut driver = driver;
                for job in receiver {
                    job(driver.as_mut());
                }
                driver.close();
            })
            .expect("spawn database session thread");
        Arc::new(Self { kind, jobs, canceller })
    }

    pub fn kind(&self) -> DatabaseKind {
        self.kind
    }

    /// Queues `work` on the session thread; the future resolves on whichever
    /// executor awaits it (the GTK main loop in the app).
    pub fn run<T, F>(&self, work: F) -> impl Future<Output = DbResult<T>> + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&mut dyn Driver) -> DbResult<T> + Send + 'static,
    {
        let (sender, receiver) = futures_channel::oneshot::channel();
        let queued = self.jobs.send(Box::new(move |driver: &mut dyn Driver| {
            let _ = sender.send(work(driver));
        }));
        async move {
            if queued.is_err() {
                return Err(DbError::new("The connection is closed"));
            }
            receiver.await.unwrap_or_else(|_| Err(DbError::new("The connection is closed")))
        }
    }

    /// Asks the server to stop the running statement, without waiting.
    pub fn cancel(&self) {
        let canceller = self.canceller.clone();
        thread::spawn(move || canceller.cancel());
    }

    /// Closes the connection once already queued work has finished.
    pub fn close(&self) {
        let _ = self.jobs.send(Box::new(|driver: &mut dyn Driver| driver.close()));
    }
}
