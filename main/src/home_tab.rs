use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, Hsla, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div,
    px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::{List, ListState},
    menu::PopupMenuItem,
    popover::Popover,
    v_flex,
};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event, get_notifier};
use one_core::crypto;
use one_core::key_storage;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ActiveConnections, ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection,
    Workspace, WorkspaceRepository,
};
use one_core::tab_container::{TabContainer, TabContainerEvent, TabContent, TabContentEvent};
use port_forwarding::{
    DynamicForwardingRequest, LocalForwardingRequest, PortForwardingRuntime,
    build_dynamic_forwarding_request, build_local_forwarding_request,
};
use port_forwarding_view::{PortForwardingFormWindow, PortForwardingFormWindowConfig};
use rust_i18n::t;
use terminal_view::{SshFormWindow, SshFormWindowConfig};

use crate::home::home_connection_quick_open::ConnectionQuickOpenDelegate;
use crate::home::home_export_import::{
    ExportFormat, ExportScope, ImportCategory, ImportPreflightReport, MergeStrategy,
    build_export_item, generate_export_content, parse_import_payload, plan_import_execution,
    preflight_import, sample_import_json, save_export_file,
};
use crate::home::home_strategy::build_connection_open_strategy;
use crate::home::home_workspace_filter::show_workspace_dialog;
use crate::new_connection::NewConnectionWindow;
use one_ui::{CardDensity, ConnectionCard, EmptyState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RailView {
    #[default]
    All,
    Favorite,
    Recent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportStep {
    #[default]
    Step1,
    Step2,
}

actions!(
    home_tab,
    [
        OpenConnectionQuickOpen,
        NewConnectionShortcut,
        OpenLocalTerminalShortcut,
        FocusSearchShortcut,
        EscapeShortcut,
        OpenSelectedConnection,
        SelectPreviousConnection,
        SelectNextConnection
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

/// 首页方向键/回车键绑定（无用户覆盖，init 与 refresh 相同）。
fn navigation_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", OpenSelectedConnection, Some("HomePage")),
        KeyBinding::new("up", SelectPreviousConnection, Some("HomePage")),
        KeyBinding::new("left", SelectPreviousConnection, Some("HomePage")),
        KeyBinding::new("down", SelectNextConnection, Some("HomePage")),
        KeyBinding::new("right", SelectNextConnection, Some("HomePage")),
    ]
}

/// 首页快捷键清单：action_id → 默认键。init 与 refresh 两条注册路径共用。
fn home_shortcut_specs() -> Vec<(&'static str, Vec<&'static str>)> {
    use one_core::keybindings::platform_shortcut as platform;
    vec![
        (action_id::HOME_QUICK_OPEN, vec![platform("cmd-o", "alt-o")]),
        (
            action_id::HOME_NEW_CONNECTION,
            vec![platform("cmd-n", "alt-n")],
        ),
    ]
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = navigation_keybindings();
    keybindings.push(KeyBinding::new(
        "cmd-t",
        OpenLocalTerminalShortcut,
        Some("HomePage"),
    ));
    keybindings.push(KeyBinding::new(
        "ctrl-t",
        OpenLocalTerminalShortcut,
        Some("HomePage"),
    ));
    keybindings.push(KeyBinding::new(
        "cmd-k",
        FocusSearchShortcut,
        Some("HomePage"),
    ));
    keybindings.push(KeyBinding::new(
        "ctrl-k",
        FocusSearchShortcut,
        Some("HomePage"),
    ));
    keybindings.push(KeyBinding::new("escape", EscapeShortcut, Some("HomePage")));
    keybindings.extend(
        home_shortcut_specs()
            .into_iter()
            .flat_map(|(id, defaults)| {
                shortcuts_for(cx, id, &defaults)
                    .into_iter()
                    .map(move |key| match id {
                        action_id::HOME_QUICK_OPEN => {
                            KeyBinding::new(&key, OpenConnectionQuickOpen, None)
                        }
                        action_id::HOME_NEW_CONNECTION => {
                            KeyBinding::new(&key, NewConnectionShortcut, None)
                        }
                        _ => unreachable!("home_shortcut_specs 只包含已知 action_id"),
                    })
            }),
    );
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = navigation_keybindings();
    for (id, defaults) in home_shortcut_specs() {
        let rebound = match id {
            action_id::HOME_QUICK_OPEN => {
                rebind_keybindings(cx, id, &defaults, None, OpenConnectionQuickOpen)
            }
            action_id::HOME_NEW_CONNECTION => {
                rebind_keybindings(cx, id, &defaults, None, NewConnectionShortcut)
            }
            _ => unreachable!("home_shortcut_specs 只包含已知 action_id"),
        };
        keybindings.extend(rebound);
    }
    keybindings
}

// HomePage Entity - 管理 home 页面的所有状态

pub struct HomePage {
    focus_handle: FocusHandle,
    selected_filter: ConnectionType,
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) connections: Vec<StoredConnection>,
    pub(crate) tab_container: Entity<TabContainer>,
    search_input: Entity<InputState>,
    search_query: Entity<String>,
    pub(crate) editing_connection_id: Option<i64>,
    selected_connection_id: Option<i64>,
    pub(crate) filtered_workspace_ids: HashSet<i64>,
    pub(crate) workspace_filter_active: bool,
    #[allow(dead_code)]
    pub(crate) workspace_filter_open: bool,
    pub(crate) _subscriptions: HashMap<String, Vec<Subscription>>,
    port_forwarding_runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    master_key_dialog_open: bool,
    master_key_unlock_prompt_pending: bool,
    pub(crate) pending_jms_connections: Vec<StoredConnection>,
    pub(crate) pending_jms_koko: Vec<(
        jms::KokoConnectParams,
        Option<terminal_view::JmsSidebarContext>,
    )>,
    /// 登录成功后待打开的 JMS 占位终端(只有资产树,无连接)
    pub(crate) pending_jms_placeholder: Vec<terminal_view::JmsSidebarContext>,
    /// 折叠的分组名称集合
    pub(crate) collapsed_groups: HashSet<String>,
    /// 紧凑卡片密度：网格 vs 列表
    pub(crate) density: CardDensity,
    /// 左栏视图：全部 / 收藏 / 最近
    pub(crate) rail_view: RailView,
    /// 当前选中的工作区分组（None 表示全选）
    pub(crate) selected_group_id: Option<i64>,
    /// 顶栏协议筛选：None 为全部，Some 为 SSH/JMS 等
    pub(crate) protocol_filter: Option<ConnectionType>,
    /// 顶栏仅看在线开关
    pub(crate) online_only_filter: bool,
    /// 批量选择的连接 ID 集合
    pub(crate) selected_connection_ids: HashSet<i64>,
    /// 导出弹层状态
    pub(crate) export_popover_open: bool,
    pub(crate) export_scope: ExportScope,
    pub(crate) export_format: ExportFormat,
    pub(crate) export_include_passwords: bool,
    /// 导入模态状态
    pub(crate) import_modal_open: bool,
    pub(crate) import_step: ImportStep,
    pub(crate) import_preflight_report: Option<ImportPreflightReport>,
    pub(crate) import_merge_strategy: MergeStrategy,
}

impl HomePage {
    pub fn new(
        tab_container: Entity<TabContainer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_query = cx.new(|_| String::new());
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Home.search_placeholder"))
                .clean_on_escape()
        });

        // 订阅搜索输入变化
        let query_clone = search_query.clone();
        cx.subscribe_in(
            &search_input,
            window,
            move |_this, _input, event, _window, cx| {
                if let InputEvent::Change = event {
                    query_clone.update(cx, |q, cx| {
                        *q = _input.read(cx).text().to_string();
                        cx.notify();
                    });
                    cx.notify();
                }
            },
        )
        .detach();

        let mut page = Self {
            focus_handle: cx.focus_handle(),
            selected_filter: ConnectionType::All,
            workspaces: Vec::new(),
            connections: Vec::new(),
            tab_container: tab_container.clone(),
            search_input,
            search_query,
            editing_connection_id: None,
            selected_connection_id: None,
            filtered_workspace_ids: HashSet::new(),
            workspace_filter_active: false,
            workspace_filter_open: false,
            _subscriptions: HashMap::new(),
            port_forwarding_runtime: Arc::new(
                tokio::sync::Mutex::new(PortForwardingRuntime::new()),
            ),
            master_key_dialog_open: false,
            master_key_unlock_prompt_pending: crypto::has_repo_password_set()
                && !crypto::has_master_key(),
            pending_jms_connections: Vec::new(),
            pending_jms_koko: Vec::new(),
            pending_jms_placeholder: Vec::new(),
            collapsed_groups: HashSet::new(),
            density: CardDensity::Grid,
            rail_view: RailView::All,
            selected_group_id: None,
            protocol_filter: None,
            online_only_filter: false,
            selected_connection_ids: HashSet::new(),
            export_popover_open: false,
            export_scope: ExportScope::All,
            export_format: ExportFormat::Json,
            export_include_passwords: false,
            import_modal_open: false,
            import_step: ImportStep::Step1,
            import_preflight_report: None,
            import_merge_strategy: MergeStrategy::KeepLocal,
        };

        // 异步加载工作区
        page.load_workspaces(cx);

        // 加载连接
        page.load_connections(cx);

        // 订阅全局连接事件，当连接创建/更新时刷新列表
        if let Some(notifier) = get_notifier(cx) {
            cx.subscribe(
                &notifier,
                |this, _, event: &ConnectionDataEvent, cx| match event {
                    ConnectionDataEvent::ConnectionCreated { connection } => {
                        // 立即将新连接添加到列表，避免异步加载的时序问题
                        this.connections.push(connection.clone());
                        cx.notify();
                        // 然后异步重新加载以确保数据一致性
                        this.load_connections(cx);
                    }
                    ConnectionDataEvent::ConnectionUpdated { connection } => {
                        // 立即更新列表中的连接，避免异步加载的时序问题
                        if let Some(pos) =
                            this.connections.iter().position(|c| c.id == connection.id)
                        {
                            this.connections[pos] = connection.clone();
                        } else {
                            // 如果找不到，添加到列表
                            this.connections.push(connection.clone());
                        }
                        cx.notify();
                        // 然后异步重新加载以确保数据一致性
                        this.load_connections(cx);
                    }
                    ConnectionDataEvent::ConnectionDeleted { connection_id } => {
                        // 立即从列表中移除连接
                        this.connections.retain(|c| c.id != Some(*connection_id));
                        cx.notify();
                        // 然后异步重新加载以确保数据一致性
                        this.load_connections(cx);
                    }
                    ConnectionDataEvent::WorkspaceCreated { .. }
                    | ConnectionDataEvent::WorkspaceUpdated { .. }
                    | ConnectionDataEvent::WorkspaceDeleted { .. } => {
                        this.load_workspaces(cx);
                    }
                    ConnectionDataEvent::SchemaChanged { .. } => {
                        // SchemaChanged 由 db_tree_view 处理，此处无需操作
                    }
                },
            )
            .detach();
        }

        // 订阅 TabContainer 事件（标签页激活、关闭、复制等）
        let tab_container_for_events = tab_container.clone();
        cx.subscribe_in(
            &tab_container_for_events,
            window,
            move |this, _, event: &TabContainerEvent, window, cx| match event {
                TabContainerEvent::TabDuplicated { index } => {
                    this.duplicate_tab_by_index(*index, window, cx);
                }
                TabContainerEvent::TabReconnectRequested { index } => {
                    this.reconnect_tab_by_index(*index, window, cx);
                }
                TabContainerEvent::TabClosed { id } => {
                    this.cleanup_tab_subscriptions(id);
                }
                _ => {}
            },
        )
        .detach();

        page
    }

    pub(crate) fn store_tab_subscription(&mut self, tab_id: &str, subscription: Subscription) {
        self._subscriptions
            .entry(tab_id.to_string())
            .or_default()
            .push(subscription);
    }

    pub(crate) fn cleanup_tab_subscriptions(&mut self, tab_id: &str) {
        self._subscriptions.remove(tab_id);
    }

    fn load_workspaces(&mut self, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = (|| {
                let repo = storage
                    .get::<WorkspaceRepository>()
                    .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))?;
                repo.list()
            })();

            match result {
                Ok(workspaces) => {
                    _ = this.update(cx, |this, cx| {
                        this.workspaces = workspaces;
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Task join error: {}", e);
                }
            }
        })
        .detach();
    }

    pub(crate) fn load_connections(&mut self, cx: &mut Context<Self>) {
        if self.saved_connections_locked() {
            tracing::warn!("主密钥未解锁，暂缓加载本地连接，避免将加密密码解密为空");
            self.connections.clear();
            cx.notify();
            return;
        }

        let storage = cx.global::<GlobalStorageState>().storage.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = (|| {
                let repo = storage
                    .get::<ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
                repo.list()
            })();

            match result {
                Ok(connections) => {
                    _ = this.update(cx, |this, cx| {
                        this.connections = connections;
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Task join error: {}", e);
                }
            }
        })
        .detach();
    }

    fn refresh_local_home_data(&mut self, cx: &mut Context<Self>) {
        self.load_workspaces(cx);
        self.load_connections(cx);
    }

    /// 复制连接，创建一个副本
    fn duplicate_connection(
        &mut self,
        conn: StoredConnection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result: anyhow::Result<StoredConnection> = (|| {
                let repo = storage
                    .get::<ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

                // 获取现有连接名称列表，用于生成唯一名称
                let existing_names: HashSet<String> = repo
                    .list()
                    .unwrap_or_default()
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();

                // 生成新的唯一名称
                let new_name = generate_duplicate_name(&conn.name, &existing_names);

                // 克隆连接，清除 id 和云同步相关字段
                let mut new_conn = conn.clone();
                new_conn.id = None;
                new_conn.name = new_name;
                new_conn.owner_id = None;

                // 保存新连接
                repo.insert(&mut new_conn)?;
                Ok(new_conn)
            })();

            match result {
                Ok(saved_conn) => {
                    // 发出 ConnectionCreated 事件，首页自动刷新
                    _ = this.update(cx, |_this, cx| {
                        if let Some(notifier) = get_notifier(cx) {
                            notifier.update(cx, |_, cx| {
                                cx.emit(ConnectionDataEvent::ConnectionCreated {
                                    connection: saved_conn,
                                });
                            });
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("复制连接失败: {}", e);
                }
            }
        })
        .detach();
    }

    fn confirm_delete_connection(
        &mut self,
        conn_id: i64,
        conn_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_active = cx.global::<ActiveConnections>().is_active(conn_id);
        let view = cx.entity().clone();

        if is_active {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                dialog
                    .title(t!("Connection.in_use_title").to_string().into_any_element())
                    .child(
                        t!("Connection.in_use_cannot_delete", conn_name = conn_name)
                            .to_string()
                            .into_any_element(),
                    )
                    .alert()
            });
        } else {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                let view_clone = view.clone();
                dialog
                    .title(t!("Common.delete").to_string().into_any_element())
                    .child(
                        t!("Connection.delete_confirm", conn_name = conn_name)
                            .to_string()
                            .into_any_element(),
                    )
                    .confirm()
                    .on_ok(move |_, _, cx| {
                        let _ = view_clone.update(cx, |this, cx| {
                            this.delete_connection(conn_id, cx);
                        });
                        true
                    })
            });
        }
    }

    fn delete_connection(&mut self, conn_id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 删除本地连接
            let result = (|| {
                let repo = storage
                    .get::<ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
                repo.delete(conn_id)
            })();

            match result {
                Ok(_) => {
                    _ = this.update(cx, |this, cx| {
                        this.connections.retain(|c| c.id != Some(conn_id));
                        this.selected_connection_ids.remove(&conn_id);
                        if this.selected_connection_id == Some(conn_id) {
                            this.selected_connection_id = None;
                        }
                        emit_connection_event(
                            ConnectionDataEvent::ConnectionDeleted {
                                connection_id: conn_id,
                            },
                            cx,
                        );
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to delete connection: {}", e);
                }
            }
        })
        .detach();
    }

    pub(crate) fn show_connection_quick_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let parent = cx.entity();
        let connections = self.connections.clone();
        let list = cx.new(|cx| {
            let mut delegate = ConnectionQuickOpenDelegate::new(parent);
            delegate.update_items(&connections);
            ListState::new(delegate, window, cx).searchable(true)
        });

        let list_for_focus = list.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .title(t!("Home.open_connection").to_string())
                .w(px(520.0))
                .child(
                    v_flex().gap_2().child(
                        List::new(&list)
                            .w_full()
                            .max_h(px(360.0))
                            .p(px(8.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(cx.theme().radius),
                    ),
                )
                .alert()
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .ok_text(t!("Common.close")),
                )
        });
        // 将焦点设置到 List 搜索框，使上下键和 Enter 键可用
        list_for_focus.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }

    pub(crate) fn show_new_connection_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_connection_id = None;

        if !self.ensure_master_key_ready_for_new_connection(window, cx) {
            return;
        }

        let parent = cx.entity();
        let parent_window = window.window_handle();
        open_popup_window(
            PopupWindowOptions::new(t!("Home.new_connection").to_string()).size(1100.0, 700.0),
            move |window, cx| {
                cx.new(|cx| NewConnectionWindow::new(parent, parent_window, window, cx))
            },
            cx,
        );
    }

    /// 从已保存的 JMS 连接打开窗口,自动填充 URL/用户名/密码
    pub(crate) fn open_jms_connection_prefilled(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::jms_connection_window::JmsConnectionWindow;
        use one_core::popup_window::{PopupWindowOptions, open_popup_window};

        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        // 解密 params 后解析(密码字段已加密存储)
        let decrypted = connection.with_decrypted_params();
        let params = match decrypted.to_jms_params() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("解析 JMS 连接参数失败: {e}");
                return;
            }
        };
        let prefill = Some((connection.id, params));
        let parent = cx.entity();
        open_popup_window(
            PopupWindowOptions::new("JMS 连接".to_string()).size(800.0, 600.0),
            move |window, cx| {
                cx.new(|cx| {
                    JmsConnectionWindow::new_with_prefill(
                        parent.clone(),
                        prefill.clone(),
                        window,
                        cx,
                    )
                })
            },
            cx,
        );
    }

    pub(crate) fn open_connection_from_quick(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let connection = connection.clone();
        self.touch_connection_last_used(connection.id, cx);
        let workspace = connection
            .workspace_id
            .and_then(|id| self.workspaces.iter().find(|w| w.id == Some(id)).cloned());
        let strategy = build_connection_open_strategy(connection, workspace);
        strategy.open(self, window, cx);
        cx.notify();
    }

    fn touch_connection_last_used(&mut self, connection_id: Option<i64>, cx: &mut Context<Self>) {
        let Some(connection_id) = connection_id else {
            return;
        };
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))
            .and_then(|repo| repo.touch_last_used(connection_id));

        if let Err(err) = result {
            tracing::warn!("更新连接最近使用时间失败: {err}");
            return;
        }
        self.load_connections(cx);
    }

    fn open_selected_connection(
        &mut self,
        _: &OpenSelectedConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.selected_connection_id else {
            return;
        };
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == Some(connection_id))
            .cloned()
        else {
            return;
        };
        self.open_connection_from_quick(&connection, window, cx);
    }
    fn handle_local_terminal_shortcut(
        &mut self,
        _: &OpenLocalTerminalShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_terminal_tab(window, cx);
    }

    fn handle_focus_search_shortcut(
        &mut self,
        _: &FocusSearchShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }

    fn handle_escape_shortcut(
        &mut self,
        _: &EscapeShortcut,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.export_popover_open {
            self.export_popover_open = false;
            cx.notify();
        } else if self.import_modal_open {
            self.import_modal_open = false;
            cx.notify();
        } else if !self.selected_connection_ids.is_empty() {
            self.selected_connection_ids.clear();
            cx.notify();
        } else if !self.search_query.read(cx).is_empty() {
            self.search_input.update(cx, |input, cx| {
                input.set_value("", _window, cx);
            });
            self.search_query.update(cx, |q, cx| {
                q.clear();
                cx.notify();
            });
            cx.notify();
        }
    }

    pub(crate) fn toggle_favorite_connection(&mut self, conn_id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = (|| -> anyhow::Result<bool> {
            let repo = storage
                .get::<ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
            repo.toggle_favorite(conn_id)
        })();

        match result {
            Ok(is_fav) => {
                if let Some(conn) = self.connections.iter_mut().find(|c| c.id == Some(conn_id)) {
                    conn.is_favorite = is_fav;
                }
                cx.notify();
            }
            Err(e) => {
                tracing::error!("切换收藏状态失败: {}", e);
            }
        }
    }

    pub(crate) fn handle_batch_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.selected_connection_ids.len();
        if count == 0 {
            return;
        }

        let view = cx.entity().clone();
        let ids: Vec<i64> = self.selected_connection_ids.iter().copied().collect();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            let ids_clone = ids.clone();
            dialog
                .title("批量删除连接".to_string().into_any_element())
                .child(
                    format!("确定要删除选中的 {} 个连接吗？删除后将无法恢复。", count)
                        .into_any_element(),
                )
                .confirm()
                .on_ok(move |_, window, cx| {
                    let _ = view_clone.update(cx, |this, cx| {
                        let storage = cx.global::<GlobalStorageState>().storage.clone();
                        if let Some(repo) = storage.get::<ConnectionRepository>() {
                            for id in &ids_clone {
                                let _ = repo.delete(*id);
                                this.connections.retain(|c| c.id != Some(*id));
                                emit_connection_event(
                                    ConnectionDataEvent::ConnectionDeleted { connection_id: *id },
                                    cx,
                                );
                            }
                        }
                        this.selected_connection_ids.clear();
                        window.push_notification(format!("已删除 {} 个连接", ids_clone.len()), cx);
                        this.load_connections(cx);
                        cx.notify();
                    });
                    true
                })
        });
    }

    pub(crate) fn handle_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.export_include_passwords
            && (crypto::has_repo_password_set() || crypto::has_master_key())
        {
            self.show_confirm_master_key_for_export_dialog(window, cx);
        } else {
            let include_pwd = self.export_include_passwords;
            self.execute_export(include_pwd, window, cx);
        }
    }

    fn show_confirm_master_key_for_export_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("输入主密码以确认")
                .clean_on_escape()
        });
        let error_message: Entity<Option<String>> = cx.new(|_| None);

        let key_input_ok = key_input.clone();
        let error_msg_ok = error_message.clone();
        let key_input_render = key_input.clone();
        let error_msg_render = error_message.clone();
        let view = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _window, cx| {
            let key_input_ok = key_input_ok.clone();
            let error_msg_ok = error_msg_ok.clone();
            let view_ok = view.clone();

            dialog
                .title("确认主密码以导出密码".to_string().into_any_element())
                .width(px(400.0))
                .confirm()
                .on_ok(move |_, window, cx| {
                    let input_key = key_input_ok.read(cx).text().to_string();
                    if input_key.is_empty() {
                        error_msg_ok.update(cx, |msg, cx| {
                            *msg = Some("请输入主密码".to_string());
                            cx.notify();
                        });
                        return false;
                    }

                    // 验证主密码是否正确
                    let is_valid = if let Some(raw_key) = crypto::get_raw_master_key() {
                        input_key == raw_key
                    } else {
                        crypto::verify_and_set_master_key(&input_key).is_ok()
                    };

                    if is_valid {
                        view_ok.update(cx, |this, cx| {
                            this.execute_export(true, window, cx);
                        });
                        true
                    } else {
                        error_msg_ok.update(cx, |msg, cx| {
                            *msg = Some("主密码错误，请重新输入".to_string());
                            cx.notify();
                        });
                        false
                    }
                })
                .child(
                    v_flex()
                        .gap_3()
                        .p_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "导出包含明文连接密码的文件，需要重新输入主密码进行身份确认：",
                                ),
                        )
                        .child(Input::new(&key_input_render).mask_toggle().w_full())
                        .when_some(error_msg_render.read(cx).clone(), |this, err| {
                            this.child(div().text_xs().text_color(cx.theme().danger).child(err))
                        }),
                )
        });
    }

    pub(crate) fn execute_export(
        &mut self,
        include_passwords: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let list: Vec<StoredConnection> = match self.export_scope {
            ExportScope::All => self.connections.clone(),
            ExportScope::CurrentGroup => {
                if let Some(gid) = self.selected_group_id {
                    self.connections
                        .iter()
                        .filter(|c| c.workspace_id == Some(gid))
                        .cloned()
                        .collect()
                } else {
                    self.connections.clone()
                }
            }
            ExportScope::Selected => self
                .connections
                .iter()
                .filter(|c| {
                    c.id.map_or(false, |id| self.selected_connection_ids.contains(&id))
                })
                .cloned()
                .collect(),
        };

        if list.is_empty() {
            window.push_notification("当前范围没有可导出的连接", cx);
            return;
        }

        let items = list
            .iter()
            .map(|c| build_export_item(c, &self.workspaces, include_passwords))
            .collect();

        match generate_export_content(items, self.export_format, include_passwords) {
            Ok(content) => match save_export_file(&content, self.export_format) {
                Ok(path) => {
                    self.export_popover_open = false;
                    let filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if include_passwords {
                        window.push_notification(
                            format!("已导出 {} 个连接（含密码）→ {}", list.len(), filename),
                            cx,
                        );
                    } else {
                        window.push_notification(
                            format!("已脱敏导出 {} 个连接 → {}", list.len(), filename),
                            cx,
                        );
                    }
                    cx.notify();
                }
                Err(e) => {
                    window.push_notification(format!("保存导出文件失败: {}", e), cx);
                }
            },
            Err(e) => {
                window.push_notification(format!("生成导出内容失败: {}", e), cx);
            }
        }
    }

    pub(crate) fn pick_and_parse_import_file(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let future = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            multiple: false,
            directories: false,
            prompt: Some("选择 MyTerm 配置文件（.json / .csv）".to_string().into()),
        });

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = future.await {
                if let Some(path) = paths.into_iter().next() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let _ = this.update(cx, |this, cx| {
                            this.process_import_content(&content, cx);
                        });
                    }
                }
            }
        })
        .detach();
    }

    pub(crate) fn use_sample_import(&mut self, cx: &mut Context<Self>) {
        self.process_import_content(sample_import_json(), cx);
    }

    fn process_import_content(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_import_payload(text) {
            Ok(items) => {
                let report = preflight_import(items, &self.connections);
                self.import_preflight_report = Some(report);
                self.import_step = ImportStep::Step2;
                cx.notify();
            }
            Err(e) => {
                tracing::error!("导入文件解析失败: {}", e);
            }
        }
    }

    pub(crate) fn apply_import_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(report) = self.import_preflight_report.take() else {
            return;
        };
        let strategy = self.import_merge_strategy;
        let (to_create, to_update) = plan_import_execution(&report, strategy, &self.workspaces);

        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let mut added_count = 0;
        let mut updated_count = 0;

        if let Some(repo) = storage.get::<ConnectionRepository>() {
            for mut conn in to_create {
                if let Ok(id) = repo.insert(&mut conn) {
                    conn.id = Some(id);
                    self.connections.push(conn.clone());
                    emit_connection_event(
                        ConnectionDataEvent::ConnectionCreated { connection: conn },
                        cx,
                    );
                    added_count += 1;
                }
            }
            for (id, conn) in to_update {
                if repo.update(&conn).is_ok() {
                    if let Some(pos) = self.connections.iter().position(|c| c.id == Some(id)) {
                        self.connections[pos] = conn.clone();
                    }
                    emit_connection_event(
                        ConnectionDataEvent::ConnectionUpdated { connection: conn },
                        cx,
                    );
                    updated_count += 1;
                }
            }
        }

        self.import_modal_open = false;
        self.import_step = ImportStep::Step1;
        let summary_msg = if strategy == MergeStrategy::Overwrite {
            format!("导入完成：新增 {}，覆盖冲突 {}", added_count, updated_count)
        } else {
            format!(
                "导入完成：新增 {}，保留本地冲突 {}",
                added_count, report.conflict_count
            )
        };
        window.push_notification(summary_msg, cx);
        self.load_connections(cx);
        cx.notify();
    }

    fn visible_connections(&self, cx: &App) -> Vec<StoredConnection> {
        let search_query = self.search_query.read(cx).to_lowercase();
        let active_conns = cx.global::<ActiveConnections>();
        let mut list: Vec<StoredConnection> = self.connections.clone();

        // 1. 左栏视图筛选
        match self.rail_view {
            RailView::All => {}
            RailView::Favorite => {
                list.retain(|c| c.is_favorite);
            }
            RailView::Recent => {
                list.sort_by(|a, b| {
                    b.last_used_at
                        .unwrap_or(0)
                        .cmp(&a.last_used_at.unwrap_or(0))
                });
                list.truncate(5);
            }
        }

        // 2. 当前选中分组
        if let Some(gid) = self.selected_group_id {
            list.retain(|c| c.workspace_id == Some(gid));
        }

        // 3. 顶栏协议筛选
        if let Some(proto) = self.protocol_filter {
            list.retain(|c| c.connection_type == proto);
        }

        // 4. 仅看在线筛选
        if self.online_only_filter {
            list.retain(|c| c.id.map_or(false, |id| active_conns.is_active(id)));
        }

        // 5. 关键词搜索（实时过滤名称、主机、用户、环境、标签、分组）
        if !search_query.is_empty() {
            list.retain(|c| self.match_connection(c, &search_query));
        }

        // 兼容原工作区过滤
        if self.workspace_filter_active {
            list.retain(|c| {
                c.workspace_id.is_none()
                    || c.workspace_id
                        .is_some_and(|id| self.filtered_workspace_ids.contains(&id))
            });
        }

        list
    }

    fn select_connection(&mut self, offset: isize, cx: &mut Context<Self>) {
        let connections = self.visible_connections(cx);
        if connections.is_empty() {
            self.selected_connection_id = None;
            return;
        }

        let current_index = self.selected_connection_id.and_then(|selected| {
            connections
                .iter()
                .position(|connection| connection.id == Some(selected))
        });
        let next_index = match current_index {
            Some(index) => {
                (index as isize + offset).rem_euclid(connections.len() as isize) as usize
            }
            None if offset < 0 => connections.len() - 1,
            None => 0,
        };
        self.selected_connection_id = connections[next_index].id;
        cx.notify();
    }

    fn select_previous_connection(
        &mut self,
        _: &SelectPreviousConnection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_connection(-1, cx);
    }

    fn select_next_connection(
        &mut self,
        _: &SelectNextConnection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_connection(1, cx);
    }
    pub(crate) fn handle_save_workspace(
        &mut self,
        workspace_id: Option<i64>,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let editing_id = workspace_id;

        let mut workspace = if let Some(id) = editing_id {
            // 编辑模式：从现有工作区更新
            let mut ws = self
                .workspaces
                .iter()
                .find(|w| w.id == Some(id))
                .cloned()
                .unwrap_or_else(|| Workspace::new(name.clone()));
            ws.name = name;
            ws
        } else {
            // 新建模式
            Workspace::new(name)
        };

        let result: anyhow::Result<Workspace> = (|| {
            let repo = storage
                .get::<WorkspaceRepository>()
                .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))?;

            if editing_id.is_some() {
                repo.update(&mut workspace)?;
            } else {
                repo.insert(&mut workspace)?;
            }

            Ok(workspace)
        })();

        cx.spawn(async move |this, cx| match result {
            Ok(workspace) => {
                _ = this.update(cx, |this, cx| {
                    let workspace_id = workspace.id.unwrap_or(0);
                    if let Some(editing_id) = editing_id {
                        if let Some(pos) = this
                            .workspaces
                            .iter()
                            .position(|w| w.id == Some(editing_id))
                        {
                            this.workspaces[pos] = workspace;
                        }
                        emit_connection_event(
                            ConnectionDataEvent::WorkspaceUpdated { workspace_id },
                            cx,
                        );
                    } else {
                        this.workspaces.push(workspace);
                        emit_connection_event(
                            ConnectionDataEvent::WorkspaceCreated { workspace_id },
                            cx,
                        );
                    }
                    cx.notify();
                });
            }
            Err(e) => {
                tracing::error!("Failed to save workspace: {}", e);
            }
        })
        .detach();
    }

    pub(crate) fn delete_workspace(
        &mut self,
        workspace_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace_name = self
            .workspaces
            .iter()
            .find(|w| w.id == Some(workspace_id))
            .map(|w| w.name.clone())
            .unwrap_or_default();

        let view = cx.entity().clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            dialog
                .title("移除分组".to_string().into_any_element())
                .child(
                    format!(
                        "确定要移除分组「{}」吗？移除后该分组下的连接将被移至未分组。",
                        workspace_name
                    )
                    .into_any_element(),
                )
                .confirm()
                .on_ok(move |_, _window, cx| {
                    let _ = view_clone.update(cx, |this, cx| {
                        this.handle_delete_workspace(workspace_id, cx);
                    });
                    true
                })
        });
    }

    fn handle_delete_workspace(&mut self, workspace_id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 删除本地工作空间
            let result = (|| {
                let repo = storage
                    .get::<WorkspaceRepository>()
                    .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))?;
                repo.delete(workspace_id)
            })();

            match result {
                Ok(_) => {
                    _ = this.update(cx, |this, cx| {
                        this.workspaces.retain(|w| w.id != Some(workspace_id));
                        this.filtered_workspace_ids.remove(&workspace_id);
                        if this.selected_group_id == Some(workspace_id) {
                            this.selected_group_id = None;
                        }
                        for conn in &mut this.connections {
                            if conn.workspace_id == Some(workspace_id) {
                                conn.workspace_id = None;
                            }
                        }
                        emit_connection_event(
                            ConnectionDataEvent::WorkspaceDeleted { workspace_id },
                            cx,
                        );
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to delete workspace: {}", e);
                }
            }
        })
        .detach();
    }

    pub(crate) fn show_ssh_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::SshSftp)
                .cloned()
        });

        let config = SshFormWindowConfig {
            editing_connection: editing_conn,
            workspaces: self.workspaces.clone(),
        };

        self.editing_connection_id = None;

        open_popup_window(
            PopupWindowOptions::new(if config.editing_connection.is_some() {
                t!("SSH.edit").to_string()
            } else {
                t!("SSH.new").to_string()
            })
            .size(700.0, 650.0),
            move |window, cx| cx.new(|cx| SshFormWindow::new(config, window, cx)),
            cx,
        );
    }

    pub(crate) fn show_port_forwarding_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_connection = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::PortForwarding)
                .cloned()
        });
        let ssh_connections = self
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();

        let config = PortForwardingFormWindowConfig {
            editing_connection,
            ssh_connections,
            workspaces: self.workspaces.clone(),
        };

        self.editing_connection_id = None;

        open_popup_window(
            PopupWindowOptions::new(if config.editing_connection.is_some() {
                t!("PortForwarding.edit").to_string()
            } else {
                t!("PortForwarding.new").to_string()
            })
            .size(700.0, 520.0),
            move |window, cx| cx.new(|cx| PortForwardingFormWindow::new(config, window, cx)),
            cx,
        );
    }

    pub(crate) fn open_port_forwarding(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_name = connection.name.clone();
        let Some(connection_id) = connection.id else {
            window.push_notification(
                t!(
                    "Home.port_forwarding_failed",
                    name = connection_name,
                    error = "missing connection id"
                )
                .to_string(),
                cx,
            );
            return;
        };
        let params = match connection.to_port_forwarding_params() {
            Ok(params) => params,
            Err(error) => {
                window.push_notification(
                    t!(
                        "Home.port_forwarding_failed",
                        name = connection_name,
                        error = error.to_string()
                    )
                    .to_string(),
                    cx,
                );
                return;
            }
        };
        let Some(ssh_connection) = self
            .connections
            .iter()
            .find(|conn| conn.id == Some(params.ssh_connection_id))
            .cloned()
        else {
            window.push_notification(t!("Home.port_forwarding_missing_ssh").to_string(), cx);
            return;
        };

        enum StartRequest {
            Local(LocalForwardingRequest),
            Dynamic(DynamicForwardingRequest),
        }

        let request = match params.kind {
            one_core::storage::PortForwardingKind::Local => {
                build_local_forwarding_request(&connection, &ssh_connection)
                    .map(StartRequest::Local)
            }
            one_core::storage::PortForwardingKind::Dynamic => {
                build_dynamic_forwarding_request(&connection, &ssh_connection)
                    .map(StartRequest::Dynamic)
            }
        };
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                window.push_notification(
                    t!(
                        "Home.port_forwarding_failed",
                        name = connection_name,
                        error = error.to_string()
                    )
                    .to_string(),
                    cx,
                );
                return;
            }
        };

        let runtime = Arc::clone(&self.port_forwarding_runtime);
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = {
                let mut runtime = runtime.lock().await;
                match request {
                    StartRequest::Local(request) => {
                        runtime.start_local(connection_id, request).await
                    }
                    StartRequest::Dynamic(request) => {
                        runtime.start_dynamic(connection_id, request).await
                    }
                }
            };
            if result.is_ok() {
                let _ = cx.update(|cx| {
                    cx.global_mut::<ActiveConnections>().add(connection_id);
                });
            }
            let message = match result {
                Ok(local_addr) => t!(
                    "Home.port_forwarding_started",
                    name = connection_name,
                    addr = local_addr.to_string()
                )
                .to_string(),
                Err(error) => t!(
                    "Home.port_forwarding_failed",
                    name = connection_name,
                    error = error.to_string()
                )
                .to_string(),
            };
            push_notification_on_active_window(message, cx);
        })
        .detach();
    }

    pub(crate) fn stop_port_forwarding(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_name = connection.name.clone();
        let Some(connection_id) = connection.id else {
            window.push_notification(
                t!(
                    "Home.port_forwarding_stop_failed",
                    error = "missing connection id"
                )
                .to_string(),
                cx,
            );
            return;
        };

        let runtime = Arc::clone(&self.port_forwarding_runtime);
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = {
                let mut runtime = runtime.lock().await;
                runtime.stop(connection_id).await
            };

            match result {
                Ok(()) => {
                    let _ = cx.update(|cx| {
                        cx.global_mut::<ActiveConnections>().remove(connection_id);
                    });
                    push_notification_on_active_window(
                        t!("Home.port_forwarding_stopped", name = connection_name).to_string(),
                        cx,
                    );
                }
                Err(error) => {
                    push_notification_on_active_window(
                        t!(
                            "Home.port_forwarding_stop_failed",
                            error = error.to_string()
                        )
                        .to_string(),
                        cx,
                    );
                }
            }
        })
        .detach();
    }

    pub(crate) fn ensure_master_key_ready_for_new_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_master_key_ready_for_new_connection() {
            return true;
        }

        self.show_encryption_key_dialog(window, cx);
        false
    }

    pub(crate) fn is_master_key_ready_for_new_connection(&self) -> bool {
        crypto::has_master_key()
    }

    fn ensure_master_key_ready_for_saved_connections(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.saved_connections_locked() {
            return true;
        }

        self.show_encryption_key_dialog(window, cx);
        false
    }

    fn saved_connections_locked(&self) -> bool {
        crypto::has_repo_password_set() && !crypto::has_master_key()
    }

    fn show_encryption_key_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.master_key_dialog_open {
            return;
        }
        self.master_key_dialog_open = true;

        let view = cx.entity();
        let has_password_set = crypto::has_repo_password_set();
        let has_key_in_memory = crypto::has_master_key();
        let is_first_setup = !has_password_set;
        let is_change_mode = has_password_set && has_key_in_memory;
        let initial_master_key = crypto::get_raw_master_key().or_else(|| {
            let storage = key_storage::get_key_storage();
            storage.load()
        });

        let key_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Encryption.repo_password_placeholder"))
                .masked(true);

            if let Some(ref value) = initial_master_key {
                state = state.default_value(value);
            }

            state
        });

        let error_message = cx.new(|_| Option::<String>::None);

        let key_input_for_ok = key_input.clone();
        let error_msg_for_ok = error_message.clone();

        let key_input_for_render = key_input.clone();
        let error_msg_for_render = error_message.clone();

        let dialog_title = if is_first_setup {
            t!("Encryption.set_repo_password")
        } else if is_change_mode {
            t!("Encryption.change_repo_password")
        } else {
            t!("Encryption.unlock_repo_password")
        };

        window.open_dialog(cx, move |dialog, _window, cx| {
            let key_input_ok = key_input_for_ok.clone();
            let error_msg_ok = error_msg_for_ok.clone();

            dialog
                .title(dialog_title.to_string())
                .width(px(450.))
                .confirm()
                .on_ok(move |_, _window, cx| {
                    let input_key = key_input_ok.read(cx).text().to_string();

                    if input_key.is_empty() {
                        error_msg_ok.update(cx, |msg, cx| {
                            *msg = Some(t!("Encryption.key_empty").to_string());
                            cx.notify();
                        });
                        return false;
                    }

                    if is_first_setup {
                        crypto::set_master_key(&input_key);
                        return true;
                    }

                    if is_change_mode {
                        let old_key = match crypto::get_raw_master_key() {
                            Some(key) if !key.is_empty() => key,
                            _ => {
                                error_msg_ok.update(cx, |msg, cx| {
                                    *msg = Some(t!("Encryption.password_incorrect").to_string());
                                    cx.notify();
                                });
                                return false;
                            }
                        };

                        if input_key != old_key {
                            match crypto::change_master_key(&old_key, &input_key, &input_key) {
                                Ok(()) => {
                                    let storage = cx.global::<GlobalStorageState>().storage.clone();
                                    match re_encrypt_all_connections(&storage) {
                                        Ok(count) => {
                                            tracing::info!(
                                                "主密钥修改成功，已重新加密 {} 个本地连接",
                                                count
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!("重新加密本地连接失败: {}", e);
                                            error_msg_ok.update(cx, |msg, cx| {
                                                *msg = Some(e.to_string());
                                                cx.notify();
                                            });
                                            return false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error_msg_ok.update(cx, |msg, cx| {
                                        *msg = Some(e.to_string());
                                        cx.notify();
                                    });
                                    return false;
                                }
                            }
                        }

                        return true;
                    }

                    match crypto::verify_and_set_master_key(&input_key) {
                        Ok(()) => true,
                        Err(_) => {
                            error_msg_ok.update(cx, |msg, cx| {
                                *msg = Some(t!("Encryption.password_incorrect").to_string());
                                cx.notify();
                            });
                            false
                        }
                    }
                })
                .on_close({
                    let view_for_sync = view.clone();
                    move |_window, _result, cx| {
                        view_for_sync.update(cx, |this, cx| {
                            this.master_key_dialog_open = false;
                            if crypto::has_master_key() {
                                // 密钥已就绪后刷新连接列表，修复启动时序导致的空密码回显
                                this.load_connections(cx);
                            }
                        });
                    }
                })
                .child(
                    v_flex()
                        .gap_4()
                        .p_4()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_shrink_0()
                                        .w(px(80.))
                                        .child(t!("Encryption.repo_password_label").to_string()),
                                )
                                .child(Input::new(&key_input_for_render).mask_toggle().w_full()),
                        )
                        .child(
                            v_flex()
                                .gap_2()
                                .child(
                                    div().text_base().font_weight(FontWeight::SEMIBOLD).child(
                                        t!("Encryption.remember_password_title").to_string(),
                                    ),
                                )
                                .child(div().text_sm().child(
                                    t!("Encryption.remember_password_detail_local").to_string(),
                                ))
                                .child(div().text_sm().text_color(cx.theme().warning).child(
                                    t!("Encryption.remember_password_detail_cloud").to_string(),
                                )),
                        )
                        .when_some(error_msg_for_render.read(cx).clone(), |this, msg| {
                            this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                        }),
                )
        });
    }

    #[allow(dead_code)]
    pub(crate) fn toggle_workspace_filter(&mut self, workspace_id: i64, cx: &mut Context<Self>) {
        if !self.workspace_filter_active {
            self.filtered_workspace_ids = self
                .workspaces
                .iter()
                .filter_map(|workspace| workspace.id)
                .collect();
            self.workspace_filter_active = true;
        }

        if self.filtered_workspace_ids.contains(&workspace_id) {
            self.filtered_workspace_ids.remove(&workspace_id);
        } else {
            self.filtered_workspace_ids.insert(workspace_id);
        }
        cx.notify();
    }

    fn clear_workspace_filter(&mut self, cx: &mut Context<Self>) {
        self.filtered_workspace_ids.clear();
        self.workspace_filter_active = false;
        cx.notify();
    }

    fn clear_connection_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_filter = ConnectionType::All;
        self.protocol_filter = None;
        self.online_only_filter = false;
        self.rail_view = RailView::All;
        self.selected_group_id = None;
        self.selected_connection_ids.clear();
        self.clear_workspace_filter(cx);
        self.search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.search_query.update(cx, |q, cx| {
            q.clear();
            cx.notify();
        });
        cx.notify();
    }

    fn render_rail(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_all = self.rail_view == RailView::All && self.selected_group_id.is_none();
        let is_fav = self.rail_view == RailView::Favorite;
        let is_recent = self.rail_view == RailView::Recent;
        let total_count = self.connections.len();
        let fav_count = self.connections.iter().filter(|c| c.is_favorite).count();
        let recent_count = std::cmp::min(5, total_count);
        let has_master_key = crypto::has_master_key();

        v_flex()
            .w(px(216.0))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .p_2()
            .gap_1()
            .child(
                Button::new("rail-all")
                    .w_full()
                    .justify_start()
                    .when(is_all, |b| b.primary())
                    .when(!is_all, |b| b.ghost())
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(IconName::Apps.mono().with_size(px(15.0)))
                            .child(div().text_sm().child("全部连接"))
                            .child(
                                div()
                                    .ml_auto()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(total_count.to_string()),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rail_view = RailView::All;
                        this.selected_group_id = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("rail-fav")
                    .w_full()
                    .justify_start()
                    .when(is_fav, |b| b.primary())
                    .when(!is_fav, |b| b.ghost())
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(IconName::Star.mono().with_size(px(15.0)))
                            .child(div().text_sm().child("收藏"))
                            .child(
                                div()
                                    .ml_auto()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(fav_count.to_string()),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rail_view = RailView::Favorite;
                        this.selected_group_id = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("rail-recent")
                    .w_full()
                    .justify_start()
                    .when(is_recent, |b| b.primary())
                    .when(!is_recent, |b| b.ghost())
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(IconName::Undo.mono().with_size(px(15.0)))
                            .child(div().text_sm().child("最近使用"))
                            .child(
                                div()
                                    .ml_auto()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(recent_count.to_string()),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rail_view = RailView::Recent;
                        this.selected_group_id = None;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("分组"),
                    )
                    .child(
                        Button::new("btn-new-workspace")
                            .icon(IconName::Plus)
                            .xsmall()
                            .ghost()
                            .tooltip("新建分组")
                            .on_click(cx.listener(|_, _, window, cx| {
                                show_workspace_dialog(cx.entity(), None, String::new(), window, cx);
                            })),
                    ),
            )
            // Groups list: 独立滚动容器，确保底部设置区域始终可见
            .child(
                v_flex()
                    .id("rail-groups-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .gap_1()
                    .children(self.workspaces.iter().map(|ws| {
                        let ws_id = ws.id;
                        let count = self
                            .connections
                            .iter()
                            .filter(|c| c.workspace_id == ws_id)
                            .count();
                        let is_grp_selected =
                            self.selected_group_id == ws_id && self.rail_view == RailView::All;
                        let color = ws.color.as_deref().unwrap_or("#7c5cff");
                        let dot_color: Hsla = if color.starts_with('#') {
                            gpui::rgb(u32::from_str_radix(&color[1..], 16).unwrap_or(0x7c5cff))
                                .into()
                        } else {
                            cx.theme().accent
                        };

                        let row_id =
                            SharedString::from(format!("rail-grp-row-{}", ws_id.unwrap_or(0)));
                        let ws_name = ws.name.clone();

                        h_flex()
                            .id(row_id.clone())
                            .group(row_id.clone())
                            .w_full()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "rail-grp-{}",
                                    ws_id.unwrap_or(0)
                                )))
                                .flex_1()
                                .justify_start()
                                .when(is_grp_selected, |b| b.primary())
                                .when(!is_grp_selected, |b| b.ghost())
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            IconName::Folder
                                                .mono()
                                                .with_size(px(15.0))
                                                .text_color(dot_color),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .child(ws_name),
                                        )
                                        .child(
                                            div()
                                                .ml_auto()
                                                .text_xs()
                                                .text_color(if is_grp_selected {
                                                    cx.theme().primary_foreground.opacity(0.8)
                                                } else {
                                                    cx.theme().muted_foreground
                                                })
                                                .child(count.to_string()),
                                        ),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.selected_group_id == ws_id {
                                            this.selected_group_id = None;
                                        } else {
                                            this.selected_group_id = ws_id;
                                            this.rail_view = RailView::All;
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .when_some(ws_id, |this, id| {
                                this.child(
                                    div()
                                        .invisible()
                                        .group_hover(row_id.clone(), |s| s.visible())
                                        .flex_shrink_0()
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "del-ws-{}",
                                                id
                                            )))
                                            .icon(IconName::Remove)
                                            .xsmall()
                                            .ghost()
                                            .tooltip("移除分组")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.delete_workspace(id, window, cx);
                                            })),
                                        ),
                                )
                            })
                    })),
            )
            // Footer: Master key & Settings (始终吸底)
            .child(
                v_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("rail-encryption")
                            .ghost()
                            .w_full()
                            .justify_start()
                            .child(
                                div()
                                    .w(px(8.0))
                                    .h(px(8.0))
                                    .rounded_full()
                                    .when(has_master_key, |d| d.bg(cx.theme().success))
                                    .when(!has_master_key, |d| d.bg(cx.theme().muted)),
                            )
                            .label(if has_master_key {
                                "主密码已解锁"
                            } else {
                                "解锁主密码"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_encryption_key_dialog(window, cx);
                            })),
                    )
                    .child(
                        Button::new("rail-settings")
                            .ghost()
                            .w_full()
                            .justify_start()
                            .icon(IconName::Settings)
                            .label("设置")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_settings_tab(window, cx);
                            })),
                    ),
            )
    }

    fn render_unified_toolbar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_proto_all = self.protocol_filter.is_none();
        let is_proto_ssh = self.protocol_filter == Some(ConnectionType::SshSftp);
        let is_proto_jms = self.protocol_filter == Some(ConnectionType::Jms);
        let is_online = self.online_only_filter;

        let visible_count = self.visible_connections(cx).len();
        let selected_count = self.selected_connection_ids.len();
        let search_query = self.search_query.read(cx).clone();

        let mut summary = format!("{} 个连接", visible_count);
        if let Some(gid) = self.selected_group_id {
            if let Some(ws) = self.workspaces.iter().find(|w| w.id == Some(gid)) {
                summary.push_str(&format!(" · 分组 {}", ws.name));
            }
        }
        if !search_query.is_empty() {
            summary.push_str(&format!(" · 匹配“{}”", search_query));
        }

        let density_icon = if self.density == CardDensity::Grid {
            "▦"
        } else {
            "☰"
        };

        let export_popover_open = self.export_popover_open;
        let export_popover = self.render_export_popover(export_popover_open, window, cx);

        h_flex()
            .w_full()
            .h(px(38.0))
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .items_center()
            .gap_2()
            // 本地终端
            .child(
                Button::new("toolbar-local-term")
                    .ghost()
                    .xsmall()
                    .label("🖥 本地终端")
                    .tooltip("新建本地终端 (Ctrl+T)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_terminal_tab(window, cx);
                    })),
            )
            // 新建连接
            .child(
                Button::new("toolbar-new-conn")
                    .primary()
                    .xsmall()
                    .label("＋ 新建连接")
                    .tooltip("新建连接配置 (Ctrl+N)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_new_connection_dialog(window, cx);
                    })),
            )
            // 搜索框（220px 宽度，紧凑型，Ctrl+K 提示）
            .child(
                h_flex()
                    .w(px(220.0))
                    .flex_shrink_0()
                    .items_center()
                    .relative()
                    .child(Input::new(&self.search_input).cleanable(true).w_full())
                    .child(
                        div()
                            .absolute()
                            .right_2()
                            .px_1()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_color(cx.theme().muted_foreground)
                            .text_xs()
                            .child("Ctrl K"),
                    ),
            )
            // 协议过滤 Chips
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("chip-proto-all")
                            .xsmall()
                            .when(is_proto_all, |b| b.primary())
                            .when(!is_proto_all, |b| b.ghost())
                            .label("全部")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.protocol_filter = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("chip-proto-ssh")
                            .xsmall()
                            .when(is_proto_ssh, |b| b.primary())
                            .when(!is_proto_ssh, |b| b.ghost())
                            .label("SSH")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.protocol_filter = Some(ConnectionType::SshSftp);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("chip-proto-jms")
                            .xsmall()
                            .when(is_proto_jms, |b| b.primary())
                            .when(!is_proto_jms, |b| b.ghost())
                            .label("JMS")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.protocol_filter = Some(ConnectionType::Jms);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("chip-filter-online")
                            .xsmall()
                            .when(is_online, |b| b.primary())
                            .when(!is_online, |b| b.ghost())
                            .label("在线")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.online_only_filter = !this.online_only_filter;
                                cx.notify();
                            })),
                    ),
            )
            // 弹性空白
            .child(div().flex_1())
            // 状态摘要
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(summary),
            )
            // 批量选择条
            .when(selected_count > 0, |this| {
                this.child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .px_2()
                        .py_0p5()
                        .rounded(px(6.0))
                        .bg(cx.theme().accent.opacity(0.1))
                        .text_color(cx.theme().accent)
                        .text_xs()
                        .child(format!("已选 {} 个", selected_count))
                        .child(
                            Button::new("btn-sel-export")
                                .xsmall()
                                .ghost()
                                .label("导出所选")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.export_scope = ExportScope::Selected;
                                    this.export_popover_open = true;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("btn-sel-delete")
                                .xsmall()
                                .danger()
                                .label("删除")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.handle_batch_delete(window, cx);
                                })),
                        )
                        .child(
                            Button::new("btn-sel-clear")
                                .xsmall()
                                .ghost()
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.selected_connection_ids.clear();
                                    cx.notify();
                                })),
                        ),
                )
            })
            // ⤓ 导出
            .child(export_popover)
            // ⤒ 导入
            .child(
                Button::new("btn-import-modal")
                    .ghost()
                    .xsmall()
                    .label("⤒ 导入")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.import_modal_open = true;
                        this.import_step = ImportStep::Step1;
                        this.import_preflight_report = None;
                        cx.notify();
                    })),
            )
            // 密度切换 ▦/☰
            .child(
                Button::new("btn-density-toggle")
                    .ghost()
                    .xsmall()
                    .label(density_icon)
                    .tooltip(if self.density == CardDensity::Grid {
                        "切换为列表视图"
                    } else {
                        "切换为网格视图"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.density = if this.density == CardDensity::Grid {
                            CardDensity::List
                        } else {
                            CardDensity::Grid
                        };
                        cx.notify();
                    })),
            )
            // 刷新状态
            .child(
                Button::new("btn-refresh-status")
                    .icon(IconName::Refresh)
                    .ghost()
                    .xsmall()
                    .tooltip("刷新状态")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_local_home_data(cx);
                    })),
            )
    }

    fn render_export_popover(
        &mut self,
        open: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all_count = self.connections.len();
        let group_count = self.selected_group_id.map_or(0, |gid| {
            self.connections
                .iter()
                .filter(|c| c.workspace_id == Some(gid))
                .count()
        });
        let sel_count = self.selected_connection_ids.len();

        let current_group_name = self.selected_group_id.and_then(|gid| {
            self.workspaces
                .iter()
                .find(|w| w.id == Some(gid))
                .map(|w| w.name.clone())
        });

        let view = cx.entity().clone();

        Popover::new("export-popover")
            .trigger(
                Button::new("btn-export-trigger")
                    .ghost()
                    .xsmall()
                    .label("⤓ 导出"),
            )
            .open(open)
            .on_open_change(cx.listener(|this, open, _, cx| {
                this.export_popover_open = *open;
                cx.notify();
            }))
            .content(move |_, _, cx| {
                let view_for_scope_all = view.clone();
                let view_for_scope_group = view.clone();
                let view_for_scope_sel = view.clone();
                let view_for_json = view.clone();
                let view_for_csv = view.clone();
                let view_for_yaml = view.clone();
                let view_for_include_pwd = view.clone();
                let view_for_cancel = view.clone();
                let view_for_cancel2 = view.clone();
                let view_for_export = view.clone();

                let cur_scope = view.read(cx).export_scope;
                let cur_fmt = view.read(cx).export_format;
                let cur_include_pwd = view.read(cx).export_include_passwords;

                v_flex()
                    .w(px(320.0))
                    .p_4()
                    .gap_3()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_sm()
                                    .child("导出连接配置"),
                            )
                            .child(
                                div()
                                    .id("btn-close-export")
                                    .cursor_pointer()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("×")
                                    .on_click(move |_, _, cx| {
                                        view_for_cancel.update(cx, |this, cx| {
                                            this.export_popover_open = false;
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    // Scope radios
                    .child(
                        v_flex()
                            .gap_1p5()
                            .child(
                                h_flex()
                                    .id("scope-all-opt")
                                    .gap_2()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| {
                                        view_for_scope_all.update(cx, |this, cx| {
                                            this.export_scope = ExportScope::All;
                                            cx.notify();
                                        });
                                    })
                                    .child(
                                        div()
                                            .w(px(14.0))
                                            .h(px(14.0))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(cur_scope == ExportScope::All, |d| {
                                                d.child(
                                                    div()
                                                        .w(px(8.0))
                                                        .h(px(8.0))
                                                        .rounded_full()
                                                        .bg(cx.theme().accent),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .child(format!("全部分组 · {} 个连接", all_count)),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .id("scope-grp-opt")
                                    .gap_2()
                                    .items_center()
                                    .when(current_group_name.is_some(), |h| {
                                        h.cursor_pointer().on_click(move |_, _, cx| {
                                            view_for_scope_group.update(cx, |this, cx| {
                                                this.export_scope = ExportScope::CurrentGroup;
                                                cx.notify();
                                            });
                                        })
                                    })
                                    .when(current_group_name.is_none(), |h| {
                                        h.text_color(cx.theme().muted)
                                    })
                                    .child(
                                        div()
                                            .w(px(14.0))
                                            .h(px(14.0))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(cur_scope == ExportScope::CurrentGroup, |d| {
                                                d.child(
                                                    div()
                                                        .w(px(8.0))
                                                        .h(px(8.0))
                                                        .rounded_full()
                                                        .bg(cx.theme().accent),
                                                )
                                            }),
                                    )
                                    .child(div().text_xs().child(format!(
                                        "仅当前分组 {} · {} 个",
                                        current_group_name.as_deref().unwrap_or("（未选）"),
                                        group_count
                                    ))),
                            )
                            .child(
                                h_flex()
                                    .id("scope-sel-opt")
                                    .gap_2()
                                    .items_center()
                                    .when(sel_count > 0, |h| {
                                        h.cursor_pointer().on_click(move |_, _, cx| {
                                            view_for_scope_sel.update(cx, |this, cx| {
                                                this.export_scope = ExportScope::Selected;
                                                cx.notify();
                                            });
                                        })
                                    })
                                    .when(sel_count == 0, |h| h.text_color(cx.theme().muted))
                                    .child(
                                        div()
                                            .w(px(14.0))
                                            .h(px(14.0))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(cur_scope == ExportScope::Selected, |d| {
                                                d.child(
                                                    div()
                                                        .w(px(8.0))
                                                        .h(px(8.0))
                                                        .rounded_full()
                                                        .bg(cx.theme().accent),
                                                )
                                            }),
                                    )
                                    .child(
                                        div().text_xs().child(format!(
                                            "已选连接 · {} 个（卡片勾选）",
                                            sel_count
                                        )),
                                    ),
                            ),
                    )
                    // Format chips
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("格式"),
                            )
                            .child(
                                Button::new("fmt-json")
                                    .small()
                                    .when(cur_fmt == ExportFormat::Json, |b| b.primary())
                                    .when(cur_fmt != ExportFormat::Json, |b| b.ghost())
                                    .label("JSON")
                                    .on_click(move |_, _, cx| {
                                        view_for_json.update(cx, |this, cx| {
                                            this.export_format = ExportFormat::Json;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                Button::new("fmt-csv")
                                    .small()
                                    .when(cur_fmt == ExportFormat::Csv, |b| b.primary())
                                    .when(cur_fmt != ExportFormat::Csv, |b| b.ghost())
                                    .label("CSV")
                                    .on_click(move |_, _, cx| {
                                        view_for_csv.update(cx, |this, cx| {
                                            this.export_format = ExportFormat::Csv;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                Button::new("fmt-yaml")
                                    .small()
                                    .when(cur_fmt == ExportFormat::Yaml, |b| b.primary())
                                    .when(cur_fmt != ExportFormat::Yaml, |b| b.ghost())
                                    .label("YAML")
                                    .on_click(move |_, _, cx| {
                                        view_for_yaml.update(cx, |this, cx| {
                                            this.export_format = ExportFormat::Yaml;
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    // Include passwords checkbox
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Checkbox::new("chk-include-passwords")
                                    .checked(cur_include_pwd)
                                    .on_click(move |checked, _window, cx| {
                                        view_for_include_pwd.update(cx, |this, cx| {
                                            this.export_include_passwords = *checked;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .child("包含连接密码（导出时需重新确认主密码）"),
                            ),
                    )
                    // Note
                    .child(
                        div()
                            .p_2()
                            .rounded(px(6.0))
                            .when(cur_include_pwd, |d| {
                                d.bg(cx.theme().warning.opacity(0.1))
                                    .text_color(cx.theme().warning)
                                    .text_xs()
                                    .child("⚠ 包含凭据的导出文件含有明文连接密码，请妥善保管。")
                            })
                            .when(!cur_include_pwd, |d| {
                                d.bg(cx.theme().surface)
                                    .text_color(cx.theme().muted_foreground)
                                    .text_xs()
                                    .child("🛡 已脱敏导出（不含密码 / 私钥口令），文件可安全分享。")
                            }),
                    )
                    // Action buttons
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("btn-export-cancel")
                                    .small()
                                    .ghost()
                                    .label("取消")
                                    .on_click(move |_, _, cx| {
                                        view_for_cancel2.update(cx, |this, cx| {
                                            this.export_popover_open = false;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                Button::new("btn-export-confirm")
                                    .small()
                                    .primary()
                                    .label("导出文件")
                                    .on_click(move |_, window, cx| {
                                        view_for_export.update(cx, |this, cx| {
                                            this.handle_export(window, cx);
                                        });
                                    }),
                            ),
                    )
            })
            .into_any_element()
    }

    fn render_import_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();
        let step = self.import_step;
        let report = self.import_preflight_report.clone();
        let cur_strategy = self.import_merge_strategy;

        div()
            .id("import-modal-overlay")
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x0000008c)) // rgba(0,0,0,0.55)
            .flex()
            .items_center()
            .justify_center()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                v_flex()
                    .w(px(560.0))
                    .max_h(px(600.0))
                    .overflow_hidden()
                    .rounded(px(12.0))
                    .bg(cx.theme().surface)
                    .border_1()
                    .border_color(cx.theme().border)
                    .shadow_lg()
                    .p_5()
                    .gap_4()
                    // Header
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_base()
                                    .text_color(cx.theme().foreground)
                                    .child("导入连接配置"),
                            )
                            .child({
                                let v = view.clone();
                                div()
                                    .id("btn-close-import-x")
                                    .cursor_pointer()
                                    .text_base()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("×")
                                    .on_click(move |_, _, cx| {
                                        v.update(cx, |this, cx| {
                                            this.import_modal_open = false;
                                            cx.notify();
                                        });
                                    })
                            }),
                    )
                    // Body Step 1 or Step 2
                    .when(step == ImportStep::Step1, |this| {
                        let v_pick = view.clone();
                        let v_sample = view.clone();
                        this.child(
                            v_flex()
                                .gap_4()
                                .child(
                                    v_flex()
                                        .items_center()
                                        .justify_center()
                                        .gap_3()
                                        .p_6()
                                        .border_2()
                                        .border_color(cx.theme().border)
                                        .rounded(px(10.0))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("拖入 .json / .csv 配置文件，或"),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Button::new("btn-import-pick")
                                                        .label("选择文件…")
                                                        .on_click(move |_, window, cx| {
                                                            v_pick.update(cx, |this, cx| {
                                                                this.pick_and_parse_import_file(window, cx);
                                                            });
                                                        }),
                                                )
                                                .child(
                                                    Button::new("btn-import-sample")
                                                        .primary()
                                                        .label("使用示例文件（演示）")
                                                        .on_click(move |_, _, cx| {
                                                            v_sample.update(cx, |this, cx| {
                                                                this.use_sample_import(cx);
                                                            });
                                                        }),
                                                ),
                                        ),
                                ),
                        )
                    })
                    .when(step == ImportStep::Step2 && report.is_some(), |this| {
                        let rep = report.unwrap();
                        let v_strategy_keep = view.clone();
                        let v_strategy_over = view.clone();
                        let v_cancel = view.clone();
                        let v_apply = view.clone();

                        this.child(
                            v_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "解析到 {} 条：新增 {} · 冲突 {} · 相同 {}（按 主机:端口:用户 匹配）",
                                            rep.total, rep.new_count, rep.conflict_count, rep.same_count
                                        )),
                                )
                                // Scrollable diff list
                                .child(
                                    v_flex()
                                        .id("import-diff-scroll-list")
                                        .max_h(px(260.0))
                                        .overflow_y_scroll()
                                        .gap_2()
                                        .children(rep.items.iter().map(|item| {
                                            match &item.category {
                                                ImportCategory::New => {
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .p_2()
                                                        .rounded(px(8.0))
                                                        .border_1()
                                                        .border_color(cx.theme().success.opacity(0.4))
                                                        .bg(cx.theme().surface)
                                                        .child(
                                                            div()
                                                                .px_1p5()
                                                                .py_0p5()
                                                                .rounded(px(4.0))
                                                                .border_1()
                                                                .border_color(cx.theme().success)
                                                                .text_color(cx.theme().success)
                                                                .text_xs()
                                                                .child("新增"),
                                                        )
                                                        .child(div().font_weight(FontWeight::SEMIBOLD).text_xs().child(item.item.name.clone()))
                                                        .child(div().font_family("Consolas").text_xs().text_color(cx.theme().muted_foreground).child(format!("{}@{}:{}", item.item.user, item.item.host, item.item.port)))
                                                        .child(div().ml_auto().text_xs().text_color(cx.theme().muted_foreground).child(item.item.group.clone().unwrap_or_default()))
                                                }
                                                ImportCategory::Conflict { local_name, diff_desc } => {
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .p_2()
                                                        .rounded(px(8.0))
                                                        .border_1()
                                                        .border_color(cx.theme().warning.opacity(0.4))
                                                        .bg(cx.theme().surface)
                                                        .child(
                                                            div()
                                                                .px_1p5()
                                                                .py_0p5()
                                                                .rounded(px(4.0))
                                                                .border_1()
                                                                .border_color(cx.theme().warning)
                                                                .text_color(cx.theme().warning)
                                                                .text_xs()
                                                                .child("冲突"),
                                                        )
                                                        .child(div().font_weight(FontWeight::SEMIBOLD).text_xs().child(format!("{} → {}", local_name, item.item.name)))
                                                        .child(div().font_family("Consolas").text_xs().text_color(cx.theme().muted_foreground).child(format!("{}@{}:{}", item.item.user, item.item.host, item.item.port)))
                                                        .child(div().ml_auto().text_xs().text_color(cx.theme().muted_foreground).child(diff_desc.clone()))
                                                }
                                                ImportCategory::Same => {
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .p_2()
                                                        .rounded(px(8.0))
                                                        .border_1()
                                                        .border_color(cx.theme().border)
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(div().text_xs().child("相同"))
                                                        .child(div().text_xs().child(item.item.name.clone()))
                                                        .child(div().font_family("Consolas").text_xs().child(format!("{}@{}:{}", item.item.user, item.item.host, item.item.port)))
                                                }
                                            }
                                        })),
                                )
                                // Merge strategy radio
                                .child(
                                    h_flex()
                                        .gap_4()
                                        .items_center()
                                        .text_xs()
                                        .child(
                                            h_flex()
                                                .id("opt-strategy-keep")
                                                .gap_1p5()
                                                .items_center()
                                                .cursor_pointer()
                                                .on_click(move |_, _, cx| {
                                                    v_strategy_keep.update(cx, |this, cx| {
                                                        this.import_merge_strategy = MergeStrategy::KeepLocal;
                                                        cx.notify();
                                                    });
                                                })
                                                .child(
                                                    div()
                                                        .w(px(14.0))
                                                        .h(px(14.0))
                                                        .rounded_full()
                                                        .border_1()
                                                        .border_color(cx.theme().border)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .when(cur_strategy == MergeStrategy::KeepLocal, |d| {
                                                            d.child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(cx.theme().accent))
                                                        }),
                                                )
                                                .child("冲突时保留本地"),
                                        )
                                        .child(
                                            h_flex()
                                                .id("opt-strategy-over")
                                                .gap_1p5()
                                                .items_center()
                                                .cursor_pointer()
                                                .on_click(move |_, _, cx| {
                                                    v_strategy_over.update(cx, |this, cx| {
                                                        this.import_merge_strategy = MergeStrategy::Overwrite;
                                                        cx.notify();
                                                    });
                                                })
                                                .child(
                                                    div()
                                                        .w(px(14.0))
                                                        .h(px(14.0))
                                                        .rounded_full()
                                                        .border_1()
                                                        .border_color(cx.theme().border)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .when(cur_strategy == MergeStrategy::Overwrite, |d| {
                                                            d.child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(cx.theme().accent))
                                                        }),
                                                )
                                                .child("冲突时采用导入值"),
                                        ),
                                )
                                // Actions
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("btn-import-cancel-step2")
                                                .ghost()
                                                .label("取消")
                                                .on_click(move |_, _, cx| {
                                                    v_cancel.update(cx, |this, cx| {
                                                        this.import_modal_open = false;
                                                        cx.notify();
                                                    });
                                                }),
                                        )
                                        .child(
                                            Button::new("btn-import-apply")
                                                .primary()
                                                .label("应用导入")
                                                .on_click(move |_, window, cx| {
                                                    v_apply.update(cx, |this, cx| {
                                                        this.apply_import_data(window, cx);
                                                    });
                                                }),
                                        ),
                                ),
                        )
                    }),
            )
    }

    fn match_connection(&self, conn: &StoredConnection, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        if conn.name.to_lowercase().contains(query) {
            return true;
        }

        if let Some(env) = &conn.env {
            if env.to_lowercase().contains(query) {
                return true;
            }
        }

        for tag in conn.tags_vec() {
            if tag.to_lowercase().contains(query) {
                return true;
            }
        }

        if let Some(ws_id) = conn.workspace_id {
            if let Some(ws) = self.workspaces.iter().find(|w| w.id == Some(ws_id)) {
                if ws.name.to_lowercase().contains(query) {
                    return true;
                }
            }
        }

        let (user, host, port) = conn.endpoint_display();
        if user.to_lowercase().contains(query)
            || host.to_lowercase().contains(query)
            || port.to_string().contains(query)
            || format!("{user}@{host}:{port}")
                .to_lowercase()
                .contains(query)
        {
            return true;
        }

        match conn.connection_type {
            ConnectionType::SshSftp => {
                if let Ok(params) = conn.to_ssh_params() {
                    if params.host.to_lowercase().contains(query)
                        || params.port.to_string().contains(query)
                        || params.username.to_lowercase().contains(query)
                    {
                        return true;
                    }
                }
            }
            ConnectionType::PortForwarding => {
                if let Ok(params) = conn.to_port_forwarding_params()
                    && port_forwarding_connection_info(&params)
                        .to_lowercase()
                        .contains(query)
                {
                    return true;
                }
            }
            ConnectionType::Jms => {
                if let Ok(params) = conn.to_jms_params()
                    && (params.url.to_lowercase().contains(query)
                        || params.username.to_lowercase().contains(query))
                {
                    return true;
                }
            }
            _ => {}
        }

        false
    }

    fn render_content_area(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let search_query = self.search_query.read(cx).to_lowercase();
        let selected_id = self.selected_connection_id;
        self.render_workspace_view(&search_query, selected_id, cx)
            .into_any_element()
    }

    fn render_workspace_view(
        &mut self,
        _search_query: &str,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let visible = self.visible_connections(cx);

        if visible.is_empty() {
            if self.connections.is_empty() {
                return EmptyState::new("暂无连接")
                    .icon(IconName::Plus)
                    .description("创建你的第一个连接以开始使用。")
                    .action(
                        Button::new("empty-new-connection")
                            .primary()
                            .label("＋ 新建连接")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_new_connection_dialog(window, cx);
                            })),
                    )
                    .into_any_element();
            } else {
                return EmptyState::new("无匹配连接")
                    .icon(IconName::Search)
                    .description("试试调整搜索关键词或重置筛选条件。")
                    .action(
                        Button::new("clear-connection-filters")
                            .label("清空筛选")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_connection_filters(window, cx);
                            })),
                    )
                    .into_any_element();
            }
        }

        // 按分组归类
        let mut grouped: HashMap<String, Vec<StoredConnection>> = HashMap::new();
        for conn in visible {
            let gname = conn
                .workspace_id
                .and_then(|wid| {
                    self.workspaces
                        .iter()
                        .find(|w| w.id == Some(wid))
                        .map(|w| w.name.clone())
                })
                .unwrap_or_else(|| "默认分组".to_string());
            grouped.entry(gname).or_default().push(conn);
        }

        let mut ordered_groups: Vec<(String, Vec<StoredConnection>)> = Vec::new();
        for ws in &self.workspaces {
            if let Some(conns) = grouped.remove(&ws.name) {
                ordered_groups.push((ws.name.clone(), conns));
            }
        }
        for (gname, conns) in grouped {
            ordered_groups.push((gname, conns));
        }

        let mut sections = Vec::new();
        for (gname, conns) in ordered_groups {
            sections.push(self.render_workspace_section(gname, conns, selected_id, cx));
        }

        div()
            .id("home-content")
            .size_full()
            .overflow_y_scroll()
            .px_6()
            .py_4()
            .child(v_flex().gap_6().w_full().children(sections))
            .into_any_element()
    }

    fn render_connection_grid(
        &self,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_list = self.density == CardDensity::List;

        if is_list {
            let mut container = v_flex().w_full().gap_2();
            for connection in connections {
                container =
                    container.child(self.render_connection_card(connection, selected_id, cx));
            }
            container.into_any_element()
        } else {
            let mut container = div().flex().flex_wrap().w_full().gap_3();
            for connection in connections {
                container = container.child(
                    div()
                        .w(px(300.0))
                        .flex_shrink_0()
                        .child(self.render_connection_card(connection, selected_id, cx)),
                );
            }
            container.into_any_element()
        }
    }

    fn render_workspace_section(
        &self,
        group_name: String,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_collapsed = self.collapsed_groups.contains(&group_name);
        let gname_for_click = group_name.clone();
        let gname_for_collapse = group_name.clone();
        let home_for_collapse = cx.entity().clone();
        let ws_id = self
            .workspaces
            .iter()
            .find(|w| w.name == group_name)
            .and_then(|w| w.id);

        v_flex()
            .gap_3()
            // Sticky group header
            .child(
                h_flex()
                    .id(ElementId::Name(SharedString::from(format!(
                        "grp-head-{}",
                        gname_for_click
                    ))))
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        home_for_collapse.update(cx, |this, cx| {
                            if this.collapsed_groups.contains(&gname_for_collapse) {
                                this.collapsed_groups.remove(&gname_for_collapse);
                            } else {
                                this.collapsed_groups.insert(gname_for_collapse.clone());
                            }
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().foreground)
                            .child(if is_collapsed { "▶" } else { "▼" }),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(group_name),
                    )
                    .child(
                        div()
                            .px_2()
                            .rounded_full()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().surface)
                            .text_color(cx.theme().muted_foreground)
                            .text_xs()
                            .child(connections.len().to_string()),
                    )
                    .child(div().flex_1())
                    .when_some(ws_id, |this, id| {
                        this.child(
                            Button::new(SharedString::from(format!("sec-del-ws-{}", id)))
                                .icon(IconName::Remove)
                                .xsmall()
                                .ghost()
                                .tooltip("移除分组")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.delete_workspace(id, window, cx);
                                })),
                        )
                    }),
            )
            .when(!is_collapsed && !connections.is_empty(), |this| {
                this.child(self.render_connection_grid(connections, selected_id, cx))
            })
            .into_any_element()
    }
}

