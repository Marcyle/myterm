use crate::crypto;
use crate::storage::traits::Entity;
use gpui::Global;
use gpui_component::IconName;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

/// 活跃连接状态 - 用于跟踪哪些连接当前已打开
#[derive(Default)]
pub struct ActiveConnections {
    active_ids: HashSet<i64>,
}

impl Global for ActiveConnections {}

impl ActiveConnections {
    pub fn new() -> Self {
        Self {
            active_ids: HashSet::new(),
        }
    }

    pub fn add(&mut self, conn_id: i64) {
        self.active_ids.insert(conn_id);
    }

    pub fn remove(&mut self, conn_id: i64) {
        self.active_ids.remove(&conn_id);
    }

    pub fn is_active(&self, conn_id: i64) -> bool {
        self.active_ids.contains(&conn_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConnectionType {
    All,
    SshSftp,
    PortForwarding,
    Jms,
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ConnectionType::All => "All",
            ConnectionType::SshSftp => "SshSftp",
            ConnectionType::PortForwarding => "PortForwarding",
            ConnectionType::Jms => "Jms",
        };
        write!(f, "{}", s)
    }
}

impl ConnectionType {
    pub fn all() -> Vec<ConnectionType> {
        vec![
            ConnectionType::All,
            ConnectionType::SshSftp,
            ConnectionType::PortForwarding,
            ConnectionType::Jms,
        ]
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "SshSftp" => ConnectionType::SshSftp,
            "PortForwarding" => ConnectionType::PortForwarding,
            "Jms" => ConnectionType::Jms,
            _ => ConnectionType::SshSftp,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ConnectionType::All => "全部连接",
            ConnectionType::SshSftp => "SSH/SFTP",
            ConnectionType::PortForwarding => "端口转发",
            ConnectionType::Jms => "JMS 连接",
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            ConnectionType::All => IconName::Server,
            ConnectionType::SshSftp => IconName::SquareTerminal,
            ConnectionType::PortForwarding => IconName::Network,
            ConnectionType::Jms => IconName::Key,
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    /// 连接超时（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_timeout: Option<u64>,
    /// 心跳间隔（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keepalive_interval: Option<u64>,
    /// 最大心跳失败次数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keepalive_max: Option<usize>,
    /// 默认工作目录
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_directory: Option<String>,
    /// 初始化脚本
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_script: Option<String>,
    /// 关闭 shell integration 注入(走裸 request_shell,牺牲 prompt hook / 命令记录)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_shell_integration: Option<bool>,
    /// 跳板机配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump_server: Option<JumpServerConfig>,
    /// 代理配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

/// 跳板机配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
}

/// 代理类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyType {
    Socks5,
    Http,
}

/// 代理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SshAuthMethod {
    Password {
        password: String,
    },
    PrivateKey {
        key_path: String,
        passphrase: Option<String>,
    },
    Agent,
    AutoPublicKey,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PortForwardingKind {
    #[default]
    Local,
    Dynamic,
}

impl PortForwardingKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Dynamic => "Dynamic SOCKS",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForwardingParams {
    pub ssh_connection_id: i64,
    #[serde(default)]
    pub kind: PortForwardingKind,
    #[serde(default = "default_forward_bind_host")]
    pub bind_host: String,
    #[serde(default)]
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
}

fn default_forward_bind_host() -> String {
    "127.0.0.1".to_string()
}

/// JMS(JumpServer)连接参数
///
/// JMS 因强制验证码+MFA 无法自动连接,此参数仅用于保存 URL/用户名/密码,
/// 下次打开 JMS 连接窗口时自动填充。password 字段在持久化时由
/// [`StoredConnection::encrypt_params`] 自动加密。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsParams {
    /// JMS 服务器地址(如 https://jumpserver.example.com)
    pub url: String,
    /// 用户名
    pub username: String,
    /// 密码(持久化时自动加密)
    #[serde(default)]
    pub password: String,
    /// 是否使用本地代理
    #[serde(default = "default_true")]
    pub use_local_proxy: bool,
}

fn default_true() -> bool {
    true
}

/// Workspace for organizing connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 云端 ID（用于同步）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
}

impl Entity for Workspace {
    fn id(&self) -> Option<i64> {
        self.id
    }

    fn created_at(&self) -> i64 {
        self.created_at
            .expect("created_at 在从数据库读取后应该存在")
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
            .expect("updated_at 在从数据库读取后应该存在")
    }
}

impl Workspace {
    pub fn new(name: String) -> Self {
        Self {
            id: None,
            name,
            color: None,
            icon: None,
            created_at: None,
            updated_at: None,
            cloud_id: None,
        }
    }
}

/// Stored connection with ID
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub name: String,
    pub connection_type: ConnectionType,
    pub params: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    /// 已选中的数据库ID列表（JSON数组），None表示全选
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_databases: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    /// 是否启用云同步（默认 true）
    #[serde(default = "default_sync_enabled")]
    pub sync_enabled: bool,
    /// 云端记录 ID（同步成功后获得）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
    /// 最后同步时间戳
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    /// 最近使用时间戳，仅用于本地列表排序，不参与云同步。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 团队归属 ID（None = 个人数据）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// 连接创建者 ID（用户 UUID，用于权限判断）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

fn default_sync_enabled() -> bool {
    true
}

impl Entity for StoredConnection {
    fn id(&self) -> Option<i64> {
        self.id
    }

    fn created_at(&self) -> i64 {
        self.created_at
            .expect("created_at 在从数据库读取后应该存在")
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
            .expect("updated_at 在从数据库读取后应该存在")
    }
}

