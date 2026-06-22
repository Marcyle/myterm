use gpui::{Context, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::storage::{StoredConnection, Workspace};

pub trait DatabaseDriverConnectionOpener: Sized + 'static {
    fn open_database_connection(
        &mut self,
        connection: &StoredConnection,
        workspace: Option<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
}

pub fn open_database_connection_with_driver_guard<T>(
    home: &mut T,
    connection: StoredConnection,
    workspace: Option<Workspace>,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: DatabaseDriverConnectionOpener,
{
    // Database support removed - just open the connection directly
    home.open_database_connection(&connection, workspace, window, cx);
}

pub fn prompt_install_database_driver<T>(
    _driver_id: String,
    _connection_name: String,
    window: &mut Window,
    cx: &mut Context<T>,
) where
    T: 'static,
{
    window.push_notification(
        Notification::error("数据库驱动安装已禁用".to_string()),
        cx,
    );
}
