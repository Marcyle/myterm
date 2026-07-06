use crate::home_tab::HomePage;
use crate::setting_tab::SettingsPanel;
use gpui::{App, Context, Entity, Window};
use gpui::{AppContext, AsyncApp};
use gpui_component::notification::Notification;
use gpui_component::WindowExt;
use one_core::storage::{StoredConnection, Workspace};
use one_core::tab_container::TabItem;
use sftp_view::{SftpView, SftpViewEvent};
use terminal::LocalConfig;
use terminal::terminal::ConnectionState;
use terminal_view::{
    TerminalConnectionKind, TerminalView, TerminalViewEvent,
    current_settings as current_terminal_settings,
};

impl HomePage {
    fn terminal_sync_path_enabled(cx: &App) -> bool {
        current_terminal_settings(cx).sync_path_with_terminal
    }

    pub(crate) fn open_ssh_terminal(
        &mut self,
        conn: StoredConnection,
        _workspace: Option<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
        working_dir: Option<String>,
    ) {
        let conn_id = conn.id.unwrap_or(0);
        // 使用时间戳生成唯一 tab_id，支持同一连接打开多个 SSH 终端
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("ssh-terminal-{}-{}", conn_id, timestamp);

        // 统计同一连接的 SSH 终端数量，计算序号
        let prefix = format!("ssh-terminal-{}-", conn_id);
        let existing_count = self
            .tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };
        let sync_path = Self::terminal_sync_path_enabled(cx);

