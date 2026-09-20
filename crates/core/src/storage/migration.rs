use anyhow::Result;
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "20260225000001",
        include_str!("../../migrations/20260225000001_init.sql"),
    ),
    (
        "20260315000001",
        include_str!("../../migrations/20260315000001_team_sync.sql"),
    ),
    (
        "20260317000001",
        include_str!("../../migrations/20260317000001_connection_owner.sql"),
    ),
    (
        "20260610000001",
        include_str!("../../migrations/20260610000001_sftp_favorite_paths.sql"),
    ),
    (
        "20260618000001",
        include_str!("../../migrations/20260618000001_connection_last_used.sql"),
    ),
    (
        "20260823000001",
        include_str!("../../migrations/20260823000001_remove_unused_tables.sql"),
    ),
];

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );",
    )?;

    for (version, sql) in MIGRATIONS {
        let applied: i64 = conn.query_row(
            "SELECT COUNT(*) FROM _migrations WHERE version = ?1",
            [version],
            |row| row.get(0),
        )?;

        if applied == 0 {
            if let Err(e) = conn.execute_batch(sql) {
                let err_msg = e.to_string();
                if err_msg.contains("duplicate column name") {
                    tracing::warn!(
                        "Migration {} skipped (column already exists): {}",
                        version,
                        err_msg
                    );
                } else {
                    return Err(e.into());
                }
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as i64;

            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, now],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run_migrations;
    use rusqlite::Connection;

    #[test]
    fn removes_unused_legacy_tables() {
        let conn = Connection::open_in_memory().expect("open in-memory database");

        run_migrations(&conn).expect("run migrations");

        for table in ["queries", "llm_providers", "chat_sessions", "chat_messages"] {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                    )",
                    [table],
                    |row| row.get(0),
                )
                .expect("query table existence");
            assert!(!exists, "unused table should be absent: {table}");
        }

        for table in [
            "workspaces",
            "connections",
            "quick_commands",
            "pending_cloud_deletions",
            "team_key_cache",
            "sftp_favorite_paths",
        ] {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                    )",
                    [table],
                    |row| row.get(0),
                )
                .expect("query retained table existence");
            assert!(exists, "runtime table should remain: {table}");
        }
    }
}
