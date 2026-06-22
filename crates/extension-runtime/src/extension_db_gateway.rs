#![allow(dead_code)]

use extension_component::{
    DbSessionResource, ExtensionDbHost, PermissionSet, SqlAccess,
    protocol::{ConnectionInfo, DbError, RowBatch},
};

pub struct ExtensionDbGateway {
    extension_id: String,
    permissions: PermissionSet,
}

impl ExtensionDbGateway {
    pub fn new(
        extension_id: impl Into<String>,
        permissions: PermissionSet,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            permissions,
        }
    }

    pub fn list_connections(&self) -> Result<Vec<ConnectionInfo>, DbError> {
        if !self.permissions.allows_connection_list() {
            return Err(DbError::permission_denied("db:connections:list"));
        }
        Ok(Vec::new())
    }

    pub async fn open_session(
        &self,
        _request: extension_component::protocol::OpenSessionRequest,
    ) -> Result<DbSessionResource, DbError> {
        Err(DbError::query_failed("database support removed"))
    }

    pub async fn execute(
        &self,
        _request: extension_component::protocol::ExecuteSqlRequest,
    ) -> Result<RowBatch, DbError> {
        Err(DbError::query_failed("database support removed"))
    }

    pub async fn list_databases(&self, _connection_id: String) -> Result<Vec<String>, DbError> {
        Err(DbError::query_failed("database support removed"))
    }

    pub async fn list_schemas(
        &self,
        _connection_id: String,
        _database: String,
    ) -> Result<Vec<String>, DbError> {
        Err(DbError::query_failed("database support removed"))
    }

    pub async fn close_session(&self, _session: &mut DbSessionResource) -> Result<(), DbError> {
        Ok(())
    }

    fn ensure_session_resource(&self, session: &DbSessionResource) -> Result<(), DbError> {
        if session.extension_id() != self.extension_id {
            return Err(DbError::permission_denied("foreign session resource"));
        }
        if session.is_closed() {
            return Err(DbError::invalid_resource("closed session resource"));
        }
        Ok(())
    }

    fn ensure_db_permission(&self, access: SqlAccess, connection_id: &str) -> Result<(), DbError> {
        if self.permissions.allows_db(access, connection_id) {
            return Ok(());
        }
        Err(DbError::permission_denied(format!(
            "db:{access:?}:{connection_id}"
        )))
    }
}

#[async_trait::async_trait]
impl ExtensionDbHost for ExtensionDbGateway {
    fn list_connections(&self) -> Result<Vec<ConnectionInfo>, DbError> {
        ExtensionDbGateway::list_connections(self)
    }

    async fn open_session(
        &self,
        request: extension_component::protocol::OpenSessionRequest,
    ) -> Result<DbSessionResource, DbError> {
        ExtensionDbGateway::open_session(self, request).await
    }

    async fn execute(
        &self,
        session: &DbSessionResource,
        sql: String,
        options: extension_component::protocol::ExecOptions,
    ) -> Result<RowBatch, DbError> {
        self.ensure_session_resource(session)?;
        ExtensionDbGateway::execute(
            self,
            extension_component::protocol::ExecuteSqlRequest {
                session_id: session.session_id().to_string(),
                connection_id: session.connection_id().to_string(),
                sql,
                options,
            },
        )
        .await
    }

    async fn list_databases(&self, session: &DbSessionResource) -> Result<Vec<String>, DbError> {
        self.ensure_session_resource(session)?;
        ExtensionDbGateway::list_databases(self, session.connection_id().to_string()).await
    }

    async fn list_schemas(
        &self,
        session: &DbSessionResource,
        database: String,
    ) -> Result<Vec<String>, DbError> {
        self.ensure_session_resource(session)?;
        ExtensionDbGateway::list_schemas(self, session.connection_id().to_string(), database).await
    }

    async fn close_session(&self, session: &mut DbSessionResource) -> Result<(), DbError> {
        ExtensionDbGateway::close_session(self, session).await
    }
}
