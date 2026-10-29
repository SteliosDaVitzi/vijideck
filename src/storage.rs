use rusqlite::{Connection, OptionalExtension, params};
use std::{error::Error, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub auto_refresh: bool,
    pub listeners_only: bool,
    pub protocol: usize,
    pub sort: usize,
    pub descending: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            auto_refresh: true,
            listeners_only: true,
            protocol: 0,
            sort: 0,
            descending: false,
        }
    }
}

pub struct Storage {
    connection: Connection,
}

impl Storage {
    pub fn open_cache() -> Option<Self> {
        if cfg!(feature = "ui-preview") && std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some() {
            None
        } else {
            Self::open().ok()
        }
    }

    pub fn open() -> Result<Self> {
        let dir = data_dir().ok_or("no user data directory available")?;
        std::fs::create_dir_all(&dir)?;
        Self::initialize(Connection::open(dir.join("vijideck.sqlite3"))?)
    }

    fn initialize(connection: Connection) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_millis(250))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => connection.execute_batch("BEGIN; CREATE TABLE preferences (key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL); CREATE TABLE cache (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL); PRAGMA user_version = 2; COMMIT;")?,
            1 => connection.execute_batch("BEGIN; CREATE TABLE cache (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL); PRAGMA user_version = 2; COMMIT;")?,
            2 => (),
            _ => return Err(format!("unsupported database version {version}").into()),
        }
        Ok(Self { connection })
    }

    pub fn load_cache(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT value FROM cache WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub fn save_cache(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.connection.execute("INSERT INTO cache (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, serde_json::to_string(value)?])?;
        Ok(())
    }

    pub fn load_preferences(&self) -> Result<Preferences> {
        let mut prefs = Preferences::default();
        let get = |key| -> rusqlite::Result<Option<i64>> {
            self.connection
                .query_row(
                    "SELECT value FROM preferences WHERE key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .optional()
        };
        if let Some(value @ 0..=1) = get("auto_refresh")? {
            prefs.auto_refresh = value == 1;
        }
        if let Some(value @ 0..=1) = get("listeners_only")? {
            prefs.listeners_only = value == 1;
        }
        if let Some(value @ 0..=2) = get("protocol")? {
            prefs.protocol = value as usize;
        }
        if let Some(value @ 0..=3) = get("sort")? {
            prefs.sort = value as usize;
        }
        if let Some(value @ 0..=1) = get("descending")? {
            prefs.descending = value == 1;
        }
        Ok(prefs)
    }

    pub fn save_preferences(&mut self, prefs: Preferences) -> Result<()> {
        let tx = self.connection.transaction()?;
        for (key, value) in [
            ("auto_refresh", prefs.auto_refresh as i64),
            ("listeners_only", prefs.listeners_only as i64),
            ("protocol", prefs.protocol as i64),
            ("sort", prefs.sort as i64),
            ("descending", prefs.descending as i64),
        ] {
            tx.execute("INSERT INTO preferences (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(|v| PathBuf::from(v).join("VijiDeck"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(|v| PathBuf::from(v).join("Library/Application Support/VijiDeck"))
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty() && PathBuf::from(v).is_absolute())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|v| PathBuf::from(v).join(".local/share"))
            })
            .map(|v| v.join("vijideck"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_and_validate() {
        let mut store = Storage::initialize(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(store.load_preferences().unwrap(), Preferences::default());
        let prefs = Preferences {
            auto_refresh: false,
            listeners_only: false,
            protocol: 2,
            sort: 3,
            descending: true,
        };
        store.save_preferences(prefs).unwrap();
        assert_eq!(store.load_preferences().unwrap(), prefs);
        store
            .connection
            .execute(
                "UPDATE preferences SET value = 99 WHERE key = 'protocol'",
                [],
            )
            .unwrap();
        assert_eq!(store.load_preferences().unwrap().protocol, 0);
    }
    #[test]
    fn migrates_existing_preferences_and_round_trips_cache() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE preferences (key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL); INSERT INTO preferences VALUES ('auto_refresh', 0); PRAGMA user_version = 1;").unwrap();
        let store = Storage::initialize(connection).unwrap();
        assert!(!store.load_preferences().unwrap().auto_refresh);
        let value = serde_json::json!({"paths": ["one", "two"]});
        store.save_cache("cleanup", &value).unwrap();
        assert_eq!(store.load_cache("cleanup").unwrap(), Some(value));
    }
    #[test]
    fn rejects_future_schema() {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        assert!(Storage::initialize(connection).is_err());
    }
}
