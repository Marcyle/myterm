//! JMS 数据模型

use serde::{Deserialize, Serialize};

/// 图片验证码信息
#[derive(Debug, Clone, Default)]
pub struct CaptchaInfo {
    /// 验证码 key(对应表单字段 captcha_0)
    pub key: String,
    /// 验证码图片绝对 URL
    pub image_url: String,
    /// 已下载的图片字节
    pub image_data: Option<Vec<u8>>,
    /// 图片 MIME 类型(如 image/png)
    pub image_mime: Option<String>,
}

/// JMS 连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsConnectionConfig {
    /// JMS 服务器地址（如 https://jumpserver.example.com）
    pub url: String,
    /// 用户名
    pub username: String,
    /// 密码
    pub password: String,
}

/// JMS 资产节点树节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsAssetNode {
    /// 节点 ID
    pub id: String,
    /// 节点名称
    pub name: String,
    /// 标题
    pub title: String,
    /// 父节点 ID（API 返回驼峰 pId）
    #[serde(default, rename = "pId")]
    pub p_id: String,
    /// 是否有子节点（API 返回驼峰 isParent）
    #[serde(default, rename = "isParent")]
    pub is_parent: bool,
    /// 是否展开
    #[serde(default)]
    pub open: bool,
    /// 元数据
    pub meta: Option<JmsNodeMeta>,
}

/// JMS 节点元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsNodeMeta {
    pub data: JmsNodeData,
    #[serde(rename = "type")]
    pub node_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsNodeData {
    /// 节点/资产 ID（目录节点与资产节点结构不同，故全部可选）
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
}

/// JMS 授权规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsAssetPermission {
    /// 规则 ID
    pub id: String,
    /// 规则名称
    pub name: String,
    /// 资产数量
    pub assets_amount: u32,
    /// 节点数量
    pub nodes_amount: u32,
    /// 账号
    #[serde(default)]
    pub accounts: Vec<String>,
    /// 协议
    #[serde(default)]
    pub protocols: Vec<String>,
    /// 操作
    #[serde(default)]
    pub actions: Vec<JmsAction>,
    /// 组织 ID
    pub org_id: String,
}

/// JMS 操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsAction {
    pub value: String,
    pub label: String,
}

/// JMS 资产
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsAsset {
    /// 资产 ID
    pub id: String,
    /// 资产名称
    pub name: String,
    /// IP 地址
    pub ip: String,
    /// SSH 端口
    pub ssh_port: u16,
    /// 协议
    pub protocol: String,
    /// 平台
    pub platform: String,
}

/// JMS 资产树节点（层次结构，支持懒加载）
#[derive(Debug, Clone)]
pub struct JmsAssetTreeNode {
    /// 原始节点数据
    pub node: JmsAssetNode,
    /// 子节点
    pub children: Vec<JmsAssetTreeNode>,
    /// 子节点是否已加载（懒加载标记）
    pub loaded: bool,
}

impl JmsAssetTreeNode {
    /// 节点用于懒加载的 key（优先 meta.data.key，回退到 id）
    pub fn load_key(&self) -> String {
        self.node
            .meta
            .as_ref()
            .map(|m| m.data.key.clone())
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| self.node.id.clone())
    }

    /// 是否为资产节点（叶子，可连接）
    pub fn is_asset(&self) -> bool {
        self.node
            .meta
            .as_ref()
            .map(|m| m.node_type == "asset")
            .unwrap_or(false)
    }

    /// 是否为可展开的目录节点
    pub fn is_dir(&self) -> bool {
        !self.is_asset()
    }
}

/// 将扁平的 JMS 资产节点列表构建为层次树结构
pub fn build_asset_tree(nodes: Vec<JmsAssetNode>) -> Vec<JmsAssetTreeNode> {
    use std::collections::HashMap;

    // id → 节点 映射
    let mut all_nodes: HashMap<String, JmsAssetNode> = HashMap::new();
    // parent_id → [child_id, ...] 邻接表
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

    for n in nodes {
        let id = n.id.clone();
        let p_id = n.p_id.clone();
        all_nodes.insert(id.clone(), n);

        let is_root = p_id.is_empty() || p_id == "None" || p_id == "0";
        if is_root {
            children_map.entry(String::new()).or_default().push(id);
        } else {
            children_map.entry(p_id).or_default().push(id);
        }
    }

    fn build_subtree(
        node_id: &str,
        all_nodes: &HashMap<String, JmsAssetNode>,
        children_map: &HashMap<String, Vec<String>>,
    ) -> Option<JmsAssetTreeNode> {
        let node = all_nodes.get(node_id)?.clone();
        let mut tree_node = JmsAssetTreeNode {
            node,
            children: Vec::new(),
            loaded: false,
        };
        if let Some(child_ids) = children_map.get(node_id) {
            for child_id in child_ids {
                if let Some(child) = build_subtree(child_id, all_nodes, children_map) {
                    tree_node.children.push(child);
                }
            }
            tree_node.loaded = true;
        }
        Some(tree_node)
    }

    let mut roots = Vec::new();
    if let Some(root_ids) = children_map.get("") {
        for id in root_ids {
            if let Some(node) = build_subtree(id, &all_nodes, &children_map) {
                roots.push(node);
            }
        }
    }

    roots
}

/// JMS 会话信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsSession {
    /// 会话 ID
    pub id: String,
    /// 会话令牌
    pub token: String,
    /// WebSocket URL
    pub ws_url: String,
}

/// JMS 资产连接信息（从 connect API 获取）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JmsConnectInfo {
    /// 连接 token（用于 Koko WebSocket）
    #[serde(default)]
    pub token: String,
    /// Koko WebSocket 地址
    #[serde(default)]
    pub koko_host: String,
    /// SSH 目标主机
    #[serde(default)]
    pub host: String,
    /// SSH 端口
    #[serde(default)]
    pub port: u16,
    /// SSH 用户名
    #[serde(default)]
    pub username: String,
    /// SSH 密码（如果可用）
    #[serde(default)]
    pub password: String,
    /// SSH 私钥（如果可用）
    #[serde(default)]
    pub private_key: String,
    /// 协议
    #[serde(default)]
    pub protocol: String,
}

/// JMS 连接 token 响应（用于 Koko WebSocket / SSH 认证）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsConnectToken {
    /// token ID
    #[serde(default)]
    pub id: String,
    /// 连接 token 值（响应字段名为 value，作为 Koko SSH 登录凭据）
    #[serde(default, alias = "value")]
    pub token: String,
    /// 过期时间
    #[serde(default)]
    pub date_expired: String,
    /// 目标资产（响应中为对象）
    #[serde(default)]
    pub asset: Option<JmsTokenAsset>,
    /// 账号名
    #[serde(default)]
    pub account: String,
    /// 协议
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

/// 连接 token 中的资产信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JmsTokenAsset {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 资产地址（IP/域名）
    #[serde(default)]
    pub address: String,
}

fn default_protocol() -> String {
    "ssh".to_string()
}

/// JMS 资产账号
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmsAssetAccount {
    /// 账号 ID
    pub id: String,
    /// 账号名（如 root, admin）
    #[serde(default)]
    pub name: String,
    /// 用户名
    #[serde(default)]
    pub username: String,
    /// 是否特权账号
    #[serde(default)]
    pub privileged: bool,
    /// 是否活跃（JumpServer 字段名为 is_active）
    #[serde(default, alias = "is_active")]
    pub active: bool,
}
