//! JMS 会话徽标（挂在外层标签栏尾部）
//!
//! - 绿色 `● JMS 已登录 user@host`：全局会话有效，点击 → 校验后复用会话打开/切换 JMS 资产树标签页（免登录）
//! - 灰色 `● JMS 已过期`：任一请求收到 401，点击 → 打开预填登录窗口
//! - 未登录：不渲染

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, AsyncApp, Context, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, h_flex};

use crate::home_tab::HomePage;
use crate::jms_session::{
    self, GlobalJmsSession, GlobalJmsSignal, JmsSessionInfo, push_notification_async,
};
use terminal_view::JmsSidebarContext;

/// JMS 会话徽标
pub struct JmsSessionBadge {
    home: gpui::WeakEntity<HomePage>,
    /// 后台校验是否进行中（防抖：连续点击不重复发请求）
    validating: Arc<std::sync::atomic::AtomicBool>,
}

impl JmsSessionBadge {
    pub fn new(home: gpui::WeakEntity<HomePage>, cx: &mut Context<Self>) -> Self {
        // 创建唤醒信号通道：jms 客户端 401 / 登录成功都会触发，徽标据此刷新
        let (tx, rx) = smol::channel::unbounded::<()>();
        cx.set_global(GlobalJmsSignal { tx });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            while rx.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            home,
            validating: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 绿色状态点击：立即复用会话打开/激活 JMS 标签页（使用缓存资产树，无网络等待），
    /// 同时后台静默校验刷新缓存；若会话已失效则徽标自动变灰
    fn open_session_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = current_session(cx) else {
            return;
        };
        tracing::info!("JMS badge: 打开/激活 jms-placeholder 标签页");

        // 立即打开（同步，无网络等待）
        let ctx = JmsSidebarContext {
            client: info.client.clone(),
            tree_roots: info.tree_roots.clone(),
            proxy: info.proxy.clone(),
            is_placeholder: true,
        };
        let _ = self.home.update(cx, |home, cx| {
            home.open_or_activate_jms_placeholder(ctx, window, cx);
        });

        // 后台静默校验（防抖）：成功刷新缓存树；401 → 徽标变灰 + toast
        if self
            .validating
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let validating = self.validating.clone();
        let mut client = info.client.clone();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = smol::spawn(async move { client.get_asset_tree().await }).await;

            let _ = validating.store(false, std::sync::atomic::Ordering::Relaxed);
            match result {
                Ok(nodes) => {
                    let tree = Arc::new(jms::build_asset_tree(nodes));
                    let _ = cx.update(|cx| {
                        jms_session::update_session_tree(cx, tree);
                    });
                }
                Err(_) => {
                    // 401 已在 client 内标记并触发信号 → 徽标自动变灰
                    let _ = cx.update(|cx| jms_session::signal_changed(cx));
                    push_notification_async(
                        cx,
                        "JMS 会话已过期，请点击灰色徽标重新登录".to_string(),
                    );
                }
            }
        })
        .detach();
    }

    /// 灰色状态点击：打开预填登录窗口
    fn open_relogin(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = current_session(cx) else {
            return;
        };
        let saved_id = info.saved_connection_id;
        let base_url = info.base_url.clone();
        let username = info.username.clone();
        let _ = self.home.update(cx, |home, cx| {
            home.open_jms_relogin(saved_id, base_url, username, _window, cx);
        });
    }
}

fn current_session(cx: &App) -> Option<JmsSessionInfo> {
    cx.try_global::<GlobalJmsSession>()
        .and_then(|g| g.session.clone())
}

impl Render for JmsSessionBadge {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(info) = current_session(cx) else {
            return div().into_any_element();
        };
        let expired = info.client.is_expired();
        let user = info.display_user();

        let (border, bg, fg) = if expired {
            (
                cx.theme().muted_foreground,
                cx.theme().muted_foreground.opacity(0.08),
                cx.theme().muted_foreground,
            )
        } else {
            (
                cx.theme().success,
                cx.theme().success.opacity(0.12),
                cx.theme().success,
            )
        };

        let dot_bg = if expired {
            cx.theme().muted_foreground
        } else {
            cx.theme().success
        };
        let tooltip_text: SharedString = if expired {
            format!("JMS 会话已过期 · {user}\n点击重新登录").into()
        } else {
            format!("JMS 会话有效 · {user}\n点击打开 JMS 资产树（免登录）").into()
        };

        h_flex()
            .id("jms-session-badge")
            .items_center()
            .gap_1p5()
            .h(px(26.0))
            .px_2p5()
            .mr_1()
            .rounded_full()
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .text_xs()
            .cursor_pointer()
            .flex_shrink_0()
            .when(!expired, |this| this.hover(|style| style.shadow_sm()))
            .tooltip(move |_window, cx| {
                cx.new(|_| gpui_component::tooltip::Tooltip::new(tooltip_text.clone()))
                    .into()
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if current_session(cx)
                    .map(|s| s.client.is_expired())
                    .unwrap_or(true)
                {
                    this.open_relogin(window, cx);
                } else {
                    this.open_session_tab(window, cx);
                }
            }))
            .child(
                div()
                    .w(px(8.0))
                    .h(px(8.0))
                    .rounded_full()
                    .bg(dot_bg)
                    .when(!expired, |this| this.shadow_sm()),
            )
            .child("JMS")
            .into_any_element()
    }
}
