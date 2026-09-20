-- Initial schema (merged migrations)

-- Workspaces
CREATE TABLE IF NOT EXISTS workspaces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    color TEXT,
    icon TEXT,
    cloud_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workspaces_name ON workspaces(name);
CREATE INDEX IF NOT EXISTS idx_workspaces_cloud_id ON workspaces(cloud_id);

-- Connections
CREATE TABLE IF NOT EXISTS connections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    connection_type TEXT NOT NULL,
    params TEXT NOT NULL,
    workspace_id INTEGER,
    selected_databases TEXT,
    remark TEXT,
    sync_enabled INTEGER NOT NULL DEFAULT 1,
    cloud_id TEXT,
    last_synced_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_connections_name ON connections(name);
CREATE INDEX IF NOT EXISTS idx_connections_workspace ON connections(workspace_id);
CREATE INDEX IF NOT EXISTS idx_connections_cloud_id ON connections(cloud_id);


-- Quick Commands
CREATE TABLE IF NOT EXISTS quick_commands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    command TEXT NOT NULL,
    description TEXT,
    pinned INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    connection_id INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES connections(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quick_commands_connection ON quick_commands(connection_id);
CREATE INDEX IF NOT EXISTS idx_quick_commands_order ON quick_commands(pinned DESC, sort_order ASC);

-- Pending Cloud Deletions
CREATE TABLE IF NOT EXISTS pending_cloud_deletions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cloud_id TEXT NOT NULL UNIQUE,
    entity_type TEXT NOT NULL DEFAULT 'connection',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_cloud_deletions_entity_type ON pending_cloud_deletions(entity_type);
