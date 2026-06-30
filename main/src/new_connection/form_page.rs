use gpui::{AnyView, AnyWindowHandle, AppContext, Context, Entity, Window};
use one_core::storage::ConnectionType;
use port_forwarding_view::{PortForwardingFormWindow, PortForwardingFormWindowConfig};
use terminal_view::{SshFormWindow, SshFormWindowConfig};

use crate::home_tab::HomePage;
use crate::new_connection::NewConnectionWindow;
use crate::new_connection::connection_kind::NewConnectionKind;

pub(crate) enum NewConnectionFormResult {
    Form(AnyView),
    Done,
    Blocked,
}

pub(crate) trait NewConnectionFormPage {
    fn build_form_view(
        self,
        parent: Entity<HomePage>,
        parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<NewConnectionWindow>,
    ) -> NewConnectionFormResult;
}

impl NewConnectionFormPage for NewConnectionKind {
    fn build_form_view(
        self,
        parent: Entity<HomePage>,
        _parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<NewConnectionWindow>,
    ) -> NewConnectionFormResult {
        match self {
            Self::Ssh => build_ssh_form(parent, window, cx),
            Self::PortForwarding => build_port_forwarding_form(parent, window, cx),
            Self::Terminal => {
                // 打开本地终端 - 使用 defer 确保在主窗口上执行
                let parent_clone = parent.clone();
                window.defer(cx, move |window, cx| {
                    parent_clone.update(cx, |home, cx| {
                        home.add_terminal_tab(window, cx);
                    });
                });
                NewConnectionFormResult::Done
            }
        }
    }
}

fn build_port_forwarding_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, _cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::PortForwarding)
                .cloned()
        });
        let ssh_connections = home
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();
        home.editing_connection_id = None;
        Some(PortForwardingFormWindowConfig {
            editing_connection,
            ssh_connections,
            workspaces: home.workspaces.clone(),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| PortForwardingFormWindow::new(config, window, cx))
            .into(),
    )
}

fn build_ssh_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, _cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::SshSftp)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(SshFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| SshFormWindow::new(config, window, cx)).into())
}