        let terminal_view = cx.new(|cx| {
            TerminalView::new_ssh_with_index(
                conn,
                tab_index,
                window,
                cx,
                working_dir.as_deref(),
                sync_path,
            )
        });
        self.tab_container.update(cx, |tc, cx| {
            let tab = TabItem::new(tab_id, "ssh", terminal_view);
            tc.add_and_activate_tab_with_focus(tab, window, cx);
        });
    }

    /// 打开 JumpServer Koko WebSocket 终端
    pub(crate) fn open_jms_koko_terminal(
        &mut self,
        params: jms::KokoConnectParams,
        jms_context: Option<terminal_view::JmsSidebarContext>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("jms-koko-{}", timestamp);

        // 统计已有 JMS 终端数量，计算序号
        let existing_count = self
            .tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with("jms-koko-"))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        let terminal_view = cx.new(|cx| {
            TerminalView::new_jms_koko_with_context(params, jms_context, tab_index, window, cx)
        });

        // 订阅资产树侧栏冒泡的"打开新 JMS 终端"事件,递归开新 tab(携带资产树上下文)
        let sub = cx.subscribe_in(
            &terminal_view,
            window,
            move |this, _view, event: &TerminalViewEvent, window, cx| match event {
                TerminalViewEvent::OpenJmsTerminal(params, ctx) => {
                    this.open_jms_koko_terminal(params.clone(), ctx.clone(), window, cx);
                }
            },
        );
        self._subscriptions.push(sub);

        self.tab_container.update(cx, |tc, cx| {
            let tab = TabItem::new(tab_id, "ssh", terminal_view);
            tc.add_and_activate_tab_with_focus(tab, window, cx);
        });
    }

    /// 打开 JMS 占位终端(登录成功后立即开,只有资产树,等用户选资产连接)
    pub(crate) fn open_jms_placeholder_terminal(
        &mut self,
        jms_context: terminal_view::JmsSidebarContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("jms-koko-{}", timestamp);

        let existing_count = self
            .tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with("jms-koko-"))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        let terminal_view = cx.new(|cx| {
            TerminalView::new_jms_koko_placeholder(Some(jms_context), tab_index, window, cx)
        });

        // 订阅资产树侧栏的开新 tab 事件
        let sub = cx.subscribe_in(
            &terminal_view,
            window,
            move |this, _view, event: &TerminalViewEvent, window, cx| match event {
                TerminalViewEvent::OpenJmsTerminal(params, ctx) => {
                    this.open_jms_koko_terminal(params.clone(), ctx.clone(), window, cx);
                }
            },
        );
        self._subscriptions.push(sub);

        self.tab_container.update(cx, |tc, cx| {
            let tab = TabItem::new(tab_id, "ssh", terminal_view);
            tc.add_and_activate_tab_with_focus(tab, window, cx);
        });
    }

    pub(crate) fn open_sftp_view(
        &mut self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conn_id = conn.id.unwrap_or(0);
        // 使用时间戳生成唯一 tab_id，支持同一连接打开多个 SFTP 视图
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("sftp-{}-{}", conn_id, timestamp);

        // 统计同一连接的 SFTP 视图数量，计算序号
        let prefix = format!("sftp-{}-", conn_id);
        let existing_count = self
            .tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with(&prefix))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        // 创建 SftpView 并订阅终端打开事件
        let sftp_view = cx.new(|cx| SftpView::new_with_index(conn, tab_index, window, cx));
        let tab_container = self.tab_container.clone();

        let subscription = cx.subscribe_in(
            &sftp_view,
            window,
            move |_this, _sftp, event: &SftpViewEvent, window, cx| {
                match event {
                    SftpViewEvent::OpenLocalTerminal { working_dir } => {
                        // 使用时间戳生成唯一 tab_id，支持打开多个本地终端
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let config = LocalConfig {
                            working_dir: Some(working_dir.clone()),
                            ..Default::default()
                        };
                        let tab_id = format!("local-terminal-{}", ts);
                        // 统计已有本地终端数量
                        let existing = tab_container
                            .read(cx)
                            .tabs()
                            .iter()
                            .filter(|t| {
                                t.id().starts_with("local-terminal-")
                                    || t.id().starts_with("terminal-")
                            })
                            .count();
                        let idx = if existing > 0 {
                            Some(existing + 1)
                        } else {
                            None
                        };
                        let terminal_view =
                            cx.new(|cx| TerminalView::new_with_index(config, idx, window, cx));
                        tab_container.update(cx, |tc, cx| {
                            let tab = TabItem::new(tab_id, "terminal", terminal_view);
                            tc.add_and_activate_tab_with_focus(tab, window, cx);
                        });
                    }
                    SftpViewEvent::OpenSshTerminal {
                        connection,
                        working_dir,
                    } => {
                        // 使用时间戳生成唯一 tab_id，支持打开多个 SSH 终端
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let conn_id = connection.id.unwrap_or(0);
                        let tab_id = format!("ssh-terminal-{}-{}", conn_id, ts);
                        let conn = connection.clone();
                        // 统计同一连接的 SSH 终端数量
                        let prefix = format!("ssh-terminal-{}-", conn_id);
                        let existing = tab_container
                            .read(cx)
                            .tabs()
                            .iter()
                            .filter(|t| t.id().starts_with(&prefix))
                            .count();
                        let idx = if existing > 0 {
                            Some(existing + 1)
                        } else {
                            None
                        };
                        let sync_path = HomePage::terminal_sync_path_enabled(cx);
                        let terminal_view = cx.new(|cx| {
                            TerminalView::new_ssh_with_index(
                                conn,
                                idx,
                                window,
                                cx,
                                Some(working_dir),
                                sync_path,
                            )
                        });
                        tab_container.update(cx, |tc, cx| {
                            let tab = TabItem::new(tab_id, "ssh", terminal_view);
                            tc.add_and_activate_tab_with_focus(tab, window, cx);
                        });
                    }
                }
            },
        );
        self._subscriptions.push(subscription);

        // 添加标签页
        let tab = TabItem::new(tab_id, "sftp", sftp_view);
        self.tab_container.update(cx, |tc, cx| {
            tc.add_and_activate_tab_with_focus(tab, window, cx);
        });
    }

    pub(crate) fn add_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_container = self.tab_container.clone();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tc, cx| {
                tc.activate_or_add_tab_lazy(
                    "settings",
                    |win, cx| {
                        let settings = cx.new(|cx| SettingsPanel::new(win, cx));
                        TabItem::new("settings", "home", settings)
                    },
                    window,
                    cx,
                );
            });
        });
    }

    pub(crate) fn add_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_terminal_tab_with_config(LocalConfig::default(), window, cx);
    }

    pub(crate) fn add_terminal_tab_with_config(
        &mut self,
        config: LocalConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 使用时间戳生成唯一 tab_id，支持打开多个本地终端
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let tab_id = format!("terminal-{}", timestamp);

        // 统计已有本地终端数量，计算序号
        let existing_count = self
            .tab_container
            .read(cx)
            .tabs()
            .iter()
            .filter(|t| t.id().starts_with("terminal-") || t.id().starts_with("local-terminal-"))
            .count();
        let tab_index = if existing_count > 0 {
            Some(existing_count + 1)
        } else {
            None
        };

        let tab_container = self.tab_container.clone();
        let home = cx.entity();
        window.defer(cx, move |window, cx| {
            home.update(cx, |_this, cx| {
                let terminal_view = cx.new(|cx| {
                    TerminalView::new_with_index(config, tab_index, window, cx)
                });
                tab_container.update(cx, |tc, cx| {
                    let tab = TabItem::new(tab_id, "home", terminal_view);
                    tc.add_and_activate_tab_with_focus(tab, window, cx);
                });
            });
        });
    }

    /// 复制当前活动标签并打开
    pub(crate) fn duplicate_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tc = self.tab_container.read(cx);

        // pinned tab 不支持复制
        if tc.is_pinned_tab_active() {
            return;
        }

        let Some(active_tab) = tc.active_tab() else {
            return;
        };

        if active_tab.content().content_key(cx) == "Terminal" {
            let view = active_tab.content().view();
            if let Ok(terminal_view) = view.downcast::<TerminalView>() {
                self.duplicate_terminal_view(&terminal_view, window, cx);
            }
        }
    }

    /// 按索引复制指定标签页并打开
    pub(crate) fn duplicate_tab_by_index(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tc = self.tab_container.read(cx);

        let Some(tab) = tc.tabs().get(idx) else {
            return;
        };

        if tab.content().content_key(cx) == "Terminal" {
            let view = tab.content().view();
            if let Ok(terminal_view) = view.downcast::<TerminalView>() {
                self.duplicate_terminal_view(&terminal_view, window, cx);
            }
        }
    }

    /// 复制指定 TerminalView 并打开新标签页
    fn duplicate_terminal_view(
        &mut self,
        terminal_view: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = terminal_view.read(cx).connection_kind(cx);
        match kind {
            TerminalConnectionKind::Ssh => {
                // SSH 终端：通过 connection_id 找到 StoredConnection 并打开新连接，
                // 同时保留当前工作目录
                let conn_id = terminal_view.read(cx).connection_id(cx);
                let working_dir = terminal_view.read(cx).current_working_dir(cx);
                if let Some(conn_id) = conn_id {
                    if let Some(conn) = self
                        .connections
                        .iter()
                        .find(|c| c.id == Some(conn_id))
                        .cloned()
                    {
                        self.open_ssh_terminal(conn, None, window, cx, working_dir);
                    }
                }
            }
            TerminalConnectionKind::Local => {
                // 本地终端：继承 shell、工作目录和环境变量
                let config = terminal_view
                    .read(cx)
                    .local_config()
                    .cloned()
                    .unwrap_or_default();
                self.add_terminal_tab_with_config(config, window, cx);
            }
            TerminalConnectionKind::JmsKoko => {
                // JMS Koko 终端：token 一次性，需用原 asset/account 重新申请
                let state = terminal_view.read(cx).connection_state(cx);
                if !matches!(state, ConnectionState::Connected) {
                    window.push_notification(
                        Notification::info("仅已连接的 JMS 终端可复制").autohide(true),
                        cx,
                    );
                    return;
                }

                let Some(jms_context) = terminal_view.read(cx).jms_context().cloned() else {
                    window.push_notification(
                        Notification::info("无法复制 JMS 终端：缺少资产树上下文")
                            .autohide(true),
                        cx,
                    );
                    return;
                };

                let Some(koko_params) = terminal_view.read(cx).koko_params().cloned() else {
                    window.push_notification(
                        Notification::info("无法复制 JMS 终端：缺少连接参数")
                            .autohide(true),
                        cx,
                    );
                    return;
                };

                let Some(asset_id) = koko_params.asset_id.clone() else {
                    window.push_notification(
                        Notification::info("无法复制 JMS 终端：缺少资产信息")
                            .autohide(true),
                        cx,
                    );
                    return;
                };

                let account_name = koko_params.account_name.clone().unwrap_or_default();
                let asset_id_for_new_params = asset_id.clone();
                let account_name_for_new_params = account_name.clone();
                let client = jms_context.client.clone();
                let jms_context_clone = jms_context.clone();

                cx.spawn(async move |this, cx: &mut AsyncApp| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let mut client = client;
                            client.create_connect_token(&asset_id, &account_name).await
                        })
                        .await;

                    match result {
                        Ok(token) => {
                            let new_params = jms::KokoConnectParams {
                                token_id: token.id,
                                asset_id: Some(asset_id_for_new_params),
                                account_name: Some(account_name_for_new_params),
                                ..koko_params
                            };
                            // 复制出的标签页已经是连接状态，不能再是占位终端，
                            // 否则资产树点击资产时会错误地替换当前 tab。
                            let mut jms_context_clone = jms_context_clone;
                            jms_context_clone.is_placeholder = false;
                            // 直接在当前活动窗口打开新 Tab，避免经过 render 队列的延迟
                            let _ = cx.update(|cx| {
                                if let Some(window_id) = cx.active_window() {
                                    let _ = cx.update_window(window_id, |_, window, cx| {
                                        let _ = this.update(cx, |this, cx| {
                                            this.open_jms_koko_terminal(
                                                new_params,
                                                Some(jms_context_clone),
                                                window,
                                                cx,
                                            );
                                        });
                                    });
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!("复制 JMS Koko 终端失败: {}", e);
                            let _ = cx.update(|cx| {
                                if let Some(window_id) = cx.active_window() {
                                    let _ = cx.update_window(window_id, |_, window, cx| {
                                        window.push_notification(
                                            Notification::error(format!(
                                                "复制 JMS 终端失败：{}",
                                                e
                                            ))
                                            .autohide(true),
                                            cx,
                                        );
                                    });
                                }
                            });
                        }
                    }
                })
                .detach();
            }
        }
    }
}
