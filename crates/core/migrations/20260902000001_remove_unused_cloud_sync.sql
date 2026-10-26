-- Remove dormant cloud synchronization metadata and deletion queue.
DROP TABLE IF EXISTS pending_cloud_deletions;
DROP TABLE IF EXISTS team_key_cache;
DROP INDEX IF EXISTS idx_workspaces_cloud_id;
ALTER TABLE workspaces DROP COLUMN cloud_id;
ALTER TABLE connections DROP COLUMN sync_enabled;
ALTER TABLE connections DROP COLUMN last_synced_at;
