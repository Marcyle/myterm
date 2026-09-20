use gpui::{Styled, px};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable};
use rust_i18n::t;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NewConnectionKind {
    Ssh,
    Jms,
    PortForwarding,
}

impl NewConnectionKind {
    pub(super) fn all() -> [Self; 3] {
        [Self::Ssh, Self::Jms, Self::PortForwarding]
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Ssh => "SSH / SFTP".to_string(),
            Self::Jms => t!("Home.jms_connection").to_string(),
            Self::PortForwarding => t!("PortForwarding.new").to_string(),
        }
    }

    pub(super) fn description(&self) -> String {
        match self {
            Self::Ssh => t!("NewConnection.description_ssh").to_string(),
            Self::Jms => t!("NewConnection.description_jms").to_string(),
            Self::PortForwarding => t!("NewConnection.description_port_forwarding").to_string(),
        }
    }

    pub(super) fn icon(&self, cx: &gpui::App) -> Icon {
        match self {
            Self::Ssh => IconName::SquareTerminal
                .mono()
                .text_color(cx.theme().connection_ssh)
                .with_size(px(28.0)),
            Self::Jms => IconName::Key
                .mono()
                .text_color(cx.theme().connection_jms)
                .with_size(px(28.0)),
            Self::PortForwarding => IconName::Network
                .mono()
                .text_color(cx.theme().connection_port_forwarding)
                .with_size(px(28.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_kinds_exclude_local_terminal() {
        let kinds = NewConnectionKind::all();
        assert!(kinds.contains(&NewConnectionKind::Ssh));
        assert!(kinds.contains(&NewConnectionKind::Jms));
        assert!(kinds.contains(&NewConnectionKind::PortForwarding));
    }
}