fn format_last_used(last_used_at: Option<i64>) -> Option<String> {
    let ts = last_used_at?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        Some("刚刚".to_string())
    } else if diff < 3600 {
        Some(format!("{} 分钟前", diff / 60))
    } else if diff < 86400 {
        Some(format!("{} 小时前", diff / 3600))
    } else if diff < 172800 {
        Some("昨天".to_string())
    } else if diff < 604800 {
        Some(format!("{} 天前", diff / 86400))
    } else {
        Some(format!("{} 周前", diff / 604800))
    }
}

impl HomePage {
    fn render_connection_card(
        &self,
        conn: StoredConnection,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let conn_id = conn.id;
        let is_selected = selected_id == conn.id;
        let is_active = conn
            .id
            .map_or(false, |id| cx.global::<ActiveConnections>().is_active(id));
        let has_team = conn.team_id.is_some();
        let connection_type = conn.connection_type;

        let icon = match connection_type {
            ConnectionType::SshSftp => IconName::SquareTerminal
                .mono()
                .with_size(px(28.0))
                .text_color(cx.theme().connection_ssh),
            ConnectionType::PortForwarding => IconName::Network
                .mono()
                .with_size(px(28.0))
                .text_color(cx.theme().connection_port_forwarding),
            ConnectionType::Jms => IconName::Key
                .mono()
                .with_size(px(28.0))
                .text_color(cx.theme().connection_jms),
            _ => IconName::Server
                .mono()
                .with_size(px(28.0))
                .text_color(cx.theme().connection_db),
        };

        let (user, host, port) = conn.endpoint_display();
        let endpoint_str = if !host.is_empty() {
            format!("{user}@{host}:{port}")
        } else {
            String::new()
        };

        let proto_label = match connection_type {
            ConnectionType::SshSftp => "SSH",
            ConnectionType::PortForwarding => "PortForwarding",
            ConnectionType::Jms => "JMS",
            _ => "DB",
        };

        let is_checked = conn
            .id
            .map_or(false, |id| self.selected_connection_ids.contains(&id));
        let show_chk = !self.selected_connection_ids.is_empty();

        let open_conn = conn.clone();
        let connect_conn = conn.clone();
        let view_for_connect = cx.entity().clone();
        let view_for_check = cx.entity().clone();
        let view_for_fav = cx.entity().clone();

        let mut card = ConnectionCard::new(
            SharedString::from(format!("conn-card-{}", conn.id.unwrap_or(0))),
            icon,
            conn.name.clone(),
        )
        .menu_id(SharedString::from(format!(
            "conn-more-{}",
            conn.id.unwrap_or(0)
        )))
        .density(self.density)
        .selected(is_selected)
        .active(is_active)
        .favorite(conn.is_favorite)
        .checked(is_checked)
        .show_checkbox(show_chk)
        .protocol(proto_label)
        .endpoint(endpoint_str)
        .tags(conn.tags_vec())
        .on_click(cx.listener(move |this, _, window, cx| {
            this.selected_connection_id = conn_id;
            this.open_connection_from_quick(&open_conn, window, cx);
        }))
        .on_connect(move |window, cx| {
            view_for_connect.update(cx, |this, cx| {
                this.open_connection_from_quick(&connect_conn, window, cx);
            });
        });

        if let Some(env_str) = &conn.env {
            card = card.env(env_str.clone());
        }

        if let Some(last_used_str) = format_last_used(conn.last_used_at) {
            card = card.last_used(last_used_str);
        }

        if let Some(id) = conn_id {
            card = card.on_check(move |checked, _window, cx| {
                view_for_check.update(cx, |this, cx| {
                    if checked {
                        this.selected_connection_ids.insert(id);
                    } else {
                        this.selected_connection_ids.remove(&id);
                    }
                    cx.notify();
                });
            });

            card = card.on_toggle_favorite(move |_window, cx| {
                view_for_fav.update(cx, |this, cx| {
                    this.toggle_favorite_connection(id, cx);
                });
            });
        }

        if has_team {
            card = card.team_badge(t!("Home.team_badge"));
        }

        let card_id = conn.id.unwrap_or(0);
        if connection_type == ConnectionType::PortForwarding && is_active {
            let stop_conn = conn.clone();
            card = card.action(
                Button::new(SharedString::from(format!("stop-conn-{}", card_id)))
                    .icon(IconName::Stop.mono())
                    .with_size(Size::Small)
                    .danger()
                    .tooltip(t!("PortForwarding.stop"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.stop_port_forwarding(stop_conn.clone(), window, cx);
                    })),
            );
        }

