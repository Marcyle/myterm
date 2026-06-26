//! 终端侧边栏模块
//!
//! 提供终端视图的侧边栏功能，包括：
//! - 设置面板（搜索、字体、主题）
//! - 快捷命令面板
//! - 文件管理器面板（仅 SSH 终端）

pub mod file_manager_panel;
pub mod jms_asset_tree_panel;
mod quick_command_panel;
mod settings_panel;

pub use file_manager_panel::{FileManagerPanel, FileManagerPanelEvent};
pub use jms_asset_tree_panel::{JmsAssetTreePanel, JmsAssetTreePanelEvent};
pub use quick_command_panel::QuickCommandPanel;
pub use settings_panel::SettingsPanel;

use crate::{
    TerminalHighlightRule,
    theme::{TerminalColors, TerminalTheme},
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, v_flex};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::storage::models::StoredConnection;
use ssh::SshSessionManager;
use std::sync::Arc;
use terminal::terminal::SshTerminalConfig;

/// JMS 资产树侧栏所需的上下文
///
/// 由 JMS 登录窗口在登录成功后构造,携带已认证的 [`jms::JmsClient`] 克隆、
/// 已加载的资产树根节点,以及预先算好的代理配置,传递给终端侧栏的资产树面板。
#[derive(Clone)]
pub struct JmsSidebarContext {
    /// 已认证的 JMS 客户端(克隆持有)
    pub client: jms::JmsClient,
    /// 已加载的资产树根节点
    pub tree_roots: Vec<jms::JmsAssetTreeNode>,
    /// 预先从全局设置算好的代理配置
    pub proxy: Option<jms::KokoProxy>,
    /// 所在终端是否为占位终端(占位时点资产→本 tab 连接;否则→新开 tab)
    pub is_placeholder: bool,
}

impl std::fmt::Debug for JmsSidebarContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmsSidebarContext")
            .field("tree_roots_len", &self.tree_roots.len())
            .field("has_proxy", &self.proxy.is_some())
            .finish()
    }
}

/// 侧边栏面板类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    /// 设置面板（搜索 + 字体 + 主题）
    Settings,
    /// 快捷命令面板
    QuickCommand,
    /// 文件管理器面板（仅 SSH 终端）
    FileManager,
    /// JMS 资产树面板（仅 JMS 终端）
    JmsAssetTree,
}

impl SidebarPanel {
    /// 获取面板图标
    pub fn icon(&self) -> Icon {
        match self {
            SidebarPanel::Settings => IconName::Settings.mono(),
            SidebarPanel::QuickCommand => IconName::SquareTerminal.mono(),
            SidebarPanel::FileManager => IconName::FolderOpen.mono(),
            SidebarPanel::JmsAssetTree => IconName::Server.mono(),
        }
    }

    /// 获取面板标题
    pub fn title(&self) -> &'static str {
        match self {
            SidebarPanel::Settings => "Settings",
            SidebarPanel::QuickCommand => "Quick Commands",
            SidebarPanel::FileManager => "File Manager",
            SidebarPanel::JmsAssetTree => "JMS Assets",
        }
    }
}

/// 终端侧边栏事件
#[derive(Clone, Debug)]
pub enum TerminalSidebarEvent {
    /// 面板切换
    PanelChanged(Option<SidebarPanel>),
    /// 搜索模式变化
    SearchPatternChanged(String),
    /// 搜索前一个
    SearchPrevious,
    /// 搜索下一个
    SearchNext,
    /// 字体大小变更
    FontSizeChanged(f32),
    /// 字体变更
    FontFamilyChanged(String),
    /// 主题变更
    ThemeChanged(TerminalTheme),
    /// 粘贴命令到终端输入区（不自动回车）
    ExecuteCommand(String),
    /// 光标闪烁变更
    CursorBlinkChanged(bool),
    /// 非 bracketed 模式下，多行粘贴确认开关
    ConfirmMultilinePasteChanged(bool),
    /// 高危命令确认开关
    ConfirmHighRiskCommandChanged(bool),
    /// 选中自动复制开关
    AutoCopyChanged(bool),
    /// 自动补全开关
    AutocompleteChanged(bool),
    /// 中键粘贴开关
    MiddleClickPasteChanged(bool),
    /// vim/TUI 滚轮转方向键开关
    VimScrollToArrowKeysChanged(bool),
    /// 路径与终端同步开关
    SyncPathChanged(bool),
    /// 自定义高亮规则变更
    CustomHighlightsChanged(Vec<TerminalHighlightRule>),
    /// 在终端中 cd 到指定路径
    CdToTerminal(String),
    /// 请求将终端当前工作目录同步到文件管理器
    SyncWorkingDir,
    /// 请求打开新的 JMS 终端(从资产树面板冒泡)
    OpenJmsTerminal(jms::KokoConnectParams),
    /// 在当前(占位)终端 tab 上直接连接 JMS 资产
    ConnectJmsInCurrentTab(jms::KokoConnectParams),
}

