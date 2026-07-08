use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window, actions, div,
};
use gpui_component::{
    Icon, IconName, WindowExt,
    dialog::DialogButtonProps,
    dock::{DockArea, DockItem, PanelView, TabPanel},
    v_flex,
};
use rust_i18n::t;
use one_core::tab_container::{TabContent, TabContentEvent};
use std::sync::{Arc, Mutex as StdMutex};

use terminal::LocalConfig;
use terminal::terminal::TerminalConnectionKind;

use crate::sidebar::JmsSidebarContext;
use crate::view::{SplitPaneRequest, TerminalView, TerminalViewEvent};
use jms;
use one_core::storage::StoredConnection;

actions!(
    terminal_pane_area,
    [
        SplitPaneRight,
        SplitPaneDown,
        ClosePane,
        TogglePaneZoom,
    ]
);

/// 一个标签页内的终端分屏区域。
///
/// 基于 `DockArea` 实现：每个 pane 是一个独立的 `TerminalView`，
/// 支持水平/垂直分屏、拖拽调整大小、标签式堆叠等。
pub struct TerminalPaneArea {
    dock_area: Entity<DockArea>,
    focus_handle: FocusHandle,
    zoomed: bool,
    on_request_split: Option<Arc<dyn Fn(SplitPaneRequest, &mut Window, &mut App, &Entity<TerminalPaneArea>) + 'static>>,
    _subscriptions: Vec<Subscription>,
    next_pane_index: usize,
}

impl TerminalPaneArea {
    /// 创建本地终端分屏区域，初始包含一个本地终端 pane。
    pub fn new_local(
        config: LocalConfig,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal =
            cx.new(|cx| TerminalView::new_with_index(config, tab_index, window, cx));
        Self::with_terminal(terminal, window, cx)
    }

    /// 创建 SSH 终端分屏区域，初始包含一个 SSH 终端 pane。
    pub fn new_ssh(
        conn: StoredConnection,
        tab_index: Option<usize>,
        working_dir: Option<String>,
        sync_path: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(|cx| {
            TerminalView::new_ssh_with_index(
                conn,
                tab_index,
                window,
                cx,
                working_dir.as_deref(),
                sync_path,
            )
        });
        Self::with_terminal(terminal, window, cx)
    }

    /// 创建 JumpServer Koko 终端分屏区域。
    pub fn new_jms_koko(
        params: jms::KokoConnectParams,
        jms_context: Option<JmsSidebarContext>,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(|cx| {
            TerminalView::new_jms_koko_with_context(params, jms_context, tab_index, window, cx)
        });
        Self::with_terminal(terminal, window, cx)
    }

    /// 创建 JMS Koko 占位终端分屏区域。
    pub fn new_jms_koko_placeholder(
        jms_context: Option<JmsSidebarContext>,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal =
            cx.new(|cx| TerminalView::new_jms_koko_placeholder(jms_context, tab_index, window, cx));
        Self::with_terminal(terminal, window, cx)
    }

    fn with_terminal(
        terminal: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let dock_area = cx.new(|cx| DockArea::new("terminal-pane-area", None, window, cx));
        let weak_dock = dock_area.downgrade();
        dock_area.update(cx, |dock, cx| {
            // 必须用 Split 包裹 Tabs，这样 Tabs 内的 TabPanel 才有 StackPanel 父节点，
            // 后续 TabPanel::add_panel_at 才能正常分屏。
            let tabs = DockItem::tabs(vec![Arc::new(terminal.clone())], &weak_dock, window, cx);
            let item = DockItem::split(
                gpui::Axis::Horizontal,
                vec![tabs],
                &weak_dock,
                window,
                cx,
            );
            dock.set_center(item, window, cx);
        });

        // 把子终端的分屏/打开新 JMS 终端等事件进行处理或向上冒泡
        let jms_subscription = cx.subscribe_in(
            &terminal,
            window,
            |this, _, event: &TerminalViewEvent, window, cx| {
                match event {
                    TerminalViewEvent::OpenJmsTerminal(params, ctx) => {
                        cx.emit(TerminalViewEvent::OpenJmsTerminal(params.clone(), ctx.clone()));
                    }
                    TerminalViewEvent::SplitPaneRight => {
                        this.split_active_pane(gpui_component::Placement::Right, window, cx);
                    }
                    TerminalViewEvent::SplitPaneDown => {
                        this.split_active_pane(gpui_component::Placement::Bottom, window, cx);
                    }
                    TerminalViewEvent::ClosePane => {
                        this.close_active_pane(window, cx);
                    }
                    TerminalViewEvent::TogglePaneZoom => {
                        this.toggle_zoom(window, cx);
                    }
                    TerminalViewEvent::RequestSplitPane(request) => {
                        if let Some(on_request_split) = this.on_request_split.clone() {
                            let self_entity = cx.entity();
                            on_request_split(request.clone(), window, cx, &self_entity);
                        } else {
                            cx.emit(TerminalViewEvent::RequestSplitPane(request.clone()));
                        }
                    }
                }
            },
        );

        // 子终端标题变化时，让外层标签页标题也刷新
        let title_subscription = cx.subscribe(&terminal, |_, _, event: &TabContentEvent, cx| {
            if matches!(event, TabContentEvent::StateChanged) {
                cx.emit(TabContentEvent::StateChanged);
            }
        });

        Self {
            dock_area,
            focus_handle,
            zoomed: false,
            on_request_split: None,
            _subscriptions: vec![jms_subscription, title_subscription],
            next_pane_index: 1,
        }
    }