        let home = cx.entity();
        card = card.menu(move |mut menu, window, _cx| {
            let fav_conn_id = conn.id;
            let is_fav_conn = conn.is_favorite;
            menu = menu.item(
                PopupMenuItem::new(if is_fav_conn {
                    "取消收藏"
                } else {
                    "收藏"
                })
                .icon(IconName::Star.mono())
                .on_click(window.listener_for(
                    &home,
                    move |this, _, _window, cx| {
                        if let Some(id) = fav_conn_id {
                            this.toggle_favorite_connection(id, cx);
                        }
                    },
                )),
            );

            if connection_type == ConnectionType::SshSftp {
                let sftp_conn = conn.clone();
                menu = menu.item(
                    PopupMenuItem::new(t!("Home.open_sftp"))
                        .icon(IconName::Folder1.mono())
                        .on_click(window.listener_for(&home, move |this, _, window, cx| {
                            this.open_sftp_view(sftp_conn.clone(), window, cx);
                        })),
                );
            }

            let duplicate_conn = conn.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("Home.duplicate_connection"))
                    .icon(IconName::Copy)
                    .on_click(window.listener_for(&home, move |this, _, window, cx| {
                        this.duplicate_connection(duplicate_conn.clone(), window, cx);
                    })),
            );

            let edit_conn = conn.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("Home.edit_connection"))
                    .icon(IconName::Edit)
                    .on_click(window.listener_for(&home, move |this, _, window, cx| {
                        if let Some(conn_id) = edit_conn.id {
                            match connection_type {
                                ConnectionType::SshSftp => {
                                    this.editing_connection_id = Some(conn_id);
                                    this.show_ssh_form(window, cx);
                                }
                                ConnectionType::PortForwarding => {
                                    this.editing_connection_id = Some(conn_id);
                                    this.show_port_forwarding_form(window, cx);
                                }
                                ConnectionType::Jms => {
                                    this.open_jms_connection_prefilled(
                                        edit_conn.clone(),
                                        window,
                                        cx,
                                    );
                                }
                                _ => {}
                            }
                        }
                    })),
            );

            let delete_conn_name = conn.name.clone();
            menu.item(
                PopupMenuItem::new(t!("Home.delete_connection"))
                    .icon(IconName::Remove)
                    .on_click(window.listener_for(&home, move |this, _, window, cx| {
                        if let Some(conn_id) = conn_id {
                            this.confirm_delete_connection(
                                conn_id,
                                delete_conn_name.clone(),
                                window,
                                cx,
                            );
                        }
                    })),
            )
        });

        card.into_any_element()
    }
}

