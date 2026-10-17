use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mysql::consts::ColumnType;
use mysql::prelude::Queryable;
use mysql::{Conn, DriverError, OptsBuilder, SslOpts, Value};

use super::{Canceller, ColumnInfo, DbError, DbResult, Driver, IndexInfo, MAX_ROWS, QueryResult, TableKind, TableRef};
use crate::models::connection::{ConnectionConfig, DatabaseKind, SslMode};

/// MySQL's "binary" character set: such columns hold raw bytes.
const BINARY_CHARSET: u16 = 63;
/// Server errors meaning the connection is gone and worth re-opening.
const CONNECTION_LOST_CODES: [u32; 3] = [2006, 2013, 4031];

#[derive(Clone)]
struct Target {
    host: String,
    port: u16,
    user: String,
    password: String,
    ssl_mode: SslMode,
}

impl Target {
    fn open(&self, database: Option<&str>) -> DbResult<Conn> {
        let ssl = self.ssl_mode != SslMode::Disable;
        match Conn::new(self.options(database, ssl)) {
            Err(mysql::Error::DriverError(DriverError::TlsNotSupported)) if self.ssl_mode == SslMode::Prefer => {
                Conn::new(self.options(database, false)).map_err(map_error)
            }
            result => result.map_err(map_error),
        }
    }

    fn options(&self, database: Option<&str>, ssl: bool) -> OptsBuilder {
        let ssl_opts = ssl.then(|| {
            // Encrypt without verifying the certificate, like libmysqlclient's
            // SSL_MODE_PREFERRED / SSL_MODE_REQUIRED.
            SslOpts::default().with_danger_accept_invalid_certs(true).with_danger_skip_domain_validation(true)
        });
        OptsBuilder::new()
            .ip_or_hostname(Some(self.host.clone()))
            .tcp_port(self.port)
            .user(Some(self.user.clone()))
            .pass(Some(self.password.clone()))
            .db_name(database)
            .prefer_socket(false)
            .tcp_connect_timeout(Some(Duration::from_secs(10)))
            .ssl_opts(ssl_opts)
    }
}

/// Kills the running statement through a second connection (`KILL QUERY`).
struct MySqlCanceller {
    target: Target,
    connection_id: Mutex<u32>,
}

impl Canceller for MySqlCanceller {
    fn cancel(&self) {
        let id = *self.connection_id.lock().unwrap();
        if id == 0 {
            return;
        }
        if let Ok(mut killer) = self.target.open(None) {
            let _ = killer.query_drop(format!("KILL QUERY {id}"));
        }
    }
}

pub struct MySqlDriver {
    target: Target,
    database: Option<String>,
    conn: Option<Conn>,
    version: String,
    canceller: Arc<MySqlCanceller>,
}

impl MySqlDriver {
    pub fn new(config: &ConnectionConfig, password: &str) -> Self {
        super::tls::install_default_provider();
        let target = Target {
            host: config.host.clone(),
            port: config.port,
            user: config.user.clone(),
            password: password.to_string(),
            ssl_mode: config.ssl_mode,
        };
        let canceller = Arc::new(MySqlCanceller { target: target.clone(), connection_id: Mutex::new(0) });
        Self {
            target,
            database: (!config.database.is_empty()).then(|| config.database.clone()),
            conn: None,
            version: String::new(),
            canceller,
        }
    }

    fn ensure_connected(&mut self) -> DbResult<&mut Conn> {
        if self.conn.is_none() {
            self.connect()?;
        }
        Ok(self.conn.as_mut().expect("connected"))
    }

