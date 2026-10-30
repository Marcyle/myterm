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
}

/// 将扁平的 JMS 资产节点列表构建为层次树结构
pub fn build_asset_tree(nodes: Vec<JmsAssetNode>) -> Vec<JmsAssetTreeNode> {
    use std::collections::{HashMap, HashSet};

    let mut all_nodes: HashMap<String, JmsAssetNode> = HashMap::new();
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

    for n in nodes {
        let id = n.id.clone();
        let p_id = n.p_id.clone();
        all_nodes.insert(id.clone(), n);
        children_map.entry(p_id).or_default().push(id);
    }

    // 确定根节点：
    // 1. p_id 为空、"0"、"None"、"root"、"-1"
    // 2. 或者其 p_id 在 all_nodes 中不存在（说明其父节点不在此次返回的子集中，自身即为当前树顶层）
    let mut root_ids: Vec<String> = Vec::new();
    let mut seen_roots = HashSet::new();

    for (id, node) in &all_nodes {
        let p = &node.p_id;
        let is_root = p.is_empty()
            || p == "0"
            || p == "None"
            || p == "root"
            || p == "-1"
            || !all_nodes.contains_key(p);

        if is_root && seen_roots.insert(id.clone()) {
            root_ids.push(id.clone());
        }
    }

    // 排序根节点：文件夹在前，资产在后；同类型按名称排序
    root_ids.sort_by(|a, b| {
        let node_a = all_nodes.get(a);
        let node_b = all_nodes.get(b);
        let a_is_asset = node_a
            .and_then(|n| n.meta.as_ref())
            .map(|m| m.node_type == "asset")
            .unwrap_or(false);
        let b_is_asset = node_b
            .and_then(|n| n.meta.as_ref())
            .map(|m| m.node_type == "asset")
            .unwrap_or(false);
        match (a_is_asset, b_is_asset) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => {
                let name_a = node_a.map(|n| n.name.as_str()).unwrap_or("");
                let name_b = node_b.map(|n| n.name.as_str()).unwrap_or("");
                name_a.cmp(name_b)
            }
        }
    });

    fn build_subtree(
        node_id: &str,
        all_nodes: &HashMap<String, JmsAssetNode>,
        children_map: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
    ) -> Option<JmsAssetTreeNode> {
        if !visited.insert(node_id.to_string()) {
            return None;
        }
        let node = all_nodes.get(node_id)?.clone();
        let mut tree_node = JmsAssetTreeNode {
            node,
            children: Vec::new(),
            loaded: false,
        };

        if let Some(child_ids) = children_map.get(node_id) {
            let mut sorted_child_ids = child_ids.clone();
            sorted_child_ids.sort_by(|a, b| {
                let node_a = all_nodes.get(a);
                let node_b = all_nodes.get(b);
                let a_is_asset = node_a
                    .and_then(|n| n.meta.as_ref())
                    .map(|m| m.node_type == "asset")
                    .unwrap_or(false);
                let b_is_asset = node_b
                    .and_then(|n| n.meta.as_ref())
                    .map(|m| m.node_type == "asset")
                    .unwrap_or(false);
                match (a_is_asset, b_is_asset) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    _ => {
                        let name_a = node_a.map(|n| n.name.as_str()).unwrap_or("");
                        let name_b = node_b.map(|n| n.name.as_str()).unwrap_or("");
                        name_a.cmp(name_b)
                    }
                }
            });

            for child_id in &sorted_child_ids {
                if let Some(child) = build_subtree(child_id, all_nodes, children_map, visited) {
                    tree_node.children.push(child);
                }
            }
            tree_node.loaded = true;
        }

        Some(tree_node)
    }

    let mut roots = Vec::new();
    let mut visited = HashSet::new();
    for id in root_ids {
        if let Some(node) = build_subtree(&id, &all_nodes, &children_map, &mut visited) {
            roots.push(node);
        }
    }

    roots
}