impl StoredConnection {
    pub fn new_ssh(name: String, params: SshParams, workspace_id: Option<i64>) -> Self {
        Self {
            id: None,
            name,
            connection_type: ConnectionType::SshSftp,
            params: serde_json::to_string(&params).expect("SshParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn to_ssh_params(&self) -> Result<SshParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn new_port_forwarding(
        name: String,
        params: PortForwardingParams,
        workspace_id: Option<i64>,
    ) -> Self {
        Self {
            id: None,
            name,
            connection_type: ConnectionType::PortForwarding,
            params: serde_json::to_string(&params).expect("PortForwardingParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn to_port_forwarding_params(&self) -> Result<PortForwardingParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn new_jms(name: String, params: JmsParams, workspace_id: Option<i64>) -> Self {
        Self {
            id: None,
            name,
            connection_type: ConnectionType::Jms,
            params: serde_json::to_string(&params).expect("JmsParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn to_jms_params(&self) -> Result<JmsParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    /// 获取已选中的数据库列表，None表示全选
    pub fn get_selected_databases(&self) -> Option<Vec<String>> {
        self.selected_databases
            .as_ref()
            .and_then(|json| serde_json::from_str(json).ok())
    }

    /// 设置已选中的数据库列表，None表示全选
    pub fn set_selected_databases(&mut self, databases: Option<Vec<String>>) {
        self.selected_databases =
            databases.map(|dbs| serde_json::to_string(&dbs).unwrap_or_default());
    }

    /// 对 params 中的敏感字段进行加密，返回加密后的 params 字符串。
    /// 敏感字段包括：password、passphrase 以及嵌套结构中的同名字段。
    pub fn encrypt_params(&self) -> String {
        encrypt_json_passwords(&self.params)
    }

    /// 对 params 中的加密字段进行解密，返回解密后的 params 字符串。
    pub fn decrypt_params(&self) -> String {
        decrypt_json_passwords(&self.params)
    }

    /// 返回一个新的 StoredConnection，其 params 中的密码字段已解密
    pub fn with_decrypted_params(&self) -> Self {
        let mut cloned = self.clone();
        cloned.params = cloned.decrypt_params();
        cloned
    }
}

/// 递归加密 JSON 中所有名为 password 或 passphrase 的字符串字段
fn encrypt_json_passwords(json_str: &str) -> String {
    match serde_json::from_str::<Value>(json_str) {
        Ok(mut value) => {
            encrypt_value(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| json_str.to_string())
        }
        Err(_) => json_str.to_string(),
    }
}

/// 递归解密 JSON 中所有名为 password 或 passphrase 的字符串字段
fn decrypt_json_passwords(json_str: &str) -> String {
    match serde_json::from_str::<Value>(json_str) {
        Ok(mut value) => {
            decrypt_value(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| json_str.to_string())
        }
        Err(_) => json_str.to_string(),
    }
}

/// 判断字段名是否为敏感字段
fn is_sensitive_field(key: &str) -> bool {
    key == "password"
        || key == "passphrase"
        || key.ends_with("_password")
        || key.ends_with("_passphrase")
}

/// 递归遍历 JSON Value，加密敏感字段
fn encrypt_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_sensitive_field(key) {
                    if let Value::String(s) = val {
                        *s = crypto::encrypt_password(s);
                    }
                } else {
                    encrypt_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                encrypt_value(item);
            }
        }
        _ => {}
    }
}

/// 递归遍历 JSON Value，解密敏感字段
fn decrypt_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_sensitive_field(key) {
                    if let Value::String(s) = val {
                        *s = crypto::decrypt_password(s);
                    }
                } else {
                    decrypt_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                decrypt_value(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_connection_port_forwarding_roundtrip() {
        let params = PortForwardingParams {
            ssh_connection_id: 7,
            kind: PortForwardingKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 15432,
            target_host: "db.internal".to_string(),
            target_port: 5432,
        };
        let conn =
            StoredConnection::new_port_forwarding("postgres tunnel".to_string(), params, Some(42));
        assert_eq!(conn.connection_type, ConnectionType::PortForwarding);
        assert_eq!(conn.name, "postgres tunnel");
        assert_eq!(conn.workspace_id, Some(42));

        let rt = conn.to_port_forwarding_params().unwrap();
        assert_eq!(rt.ssh_connection_id, 7);
        assert_eq!(rt.kind, PortForwardingKind::Local);
        assert_eq!(rt.bind_host, "127.0.0.1");
        assert_eq!(rt.bind_port, 15432);
        assert_eq!(rt.target_host, "db.internal");
        assert_eq!(rt.target_port, 5432);
    }

    #[test]
    fn connection_type_port_forwarding_methods() {
        assert_eq!(ConnectionType::PortForwarding.label(), "端口转发");
        assert_eq!(
            ConnectionType::from_str("PortForwarding"),
            ConnectionType::PortForwarding
        );
        assert_eq!(
            format!("{}", ConnectionType::PortForwarding),
            "PortForwarding"
        );
        assert!(ConnectionType::all().contains(&ConnectionType::PortForwarding));
    }

    #[test]
    fn ssh_auth_method_agent_serialize_deserialize() {
        let auth = SshAuthMethod::Agent;
        let json = serde_json::to_string(&auth).expect("Agent 认证方式应可序列化");
        let parsed: SshAuthMethod =
            serde_json::from_str(&json).expect("Agent 认证方式应可反序列化");
        assert!(matches!(parsed, SshAuthMethod::Agent));
    }

    #[test]
    fn ssh_auth_method_auto_publickey_serialize_deserialize() {
        let auth = SshAuthMethod::AutoPublicKey;
        let json = serde_json::to_string(&auth).expect("自动公钥认证方式应可序列化");
        let parsed: SshAuthMethod =
            serde_json::from_str(&json).expect("自动公钥认证方式应可反序列化");
        assert!(matches!(parsed, SshAuthMethod::AutoPublicKey));
    }
}
