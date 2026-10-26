use anyhow::Result;
use gpui::{App, SharedString};
use rusqlite::params;

use crate::storage::connection::SqliteConnection;
use crate::storage::manager::{GlobalStorageState, now};
use crate::storage::quick_command::QuickCommandRepository;
use crate::storage::row_mapping::FromSqliteRow;
use crate::storage::sftp_favorite_path::SftpFavoritePathRepository;
use crate::storage::traits::Repository;
use crate::storage::{ConnectionType, StoredConnection, Workspace};

struct ConnectionRow {
    id: i64,
    name: String,
    connection_type: String,
    params: String,
    workspace_id: Option<i64>,
    selected_databases: Option<String>,
    remark: Option<String>,
    cloud_id: Option<String>,
    last_used_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    team_id: Option<String>,
    owner_id: Option<String>,
    env: Option<String>,
    tags: Option<String>,
    is_favorite: bool,
}

const CONNECTION_SELECT_FIELDS: &str = "id, name, connection_type, params, workspace_id, selected_databases, remark, cloud_id, last_used_at, created_at, updated_at, team_id, owner_id, env, tags, is_favorite";

impl FromSqliteRow for ConnectionRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ConnectionRow {
            id: row.get("id")?,
            name: row.get("name")?,
            connection_type: row.get("connection_type")?,
            params: row.get("params")?,
            workspace_id: row.get("workspace_id")?,
            selected_databases: row.get("selected_databases")?,
            remark: row.get("remark")?,
            cloud_id: row.get("cloud_id")?,
            last_used_at: row.get("last_used_at")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            team_id: row.get("team_id").unwrap_or(None),
            owner_id: row.get("owner_id").unwrap_or(None),
            env: row.get("env").unwrap_or(None),
            tags: row.get("tags").unwrap_or(None),
            is_favorite: row
                .get::<_, i64>("is_favorite")
                .map(|v| v != 0)
                .unwrap_or(false),
        })
    }
}

impl From<ConnectionRow> for StoredConnection {
    fn from(row: ConnectionRow) -> Self {
        let mut conn = StoredConnection {
            id: Some(row.id),
            name: row.name,
            connection_type: ConnectionType::from_str(&row.connection_type),
            params: row.params,
            workspace_id: row.workspace_id,
            selected_databases: row.selected_databases,
            remark: row.remark,
            cloud_id: row.cloud_id,
            last_used_at: row.last_used_at,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
            team_id: row.team_id,
            owner_id: row.owner_id,
            env: row.env,
            tags: row
                .tags
                .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()),
            is_favorite: row.is_favorite,
        };
        // 从数据库读取后自动解密敏感字段
        conn.params = conn.decrypt_params();
        conn
    }
}

struct WorkspaceRow {
    id: i64,
    name: String,
    color: Option<String>,
    icon: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl FromSqliteRow for WorkspaceRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(WorkspaceRow {
            id: row.get("id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            icon: row.get("icon")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl From<WorkspaceRow> for Workspace {
    fn from(row: WorkspaceRow) -> Self {
        Workspace {
            id: Some(row.id),
            name: row.name,
            color: row.color,
            icon: row.icon,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
        }
    }
}

#[derive(Clone)]
pub struct ConnectionRepository {
    conn: SqliteConnection,
}

impl ConnectionRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }
}

impl Repository for ConnectionRepository {
    type Entity = StoredConnection;