/// 为指定父节点构建懒加载获取到的子节点树（保持多层级关系与资产正确归位）
pub fn build_asset_tree_for_children(
    parent_id: &str,
    parent_key: &str,
    nodes: Vec<JmsAssetNode>,
) -> Vec<JmsAssetTreeNode> {
    use std::collections::{HashMap, HashSet};

    let mut all_nodes: HashMap<String, JmsAssetNode> = HashMap::new();
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

    for n in nodes {
        let id = n.id.clone();
        let p_id = n.p_id.clone();
        all_nodes.insert(id.clone(), n);
        children_map.entry(p_id).or_default().push(id);
    }

    // 找到属于该父节点的直接子节点：
    // p_id == parent_id 或 p_id == parent_key，或者其 p_id 不在 all_nodes 中
    let mut direct_child_ids: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    // 先检查显式匹配 parent_id 或 parent_key 的子项
    for p in [parent_id, parent_key] {
        if !p.is_empty() {
            if let Some(ids) = children_map.get(p) {
                for id in ids {
                    if seen.insert(id.clone()) {
                        direct_child_ids.push(id.clone());
                    }
                }
            }
        }
    }

    // 如果没有显式匹配，收集所有其 p_id 不在 all_nodes 中的顶层项
    if direct_child_ids.is_empty() {
        for (id, node) in &all_nodes {
            if id != parent_id && id != parent_key {
                if !all_nodes.contains_key(&node.p_id) && seen.insert(id.clone()) {
                    direct_child_ids.push(id.clone());
                }
            }
        }
    }

    // 排序：目录在前，资产在后
    direct_child_ids.sort_by(|a, b| {
        let node_a = all_nodes.get(a);
        let node_b = all_nodes.get(b);
        let a_is_asset = node_a
            .and_then(|n| n.meta.as_ref())
            .map(|m| m.node_type == "asset")
            .unwrap_or(false);
        let b_is_asset = node_b
            .and_then(|n| n.meta.as_ref())
            .map(|m| m.node_type == "asset")
            .unwrap_or(false);
        match (a_is_asset, b_is_asset) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => {
                let name_a = node_a.map(|n| n.name.as_str()).unwrap_or("");
                let name_b = node_b.map(|n| n.name.as_str()).unwrap_or("");
                name_a.cmp(name_b)
            }
        }
    });

    fn build_sub(
        node_id: &str,
        all_nodes: &HashMap<String, JmsAssetNode>,
        children_map: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
    ) -> Option<JmsAssetTreeNode> {
        if !visited.insert(node_id.to_string()) {
            return None;
        }
        let node = all_nodes.get(node_id)?.clone();
        let mut tree_node = JmsAssetTreeNode {
            node,
            children: Vec::new(),
            loaded: false,
        };

        if let Some(child_ids) = children_map.get(node_id) {
            let mut sorted_child_ids = child_ids.clone();
            sorted_child_ids.sort_by(|a, b| {
                let node_a = all_nodes.get(a);
                let node_b = all_nodes.get(b);
                let a_is_asset = node_a
                    .and_then(|n| n.meta.as_ref())
                    .map(|m| m.node_type == "asset")
                    .unwrap_or(false);
                let b_is_asset = node_b
                    .and_then(|n| n.meta.as_ref())
                    .map(|m| m.node_type == "asset")
                    .unwrap_or(false);
                match (a_is_asset, b_is_asset) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    _ => {
                        let name_a = node_a.map(|n| n.name.as_str()).unwrap_or("");
                        let name_b = node_b.map(|n| n.name.as_str()).unwrap_or("");
                        name_a.cmp(name_b)
                    }
                }
            });

            for child_id in &sorted_child_ids {
                if let Some(child) = build_sub(child_id, all_nodes, children_map, visited) {
                    tree_node.children.push(child);
                }
            }
            tree_node.loaded = true;
        }

        Some(tree_node)
    }

    let mut result = Vec::new();
    let mut visited = HashSet::new();
    for id in direct_child_ids {
        if let Some(node) = build_sub(&id, &all_nodes, &children_map, &mut visited) {
            result.push(node);
        }
    }

    result
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, name: &str, pid: &str, is_asset: bool) -> JmsAssetNode {
        JmsAssetNode {
            id: id.to_string(),
            name: name.to_string(),
            title: name.to_string(),
            p_id: pid.to_string(),
            is_parent: !is_asset,
            open: false,
            meta: Some(JmsNodeMeta {
                data: JmsNodeData {
                    id: id.to_string(),
                    key: id.to_string(),
                    value: name.to_string(),
                },
                node_type: if is_asset { "asset" } else { "node" }.to_string(),
            }),
        }
    }

    #[test]
    fn test_build_asset_tree_hierarchy() {
        let nodes = vec![
            make_node("1", "Default", "", false),
            make_node("1:1", "Production", "1", false),
            make_node("asset-1", "web-server-1", "1:1", true),
            make_node("asset-2", "root-server", "1", true),
        ];

        let tree = build_asset_tree(nodes);
        assert_eq!(tree.len(), 1);
        let root = &tree[0];
        assert_eq!(root.node.id, "1");
        assert_eq!(root.node.name, "Default");
        assert!(!root.is_asset());
        assert_eq!(root.children.len(), 2);

        // 文件夹优先：Production 在前，asset-2 在后
        assert_eq!(root.children[0].node.id, "1:1");
        assert!(!root.children[0].is_asset());
        assert_eq!(root.children[1].node.id, "asset-2");
        assert!(root.children[1].is_asset());

        // Production 下包含 asset-1
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].node.id, "asset-1");
        assert!(root.children[0].children[0].is_asset());
    }

    #[test]
    fn test_build_asset_tree_for_children() {
        let child_nodes = vec![
            make_node("1:1", "Production", "1", false),
            make_node("asset-1", "server-1", "1:1", true),
            make_node("asset-2", "server-2", "1", true),
        ];

        let children = build_asset_tree_for_children("1", "1", child_nodes);
        assert_eq!(children.len(), 2);
        // Production 文件夹
        assert_eq!(children[0].node.id, "1:1");
        assert!(!children[0].is_asset());
        assert_eq!(children[0].children.len(), 1);
        assert_eq!(children[0].children[0].node.id, "asset-1");

        // 资产
        assert_eq!(children[1].node.id, "asset-2");
        assert!(children[1].is_asset());
    }
}