    /// 设置 SSH/JMS 分屏请求的外部处理回调。
    /// 回调接收请求和当前的 App 上下文，可通过 App 获取窗口和更新实体。
    pub fn on_request_split(
        mut self,
        f: impl Fn(SplitPaneRequest, &mut Window, &mut App, &Entity<TerminalPaneArea>) + 'static,
    ) -> Self {
        self.on_request_split = Some(Arc::new(f));
        self
    }

    /// 返回当前分屏区域内的所有 TerminalView。
    pub fn collect_terminal_views(&self, cx: &App) -> Vec<Entity<TerminalView>> {
        let mut views = Vec::new();
        self.collect_in_item(self.dock_area.read(cx).center(), &mut views, cx);
        views
    }

    fn collect_in_item(
        &self,
        item: &DockItem,
        views: &mut Vec<Entity<TerminalView>>,
        _cx: &App,
    ) {
        match item {
            DockItem::Panel { view, .. } => {
                if let Ok(view) = view.view().downcast::<TerminalView>() {
                    views.push(view);
                }
            }
            DockItem::Tabs { items, .. } => {
                for panel in items {
                    if let Ok(view) = panel.view().downcast::<TerminalView>() {
                        views.push(view);
                    }
                }
            }
            DockItem::Split { items, .. } => {
                for child in items {
                    self.collect_in_item(child, views, _cx);
                }
            }
            DockItem::Tiles { .. } => {
                // Tiles 中 panel 字段私有，且当前终端 pane 不使用 Tiles 布局
            }
        }
    }

    /// 找到当前布局中活跃的 TerminalView（用于标题显示、分屏/关闭操作）。
    pub fn active_terminal_view(&self, cx: &App) -> Option<Entity<TerminalView>> {
        self.active_panel_in_item(self.dock_area.read(cx).center(), cx)
            .and_then(|panel| panel.view().downcast::<TerminalView>().ok())
    }

    fn active_panel_in_item(
        &self,
        item: &DockItem,
        cx: &App,
    ) -> Option<Arc<dyn PanelView>> {
        match item {
            DockItem::Panel { view, .. } => Some(view.clone()),
            DockItem::Tabs { items, active_ix, .. } => items
                .get(*active_ix)
                .cloned()
                .or_else(|| items.iter().find(|p| p.visible(cx)).cloned()),
            DockItem::Split { items, .. } => {
                items.iter().find_map(|child| self.active_panel_in_item(child, cx))
            }
            DockItem::Tiles { .. } => None,
        }
    }

    /// 获取包含指定 terminal 的 TabPanel，用于分屏/关闭操作。
    fn tab_panel_for_terminal(
        &self,
        terminal: &Entity<TerminalView>,
        cx: &App,
    ) -> Option<Entity<TabPanel>> {
        self.tab_panel_in_item(self.dock_area.read(cx).center(), terminal, cx)
    }

    fn tab_panel_in_item(
        &self,
        item: &DockItem,
        terminal: &Entity<TerminalView>,
        _cx: &App,
    ) -> Option<Entity<TabPanel>> {
        match item {
            DockItem::Tabs { view, items, .. } => {
                if items.iter().any(|p| p.view().entity_id() == terminal.entity_id()) {
                    Some(view.clone())
                } else {
                    None
                }
            }
            DockItem::Split { items, .. } => items
                .iter()
                .find_map(|child| self.tab_panel_in_item(child, terminal, _cx)),
            _ => None,
        }
    }

