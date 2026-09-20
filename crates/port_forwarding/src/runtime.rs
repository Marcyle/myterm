use anyhow::{Context, Result, bail};
use one_core::storage::{ConnectionType, PortForwardingKind, StoredConnection};
use ssh::{
    DynamicSocksConfig, DynamicSocksTunnel, LocalPortForwardConfig, LocalPortForwardTunnel,
    SshConnectConfig, connect_config_from_params, start_dynamic_socks_forward,
    start_local_port_forward_with_config,
};
use std::collections::HashMap;
use std::net::SocketAddr;

pub struct LocalForwardingRequest {
    pub ssh_config: SshConnectConfig,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}
pub struct DynamicForwardingRequest {
    pub ssh_config: SshConnectConfig,
    pub bind_host: String,
    pub bind_port: u16,
}

#[derive(Default)]
pub struct PortForwardingRuntime {
    local_tunnels: HashMap<i64, LocalPortForwardTunnel>,
    dynamic_tunnels: HashMap<i64, DynamicSocksTunnel>,
}

impl PortForwardingRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start_local(
        &mut self,
        connection_id: i64,
        request: LocalForwardingRequest,
    ) -> Result<SocketAddr> {
        if self.is_running(connection_id) {
            bail!("Port Forwarding connection is already running");
        }
        let tunnel = start_local_port_forward_with_config(
            request.ssh_config,
            LocalPortForwardConfig {
                bind_host: request.bind_host,
                bind_port: request.bind_port,
                target_host: request.target_host,
                target_port: request.target_port,
            },
        )
        .await?;
        let local_addr = tunnel.local_addr();
        self.local_tunnels.insert(connection_id, tunnel);
        Ok(local_addr)
    }

    pub fn is_running(&self, connection_id: i64) -> bool {
        self.local_tunnels.contains_key(&connection_id)
            || self.dynamic_tunnels.contains_key(&connection_id)
    }

    pub async fn start_dynamic(
        &mut self,
        connection_id: i64,
        request: DynamicForwardingRequest,
    ) -> Result<SocketAddr> {
        if self.is_running(connection_id) {
            bail!("Port Forwarding connection is already running");
        }
        let tunnel = start_dynamic_socks_forward(
            request.ssh_config,
            DynamicSocksConfig {
                bind_host: request.bind_host,
                bind_port: request.bind_port,
            },
        )
        .await?;
        let local_addr = tunnel.local_addr();
        self.dynamic_tunnels.insert(connection_id, tunnel);
        Ok(local_addr)
    }

    pub async fn stop(&mut self, connection_id: i64) -> Result<()> {
        if let Some(mut tunnel) = self.local_tunnels.remove(&connection_id) {
            tunnel.close().await
        } else if let Some(mut tunnel) = self.dynamic_tunnels.remove(&connection_id) {
            tunnel.close().await
        } else {
            bail!("Port Forwarding connection is not running")
        }
    }
}
pub fn build_local_forwarding_request(
    forwarding_connection: &StoredConnection,
    ssh_connection: &StoredConnection,
) -> Result<LocalForwardingRequest> {
    if forwarding_connection.connection_type != ConnectionType::PortForwarding {
        bail!("connection is not a Port Forwarding connection");
    }
    if ssh_connection.connection_type != ConnectionType::SshSftp {
        bail!("referenced connection is not an SSH/SFTP connection");
    }

    let params = forwarding_connection
        .to_port_forwarding_params()
        .context("failed to parse Port Forwarding params")?;
    if params.kind != PortForwardingKind::Local {
        bail!("only local Port Forwarding is supported by this runtime entrypoint");
    }
    if ssh_connection.id != Some(params.ssh_connection_id) {
        bail!("referenced SSH connection id does not match Port Forwarding params");
    }

    let ssh_params = ssh_connection
        .to_ssh_params()
        .context("failed to parse referenced SSH params")?;

    Ok(LocalForwardingRequest {
        ssh_config: connect_config_from_params(&ssh_params),
        bind_host: params.bind_host,
        bind_port: params.bind_port,
        target_host: params.target_host,
        target_port: params.target_port,
    })
}

pub fn build_dynamic_forwarding_request(
    forwarding_connection: &StoredConnection,
    ssh_connection: &StoredConnection,
) -> Result<DynamicForwardingRequest> {
    if forwarding_connection.connection_type != ConnectionType::PortForwarding {
        bail!("connection is not a Port Forwarding connection");
    }
    if ssh_connection.connection_type != ConnectionType::SshSftp {
        bail!("referenced connection is not an SSH/SFTP connection");
    }

    let params = forwarding_connection
        .to_port_forwarding_params()
        .context("failed to parse Port Forwarding params")?;
    if params.kind != PortForwardingKind::Dynamic {
        bail!("connection is not Dynamic SOCKS Port Forwarding");
    }
    if ssh_connection.id != Some(params.ssh_connection_id) {
        bail!("referenced SSH connection id does not match Port Forwarding params");
    }

    let ssh_params = ssh_connection
        .to_ssh_params()
        .context("failed to parse referenced SSH params")?;

    Ok(DynamicForwardingRequest {
        ssh_config: connect_config_from_params(&ssh_params),
        bind_host: params.bind_host,
        bind_port: params.bind_port,
    })
}