/// 终端侧边栏组件
pub struct TerminalSidebar {
    /// 当前激活的面板
    active_panel: Option<SidebarPanel>,
    /// 是否折叠（完全隐藏侧边栏）
    collapsed: bool,
    /// 设置面板
    settings_panel: Entity<SettingsPanel>,
    /// 快捷命令面板
    quick_command_panel: Entity<QuickCommandPanel>,
    /// 文件管理器面板（仅 SSH 终端时创建）
    file_manager_panel: Option<Entity<FileManagerPanel>>,
    /// JMS 资产树面板（仅 JMS 终端时创建）
    jms_asset_tree_panel: Option<Entity<JmsAssetTreePanel>>,
    /// 路径与终端同步开关（默认开启）
    sync_path_enabled: bool,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 终端主题配色（用于侧边栏工具栏）
    colors: TerminalColors,
    /// 订阅句柄
    _subs: Vec<Subscription>,
}

impl TerminalSidebar {
    pub fn new(
        connection_id: Option<i64>,
        stored_connection: Option<StoredConnection>,
        _ssh_config: Option<SshTerminalConfig>,
        ssh_session_manager: Option<Arc<SshSessionManager>>,
        jms_context: Option<JmsSidebarContext>,
        initial_theme: &TerminalTheme,
        initial_font_size: Pixels,
        initial_font_family: SharedString,
        sync_path_enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let colors = initial_theme.colors();
        let has_file_manager = stored_connection.is_some();
        let settings_panel = cx.new(|cx| {
            SettingsPanel::new(
                initial_theme,
                initial_font_size,
                initial_font_family,
                has_file_manager,
                true,
                true,
                true,
                sync_path_enabled,
                true,
                window,
                cx,
            )
        });
        let quick_command_panel = cx.new(|cx| QuickCommandPanel::new(connection_id, window, cx));

        // 仅 SSH 终端（有 StoredConnection）时创建文件管理器面板
        let file_manager_panel = stored_connection
            .zip(ssh_session_manager.clone())
            .map(|(conn, manager)| cx.new(|cx| FileManagerPanel::new(conn, manager, window, cx)));

        // 订阅设置面板事件
        let set_sub = cx.subscribe(
            &settings_panel,
            |this, _, event: &settings_panel::SettingsPanelEvent, cx| match event {
                settings_panel::SettingsPanelEvent::Close => {
                    this.set_active_panel(None, cx);
                }
                settings_panel::SettingsPanelEvent::SearchPatternChanged(pattern) => {
                    cx.emit(TerminalSidebarEvent::SearchPatternChanged(pattern.clone()));
                }
                settings_panel::SettingsPanelEvent::SearchPrevious => {
                    cx.emit(TerminalSidebarEvent::SearchPrevious);
                }
                settings_panel::SettingsPanelEvent::SearchNext => {
                    cx.emit(TerminalSidebarEvent::SearchNext);
                }
                settings_panel::SettingsPanelEvent::FontSizeChanged(size) => {
                    cx.emit(TerminalSidebarEvent::FontSizeChanged(*size));
                }
                settings_panel::SettingsPanelEvent::FontFamilyChanged(family) => {
                    cx.emit(TerminalSidebarEvent::FontFamilyChanged(family.clone()));
                }
                settings_panel::SettingsPanelEvent::ThemeChanged(theme) => {
                    this.colors = theme.colors();
                    cx.emit(TerminalSidebarEvent::ThemeChanged(theme.clone()));
                }
                settings_panel::SettingsPanelEvent::CursorBlinkChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::CursorBlinkChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::ConfirmMultilinePasteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::ConfirmMultilinePasteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::ConfirmHighRiskCommandChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::ConfirmHighRiskCommandChanged(
                        *enabled,
                    ));
                }
                settings_panel::SettingsPanelEvent::AutoCopyChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::AutoCopyChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::AutocompleteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::AutocompleteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::MiddleClickPasteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::MiddleClickPasteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::VimScrollToArrowKeysChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::VimScrollToArrowKeysChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::SyncPathChanged(enabled) => {
                    this.sync_path_enabled = *enabled;
                    cx.emit(TerminalSidebarEvent::SyncPathChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::CustomHighlightsChanged(rules) => {
                    cx.emit(TerminalSidebarEvent::CustomHighlightsChanged(rules.clone()));
                }
            },
        );

        // 订阅快捷命令面板事件
        let quick_sub = cx.subscribe(
            &quick_command_panel,
            |this, _, event: &quick_command_panel::QuickCommandPanelEvent, cx| match event {
                quick_command_panel::QuickCommandPanelEvent::Close => {
                    this.set_active_panel(None, cx);
                }
                quick_command_panel::QuickCommandPanelEvent::ExecuteCommand(cmd) => {
                    cx.emit(TerminalSidebarEvent::ExecuteCommand(cmd.clone()));
                }
            },
        );

        let mut subs = vec![set_sub, quick_sub];

        // 订阅文件管理器面板事件
        if let Some(ref fm_panel) = file_manager_panel {
            let fm_sub =
                cx.subscribe(
                    fm_panel,
                    |this, _, event: &FileManagerPanelEvent, cx| match event {
                        FileManagerPanelEvent::Close => {
                            this.set_active_panel(None, cx);
                        }
                        FileManagerPanelEvent::CdToTerminal(path) => {
                            cx.emit(TerminalSidebarEvent::CdToTerminal(path.clone()));
                        }
                        FileManagerPanelEvent::SyncWorkingDir => {
                            cx.emit(TerminalSidebarEvent::SyncWorkingDir);
                        }
                    },
                );
            subs.push(fm_sub);
        }

        // 仅 JMS 终端时创建资产树面板
        let jms_asset_tree_panel = jms_context.map(|ctx| {
            cx.new(|cx| {
                JmsAssetTreePanel::new(
                    ctx.client,
                    ctx.tree_roots,
                    ctx.proxy,
                    ctx.is_placeholder,
                    window,
                    cx,
                )
            })
        });

        // 订阅资产树面板事件
        if let Some(ref jms_panel) = jms_asset_tree_panel {
            let jms_sub = cx.subscribe(
                jms_panel,
                |this, _, event: &JmsAssetTreePanelEvent, cx| match event {
                    JmsAssetTreePanelEvent::Close => {
                        this.set_active_panel(None, cx);
                    }
                    JmsAssetTreePanelEvent::OpenNewTerminal(params) => {
                        cx.emit(TerminalSidebarEvent::OpenJmsTerminal(params.clone()));
                    }
                    JmsAssetTreePanelEvent::ConnectInCurrentTab(params) => {
                        cx.emit(TerminalSidebarEvent::ConnectJmsInCurrentTab(params.clone()));
                    }
                },
            );
            subs.push(jms_sub);
        }

        // JMS 终端默认展开资产树侧栏(Web Terminal 体验)
        let has_jms = jms_asset_tree_panel.is_some();
        Self {
            active_panel: if has_jms {
                Some(SidebarPanel::JmsAssetTree)
            } else {
                None
            },
            collapsed: !has_jms,
            settings_panel,
            quick_command_panel,
            file_manager_panel,
            jms_asset_tree_panel,
            sync_path_enabled,
            focus_handle: cx.focus_handle(),
            colors,
            _subs: subs,
        }
    }