fn port_forwarding_connection_info(params: &one_core::storage::PortForwardingParams) -> String {
    match params.kind {
        one_core::storage::PortForwardingKind::Local => format!(
            "{}:{} -> {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        one_core::storage::PortForwardingKind::Dynamic => {
            format!("SOCKS {}:{}", params.bind_host, params.bind_port)
        }
    }
}

fn push_notification_on_active_window(message: String, cx: &mut AsyncApp) {
    let _ = cx.update(|cx| {
        if let Some(window_id) = cx.active_window() {
            let _ = cx.update_window(window_id, |_, window, cx| {
                window.push_notification(message.clone(), cx);
            });
        }
    });
}

/// 生成复制连接的唯一名称
fn generate_duplicate_name(original_name: &str, existing_names: &HashSet<String>) -> String {
    let base_name = t!("Home.duplicate_name", name = original_name).to_string();

    if !existing_names.contains(&base_name) {
        return base_name;
    }

    // 如果基础名称已存在，添加数字序号
    for i in 2..100 {
        let name = t!(
            "Home.duplicate_name_numbered",
            name = original_name,
            index = i
        )
        .to_string();
        if !existing_names.contains(&name) {
            return name;
        }
    }

    base_name
}

impl Focusable for HomePage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for HomePage {}

