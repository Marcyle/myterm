use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::{List, ListState},
    popover::Popover,
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem, SidebarToggleButton},
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
use crate::home::home_strategy::build_connection_open_strategy;
use crate::home::home_workspace_filter::{WorkspaceFilterDelegate, show_workspace_dialog};
use crate::new_connection::NewConnectionWindow;
use one_ui::{ConnectionCard, EmptyState};

actions!(home_tab, [OpenConnectionQuickOpen, NewConnectionShortcut]);

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let quick_open_default = if cfg!(target_os = "macos") {
        "cmd-o"
    } else {
        "alt-o"
    };
    let new_connection_default = if cfg!(target_os = "macos") {
        "cmd-n"
    } else {
        "alt-n"
    };
    let mut keybindings = Vec::new();
    keybindings.extend(
        shortcuts_for(cx, action_id::HOME_QUICK_OPEN, &[quick_open_default])
            .into_iter()
            .map(|key| KeyBinding::new(&key, OpenConnectionQuickOpen, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::HOME_NEW_CONNECTION,
            &[new_connection_default],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, NewConnectionShortcut, None)),
    );
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::HOME_QUICK_OPEN,
        &[home_default_shortcut("cmd-o", "alt-o")],
        None,
        OpenConnectionQuickOpen,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::HOME_NEW_CONNECTION,
        &[home_default_shortcut("cmd-n", "alt-n")],
        None,
        NewConnectionShortcut,
    ));
    keybindings
}