    /// 获取当前激活的面板
    pub fn active_panel(&self) -> Option<SidebarPanel> {
        self.active_panel
    }

    /// 设置激活的面板
    pub fn set_active_panel(&mut self, panel: Option<SidebarPanel>, cx: &mut Context<Self>) {
        if self.active_panel != panel {
            self.active_panel = panel;
            cx.emit(TerminalSidebarEvent::PanelChanged(panel));
            cx.notify();
        }
    }

    /// 切换面板
    pub fn toggle_panel(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.active_panel == Some(panel) {
            self.set_active_panel(None, cx);
        } else {
            // 文件管理器首次激活时自动建立连接
            if panel == SidebarPanel::FileManager {
                if let Some(ref fm_panel) = self.file_manager_panel {
                    fm_panel.update(cx, |panel, cx| {
                        // 仅在 Idle 状态时自动连接
                        panel.connect_if_idle(cx);
                    });
                }
            }
            self.set_active_panel(Some(panel), cx);
        }
    }

    /// 是否显示侧边栏
    pub fn is_visible(&self) -> bool {
        !self.collapsed && self.active_panel.is_some()
    }

    /// 是否折叠
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// 切换折叠状态
    pub fn toggle_collapsed(&mut self, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }

    /// 展开侧边栏
    pub fn expand(&mut self, cx: &mut Context<Self>) {
        self.collapsed = false;
        cx.notify();
    }

