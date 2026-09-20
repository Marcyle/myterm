rust_i18n::i18n!("locales", fallback = "en");

mod dynamic_socks;
mod session_manager;
mod socks5;
mod ssh;

pub use dynamic_socks::{DynamicSocksConfig, DynamicSocksTunnel, start_dynamic_socks_forward};
pub use session_manager::SshSessionManager;
pub use ssh::{
    AuthFailureMessages, ChannelEvent, JumpServerConnectConfig, KeyboardInteractivePrompt,
    KeyboardInteractiveRequest, KeyboardInteractiveResponder, KeyboardInteractiveTarget,
    LocalPortForwardConfig, LocalPortForwardTunnel, ProxyConnectConfig, ProxyType, PtyConfig,
    RusshChannel, RusshClient, ShellIntegrationSetup, SshAuth, SshChannel, SshClient,
    SshConnectConfig, authenticate_session, authenticate_session_with_fallbacks,
    authenticate_with_strategy, defaults, expand_auto_publickey_auth, start_local_port_forward,
    start_local_port_forward_with_config,
};

use one_core::storage::SshParams;

/// 将持久化的 SSH 参数转换为各类运行时共用的连接配置。
///
/// 连接终端、SFTP 和端口转发都应通过此入口构造基础 SSH 配置，避免
/// 各业务模块分别复制认证、跳板机和代理字段的映射逻辑。
pub fn connect_config_from_params(params: &SshParams) -> SshConnectConfig {
    SshConnectConfig {
        host: params.host.clone(),
        port: params.port,
        username: params.username.clone(),
        auth: auth_from_params(&params.auth_method),
        timeout: params.connect_timeout.map(std::time::Duration::from_secs),
        keepalive_interval: params
            .keepalive_interval
            .map(std::time::Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params
            .jump_server
            .as_ref()
            .map(|jump| JumpServerConnectConfig {
                host: jump.host.clone(),
                port: jump.port,
                username: jump.username.clone(),
                auth: auth_from_params(&jump.auth_method),
            }),
        proxy: params.proxy.as_ref().map(|proxy| ProxyConnectConfig {
            proxy_type: match proxy.proxy_type {
                one_core::storage::ProxyType::Socks5 => ProxyType::Socks5,
                one_core::storage::ProxyType::Http => ProxyType::Http,
            },
            host: proxy.host.clone(),
            port: proxy.port,
            username: proxy.username.clone(),
            password: proxy.password.clone(),
        }),
        keyboard_interactive_responder: None,
    }
}

fn auth_from_params(auth: &one_core::storage::SshAuthMethod) -> SshAuth {
    match auth {
        one_core::storage::SshAuthMethod::Password { password } => {
            SshAuth::Password(password.clone())
        }
        one_core::storage::SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuth::PrivateKey {
            key_path: key_path.clone(),
            passphrase: passphrase.clone(),
            certificate_path: None,
        },
        one_core::storage::SshAuthMethod::Agent => SshAuth::Agent,
        one_core::storage::SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use one_core::storage::{ProxyConfig, ProxyType as StorageProxyType, SshAuthMethod};

    fn sample_params() -> SshParams {
        SshParams {
            host: "target.example.com".into(),
            port: 2222,
            username: "demo".into(),
            auth_method: SshAuthMethod::Password {
                password: "secret".into(),
            },
            connect_timeout: Some(12),
            keepalive_interval: Some(30),
            keepalive_max: Some(4),
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        }
    }

    #[test]
    fn converts_connection_basics_and_password_auth() {
        let config = connect_config_from_params(&sample_params());

        assert_eq!(config.host, "target.example.com");
        assert_eq!(config.port, 2222);
        assert_eq!(config.username, "demo");
        assert!(matches!(config.auth, SshAuth::Password(password) if password == "secret"));
        assert_eq!(config.timeout, Some(std::time::Duration::from_secs(12)));
        assert_eq!(
            config.keepalive_interval,
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(config.keepalive_max, Some(4));
    }

    #[test]
    fn converts_proxy_and_private_key_auth() {
        let mut params = sample_params();
        params.auth_method = SshAuthMethod::PrivateKey {
            key_path: "C:/keys/demo".into(),
            passphrase: Some("passphrase".into()),
        };
        params.proxy = Some(ProxyConfig {
            proxy_type: StorageProxyType::Socks5,
            host: "127.0.0.1".into(),
            port: 1080,
            username: Some("proxy-user".into()),
            password: Some("proxy-pass".into()),
        });

        let config = connect_config_from_params(&params);

        assert!(matches!(
            config.auth,
            SshAuth::PrivateKey { key_path, passphrase, certificate_path }
                if key_path == "C:/keys/demo"
                    && passphrase.as_deref() == Some("passphrase")
                    && certificate_path.is_none()
        ));
        let proxy = config.proxy.expect("proxy should be converted");
        assert_eq!(proxy.proxy_type, ProxyType::Socks5);
        assert_eq!(proxy.host, "127.0.0.1");
        assert_eq!(proxy.port, 1080);
        assert_eq!(proxy.username.as_deref(), Some("proxy-user"));
    }
}
