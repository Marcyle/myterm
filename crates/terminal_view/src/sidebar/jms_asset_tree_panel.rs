//! JMS 资产树侧栏面板
//!
//! 在 JMS 终端 tab 内常驻显示资产树:展开目录(懒加载)、点击资产 → 内嵌账号列表 →
//! 选账号 → 创建 connect-token → 发事件让上层新开一个终端 tab(不替换当前会话)。
//!
//! 逻辑迁移自 `main/src/jms_connection_window.rs` 的资产树部分,把原来的
//! `Option<JmsClient>` + `take()` 临时借出改为**克隆借出**(client 长期常驻)。

use std::collections::HashSet;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};

/// 资产树面板事件
#[derive(Clone, Debug)]
pub enum JmsAssetTreePanelEvent {
    /// 选完账号,请求打开新的 JMS 终端
    OpenNewTerminal(jms::KokoConnectParams),
    /// 关闭面板
    Close,
}

/// JMS 资产树侧栏面板
pub struct JmsAssetTreePanel {
    /// 已认证的 JMS 客户端(克隆持有,只读)
    client: jms::JmsClient,
    /// 资产树根节点
    tree_roots: Vec<jms::JmsAssetTreeNode>,
    /// 已展开的节点 id
    expanded_ids: HashSet<String>,
    /// 预先算好的代理配置
    proxy: Option<jms::KokoProxy>,
    /// 账号选择覆盖层:为真时叠加显示账号列表
    show_account_selector: bool,
    /// 当前选中资产的可选账号
    pending_accounts: Vec<jms::JmsAssetAccount>,
    pending_asset_id: String,
    pending_asset_name: String,
    /// 异步加载中(取账号/建 token)
    is_busy: bool,
    error_message: Option<String>,
    /// 搜索输入框
    search_input: Entity<InputState>,
    /// 当前搜索关键字(为空表示展示完整树)
    search_query: String,
    /// 搜索结果(扁平的资产节点),仅在 search_query 非空时有效
    search_results: Vec<jms::JmsAssetTreeNode>,
    /// 搜索请求进行中
    is_searching: bool,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl JmsAssetTreePanel {
    pub fn new(
        client: jms::JmsClient,
        tree_roots: Vec<jms::JmsAssetTreeNode>,
        proxy: Option<jms::KokoProxy>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索资产"));

        // 订阅搜索输入变化,触发服务端搜索
        let input_entity = search_input.clone();
        let sub = cx.subscribe_in(
            &search_input,
            window,
            move |this, _state, event, _window, cx| {
                if let InputEvent::Change = event {
                    let value = input_entity.read(cx).value().trim().to_string();
                    this.handle_search_changed(value, cx);
                }
            },
        );

        Self {
            client,
            tree_roots,
            expanded_ids: HashSet::new(),
            proxy,
            show_account_selector: false,
            pending_accounts: Vec::new(),
            pending_asset_id: String::new(),
            pending_asset_name: String::new(),
            is_busy: false,
            error_message: None,
            search_input,
            search_query: String::new(),
            search_results: Vec::new(),
            is_searching: false,
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![sub],
        }
    }

    /// 搜索关键字变化:为空恢复树视图,非空走服务端搜索
    fn handle_search_changed(&mut self, query: String, cx: &mut Context<Self>) {
        self.search_query = query.clone();
        if query.is_empty() {
            self.search_results.clear();
            self.is_searching = false;
            cx.notify();
            return;
        }

        self.is_searching = true;
        self.error_message = None;
        cx.notify();

        let client = self.client.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut client = client;
            let query2 = query.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let r = client.search_assets(&query2).await;
                    (client, r)
                })
                .await
                .1;