    fn entity_type(&self) -> SharedString {
        SharedString::from("Connection")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let name = item.name.clone();
        let connection_type = item.connection_type.to_string();
        let params_str = item.encrypt_params();
        let workspace_id = item.workspace_id;
        let selected_databases = item.selected_databases.clone();
        let remark = item.remark.clone();
        let cloud_id = item.cloud_id.clone();
        let team_id = item.team_id.clone();
        let owner_id = item.owner_id.clone();
        let env = item.env.clone();
        let tags = item
            .tags
            .as_ref()
            .map(|t| serde_json::to_string(t).unwrap_or_default());
        let is_favorite = if item.is_favorite { 1i64 } else { 0i64 };
        let ts = now();

        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO connections (name, connection_type, params, workspace_id, selected_databases, remark, cloud_id, team_id, owner_id, env, tags, is_favorite, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![name, connection_type, params_str, workspace_id, selected_databases, remark, cloud_id, team_id, owner_id, env, tags, is_favorite, ts, ts],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        item.id = Some(id);
        item.created_at = Some(ts);
        item.updated_at = Some(ts);

        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item
            .id
            .ok_or_else(|| anyhow::anyhow!("Cannot update without ID"))?;
        let name = item.name.clone();
        let connection_type = item.connection_type.to_string();
        let params_str = item.encrypt_params();
        let workspace_id = item.workspace_id;
        let selected_databases = item.selected_databases.clone();
        let remark = item.remark.clone();
        let cloud_id = item.cloud_id.clone();
        let team_id = item.team_id.clone();
        let owner_id = item.owner_id.clone();
        let env = item.env.clone();
        let tags = item
            .tags
            .as_ref()
            .map(|t| serde_json::to_string(t).unwrap_or_default());
        let is_favorite = if item.is_favorite { 1i64 } else { 0i64 };
        let ts = now();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET name = ?1, connection_type = ?2, params = ?3, workspace_id = ?4, selected_databases = ?5, remark = ?6, cloud_id = ?7, team_id = ?8, owner_id = ?9, env = ?10, tags = ?11, is_favorite = ?12, updated_at = ?13 WHERE id = ?14",
                params![name, connection_type, params_str, workspace_id, selected_databases, remark, cloud_id, team_id, owner_id, env, tags, is_favorite, ts, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute("DELETE FROM connections WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let sql = format!("SELECT {CONNECTION_SELECT_FIELDS} FROM connections WHERE id = ?1");
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ConnectionRow::from_row(row)?.into()))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let sql = format!("SELECT {CONNECTION_SELECT_FIELDS} FROM connections ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM connections", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

impl ConnectionRepository {
    pub fn list_by_workspace(&self, workspace_id: Option<i64>) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let sql = if workspace_id.is_some() {
                format!("SELECT {CONNECTION_SELECT_FIELDS} FROM connections WHERE workspace_id = ?1 ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC")
            } else {
                format!("SELECT {CONNECTION_SELECT_FIELDS} FROM connections WHERE workspace_id IS NULL ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC")
            };
            let mut stmt = conn.prepare(&sql)?;

            let mut results = Vec::new();
            if let Some(wid) = workspace_id {
                let rows = stmt.query_map(params![wid], |row| ConnectionRow::from_row(row))?;
                for row in rows {
                    results.push(row?.into());
                }
            } else {
                let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
                for row in rows {
                    results.push(row?.into());
                }
            }
            Ok(results)
        })
    }

    /// 记录连接最近一次被打开的时间，不影响内容更新时间和云同步判断。
    pub fn touch_last_used(&self, id: i64) -> Result<()> {
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET last_used_at = ?1 WHERE id = ?2",
                params![ts, id],
            )?;
            Ok(())
        })
    }

    /// 按团队 ID 查询连接
    pub fn list_by_team(&self, team_id: &str) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let sql = format!("SELECT {CONNECTION_SELECT_FIELDS} FROM connections WHERE team_id = ?1 ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![team_id], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    /// 查询个人连接（team_id 为 NULL）
    pub fn list_personal(&self) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let sql = format!("SELECT {CONNECTION_SELECT_FIELDS} FROM connections WHERE team_id IS NULL ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    /// 切换收藏状态
    pub fn toggle_favorite(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let current: i64 = conn.query_row(
                "SELECT is_favorite FROM connections WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            let next = if current == 0 { 1i64 } else { 0i64 };
            conn.execute(
                "UPDATE connections SET is_favorite = ?1 WHERE id = ?2",
                params![next, id],
            )?;
            Ok(next == 1)
        })
    }
}