    /// 更新设置面板的当前主题
    pub fn update_current_theme(
        &mut self,
        theme: &TerminalTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colors = theme.colors();
        // 更新设置面板（会同时更新颜色和主题）
        let theme_clone = theme.clone();
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_current_theme(theme_clone, window, cx);
        });

        cx.notify();
    }

    pub fn set_font_size(&mut self, font_size: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_font_size(font_size, window, cx);
        });
    }

    pub fn set_auto_copy(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_auto_copy(enabled, cx);
        });
    }

    pub fn set_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_autocomplete_enabled(enabled, cx);
        });
    }

    pub fn set_middle_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_middle_click_paste(enabled, cx);
        });
    }

    pub fn set_vim_scroll_to_arrow_keys(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_vim_scroll_to_arrow_keys(enabled, cx);
        });
    }

    pub fn set_sync_path_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.sync_path_enabled = enabled;
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_sync_path(enabled, cx);
        });
    }

    pub fn set_cursor_blink(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_cursor_blink(enabled, cx);
        });
    }

    pub fn set_confirm_multiline_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_confirm_multiline_paste(enabled, cx);
        });
    }

    pub fn set_confirm_high_risk_command(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_confirm_high_risk_command(enabled, cx);
        });
    }

    pub fn set_custom_highlights(
        &mut self,
        rules: Vec<TerminalHighlightRule>,
        cx: &mut Context<Self>,
    ) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_custom_highlights(rules, cx);
        });
    }

    /// 更新搜索输入框的值
    pub fn set_search_value(&self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_search_value(value, window, cx);
        });
    }

    /// 获取搜索输入框的值
    pub fn search_value(&self, cx: &App) -> String {
        self.settings_panel.read(cx).search_value(cx)
    }

    /// 添加快捷命令（外部调用）
    pub fn add_quick_command(&self, command: String, cx: &mut Context<Self>) {
        self.quick_command_panel.update(cx, |panel, cx| {
            panel.add_command_external(command, cx);
        });
    }

    /// 从终端 OSC 7 同步路径到文件管理器
    ///
    /// 检查 `sync_path_enabled` 且存在文件管理器面板时，导航到指定路径。
    pub fn sync_file_manager_path(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.sync_path_enabled {
            return;
        }
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, cx| {
                panel.sync_navigate_to(path, cx);
            });
        }
    }

    /// 设置文件管理器的初始工作目录（连接前调用）
    ///
    /// 当终端收到 OSC 7 但文件管理器尚未连接时，缓存路径供首次连接使用。
    pub fn set_file_manager_initial_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, _cx| {
                panel.set_initial_working_dir(path);
            });
        }
    }

    /// 在终端重连时同步重建文件管理器连接
    pub fn reconnect_file_manager(&mut self, working_dir: Option<String>, cx: &mut Context<Self>) {
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, cx| {
                panel.reconnect_with_working_dir(working_dir.clone(), cx);
            });
        }
    }

    /// 渲染工具栏按钮
    fn render_toolbar_button(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_panel == Some(panel);
        let accent_color = self.colors.accent;
        let accent_fg = self.colors.accent_foreground;
        let muted_fg = self.colors.muted_foreground;
        let muted_bg = self.colors.muted;

        div()
            .id(SharedString::from(format!("toolbar-btn-{:?}", panel)))
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .when(is_active, |this| this.bg(accent_color))
            .when(!is_active, |this| this.hover(|s| s.bg(muted_bg)))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_panel(panel, cx);
            }))
            .child(
                Icon::new(panel.icon())
                    .with_size(Size::Medium)
                    .text_color(if is_active { accent_fg } else { muted_fg }),
            )
    }

    /// 渲染工具栏
    pub fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let border_color = self.colors.border;
        let muted_bg = self.colors.background;
        let has_file_manager = self.file_manager_panel.is_some();
        let has_jms_asset_tree = self.jms_asset_tree_panel.is_some();

        v_flex()
            .flex_shrink_0()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .bg(muted_bg)
            .border_l_1()
            .border_color(border_color)
            .items_center()
            .py_2()
            .gap_1()
            // 折叠按钮放在最上面
            .child(
                div()
                    .id("sidebar-collapse-btn")
                    .w(px(36.0))
                    .h(px(36.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(self.colors.muted))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.toggle_collapsed(cx);
                    }))
                    .child(
                        Icon::new(IconName::PanelLeftClose)
                            .with_size(Size::Medium)
                            .text_color(gpui::white()),
                    ),
            )
            .child(self.render_toolbar_button(SidebarPanel::Settings, window, cx))
            .child(self.render_toolbar_button(SidebarPanel::QuickCommand, window, cx))
            .when(has_file_manager, |this| {
                this.child(self.render_toolbar_button(SidebarPanel::FileManager, window, cx))
            })
            .when(has_jms_asset_tree, |this| {
                this.child(self.render_toolbar_button(SidebarPanel::JmsAssetTree, window, cx))
            })
            .into_any_element()
    }

    /// 渲染面板内容
    pub fn render_panel_content(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        match panel {
            SidebarPanel::Settings => self.settings_panel.clone().into_any_element(),
            SidebarPanel::QuickCommand => self.quick_command_panel.clone().into_any_element(),
            SidebarPanel::FileManager => {
                if let Some(ref fm_panel) = self.file_manager_panel {
                    fm_panel.clone().into_any_element()
                } else {
                    div().into_any_element()
                }
            }
            SidebarPanel::JmsAssetTree => {
                if let Some(ref jms_panel) = self.jms_asset_tree_panel {
                    jms_panel.clone().into_any_element()
                } else {
                    div().into_any_element()
                }
            }
        }
    }
}

impl EventEmitter<TerminalSidebarEvent> for TerminalSidebar {}

impl Focusable for TerminalSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;
        let bg_color = cx.theme().background;

        if self.collapsed {
            // 折叠状态下不渲染任何内容（完全隐藏）
            div().into_any_element()
        } else if let Some(panel) = self.active_panel {
            div()
                .h_full()
                .flex_shrink_0()
                .w_full()
                .child(
                    v_flex()
                        .size_full()
                        .border_l_1()
                        .border_color(border_color)
                        .bg(bg_color)
                        .child(self.render_panel_content(panel, window, cx)),
                )
                .into_any_element()
        } else {
            div()
                .h_full()
                .flex_shrink_0()
                .child(self.render_toolbar(window, cx))
                .into_any_element()
        }
    }
}