fn home_default_shortcut(macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
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
    pub(crate) workspace_filter_open: bool,
    workspace_filter_list: Option<Entity<ListState<WorkspaceFilterDelegate>>>,
    pub(crate) _subscriptions: Vec<Subscription>,
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
    /// 侧边栏是否收起
    pub(crate) sidebar_collapsed: bool,
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
            workspace_filter_open: false,
            workspace_filter_list: None,
            _subscriptions: Vec::new(),
            port_forwarding_runtime: Arc::new(
                tokio::sync::Mutex::new(PortForwardingRuntime::new()),
            ),
            master_key_dialog_open: false,
            master_key_unlock_prompt_pending: false,
            pending_jms_connections: Vec::new(),
            pending_jms_koko: Vec::new(),
            pending_jms_placeholder: Vec::new(),
            sidebar_collapsed: false,
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
                _ => {}
            },
        )
        .detach();

        page
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

    pub(crate) fn show_jms_connection_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::jms_connection_window::JmsConnectionWindow;
        use one_core::popup_window::{PopupWindowOptions, open_popup_window};

        let parent = cx.entity();
        let parent_window = window.window_handle();
        open_popup_window(
            PopupWindowOptions::new("JMS 连接".to_string()).size(800.0, 600.0),
            move |window, cx| {
                cx.new(|cx| JmsConnectionWindow::new(parent, parent_window, window, cx))
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
        let parent_window = window.window_handle();
        open_popup_window(
            PopupWindowOptions::new("JMS 连接".to_string()).size(800.0, 600.0),
            move |window, cx| {
                cx.new(|cx| {
                    JmsConnectionWindow::new_with_prefill(
                        parent.clone(),
                        parent_window,
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
                .title(t!("Workspace.delete").to_string().into_any_element())
                .child(
                    t!("Workspace.delete_confirm", workspace_name = workspace_name)
                        .to_string()
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

    fn render_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();

        let workspace_filter_open = self.workspace_filter_open;
        let workspace_filter =
            self.render_workspace_filter_popover(workspace_filter_open, window, cx);

        let has_master_key = crypto::has_master_key();

        h_flex()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .items_center()
            // ===== 左侧功能区 =====
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    // 新建连接按钮（主要操作）
                    .child(
                        Button::new("new-connect-button")
                            .icon(IconName::Plus)
                            .primary()
                            .label(t!("Home.new_connection"))
                            .tooltip(t!("Home.new_connection"))
                            .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                this.show_new_connection_dialog(window, cx);
                            })),
                    )
                    // 本地终端按钮
                    .child(
                        Button::new("local-terminal-button")
                            .icon(IconName::SquareTerminal)
                            .label(t!("Terminal.local"))
                            .tooltip(t!("Terminal.local"))
                            .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                this.add_terminal_tab(window, cx);
                            })),
                    )
                    // 新建JMS连接按钮
                    .child(
                        Button::new("new-jms-button")
                            .icon(IconName::Key)
                            .label("JMS")
                            .tooltip("新建JMS连接")
                            .ghost()
                            .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                this.show_jms_connection_dialog(window, cx);
                            })),
                    )
                    // 分隔线
                    .child(div().h(px(20.0)).w(px(1.0)).bg(cx.theme().border).mx_1())
                    // 主密钥按钮
                    .child(
                        Button::new("encryption-key-button")
                            .icon(IconName::Key)
                            .label(if has_master_key {
                                t!("Encryption.key_unlocked").to_string()
                            } else {
                                t!("Encryption.edit_repo_password").to_string()
                            })
                            .ghost()
                            .when(has_master_key, |btn| btn.text_color(cx.theme().success))
                            .when(!has_master_key, |btn| {
                                btn.text_color(cx.theme().muted_foreground)
                            })
                            .tooltip(if has_master_key {
                                t!("Encryption.key_unlocked_tooltip")
                            } else {
                                t!("Encryption.key_locked_tooltip")
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_encryption_key_dialog(window, cx);
                            })),
                    ),
            )
            // ===== 中间弹性空间 =====
            .child(div().flex_1())
            // ===== 右侧操作区 =====
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    // 搜索框
                    .child(
                        Input::new(&self.search_input)
                            .cleanable(true)
                            .w(px(240.0))
                            .bg(cx.theme().muted),
                    )
                    // 刷新按钮
                    .child(
                        Button::new("refresh-button")
                            .icon(IconName::Refresh)
                            .ghost()
                            .tooltip(t!("Home.refresh"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_local_home_data(cx);
                            })),
                    )
                    // 工作区筛选
                    .child(workspace_filter),
            )
    }

    fn render_workspace_filter_popover(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let view_for_select = view.clone();
        let view_for_clear = view.clone();
        let view_for_new = view.clone();

        let list = self.ensure_workspace_filter_list(window, cx);

        let workspaces = &self.workspaces;
        let connections = &self.connections;
        let filtered_ids = &self.filtered_workspace_ids;
        list.update(cx, |state, _cx| {
            state
                .delegate_mut()
                .update_items_with_data(workspaces, connections, filtered_ids);
        });

        let is_all_selected = self.filtered_workspace_ids.is_empty()
            || self.filtered_workspace_ids.len()
                == self.workspaces.iter().filter(|w| w.id.is_some()).count();

        Popover::new("workspace-filter-popover")
            .trigger(
                Button::new("workspace-filter")
                    .icon(IconName::Filter)
                    .tooltip(t!("Workspace.filter")),
            )
            .open(open)
            .on_open_change(cx.listener(|this, open, _, cx| {
                this.workspace_filter_open = *open;
                cx.notify();
            }))
            .content(move |_, _, cx| {
                v_flex()
                    .w(px(280.0))
                    .max_h(px(400.0))
                    .gap_2()
                    .p_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .px_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child({
                                        let view_select = view_for_select.clone();
                                        Checkbox::new("select-all-ws")
                                            .checked(is_all_selected)
                                            .on_click(move |_, _, cx| {
                                                view_select.update(cx, |this, cx| {
                                                    if this.filtered_workspace_ids.is_empty()
                                                        || this.filtered_workspace_ids.len()
                                                            == this
                                                                .workspaces
                                                                .iter()
                                                                .filter(|w| w.id.is_some())
                                                                .count()
                                                    {
                                                        this.clear_workspace_filter(cx);
                                                    } else {
                                                        this.select_all_workspaces(cx);
                                                    }
                                                });
                                            })
                                    })
                                    .child(div().text_sm().child(
                                        t!("Workspace.select_all").to_string().into_any_element(),
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child({
                                        let view_new = view_for_new.clone();
                                        Button::new("new-workspace-from-filter")
                                            .primary()
                                            .small()
                                            .label(t!("Common.new"))
                                            .on_click(move |_, window, cx| {
                                                show_workspace_dialog(
                                                    view_new.clone(),
                                                    None,
                                                    String::new(),
                                                    window,
                                                    cx,
                                                );
                                            })
                                    })
                                    .child({
                                        let view_clear = view_for_clear.clone();
                                        Button::new("clear-ws-filter")
                                            .ghost()
                                            .small()
                                            .label(t!("Workspace.clear_filter"))
                                            .on_click(move |_, _, cx| {
                                                view_clear.update(cx, |this, cx| {
                                                    this.clear_workspace_filter(cx);
                                                });
                                            })
                                    }),
                            ),
                    )
                    .child(div().border_t_1().border_color(cx.theme().border))
                    .child(
                        List::new(&list)
                            .w_full()
                            .max_h(px(320.0))
                            .p(px(8.))
                            .flex_1()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(cx.theme().radius),
                    )
            })
            .into_any_element()
    }

    fn ensure_workspace_filter_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ListState<WorkspaceFilterDelegate>> {
        if let Some(ref list) = self.workspace_filter_list {
            return list.clone();
        }

        let parent = cx.entity();
        let list = cx.new(|cx| {
            ListState::new(WorkspaceFilterDelegate::new(parent), window, cx).searchable(true)
        });
        self.workspace_filter_list = Some(list.clone());
        list
    }

    pub(crate) fn toggle_workspace_filter(&mut self, workspace_id: i64, cx: &mut Context<Self>) {
        if self.filtered_workspace_ids.is_empty() {
            for ws in &self.workspaces {
                if let Some(id) = ws.id {
                    self.filtered_workspace_ids.insert(id);
                }
            }
        }

        if self.filtered_workspace_ids.contains(&workspace_id) {
            self.filtered_workspace_ids.remove(&workspace_id);
        } else {
            self.filtered_workspace_ids.insert(workspace_id);
        }
        cx.notify();
    }

    fn select_all_workspaces(&mut self, cx: &mut Context<Self>) {
        self.filtered_workspace_ids.clear();
        for ws in &self.workspaces {
            if let Some(id) = ws.id {
                self.filtered_workspace_ids.insert(id);
            }
        }
        cx.notify();
    }

    fn clear_workspace_filter(&mut self, cx: &mut Context<Self>) {
        self.filtered_workspace_ids.clear();
        cx.notify();
    }

    fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let filter_types = ConnectionType::all();

        Sidebar::new("home-filter-sidebar")
            .collapsible(true)
            .collapsed(collapsed)
            .header(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(!collapsed, |this| this.child(t!("Home.title"))),
                    )
                    .child(
                        SidebarToggleButton::new()
                            .collapsed(collapsed)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_collapsed = !this.sidebar_collapsed;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                SidebarMenu::new().children(filter_types.into_iter().map(|filter_type| {
                    let is_selected = self.selected_filter == filter_type;
                    SidebarMenuItem::new(filter_type.label())
                        .icon(Icon::new(filter_type.icon()).mono().with_size(Size::Large))
                        .active(is_selected)
                        .on_click(cx.listener(move |this: &mut HomePage, _, _, cx| {
                            this.selected_filter = filter_type;
                            cx.notify();
                        }))
                })),
            )
            .footer(
                Button::new("open_settings")
                    .icon(IconName::Settings)
                    .label(t!("Common.settings"))
                    .w_full()
                    .justify_start()
                    .when(collapsed, |this| this.label(""))
                    .on_click(cx.listener(|this: &mut HomePage, _, window, cx| {
                        this.add_settings_tab(window, cx);
                    })),
            )
    }

    fn match_connection_type(&self, conn: &StoredConnection) -> bool {
        match self.selected_filter {
            ConnectionType::All => true,
            filter_type => conn.connection_type == filter_type,
        }
    }

    fn match_connection(&self, conn: &StoredConnection, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        // 匹配连接名称
        if conn.name.to_lowercase().contains(query) {
            return true;
        }

        // 根据连接类型解析对应参数进行匹配
        match conn.connection_type {
            ConnectionType::SshSftp => {
                if let Ok(params) = conn.to_ssh_params() {
                    if params.host.to_lowercase().contains(query) {
                        return true;
                    }
                    if params.port.to_string().contains(query) {
                        return true;
                    }
                    if params.username.to_lowercase().contains(query) {
                        return true;
                    }
                    let conn_str = format!("{}@{}:{}", params.username, params.host, params.port);
                    if conn_str.to_lowercase().contains(query) {
                        return true;
                    }
                }
            }
            ConnectionType::PortForwarding => {
                if let Ok(params) = conn.to_port_forwarding_params() {
                    if port_forwarding_connection_info(&params)
                        .to_lowercase()
                        .contains(query)
                    {
                        return true;
                    }
                }
            }
            ConnectionType::Jms => {
                if let Ok(params) = conn.to_jms_params() {
                    if params.url.to_lowercase().contains(query)
                        || params.username.to_lowercase().contains(query)
                    {
                        return true;
                    }
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
        &self,
        search_query: &str,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspaces_with_connections: Vec<_> = self
            .workspaces
            .iter()
            .filter(|ws| {
                if self.filtered_workspace_ids.is_empty() {
                    return true;
                }
                match ws.id {
                    Some(id) => self.filtered_workspace_ids.contains(&id),
                    None => true,
                }
            })
            .map(|ws| {
                let conn_list: Vec<_> = self
                    .connections
                    .iter()
                    .filter(|conn| conn.workspace_id == ws.id)
                    .filter(|conn| self.match_connection(conn, search_query))
                    .filter(|conn| self.match_connection_type(conn))
                    .cloned()
                    .collect();
                (ws.clone(), conn_list)
            })
            .collect();

        let unassigned_connections: Vec<_> = self
            .connections
            .iter()
            .filter(|conn| conn.workspace_id.is_none())
            .filter(|conn| self.match_connection(conn, search_query))
            .filter(|conn| self.match_connection_type(conn))
            .cloned()
            .collect();

        div()
            .id("home-content")
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .child({
                let mut container = v_flex().gap_8().w_full();
                let mut has_content = false;

                // 过滤掉空的工作区
                for (workspace, connections) in workspaces_with_connections {
                    if connections.is_empty() {
                        continue;
                    }
                    has_content = true;
                    container = container.child(self.render_workspace_section(
                        workspace,
                        connections,
                        selected_id,
                        cx,
                    ));
                }

                // 如果用户没有设置工作区，直接显示连接列表；否则显示未分配工作区
                if !unassigned_connections.is_empty() {
                    has_content = true;
                    let has_workspaces = self.workspaces.iter().any(|ws| ws.id.is_some());
                    if has_workspaces {
                        container = container.child(self.render_unassigned_section(
                            unassigned_connections,
                            selected_id,
                            cx,
                        ));
                    } else {
                        // 没有工作区时，直接显示连接卡片
                        container = container.child(self.render_connections_grid(
                            unassigned_connections,
                            selected_id,
                            cx,
                        ));
                    }
                }

                if has_content {
                    container.into_any_element()
                } else {
                    EmptyState::new(t!("Home.no_connections"))
                        .icon(IconName::Search)
                        .description(t!("Home.no_connections_hint"))
                        .action(
                            Button::new("empty-new-connection")
                                .primary()
                                .label(t!("Home.new_connection"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.show_new_connection_dialog(window, cx);
                                })),
                        )
                        .into_any_element()
                }
            })
    }

    fn render_workspace_section(
        &self,
        workspace: Workspace,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace_id = workspace.id;
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        Icon::new(IconName::Apps)
                            .mono()
                            .text_color(cx.theme().muted_foreground)
                            .with_size(Size::Medium),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(SharedString::from(format!(
                                "workspace-name-{}",
                                workspace_id.unwrap_or(0)
                            ))))
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(workspace.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!("Home.connection_count", count = connections.len()).to_string(),
                            ),
                    )
                    .child(div().flex_1()),
            )
            .when(!connections.is_empty(), |this| {
                // 使用 flex 布局实现响应式卡片网格
                let mut container = div().flex().flex_wrap().w_full().gap_3();

                for conn in connections {
                    container = container.child(
                        div()
                            .flex_1()
                            .min_w(px(260.0))
                            .max_w(px(360.0))
                            .child(self.render_connection_card(conn, selected_id, cx)),
                    );
                }

                this.child(container)
            })
    }

    fn render_connections_grid(
        &self,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut container = div().flex().flex_wrap().w_full().gap_3();

        for conn in connections {
            container = container.child(
                div()
                    .flex_1()
                    .min_w(px(260.0))
                    .max_w(px(360.0))
                    .child(self.render_connection_card(conn, selected_id, cx)),
            );
        }
        container
    }

    fn render_unassigned_section(
        &self,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(
                                t!("Home.unassigned_workspace")
                                    .to_string()
                                    .into_any_element(),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!("Home.connection_count", count = connections.len()).to_string(),
                            ),
                    ),
            )
            .child({
                // 使用 flex 布局实现响应式卡片网格
                let mut container = div().flex().flex_wrap().w_full().gap_3();

                for conn in connections {
                    container = container.child(
                        div()
                            .flex_1()
                            .min_w(px(260.0))
                            .max_w(px(360.0))
                            .child(self.render_connection_card(conn, selected_id, cx)),
                    );
                }
                container
            })
    }

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

        let icon = match conn.connection_type {
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

        let subtitle: Option<String> = match conn.connection_type {
            ConnectionType::SshSftp => conn
                .to_ssh_params()
                .ok()
                .map(|params| format!("{}@{}:{}", params.username, params.host, params.port)),
            ConnectionType::PortForwarding => conn
                .to_port_forwarding_params()
                .ok()
                .map(|params| port_forwarding_connection_info(&params)),
            ConnectionType::Jms => conn
                .to_jms_params()
                .ok()
                .map(|params| format!("{}@{}", params.username, params.url)),
            _ => None,
        };

        let open_conn = conn.clone();
        let mut card = ConnectionCard::new(
            SharedString::from(format!("conn-card-{}", conn.id.unwrap_or(0))),
            icon,
            conn.name.clone(),
        )
        .selected(is_selected)
        .active(is_active)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.selected_connection_id = conn_id;
            cx.notify();
        }))
        .on_double_click(cx.listener(move |this, _, window, cx| {
            this.open_connection_from_quick(&open_conn, window, cx);
            cx.notify();
        }));

        let card_id = conn.id.unwrap_or(0);

        if has_team {
            card = card.team_badge(t!("Home.team_badge"));
        }

        if let Some(subtitle) = subtitle {
            card = card.subtitle(subtitle);
        }

        if conn.connection_type == ConnectionType::SshSftp {
            let sftp_conn = conn.clone();
            card = card.action(
                Button::new(SharedString::from(format!("sftp-conn-{}", card_id)))
                    .icon(IconName::Folder1.mono())
                    .with_size(Size::Small)
                    .primary()
                    .tooltip(t!("Home.open_sftp"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_sftp_view(sftp_conn.clone(), window, cx);
                    })),
            );
        }

        let duplicate_conn = conn.clone();
        card = card.action(
            Button::new(SharedString::from(format!("duplicate-conn-{}", card_id)))
                .icon(IconName::Copy)
                .with_size(Size::Small)
                .primary()
                .tooltip(t!("Home.duplicate_connection"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.duplicate_connection(duplicate_conn.clone(), window, cx);
                })),
        );

        let edit_conn = conn.clone();
        let edit_conn_type = conn.connection_type;
        card = card.action(
            Button::new(SharedString::from(format!("edit-conn-{}", card_id)))
                .icon(IconName::Edit)
                .with_size(Size::Small)
                .primary()
                .tooltip(t!("Home.edit_connection"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    if let Some(conn_id) = edit_conn.id {
                        match edit_conn_type {
                            ConnectionType::SshSftp => {
                                this.editing_connection_id = Some(conn_id);
                                this.show_ssh_form(window, cx);
                            }
                            ConnectionType::PortForwarding => {
                                this.editing_connection_id = Some(conn_id);
                                this.show_port_forwarding_form(window, cx);
                            }
                            ConnectionType::Jms => {
                                this.open_jms_connection_prefilled(edit_conn.clone(), window, cx);
                            }
                            _ => {}
                        }
                    }
                })),
        );

        let delete_conn_name = conn.name.clone();
        card = card.action(
            Button::new(SharedString::from(format!("delete-conn-{}", card_id)))
                .icon(IconName::Remove)
                .with_size(Size::Small)
                .danger()
                .tooltip(t!("Home.delete_connection"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    if let Some(conn_id) = conn.id {
                        let conn_name = delete_conn_name.clone();
                        this.confirm_delete_connection(conn_id, conn_name, window, cx);
                    }
                })),
        );

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
        Some(IconName::Home.mono())
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

        div().size_full().track_focus(&self.focus_handle).child(
            h_flex()
                .size_full()
                .child(self.render_sidebar(window, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .bg(cx.theme().background)
                        .child(self.render_toolbar(window, cx))
                        .child(
                            div()
                                .flex_1()
                                .w_full()
                                .overflow_hidden()
                                .bg(cx.theme().muted)
                                .child(self.render_content_area(cx)),
                        ),
                ),
        )
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