    /// 基于现有 pane 克隆出一个新的 TerminalView。
    ///
    /// 当前仅支持本地终端；SSH/JMS 需要外部连接信息，将在后续迭代中支持。
    fn duplicate_terminal(
        &mut self,
        source: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<TerminalView>> {
        let source_ref = source.read(cx);
        let kind = source_ref.connection_kind(cx);
        let local_config = source_ref.local_config().cloned();

        match kind {
            TerminalConnectionKind::Local => {
                let config = local_config.unwrap_or_default();
                let index = self.next_pane_index;
                self.next_pane_index += 1;
                let terminal = cx.new(|cx| TerminalView::new_with_index(config, None, window, cx));
                terminal.update(cx, |t, _cx| t.set_pane_index(index));
                Some(terminal)
            }
            _ => None,
        }
    }

    /// 公开：把已创建好的 TerminalView 插入到当前激活 pane 的指定方向。
    pub fn split_with_terminal(
        &mut self,
        source: &Entity<TerminalView>,
        new_terminal: Entity<TerminalView>,
        placement: gpui_component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tracing::info!("TerminalPaneArea: split_with_terminal placement={:?}", placement);
        let Some(tab_panel) = self.tab_panel_for_terminal(source, cx) else {
            tracing::warn!("TerminalPaneArea: source terminal not in any TabPanel");
            return;
        };
        tracing::info!("TerminalPaneArea: split_with_terminal found TabPanel");

        self.subscribe_new_terminal(&new_terminal, window, cx);

        let new_panel: Arc<dyn PanelView> = Arc::new(new_terminal);
        tab_panel.update(cx, |tp, cx| {
            tp.add_panel_at(new_panel, placement, None, window, cx);
        });
    }

    fn split_active_pane(
        &mut self,
        placement: gpui_component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tracing::info!("TerminalPaneArea: split_active_pane placement={:?}", placement);
        let Some(source) = self.active_terminal_view(cx) else {
            tracing::warn!("TerminalPaneArea: no active terminal to split");
            return;
        };
        let Some(tab_panel) = self.tab_panel_for_terminal(&source, cx) else {
            return;
        };
        let Some(new_terminal) = self.duplicate_terminal(&source, window, cx) else {
            // SSH/JMS 需要 HomePage 代为创建，发出请求事件
            let source_ref = source.read(cx);
            let pane_index = self.next_pane_index;
            self.next_pane_index += 1;
            let request = SplitPaneRequest {
                placement,
                source: source.clone(),
                connection_kind: source_ref.connection_kind(cx),
                connection_id: source_ref.connection_id(cx),
                working_dir: None,
                local_config: source_ref.local_config().cloned(),
                jms_context: source_ref.jms_context().cloned(),
                koko_params: source_ref.koko_params().cloned(),
                pane_index: Some(pane_index),
            };
            tracing::info!("TerminalPaneArea: emit RequestSplitPane for {:?}", request.connection_kind);
            if let Some(on_request_split) = self.on_request_split.clone() {
                tracing::info!("TerminalPaneArea: invoking on_request_split callback");
                let self_entity = cx.entity();
                on_request_split(request, window, cx, &self_entity);
                tracing::info!("TerminalPaneArea: on_request_split callback returned");
            } else {
                tracing::info!("TerminalPaneArea: no callback, emitting event");
                cx.emit(TerminalViewEvent::RequestSplitPane(request));
            }
            return;
        };

        // 订阅新 terminal 的事件
        self.subscribe_new_terminal(&new_terminal, window, cx);

        let new_panel: Arc<dyn PanelView> = Arc::new(new_terminal);
        tab_panel.update(cx, |tp, cx| {
            tp.add_panel_at(new_panel, placement, None, window, cx);
        });
    }

    fn close_active_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.active_terminal_view(cx) else {
            return;
        };
        let Some(tab_panel) = self.tab_panel_for_terminal(&source, cx) else {
            return;
        };
        let panel: Arc<dyn PanelView> = Arc::new(source.clone());
        tab_panel.update(cx, |tp, cx| {
            tp.remove_panel(panel, window, cx);
        });
    }

    fn toggle_zoom(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.zoomed {
            self.dock_area.update(cx, |dock, cx| {
                dock.set_zoomed_out(window, cx);
            });
            self.zoomed = false;
        } else if let Some(source) = self.active_terminal_view(cx) {
            self.dock_area.update(cx, |dock, cx| {
                dock.set_zoomed_in(source, window, cx);
            });
            self.zoomed = true;
        }
    }

