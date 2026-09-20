use super::connection::SqliteConnection;
use super::migration::run_migrations;
use crate::app_paths;
use anyhow::Result;
use dashmap::DashMap;
use gpui::{App, Global};
use std::any::{Any, TypeId};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::error;

pub struct StorageManager {
    conn: SqliteConnection,
    repositories: Arc<DashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

pub struct GlobalStorageState {
    pub storage: StorageManager,
}

impl Global for GlobalStorageState {}

impl Clone for GlobalStorageState {
    fn clone(&self) -> Self {
        GlobalStorageState {
            storage: self.storage.clone(),
        }
    }
}

impl StorageManager {
    pub fn new() -> Result<Self> {
        let db_path = get_db_path()?;
        migrate_legacy_companion_files(&db_path)?;
        std::fs::create_dir_all(db_path.parent().unwrap())?;
        let conn = SqliteConnection::open(&db_path)?;

        conn.with_connection(|c| {
            run_migrations(c)?;
            Ok(())
        })?;

        let manager = Self {
            conn,
            repositories: Arc::new(DashMap::new()),
        };
        Ok(manager)
    }

    pub fn connection(&self) -> SqliteConnection {
        self.conn.clone()
    }

    pub fn register<R>(&self, repo: R)
    where
        R: 'static + Send + Sync,
    {
        let type_id = TypeId::of::<R>();
        self.repositories.insert(type_id, Arc::new(repo));
    }

    pub fn get<R>(&self) -> Option<Arc<R>>
    where
        R: 'static + Send + Sync,
    {
        self.repositories
            .get(&TypeId::of::<R>())
            .and_then(|v| v.clone().downcast::<R>().ok())
    }
}

impl Clone for StorageManager {
    fn clone(&self) -> Self {
        Self {
            conn: self.conn.clone(),
            repositories: Arc::clone(&self.repositories),
        }
    }
}

pub fn get_db_path() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    let db_path = config_dir.join("myterm.db");
    let legacy_db_path = app_paths::legacy_config_dir()?.join("one-hub.db");

    if db_path.exists() {
        return Ok(db_path);
    }

    if legacy_db_path.exists() {
        std::fs::create_dir_all(&config_dir)?;
        std::fs::copy(&legacy_db_path, &db_path)?;
        tracing::info!(
            legacy_path = %legacy_db_path.display(),
            new_path = %db_path.display(),
            "Migrated legacy one-hub database path to myterm"
        );
    }

    Ok(db_path)
}

fn migrate_legacy_companion_files(db_path: &PathBuf) -> Result<()> {
    let legacy_db_path = app_paths::legacy_config_dir()?.join("one-hub.db");
    if !legacy_db_path.exists() || !db_path.exists() {
        return Ok(());
    }

    let db_metadata = std::fs::metadata(db_path)?;
    let legacy_db_metadata = std::fs::metadata(&legacy_db_path)?;
    let looks_like_migrated_legacy_db = db_metadata.len() == legacy_db_metadata.len();

    app_paths::migrate_legacy_data_file("key_storage", false)?;
    app_paths::migrate_legacy_data_file("license.json", false)?;
    app_paths::migrate_legacy_data_file("offline_license.json", false)?;

    if looks_like_migrated_legacy_db {
        app_paths::migrate_legacy_data_file("key_verification", true)?;
    } else {
        app_paths::migrate_legacy_data_file("key_verification", false)?;
    }

    Ok(())
}

pub fn get_config_dir() -> Result<PathBuf> {
    app_paths::config_dir()
}

pub fn get_queries_dir() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    let queries_dir = config_dir.join("queries");
    if !queries_dir.exists() {
        std::fs::create_dir_all(&queries_dir)?;
    }
    Ok(queries_dir)
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间不应早于 UNIX 纪元")
        .as_secs() as i64
}

pub fn init(cx: &mut App) {
    let global_storage_state = match StorageManager::new() {
        Ok(manager) => GlobalStorageState { storage: manager },
        Err(err) => {
            error!("Failed to initialize storage manager: {}", err);
            panic!("Failed to initialize storage manager: {}", err);
        }
    };
    cx.set_global(global_storage_state)
}