impl TabContent for HomePage {
    fn content_key(&self) -> &'static str {
        "Home"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("Home.title"))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        None
    }

    fn font_weight(&self, _cx: &App) -> FontWeight {
        FontWeight::BOLD
    }

    fn closeable(&self, _cx: &App) -> bool {
        false
    }

    fn width_size(&self, _cx: &App) -> Option<Size> {
        Some(Size::Small)
    }
}

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.master_key_unlock_prompt_pending && self.saved_connections_locked() {
            self.master_key_unlock_prompt_pending = false;
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.show_encryption_key_dialog(window, cx);
                });
            });
        }

        // 处理 JMS 待连接队列
        while let Some(conn) = self.pending_jms_connections.pop() {
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.open_ssh_terminal(conn, None, window, cx, None);
                });
            });
        }

        // 处理 JMS Koko WebSocket 待连接队列
        while let Some((params, jms_context)) = self.pending_jms_koko.pop() {
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.open_jms_koko_terminal(params, jms_context, window, cx);
                });
            });
        }

        // 处理 JMS 占位终端队列(登录成功后直接开,只有资产树)
        while let Some(ctx) = self.pending_jms_placeholder.pop() {
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.open_jms_placeholder_terminal(ctx, window, cx);
                });
            });
        }

        div()
            .key_context("HomePage")
            .on_action(cx.listener(Self::open_selected_connection))
            .on_action(cx.listener(Self::select_previous_connection))
            .on_action(cx.listener(Self::select_next_connection))
            .on_action(cx.listener(Self::handle_local_terminal_shortcut))
            .on_action(cx.listener(Self::handle_focus_search_shortcut))
            .on_action(cx.listener(Self::handle_escape_shortcut))
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .child(
                h_flex()
                    .size_full()
                    .child(self.render_rail(window, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .bg(cx.theme().background)
                            .child(self.render_unified_toolbar(window, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .w_full()
                                    .overflow_hidden()
                                    .bg(cx.theme().background)
                                    .child(self.render_content_area(cx)),
                            ),
                    ),
            )
            .when(self.import_modal_open, |this| {
                this.child(self.render_import_modal(window, cx))
            })
            .into_any_element()
    }
}

/// 使用当前主密钥重新加密并保存所有连接。
fn re_encrypt_all_connections(
    storage: &one_core::storage::StorageManager,
) -> anyhow::Result<usize> {
    let conn_repo = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

    let connections = conn_repo.list()?;
    let mut count = 0;

    for conn in connections {
        conn_repo.update(&conn)?;
        count += 1;
    }

    Ok(count)
}