    fn run(&mut self, sql: &str) -> DbResult<Vec<QueryResult>> {
        let conn = self.ensure_connected()?;
        let mut results = Vec::new();
        let mut result = conn.query_iter(sql).map_err(map_error)?;
        while let Some(set) = result.iter() {
            let columns = set.columns();
            let columns = columns.as_ref();
            if columns.is_empty() {
                let affected_rows = set.affected_rows();
                // Drain the (empty) set so the next one can be read.
                for row in set {
                    row.map_err(map_error)?;
                }
                results.push(QueryResult { affected_rows: Some(affected_rows), ..QueryResult::default() });
                continue;
            }
            let names = columns.iter().map(|c| c.name_str().into_owned()).collect();
            let binary: Vec<bool> = columns
                .iter()
                .map(|c| c.character_set() == BINARY_CHARSET && is_binary_type(c.column_type()))
                .collect();
            let mut rows = Vec::new();
            let mut truncated = false;
            for row in set {
                let row = row.map_err(map_error)?;
                if rows.len() >= MAX_ROWS {
                    truncated = true;
                    continue;
                }
                let values =
                    row.unwrap().into_iter().zip(&binary).map(|(value, &binary)| text_value(value, binary)).collect();
                rows.push(values);
            }
            results.push(QueryResult { columns: names, rows, affected_rows: None, truncated });
        }
        Ok(results)
    }
}

fn is_binary_type(column_type: ColumnType) -> bool {
    matches!(
        column_type,
        ColumnType::MYSQL_TYPE_TINY_BLOB
            | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
            | ColumnType::MYSQL_TYPE_LONG_BLOB
            | ColumnType::MYSQL_TYPE_BLOB
            | ColumnType::MYSQL_TYPE_STRING
            | ColumnType::MYSQL_TYPE_VAR_STRING
            | ColumnType::MYSQL_TYPE_VARCHAR
            | ColumnType::MYSQL_TYPE_BIT
            | ColumnType::MYSQL_TYPE_GEOMETRY
    )
}

/// The text protocol sends every value as bytes; binary ones are shown as hex.
fn text_value(value: Value, binary: bool) -> Option<String> {
    match value {
        Value::NULL => None,
        Value::Bytes(bytes) if binary => Some(hex(&bytes)),
        Value::Bytes(bytes) => {
            Some(String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
        }
        other => Some(other.as_sql(true).trim_matches('\'').to_string()),
    }
}

fn hex(bytes: &[u8]) -> String {
    const LIMIT: usize = 512;
    let mut text = String::with_capacity(2 + bytes.len().min(LIMIT) * 2);
    text.push_str("0x");
    for byte in bytes.iter().take(LIMIT) {
        let _ = write!(text, "{byte:02X}");
    }
    if bytes.len() > LIMIT {
        text.push('…');
    }
    text
}

fn map_error(error: mysql::Error) -> DbError {
    match error {
        mysql::Error::MySqlError(e) => DbError::with_code(e.to_string(), u32::from(e.code)),
        // Treated like CR_SERVER_LOST so `execute` reconnects once.
        mysql::Error::IoError(e) => DbError::with_code(e.to_string(), 2013),
        mysql::Error::CodecError(e) => DbError::with_code(format!("Lost connection to server: {e}"), 2013),
        other => DbError::new(other.to_string()),
    }
}

/// Escapes like `mysql_real_escape_string_quote` with a `'` quote.
fn escape_literal(value: &str, no_backslash_escapes: bool) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('\'');
    if no_backslash_escapes {
        text.push_str(&value.replace('\'', "''"));
    } else {
        for c in value.chars() {
            match c {
                '\0' => text.push_str("\\0"),
                '\n' => text.push_str("\\n"),
                '\r' => text.push_str("\\r"),
                '\\' => text.push_str("\\\\"),
                '\'' => text.push_str("\\'"),
                '"' => text.push_str("\\\""),
                '\x1a' => text.push_str("\\Z"),
                c => text.push(c),
            }
        }
    }
    text.push('\'');
    text
}