    fn subscribe_new_terminal(
        &mut self,
        terminal: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let jms_subscription = cx.subscribe_in(
            terminal,
            window,
            |this, _, event: &TerminalViewEvent, window, cx| {
                match event {
                    TerminalViewEvent::OpenJmsTerminal(params, ctx) => {
                        cx.emit(TerminalViewEvent::OpenJmsTerminal(params.clone(), ctx.clone()));
                    }
                    TerminalViewEvent::SplitPaneRight => {
                        this.split_active_pane(gpui_component::Placement::Right, window, cx);
                    }
                    TerminalViewEvent::SplitPaneDown => {
                        this.split_active_pane(gpui_component::Placement::Bottom, window, cx);
                    }
                    TerminalViewEvent::ClosePane => {
                        this.close_active_pane(window, cx);
                    }
                    TerminalViewEvent::TogglePaneZoom => {
                        this.toggle_zoom(window, cx);
                    }
                    TerminalViewEvent::RequestSplitPane(request) => {
                        if let Some(on_request_split) = this.on_request_split.clone() {
                            let self_entity = cx.entity();
                            on_request_split(request.clone(), window, cx, &self_entity);
                        } else {
                            cx.emit(TerminalViewEvent::RequestSplitPane(request.clone()));
                        }
                    }
                }
            },
        );
        let title_subscription = cx.subscribe(terminal, |_, _, event: &TabContentEvent, cx| {
            if matches!(event, TabContentEvent::StateChanged) {
                cx.emit(TabContentEvent::StateChanged);
            }
        });
        self._subscriptions.push(jms_subscription);
        self._subscriptions.push(title_subscription);
    }

    /// 公开：将当前焦点 pane 水平分屏（pane 在右）。
    pub fn split_active_pane_right(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_active_pane(gpui_component::Placement::Right, window, cx);
    }

    /// 公开：将当前焦点 pane 垂直分屏（pane 在下）。
    pub fn split_active_pane_down(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_active_pane(gpui_component::Placement::Bottom, window, cx);
    }

    /// 公开：关闭当前焦点 pane。
    pub fn close_active_pane_public(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_active_pane(window, cx);
    }

    /// 公开：放大/还原当前焦点 pane。
    pub fn toggle_active_pane_zoom(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_zoom(window, cx);
    }

    /// 获取 DockArea 实体，供外部进行高级分屏操作。
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }
}

impl Focusable for TerminalPaneArea {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for TerminalPaneArea {}
impl EventEmitter<TerminalViewEvent> for TerminalPaneArea {}

impl Render for TerminalPaneArea {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().size_full().child(self.dock_area.clone())
    }
}

impl TabContent for TerminalPaneArea {
    fn content_key(&self) -> &'static str {
        "TerminalPaneArea"
    }

    fn title(&self, cx: &App) -> SharedString {
        self.active_terminal_view(cx)
            .map(|view| view.read(cx).title(cx))
            .unwrap_or_else(|| "Terminal".into())
    }

    fn icon(
        &self, _cx: &App
    ) -> Option<Icon> {
        Some(IconName::SquareTerminal.mono())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let needs_confirm = self
            .collect_terminal_views(cx)
            .iter()
            .any(|view| view.read(cx).should_confirm_close(cx));

        if !needs_confirm {
            return Task::ready(true);
        }

        // 有本地终端正在运行命令/TUI，弹出确认对话框
        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        let tx = Arc::new(StdMutex::new(Some(tx)));
        let tx_ok = tx.clone();
        let tx_cancel = tx;

        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(t!("LocalTerminalClose.title").to_string())
                .w(gpui::px(420.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("LocalTerminalClose.message").to_string())
                        .child(t!("LocalTerminalClose.warning").to_string()),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.close").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok({
                    let tx_ok = tx_ok.clone();
                    move |_, _, _| {
                        if let Ok(mut guard) = tx_ok.lock() {
                            if let Some(sender) = guard.take() {
                                let _ = sender.send(true);
                            }
                        }
                        true
                    }
                })
                .on_cancel({
                    let tx_cancel = tx_cancel.clone();
                    move |_, _, _| {
                        if let Ok(mut guard) = tx_cancel.lock() {
                            if let Some(sender) = guard.take() {
                                let _ = sender.send(false);
                            }
                        }
                        true
                    }
                })
                .overlay_closable(false)
                .close_button(false)
        });

        cx.spawn(async move |_, _| rx.await.unwrap_or(false))
    }
}
