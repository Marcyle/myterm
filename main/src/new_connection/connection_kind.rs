use gpui::{Styled, px};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable};
use rust_i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NewConnectionCategory {
    All,
    Terminal,
}

impl NewConnectionCategory {
    pub(super) fn all() -> [Self; 2] {
        [Self::All, Self::Terminal]
    }

    pub(super) fn label(self) -> String {
        match self {
            Self::All => t!("NewConnection.category_all").to_string(),
            Self::Terminal => t!("NewConnection.category_terminal").to_string(),
        }
    }

    pub(super) fn icon(self) -> IconName {
        match self {
            Self::All => IconName::AppsColor,
            Self::Terminal => IconName::Terminal,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NewConnectionKind {
    Ssh,
    Terminal,
    PortForwarding,
}

impl NewConnectionKind {
    pub(super) fn all() -> Vec<Self> {
        vec![Self::Ssh, Self::Terminal, Self::PortForwarding]
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Ssh => "SSH / SFTP".to_string(),
            Self::Terminal => "Terminal".to_string(),
            Self::PortForwarding => t!("PortForwarding.new").to_string(),
        }
    }

    pub(super) fn description(&self) -> String {
        match self {
            Self::Ssh => t!("NewConnection.description_ssh").to_string(),
            Self::Terminal => t!("NewConnection.description_terminal").to_string(),
            Self::PortForwarding => t!("NewConnection.description_port_forwarding").to_string(),
        }
    }

    pub(super) fn category(&self) -> NewConnectionCategory {
        match self {
            Self::Ssh | Self::Terminal | Self::PortForwarding => NewConnectionCategory::Terminal,
        }
    }

    pub(super) fn icon(&self, cx: &gpui::App) -> Icon {
        match self {
            Self::Ssh => IconName::TerminalColor.color().with_size(px(40.0)),
            Self::Terminal => IconName::Terminal
                .mono()
                .text_color(cx.theme().connection_ssh)
                .with_size(px(40.0)),
            Self::PortForwarding => IconName::Network.color().with_size(px(40.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_kinds_are_available_from_new_connection() {
        let kinds = NewConnectionKind::all();
        assert!(kinds.contains(&NewConnectionKind::Ssh));
        assert!(kinds.contains(&NewConnectionKind::Terminal));
        assert_eq!(
            NewConnectionKind::Ssh.category(),
            NewConnectionCategory::Terminal
        );
        assert_eq!(
            NewConnectionKind::Terminal.category(),
            NewConnectionCategory::Terminal
        );
    }
}