impl Driver for MySqlDriver {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MySql
    }

    fn server_version(&self) -> String {
        self.version.clone()
    }

    fn current_database(&self) -> Option<String> {
        self.database.clone()
    }

    fn connect(&mut self) -> DbResult<()> {
        self.close();
        let mut conn = self.target.open(self.database.as_deref())?;
        *self.canceller.connection_id.lock().unwrap() = conn.connection_id();
        self.version = conn.query_first::<String, _>("SELECT VERSION()").map_err(map_error)?.unwrap_or_default();
        self.conn = Some(conn);
        Ok(())
    }

    fn close(&mut self) {
        *self.canceller.connection_id.lock().unwrap() = 0;
        self.conn = None;
    }

    fn execute(&mut self, sql: &str) -> DbResult<Vec<QueryResult>> {
        match self.run(sql) {
            Err(error) if CONNECTION_LOST_CODES.contains(&error.code) => {
                self.connect()?;
                self.run(sql)
            }
            result => result,
        }
    }

    fn quote_literal(&self, value: &str) -> String {
        let no_backslash_escapes = self.conn.as_ref().is_some_and(Conn::no_backslash_escape);
        escape_literal(value, no_backslash_escapes)
    }

    fn canceller(&self) -> Arc<dyn Canceller> {
        self.canceller.clone()
    }

    fn databases(&mut self) -> DbResult<Vec<String>> {
        Ok(self.query("SHOW DATABASES")?.rows.into_iter().filter_map(|mut row| row.swap_remove(0)).collect())
    }

    fn use_database(&mut self, name: &str) -> DbResult<()> {
        self.ensure_connected()?.select_db(name).map_err(map_error)?;
        self.database = Some(name.to_string());
        Ok(())
    }

    fn schemas(&mut self) -> DbResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn tables(&mut self, _schema: Option<&str>) -> DbResult<Vec<TableRef>> {
        if self.database.is_none() {
            return Ok(Vec::new());
        }
        let result = self.query(
            "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME",
        )?;
        Ok(result
            .rows
            .into_iter()
            .map(|row| TableRef {
                schema: None,
                name: row[0].clone().unwrap_or_default(),
                kind: if row[1].as_deref() == Some("VIEW") { TableKind::View } else { TableKind::Table },
            })
            .collect())
    }

    fn columns(&mut self, table: &TableRef) -> DbResult<Vec<ColumnInfo>> {
        let sql = format!(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA, COLUMN_COMMENT \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = {} \
             ORDER BY ORDINAL_POSITION",
            self.quote_literal(&table.name)
        );
        Ok(self
            .query(&sql)?
            .rows
            .into_iter()
            .map(|row| ColumnInfo {
                name: row[0].clone().unwrap_or_default(),
                data_type: row[1].clone().unwrap_or_default(),
                nullable: row[2].as_deref() == Some("YES"),
                default_value: row[3].clone(),
                is_primary_key: row[4].as_deref() == Some("PRI"),
                extra: row[5].clone().unwrap_or_default(),
                comment: row[6].clone().unwrap_or_default(),
            })
            .collect())
    }

    fn indexes(&mut self, table: &TableRef) -> DbResult<Vec<IndexInfo>> {
        let sql = format!(
            "SELECT INDEX_NAME, \
                    GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ', '), \
                    MIN(NON_UNIQUE), INDEX_TYPE \
             FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = {} \
             GROUP BY INDEX_NAME, INDEX_TYPE \
             ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME",
            self.quote_literal(&table.name)
        );
        Ok(self
            .query(&sql)?
            .rows
            .into_iter()
            .map(|row| IndexInfo {
                name: row[0].clone().unwrap_or_default(),
                columns: row[1].clone().unwrap_or_default(),
                unique: row[2].as_deref() == Some("0"),
                primary: row[0].as_deref() == Some("PRIMARY"),
                definition: row[3].clone().unwrap_or_default(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_libmysqlclient() {
        assert_eq!(escape_literal("it's \\ ok", false), "'it\\'s \\\\ ok'");
        assert_eq!(escape_literal("a\nb\0\"", false), "'a\\nb\\0\\\"'");
        assert_eq!(escape_literal("it's \\", true), "'it''s \\'");
    }

    #[test]
    fn formats_binary_as_hex() {
        assert_eq!(hex(&[1, 2, 0xAB]), "0x0102AB");
        assert!(hex(&[0; 600]).ends_with('…'));
    }
}