            let _ = this.update(cx, |this, cx| {
                // 仅当结果对应的仍是当前关键字时才应用(避免过期响应覆盖)
                if this.search_query != query {
                    return;
                }
                this.is_searching = false;
                match result {
                    Ok(nodes) => {
                        this.search_results = nodes
                            .into_iter()
                            .map(|n| jms::JmsAssetTreeNode {
                                node: n,
                                children: Vec::new(),
                                loaded: true,
                            })
                            .collect();
                    }
                    Err(e) => {
                        tracing::warn!("搜索资产失败: {}", e);
                        this.error_message = Some(format!("搜索失败: {}", e));
                        this.search_results.clear();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 展开/折叠节点,必要时懒加载子节点
    fn handle_node_toggle(&mut self, node_id: String, load_key: String, loaded: bool, cx: &mut Context<Self>) {
        if self.expanded_ids.contains(&node_id) {
            self.expanded_ids.remove(&node_id);
            cx.notify();
            return;
        }
        self.expanded_ids.insert(node_id.clone());
        if loaded {
            cx.notify();
            return;
        }

        let client = self.client.clone();
        cx.notify();
        let node_id_for_update = node_id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut client = client;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let r = client.get_node_children(&load_key).await;
                    (client, r)
                })
                .await
                .1;

            match result {
                Ok(nodes) => {
                    let children: Vec<jms::JmsAssetTreeNode> = nodes
                        .into_iter()
                        .filter(|n| n.id != node_id_for_update)
                        .map(|n| jms::JmsAssetTreeNode {
                            node: n,
                            children: Vec::new(),
                            loaded: false,
                        })
                        .collect();
                    let _ = this.update(cx, |this, cx| {
                        this.set_node_children(&node_id_for_update, children);
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::warn!("懒加载子节点失败: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.error_message = Some(format!("加载子节点失败: {}", e));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 在树中按 ID 查找节点并设置其子节点,标记为已加载
    fn set_node_children(&mut self, node_id: &str, children: Vec<jms::JmsAssetTreeNode>) {
        fn walk(
            nodes: &mut [jms::JmsAssetTreeNode],
            node_id: &str,
            children: &mut Option<Vec<jms::JmsAssetTreeNode>>,
        ) -> bool {
            for n in nodes.iter_mut() {
                if n.node.id == node_id {
                    n.children = children.take().unwrap_or_default();
                    n.loaded = true;
                    return true;
                }
                if walk(&mut n.children, node_id, children) {
                    return true;
                }
            }
            false
        }
        let mut opt = Some(children);
        walk(&mut self.tree_roots, node_id, &mut opt);
    }

    /// 点击资产,获取账号列表并显示账号选择覆盖层
    fn handle_asset_click(&mut self, asset_id: String, cx: &mut Context<Self>) {
        let asset_name = self
            .tree_roots
            .iter()
            .flat_map(|r| Self::find_asset(r, &asset_id))
            .next()
            .unwrap_or_else(|| asset_id.clone());

        let client = self.client.clone();
        self.is_busy = true;
        self.error_message = None;
        cx.notify();

        let asset_id2 = asset_id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut client = client;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let r = client.list_asset_accounts(&asset_id2).await;
                    (client, r)
                })
                .await
                .1;

            match result {
                Ok(accounts) => {
                    let _ = this.update(cx, |this, cx| {
                        this.is_busy = false;
                        this.pending_accounts = accounts;
                        this.pending_asset_id = asset_id;
                        this.pending_asset_name = asset_name;
                        this.show_account_selector = true;
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::warn!("获取账号列表失败: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.is_busy = false;
                        this.error_message = Some(format!("获取账号列表失败: {}", e));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 选择账号 → 创建 connect-token → 发事件打开新终端
    fn handle_account_selected(&mut self, account_name: String, cx: &mut Context<Self>) {
        let mut client = self.client.clone();
        let asset_id = self.pending_asset_id.clone();
        let asset_name = self.pending_asset_name.clone();
        let proxy = self.proxy.clone();

        self.is_busy = true;
        self.error_message = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let account_for_title = account_name.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let r = client.create_connect_token(&asset_id, &account_name).await;
                    (client, r)
                })
                .await;
            let (client, connect_result) = result;

            match connect_result {
                Ok(token) => {
                    let params = jms::KokoConnectParams {
                        base_url: client.base_url().to_string(),
                        token_id: token.id.clone(),
                        session_cookie: client.session_cookie().map(|s| s.to_string()),
                        org_id: client.org_id().to_string(),
                        proxy,
                        title: format!("[JMS] {} ({})", asset_name, account_for_title),
                    };
                    let _ = this.update(cx, |this, cx| {
                        this.is_busy = false;
                        this.show_account_selector = false;
                        cx.emit(JmsAssetTreePanelEvent::OpenNewTerminal(params));
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::warn!("创建连接 token 失败: {}", e);
                    let _ = this.update(cx, |this, cx| {
                        this.is_busy = false;
                        this.error_message = Some(format!("连接失败: {}", e));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 在树中按 ID 查找资产名称
    fn find_asset(node: &jms::JmsAssetTreeNode, asset_id: &str) -> Option<String> {
        if node.node.id == asset_id {
            return Some(node.node.title.clone());
        }
        for child in &node.children {
            if let Some(name) = Self::find_asset(child, asset_id) {
                return Some(name);
            }
        }
        None
    }

    // ===== 渲染 =====

    fn render_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // 搜索激活时显示扁平结果列表
        if !self.search_query.is_empty() {
            return self.render_search_results(cx);
        }
        if self.tree_roots.is_empty() {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("没有可用的资产"),
                )
                .into_any_element();
        }
        let rows = self.collect_visible_rows(cx);
        v_flex()
            .flex_1()
            .overflow_y_scrollbar()
            .child(div().w_full().children(rows))
            .into_any_element()
    }

    /// 渲染搜索结果(扁平资产列表)
    fn render_search_results(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.is_searching {
            return h_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(IconName::Loader).with_size(Size::Small))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("搜索中..."),
                )
                .into_any_element();
        }
        if self.search_results.is_empty() {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("未找到匹配的资产"),
                )
                .into_any_element();
        }
        let rows: Vec<AnyElement> = self
            .search_results
            .clone()
            .iter()
            .map(|node| self.render_tree_row(node, 0, true, false, false, cx))
            .collect();
        v_flex()
            .flex_1()
            .overflow_y_scrollbar()
            .child(div().w_full().children(rows))
            .into_any_element()
    }

    fn collect_visible_rows(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        self.collect_rows(&self.tree_roots.clone(), 0, &mut rows, cx);
        rows
    }

    fn collect_rows(
        &mut self,
        nodes: &[jms::JmsAssetTreeNode],
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        for node in nodes {
            let is_asset = node.is_asset();
            let has_children = node.node.is_parent;
            let is_expanded = self.expanded_ids.contains(&node.node.id);
            rows.push(self.render_tree_row(node, depth, is_asset, has_children, is_expanded, cx));
            if is_expanded {
                self.collect_rows(&node.children, depth + 1, rows, cx);
            }
        }
    }

    fn render_tree_row(
        &mut self,
        node: &jms::JmsAssetTreeNode,
        depth: usize,
        is_asset: bool,
        has_children: bool,
        is_expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node_id = node.node.id.clone();
        let node_id_for_toggle = node_id.clone();
        let load_key = node.load_key();
        let loaded = node.loaded;
        let real_asset_id = node
            .node
            .meta
            .as_ref()
            .map(|m| m.data.id.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| node_id.clone());
        // 资产叶子节点 name 是完整资产名(title 只是 IP),目录节点两者相同,统一用 name
        let title = node.node.name.clone();
        let title_tooltip: gpui::SharedString = title.clone().into();
        // 侧栏较窄,缩进收紧到 12px
        let indent = px(depth as f32 * 12.0);

        h_flex()
            .id(gpui::SharedString::from(format!("jms-node-{}", node_id)))
            .pl(indent)
            .py_1()
            .px_2()
            .gap_1()
            .items_center()
            .w_full()
            .overflow_hidden()
            .cursor_pointer()
            .rounded(px(4.0))
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(if has_children {
                Icon::new(if is_expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(Size::Small)
                .text_color(cx.theme().muted_foreground)
                .into_any_element()
            } else {
                div().w(px(16.0)).flex_shrink_0().into_any_element()
            })
            .child(if is_asset {
                Icon::new(IconName::SquareTerminal)
                    .with_size(Size::Small)
                    .text_color(cx.theme().accent)
                    .into_any_element()
            } else {
                Icon::new(IconName::Folder)
                    .with_size(Size::Small)
                    .text_color(cx.theme().foreground)
                    .into_any_element()
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .id(gpui::SharedString::from(format!("jms-node-title-{}", node_id)))
                    .tooltip(move |window, cx| {
                        Tooltip::new(title_tooltip.clone()).build(window, cx)
                    })
                    .child(title),
            )
            .when(has_children, |this| {
                this.on_click(cx.listener(move |this, _, _window, cx| {
                    this.handle_node_toggle(node_id_for_toggle.clone(), load_key.clone(), loaded, cx);
                }))
            })
            .when(!has_children, |this| {
                this.on_click(cx.listener(move |this, _, _window, cx| {
                    this.handle_asset_click(real_asset_id.clone(), cx);
                }))
            })
            .into_any_element()
    }

    fn render_account_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .child(
                        Button::new("jms-back-to-tree")
                            .ghost()
                            .icon(IconName::ChevronLeft)
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.show_account_selector = false;
                                this.error_message = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(self.pending_asset_name.clone()),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px_2()
                    .gap_1()
                    .children(self.pending_accounts.iter().map(|account| {
                        let name = account.name.clone();
                        let username = account.username.clone();
                        let name_for_click = name.clone();
                        let is_privileged = account.privileged;
                        div()
                            .id(gpui::SharedString::from(format!("jms-acct-{}", account.id)))
                            .w_full()
                            .p_2()
                            .rounded(px(6.0))
                            .bg(cx.theme().secondary)
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().list_hover))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.handle_account_selected(name_for_click.clone(), cx);
                            }))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(username),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(name),
                                    )
                                    .when(is_privileged, |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .px_1()
                                                .rounded(px(3.0))
                                                .bg(cx.theme().accent.opacity(0.2))
                                                .text_color(cx.theme().accent)
                                                .child("特权"),
                                        )
                                    }),
                            )
                            .into_any_element()
                    })),
            )
            .into_any_element()
    }
}

impl EventEmitter<JmsAssetTreePanelEvent> for JmsAssetTreePanel {}

impl Focusable for JmsAssetTreePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for JmsAssetTreePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            // 标题栏
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("JMS 资产"),
                    )
                    .child(
                        Button::new("jms-tree-close")
                            .ghost()
                            .icon(IconName::Close)
                            .with_size(Size::Small)
                            .on_click(cx.listener(|_this, _, _window, cx| {
                                cx.emit(JmsAssetTreePanelEvent::Close);
                            })),
                    ),
            )
            // 搜索框
            .child(
                div()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Input::new(&self.search_input).w_full().small()),
            )
            // 错误提示
            .when_some(self.error_message.clone(), |this, msg| {
                this.child(
                    div()
                        .mx_2()
                        .my_1()
                        .p_2()
                        .rounded(px(6.0))
                        .bg(cx.theme().danger.opacity(0.1))
                        .border_1()
                        .border_color(cx.theme().danger)
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(msg),
                )
            })
            // 主体:资产树 或 账号选择覆盖层
            .child(
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .child(self.render_tree(cx))
                    .when(self.show_account_selector, |this| {
                        this.child(
                            div()
                                .absolute()
                                .inset_0()
                                .child(self.render_account_selector(cx)),
                        )
                    }),
            )
    }
}
