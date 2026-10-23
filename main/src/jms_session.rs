//! 全局 JMS 登录会话仓库
//!
//! 登录成功后把已认证的 [`jms::JmsClient`]（内含 `jms_sessionid`）、已加载的资产树、
//! Koko 代理配置等写入全局状态，使**会话生命周期独立于 JMS 标签页**：
//!
//! - 关闭 JMS 标签页后，会话仍保留 → 徽标绿色，点击即可免登录重开资产树；
//! - 服务端会话失效（任一副本请求收到 401）→ 徽标自动变灰；
//! - 点击灰色徽标 → 打开预填的登录窗口重新登录。

use std::sync::Arc;

use gpui::{App, AppContext as _, BorrowAppContext as _, Global};
use jms::JmsClient;

/// 一条已登录的 JMS 会话
#[derive(Clone)]
pub struct JmsSessionInfo {
    /// 已认证客户端（Clone 共享同一登录态，401 时全体标记过期）
    pub client: JmsClient,
    /// 已加载的资产树根节点（重开标签页时复用，避免重新拉取）
    pub tree_roots: Arc<Vec<jms::JmsAssetTreeNode>>,
    /// Koko 代理配置
    pub proxy: Option<jms::KokoProxy>,
    /// 登录用户名（用于徽标展示与重登预填）
    pub username: String,
    /// 服务器地址
    pub base_url: String,
    /// 关联的已保存连接 ID（过期重登时预填）
    pub saved_connection_id: Option<i64>,
}

impl JmsSessionInfo {
    /// 徽标上展示的 `用户名@主机`
    pub fn display_user(&self) -> String {
        let host = self
            .base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("");
        if self.username.is_empty() {
            host.to_string()
        } else {
            format!("{}@{}", self.username, host)
        }
    }
}

/// 全局会话仓库（无会话时徽标隐藏）
#[derive(Default)]
pub struct GlobalJmsSession {
    pub session: Option<JmsSessionInfo>,
}
impl Global for GlobalJmsSession {}

/// 徽标唤醒信号：任何会话状态变化（登录成功/失效）都会投递，
/// 徽标实体监听该通道并刷新渲染。
#[derive(Clone)]
pub struct GlobalJmsSignal {
    pub tx: smol::channel::Sender<()>,
}
impl Global for GlobalJmsSignal {}

/// 写入（或替换）全局会话
pub fn store_session(cx: &mut App, info: JmsSessionInfo) {
    cx.set_global(GlobalJmsSession {
        session: Some(info),
    });
    signal_changed(cx);
}

/// 更新全局会话中的资产树（徽标点击校验成功后刷新缓存）
pub fn update_session_tree(cx: &mut App, tree_roots: Arc<Vec<jms::JmsAssetTreeNode>>) {
    if cx.has_global::<GlobalJmsSession>() {
        cx.update_global(|session: &mut GlobalJmsSession, _cx| {
            if let Some(info) = session.session.as_mut() {
                info.tree_roots = tree_roots;
            }
        });
    }
    signal_changed(cx);
}

/// 通知徽标刷新（线程安全，可从后台线程调用）
pub fn signal_changed(cx: &mut App) {
    if let Some(sig) = cx.try_global::<GlobalJmsSignal>() {
        let _ = sig.tx.try_send(());
    }
}

/// 通过活动窗口弹出通知（供异步上下文使用）
pub fn push_notification_async(cx: &mut gpui::AsyncApp, message: String) {
    let _ = cx.update(|cx| {
        if let Some(window_id) = cx.active_window() {
            let _ = cx.update_window(window_id, |_, window, cx| {
                use gpui_component::WindowExt as _;
                window.push_notification(message.clone(), cx);
            });
        }
    });
}
