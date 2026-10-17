use std::error::Error as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use postgres::config::SslMode as PgSslMode;
use postgres::{CancelToken, Client, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::{Canceller, ColumnInfo, DbError, DbResult, Driver, IndexInfo, MAX_ROWS, QueryResult, TableKind, TableRef};
use crate::models::connection::{ConnectionConfig, DatabaseKind, SslMode};

struct PostgresCanceller {
    token: Mutex<Option<CancelToken>>,
    tls: MakeRustlsConnect,
}

impl Canceller for PostgresCanceller {
    fn cancel(&self) {
        let token = self.token.lock().unwrap().clone();
        if let Some(token) = token {
            let _ = token.cancel_query(self.tls.clone());
        }
    }
}

pub struct PostgresDriver {
    config: postgres::Config,
    database: String,
    client: Option<Client>,
    version: String,
    tls: MakeRustlsConnect,
    canceller: Arc<PostgresCanceller>,
}

impl PostgresDriver {
    pub fn new(config: &ConnectionConfig, password: &str) -> Self {
        let mut pg = postgres::Config::new();
        pg.host(&config.host)
            .port(config.port)
            .user(&config.user)
            .password(password)
            .ssl_mode(match config.ssl_mode {
                SslMode::Disable => PgSslMode::Disable,
                SslMode::Prefer => PgSslMode::Prefer,
                SslMode::Require => PgSslMode::Require,
            })
            .connect_timeout(Duration::from_secs(10))
            .application_name("KiteDB");
        let tls = super::tls::postgres_connector();
        Self {
            config: pg,
            database: if config.database.is_empty() { "postgres".into() } else { config.database.clone() },
            client: None,
            version: String::new(),
            canceller: Arc::new(PostgresCanceller { token: Mutex::new(None), tls: tls.clone() }),
            tls,
        }
    }

    fn ensure_connected(&mut self) -> DbResult<&mut Client> {
        if self.client.as_ref().is_none_or(Client::is_closed) {
            self.connect()?;
        }
        Ok(self.client.as_mut().expect("connected"))
    }

    fn relation_oid(&self, table: &TableRef) -> String {
        format!(
            "(SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relname = {})",
            self.quote_literal(table.schema.as_deref().unwrap_or("public")),
            self.quote_literal(&table.name)
        )
    }
}

fn map_error(error: postgres::Error) -> DbError {
    if let Some(db) = error.as_db_error() {
        let mut text = format!("{}:  {}", db.severity(), db.message());
        if let Some(detail) = db.detail() {
            text.push_str(&format!("\nDETAIL:  {detail}"));
        }
        if let Some(hint) = db.hint() {
            text.push_str(&format!("\nHINT:  {hint}"));
        }
        return DbError::new(text);
    }
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    DbError::new(text)
}

fn escape_literal(value: &str) -> String {
    let quoted = value.replace('\'', "''");
    if value.contains('\\') { format!("E'{}'", quoted.replace('\\', "\\\\")) } else { format!("'{quoted}'") }
}

impl Driver for PostgresDriver {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Postgres
    }

    fn server_version(&self) -> String {
        self.version.clone()
    }

    fn current_database(&self) -> Option<String> {
        Some(self.database.clone())
    }

    fn connect(&mut self) -> DbResult<()> {
        self.close();
        let mut config = self.config.clone();
        config.dbname(&self.database);
        let mut client = config.connect(self.tls.clone()).map_err(map_error)?;
        self.version = client
            .simple_query("SHOW server_version")
            .map_err(map_error)?
            .into_iter()
            .find_map(|message| match message {
                SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
                _ => None,
            })
            .unwrap_or_default();
        *self.canceller.token.lock().unwrap() = Some(client.cancel_token());
        self.client = Some(client);
        Ok(())
    }

    fn close(&mut self) {
        *self.canceller.token.lock().unwrap() = None;
        self.client = None;
    }

    fn execute(&mut self, sql: &str) -> DbResult<Vec<QueryResult>> {
        let messages = self.ensure_connected()?.simple_query(sql).map_err(map_error)?;
        let mut results = Vec::new();
        // A row-returning statement sends RowDescription, rows, then
        // CommandComplete; any other statement only CommandComplete.
        let mut current: Option<QueryResult> = None;
        for message in messages {
            match message {
                SimpleQueryMessage::RowDescription(columns) => {
                    current = Some(QueryResult {
                        columns: columns.iter().map(|c| c.name().to_string()).collect(),
                        ..QueryResult::default()
                    });
                }
                SimpleQueryMessage::Row(row) => {
                    let result = current.get_or_insert_with(|| QueryResult {
                        columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                        ..QueryResult::default()
                    });
                    if result.rows.len() >= MAX_ROWS {
                        result.truncated = true;
                        continue;
                    }
                    result.rows.push((0..row.len()).map(|i| row.get(i).map(str::to_string)).collect());
                }
                SimpleQueryMessage::CommandComplete(count) => match current.take() {
                    Some(result) => results.push(result),
                    None => results.push(QueryResult { affected_rows: Some(count), ..QueryResult::default() }),
                },
                _ => {}
            }
        }
        results.extend(current);
        Ok(results)
    }

    fn quote_literal(&self, value: &str) -> String {
        escape_literal(value)
    }

    fn canceller(&self) -> Arc<dyn Canceller> {
        self.canceller.clone()
    }

    fn databases(&mut self) -> DbResult<Vec<String>> {
        let result =
            self.query("SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname")?;
        Ok(result.rows.into_iter().filter_map(|mut row| row.swap_remove(0)).collect())
    }

    fn use_database(&mut self, name: &str) -> DbResult<()> {
        if name == self.database {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.database, name.to_string());
        if let Err(error) = self.connect() {
            self.database = previous;
            let _ = self.connect();
            return Err(error);
        }
        Ok(())
    }

    fn schemas(&mut self) -> DbResult<Vec<String>> {
        let result = self.query(
            "SELECT nspname FROM pg_namespace \
             WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
             ORDER BY nspname = 'public' DESC, nspname",
        )?;
        Ok(result.rows.into_iter().filter_map(|mut row| row.swap_remove(0)).collect())
    }

    fn tables(&mut self, schema: Option<&str>) -> DbResult<Vec<TableRef>> {
        let schema = schema.unwrap_or("public").to_string();
        let sql = format!(
            "SELECT c.relname, c.relkind FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relkind IN ('r', 'p', 'f', 'v', 'm') \
             ORDER BY c.relname",
            self.quote_literal(&schema)
        );
        Ok(self
            .query(&sql)?
            .rows
            .into_iter()
            .map(|row| TableRef {
                schema: Some(schema.clone()),
                name: row[0].clone().unwrap_or_default(),
                kind: match row[1].as_deref() {
                    Some("v" | "m") => TableKind::View,
                    _ => TableKind::Table,
                },
            })
            .collect())
    }

    fn columns(&mut self, table: &TableRef) -> DbResult<Vec<ColumnInfo>> {
        let sql = format!(
            "SELECT a.attname, \
                    format_type(a.atttypid, a.atttypmod), \
                    NOT a.attnotnull, \
                    pg_get_expr(d.adbin, d.adrelid), \
                    EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey)), \
                    CASE WHEN a.attidentity <> '' THEN 'identity' WHEN a.attgenerated <> '' THEN 'generated' ELSE '' END, \
                    col_description(a.attrelid, a.attnum) \
             FROM pg_attribute a \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = {} AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            self.relation_oid(table)
        );
        Ok(self
            .query(&sql)?
            .rows
            .into_iter()
            .map(|row| ColumnInfo {
                name: row[0].clone().unwrap_or_default(),
                data_type: row[1].clone().unwrap_or_default(),
                nullable: row[2].as_deref() == Some("t"),
                default_value: row[3].clone(),
                is_primary_key: row[4].as_deref() == Some("t"),
                extra: row[5].clone().unwrap_or_default(),
                comment: row[6].clone().unwrap_or_default(),
            })
            .collect())
    }

    fn indexes(&mut self, table: &TableRef) -> DbResult<Vec<IndexInfo>> {
        let sql = format!(
            "SELECT i.relname, \
                    (SELECT string_agg(pg_get_indexdef(ix.indexrelid, k, true), ', ' ORDER BY k) \
                     FROM generate_series(1, ix.indnkeyatts) AS k), \
                    ix.indisunique, ix.indisprimary, pg_get_indexdef(ix.indexrelid) \
             FROM pg_index ix \
             JOIN pg_class i ON i.oid = ix.indexrelid \
             WHERE ix.indrelid = {} \
             ORDER BY ix.indisprimary DESC, i.relname",
            self.relation_oid(table)
        );
        Ok(self
            .query(&sql)?
            .rows
            .into_iter()
            .map(|row| IndexInfo {
                name: row[0].clone().unwrap_or_default(),
                columns: row[1].clone().unwrap_or_default(),
                unique: row[2].as_deref() == Some("t"),
                primary: row[3].as_deref() == Some("t"),
                definition: row[4].clone().unwrap_or_default(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::escape_literal;

    #[test]
    fn escapes_like_libpq() {
        assert_eq!(escape_literal("it's"), "'it''s'");
        assert_eq!(escape_literal("a\\b'"), "E'a\\\\b'''");
    }
}
