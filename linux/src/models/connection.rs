use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DatabaseKind {
    #[default]
    #[serde(rename = "mysql")]
    MySql,
    #[serde(rename = "postgres")]
    Postgres,
}

impl DatabaseKind {
    pub const ALL: [DatabaseKind; 2] = [DatabaseKind::MySql, DatabaseKind::Postgres];

    pub fn display_name(self) -> &'static str {
        match self {
            DatabaseKind::MySql => "MySQL",
            DatabaseKind::Postgres => "PostgreSQL",
        }
    }

    /// Two-letter label drawn on the connection badge.
    pub fn badge_text(self) -> &'static str {
        match self {
            DatabaseKind::MySql => "My",
            DatabaseKind::Postgres => "Pg",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            DatabaseKind::MySql => 3306,
            DatabaseKind::Postgres => 5432,
        }
    }

    pub fn default_user(self) -> &'static str {
        match self {
            DatabaseKind::MySql => "root",
            DatabaseKind::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
}

impl SslMode {
    pub const ALL: [SslMode; 3] = [SslMode::Disable, SslMode::Prefer, SslMode::Require];

    pub fn display_name(self) -> &'static str {
        match self {
            SslMode::Disable => "Disabled",
            SslMode::Prefer => "Preferred",
            SslMode::Require => "Required",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionColor {
    None,
    Gray,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ConnectionColor {
    pub const ALL: [ConnectionColor; 8] = [
        ConnectionColor::None,
        ConnectionColor::Gray,
        ConnectionColor::Red,
        ConnectionColor::Orange,
        ConnectionColor::Yellow,
        ConnectionColor::Green,
        ConnectionColor::Blue,
        ConnectionColor::Purple,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ConnectionColor::None => "none",
            ConnectionColor::Gray => "gray",
            ConnectionColor::Red => "red",
            ConnectionColor::Orange => "orange",
            ConnectionColor::Yellow => "yellow",
            ConnectionColor::Green => "green",
            ConnectionColor::Blue => "blue",
            ConnectionColor::Purple => "purple",
        }
    }

    /// Hex colour from the GNOME palette, or `None` for no tag.
    pub fn hex(self) -> Option<&'static str> {
        match self {
            ConnectionColor::None => None,
            ConnectionColor::Gray => Some("#9a9996"),
            ConnectionColor::Red => Some("#e01b24"),
            ConnectionColor::Orange => Some("#ff7800"),
            ConnectionColor::Yellow => Some("#f6d32d"),
            ConnectionColor::Green => Some("#33d17a"),
            ConnectionColor::Blue => Some("#3584e4"),
            ConnectionColor::Purple => Some("#9141ac"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionConfig {
    pub id: Uuid,
    pub name: String,
    pub kind: DatabaseKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub ssl_mode: SslMode,
    pub color: ConnectionColor,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            kind: DatabaseKind::MySql,
            host: "127.0.0.1".into(),
            port: 3306,
            user: "root".into(),
            database: String::new(),
            ssl_mode: SslMode::Prefer,
            color: ConnectionColor::None,
        }
    }
}

impl ConnectionConfig {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.host } else { &self.name }
    }

    pub fn summary(&self) -> String {
        let mut text = format!("{}@{}:{}", self.user, self.host, self.port);
        if !self.database.is_empty() {
            text.push('/');
            text.push_str(&self.database);
        }
        text
    }
}
