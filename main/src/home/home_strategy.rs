use crate::home_tab::HomePage;
use gpui::{Context, Window};
use one_core::storage::{ConnectionType, StoredConnection, Workspace};

pub(crate) trait ConnectionOpenStrategy {
    fn open(self: Box<Self>, home: &mut HomePage, window: &mut Window, cx: &mut Context<HomePage>);
}

pub(crate) fn build_connection_open_strategy(
    connection: StoredConnection,
    workspace: Option<Workspace>,
) -> Box<dyn ConnectionOpenStrategy> {
    match connection.connection_type {
        ConnectionType::SshSftp => Box::new(SshOpenStrategy {
            connection,
            workspace,
        }),
        ConnectionType::PortForwarding => Box::new(PortForwardingOpenStrategy { connection }),
        _ => Box::new(NoopOpenStrategy),
    }
}

struct SshOpenStrategy {
    connection: StoredConnection,
    workspace: Option<Workspace>,
}

impl ConnectionOpenStrategy for SshOpenStrategy {
    fn open(self: Box<Self>, home: &mut HomePage, window: &mut Window, cx: &mut Context<HomePage>) {
        home.open_ssh_terminal(self.connection, self.workspace, window, cx);
    }
}

struct NoopOpenStrategy;

struct PortForwardingOpenStrategy {
    connection: StoredConnection,
}

impl ConnectionOpenStrategy for PortForwardingOpenStrategy {
    fn open(self: Box<Self>, home: &mut HomePage, window: &mut Window, cx: &mut Context<HomePage>) {
        home.open_port_forwarding(self.connection, window, cx);
    }
}

impl ConnectionOpenStrategy for NoopOpenStrategy {
    fn open(
        self: Box<Self>,
        _home: &mut HomePage,
        _window: &mut Window,
        _cx: &mut Context<HomePage>,
    ) {
    }
}
