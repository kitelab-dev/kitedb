use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

use super::connection::ConnectionConfig;

/// Saved connections, persisted as JSON in the config directory. Passwords
/// are kept separately (see `secrets`).
pub struct ConnectionStore {
    connections: Vec<ConnectionConfig>,
    path: PathBuf,
}

impl ConnectionStore {
    pub fn load() -> Self {
        let path = super::config_dir().join("connections.json");
        let connections = fs::read(&path).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default();
        Self { connections, path }
    }

    pub fn connections(&self) -> &[ConnectionConfig] {
        &self.connections
    }

    pub fn connection(&self, id: Uuid) -> Option<&ConnectionConfig> {
        self.connections.iter().find(|c| c.id == id)
    }

    pub fn save(&mut self, config: ConnectionConfig) {
        match self.connections.iter_mut().find(|c| c.id == config.id) {
            Some(existing) => *existing = config,
            None => self.connections.push(config),
        }
        self.persist();
    }

    pub fn delete(&mut self, id: Uuid) {
        self.connections.retain(|c| c.id != id);
        self.persist();
    }

    /// Adds a copy right after the original and returns it.
    pub fn duplicate(&mut self, id: Uuid) -> Option<ConnectionConfig> {
        let index = self.connections.iter().position(|c| c.id == id)?;
        let mut copy = self.connections[index].clone();
        copy.id = Uuid::new_v4();
        copy.name = format!("{} copy", self.connections[index].display_name());
        self.connections.insert(index + 1, copy.clone());
        self.persist();
        Some(copy)
    }

    /// Moves a connection up (negative) or down (positive) in the list.
    pub fn shift(&mut self, id: Uuid, offset: isize) {
        let Some(index) = self.connections.iter().position(|c| c.id == id) else { return };
        let target = index.saturating_add_signed(offset).min(self.connections.len() - 1);
        if target != index {
            let config = self.connections.remove(index);
            self.connections.insert(target, config);
            self.persist();
        }
    }

    fn persist(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let Ok(data) = serde_json::to_vec_pretty(&self.connections) else { return };
        let temp = self.path.with_extension("json.tmp");
        if fs::write(&temp, data).is_ok() {
            let _ = fs::rename(temp, &self.path);
        }
    }
}