#[derive(Clone)]
pub struct WorkspaceRepository {
    conn: SqliteConnection,
}

impl WorkspaceRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }
}

impl Repository for WorkspaceRepository {
    type Entity = Workspace;

    fn entity_type(&self) -> SharedString {
        SharedString::from("Workspace")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let name = item.name.clone();
        let color = item.color.clone();
        let icon = item.icon.clone();
        let ts = now();

        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO workspaces (name, color, icon, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name, color, icon, ts, ts],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        item.id = Some(id);
        item.created_at = Some(ts);
        item.updated_at = Some(ts);

        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item
            .id
            .ok_or_else(|| anyhow::anyhow!("Cannot update without ID"))?;
        let name = item.name.clone();
        let color = item.color.clone();
        let icon = item.icon.clone();
        let ts = now();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE workspaces SET name = ?1, color = ?2, icon = ?3, updated_at = ?4 WHERE id = ?5",
                params![name, color, icon, ts, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET workspace_id = NULL WHERE workspace_id = ?1",
                params![id],
            )?;
            conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, color, icon, created_at, updated_at FROM workspaces WHERE id = ?1")?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(WorkspaceRow::from_row(row)?.into()))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, color, icon, created_at, updated_at FROM workspaces ORDER BY updated_at DESC")?;
            let rows = stmt.query_map([], |row| WorkspaceRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ConnectionRepository;
    use crate::storage::StoredConnection;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::models::{SshAuthMethod, SshParams};
    use crate::storage::traits::Repository;
    use rusqlite::params;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_repository() -> (SqliteConnection, ConnectionRepository) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "myterm-connection-repository-{}-{unique}-{counter}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("run migrations");
        let repo = ConnectionRepository::new(conn.clone());
        (conn, repo)
    }

    fn ssh_connection(name: &str) -> StoredConnection {
        StoredConnection::new_ssh(
            name.to_string(),
            SshParams {
                host: format!("{name}.example.com"),
                port: 22,
                username: "deploy".to_string(),
                auth_method: SshAuthMethod::Agent,
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
            },
            None,
        )
    }

    #[test]
    fn list_orders_by_recent_use_without_touching_updated_at() {
        let (conn, repo) = test_repository();
        let mut old_connection = ssh_connection("old");
        let old_id = repo.insert(&mut old_connection).unwrap();
        let mut new_connection = ssh_connection("new");
        let new_id = repo.insert(&mut new_connection).unwrap();

        conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET created_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![1000i64, old_id],
            )?;
            conn.execute(
                "UPDATE connections SET created_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![2000i64, new_id],
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            Some(new_id),
            repo.list().unwrap().first().and_then(|c| c.id)
        );

        repo.touch_last_used(old_id).unwrap();
        let listed = repo.list().unwrap();

        assert_eq!(Some(old_id), listed.first().and_then(|c| c.id));
        let (updated_at, last_used_at): (i64, Option<i64>) = conn
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT updated_at, last_used_at FROM connections WHERE id = ?1",
                    params![old_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!(1000, updated_at);
        assert!(last_used_at.is_some());
    }
}

pub fn init(cx: &mut App) {
    let storage_state = cx.global::<GlobalStorageState>();
    let storage = storage_state.storage.clone();

    let conn = storage.connection();
    let conn_repo = ConnectionRepository::new(conn.clone());
    let workspace_repo = WorkspaceRepository::new(conn.clone());
    let quick_cmd_repo = QuickCommandRepository::new(conn.clone());
    let sftp_favorite_path_repo = SftpFavoritePathRepository::new(conn.clone());

    storage.register(workspace_repo);
    storage.register(conn_repo);
    storage.register(quick_cmd_repo);
    storage.register(sftp_favorite_path_repo);
}
