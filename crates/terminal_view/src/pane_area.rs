use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window, actions, div,
};
use gpui_component::dock::{DockAreaState, PanelState};
use gpui_component::{
    Icon, IconName,
    dock::{DockArea, DockItem, PanelView, TabPanel},
};
use one_core::tab_container::{TabContent, TabContentEvent};
use std::collections::HashMap;
use std::collections::HashSet;

use terminal::LocalConfig;
use terminal::terminal::TerminalConnectionKind;

use crate::sidebar::JmsSidebarContext;
use crate::view::{SplitPaneRequest, TerminalView, TerminalViewEvent};
use jms;
use one_core::storage::StoredConnection;

actions!(
    terminal_pane_area,
    [SplitPaneRight, SplitPaneDown, ClosePane, TogglePaneZoom,]
);

const MAX_TERMINAL_PANES: usize = 8;
const MAX_INFLIGHT_SPLITS: usize = 4;

/// 一个标签页内的终端分屏区域。
///
/// 基于 `DockArea` 实现：每个 pane 是一个独立的 `TerminalView`，
/// 支持水平/垂直分屏、拖拽调整大小、标签式堆叠等。
pub struct TerminalPaneArea {
    dock_area: Entity<DockArea>,
    focus_handle: FocusHandle,
    zoomed: bool,
    on_request_split: Option<
        Arc<dyn Fn(SplitPaneRequest, &mut Window, &mut App, &Entity<TerminalPaneArea>) + 'static>,
    >,
    terminals: HashMap<EntityId, Entity<TerminalView>>,
    active_terminal: Option<EntityId>,
    _subscriptions: HashMap<EntityId, Vec<Subscription>>,
    pending_splits: HashSet<EntityId>,
    inflight_splits: usize,
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
        let terminal = cx.new(|cx| TerminalView::new_with_index(config, tab_index, window, cx));
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
            let item = DockItem::split(gpui::Axis::Horizontal, vec![tabs], &weak_dock, window, cx);
            dock.set_center(item, window, cx);
        });

        let terminal_id = terminal.entity_id();
        let terminal_subscription = cx.subscribe_in(
            &terminal,
            window,
            |this, source, event: &TerminalViewEvent, window, cx| {
                this.handle_terminal_event(source, event, window, cx)
            },
        );
        let title_subscription = cx.subscribe(&terminal, |_, _, event: &TabContentEvent, cx| {
            if matches!(event, TabContentEvent::StateChanged) {
                cx.emit(TabContentEvent::StateChanged);
            }
        });

        let mut terminals = HashMap::new();
        terminals.insert(terminal_id, terminal);
        let mut subscriptions = HashMap::new();
        subscriptions.insert(terminal_id, vec![terminal_subscription, title_subscription]);

        Self {
            dock_area,
            focus_handle,
            zoomed: false,
            on_request_split: None,
            terminals,
            active_terminal: Some(terminal_id),
            _subscriptions: subscriptions,
            pending_splits: HashSet::new(),
            inflight_splits: 0,
            next_pane_index: 1,
        }
    }

    fn register_terminal(
        &mut self,
        terminal: Entity<TerminalView>,
        subscriptions: Vec<Subscription>,
    ) {
        let terminal_id = terminal.entity_id();
        self.terminals.insert(terminal_id, terminal);
        self.active_terminal = Some(terminal_id);
        self._subscriptions.insert(terminal_id, subscriptions);
    }

    fn remove_terminal(&mut self, terminal_id: EntityId, cx: &mut Context<Self>) {
        if self.pending_splits.remove(&terminal_id) {
            self.inflight_splits = self.inflight_splits.saturating_sub(1);
        }
        self._subscriptions.remove(&terminal_id);
        if let Some(terminal) = self.terminals.remove(&terminal_id) {
            let _ = terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
        }
        if self.active_terminal == Some(terminal_id) {
            self.active_terminal = self.terminals.keys().next().copied();
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

    /// Return every terminal currently attached to this pane area.
    pub fn collect_terminal_views(&self, _cx: &App) -> Vec<Entity<TerminalView>> {
        self.terminals.values().cloned().collect()
    }

    /// Return the last focused terminal, falling back to any live terminal.
    pub fn active_terminal_view(&self, _cx: &App) -> Option<Entity<TerminalView>> {
        self.active_terminal
            .and_then(|terminal_id| self.terminals.get(&terminal_id).cloned())
            .or_else(|| self.terminals.values().next().cloned())
    }

    fn tab_panel_for_terminal(
        &self,
        terminal: &Entity<TerminalView>,
        cx: &App,
    ) -> Option<Entity<TabPanel>> {
        terminal.read(cx).tab_panel()
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

    /// Insert an externally created terminal next to its source pane.
    pub fn split_with_terminal(
        &mut self,
        source: &Entity<TerminalView>,
        new_terminal: Entity<TerminalView>,
        placement: gpui_component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let source_id = source.entity_id();
        if self.terminals.len() >= MAX_TERMINAL_PANES {
            tracing::warn!(target: "terminal_residue", panes = self.terminals.len(), "split rejected: pane limit reached");
            let _ = new_terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
            self.pending_splits.remove(&source_id);
            self.inflight_splits = self.inflight_splits.saturating_sub(1);
            return false;
        }
        let Some(tab_panel) = self.tab_panel_for_terminal(source, cx) else {
            let _ = new_terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
            self.pending_splits.remove(&source_id);
            self.inflight_splits = self.inflight_splits.saturating_sub(1);
            return false;
        };
        let panel: Arc<dyn PanelView> = Arc::new(new_terminal.clone());
        let inserted = tab_panel.update(cx, |tab_panel, cx| {
            tab_panel.add_panel_at(panel, placement, None, window, cx)
        });
        if inserted {
            self.finish_split_insertion(new_terminal, window, cx);
        } else {
            let _ = new_terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
        }
        self.pending_splits.remove(&source_id);
        self.inflight_splits = self.inflight_splits.saturating_sub(1);
        inserted
    }

    pub fn cancel_split_request(&mut self, source_id: EntityId) {
        self.pending_splits.remove(&source_id);
        self.inflight_splits = self.inflight_splits.saturating_sub(1);
    }

    fn split_terminal(
        &mut self,
        source: Entity<TerminalView>,
        placement: gpui_component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminals.len() + self.inflight_splits >= MAX_TERMINAL_PANES {
            tracing::warn!(target: "terminal_residue", panes = self.terminals.len(), inflight = self.inflight_splits, "split rejected: pane limit reached");
            return;
        }
        let source_id = source.entity_id();
        let Some(tab_panel) = self.tab_panel_for_terminal(&source, cx) else {
            return;
        };
        let Some(new_terminal) = self.duplicate_terminal(&source, window, cx) else {
            // Only one JMS connection-token request may be active per source pane.
            if !self.pending_splits.insert(source_id) {
                return;
            }
            if self.inflight_splits >= MAX_INFLIGHT_SPLITS {
                self.pending_splits.remove(&source_id);
                tracing::warn!(target: "terminal_residue", inflight = self.inflight_splits, "split rejected: in-flight limit reached");
                return;
            }
            self.inflight_splits += 1;
            let source_ref = source.read(cx);
            let pane_index = self.next_pane_index;
            self.next_pane_index += 1;
            let request = SplitPaneRequest {
                placement,
                source,
                connection_kind: source_ref.connection_kind(cx),
                connection_id: source_ref.connection_id(cx),
                working_dir: None,
                local_config: source_ref.local_config().cloned(),
                jms_context: source_ref.jms_context().cloned(),
                koko_params: source_ref.koko_params().cloned(),
                pane_index: Some(pane_index),
            };
            if let Some(on_request_split) = self.on_request_split.clone() {
                let self_entity = cx.entity();
                on_request_split(request, window, cx, &self_entity);
            } else {
                cx.emit(TerminalViewEvent::RequestSplitPane(request));
            }
            return;
        };

        let panel: Arc<dyn PanelView> = Arc::new(new_terminal.clone());
        if tab_panel.update(cx, |tab_panel, cx| {
            tab_panel.add_panel_at(panel, placement, None, window, cx)
        }) {
            self.finish_split_insertion(new_terminal, window, cx);
        } else {
            let _ = new_terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
        }
    }

    fn close_terminal(
        &mut self,
        source: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_id = source.entity_id();
        let Some(tab_panel) = self.tab_panel_for_terminal(&source, cx) else {
            return;
        };
        self.pending_splits.remove(&source_id);
        let panel: Arc<dyn PanelView> = Arc::new(source.clone());
        let _ = source.update(cx, |view, cx| view.shutdown_terminal(cx));
        tab_panel.update(cx, |tab_panel, cx| {
            tab_panel.remove_panel(panel, window, cx)
        });
    }

    fn toggle_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn finish_split_insertion(
        &mut self,
        new_terminal: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = new_terminal.update(cx, |view, cx| view.collapse_sidebar(cx));
        self.subscribe_new_terminal(&new_terminal, window, cx);

        // Dock insertion assigns parentage and subscriptions on `window.defer`. Request the
        // terminal resize after that deferred work so its canvas sees the stable split bounds.
        window.defer(cx, move |_, cx| {
            let _ = new_terminal.update(cx, |view, cx| view.request_post_split_resize(cx));
        });
    }

    fn subscribe_new_terminal(
        &mut self,
        terminal: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let terminal_subscription = cx.subscribe_in(
            terminal,
            window,
            |this, source, event: &TerminalViewEvent, window, cx| {
                this.handle_terminal_event(source, event, window, cx)
            },
        );
        let title_subscription = cx.subscribe(terminal, |_, _, event: &TabContentEvent, cx| {
            if matches!(event, TabContentEvent::StateChanged) {
                cx.emit(TabContentEvent::StateChanged);
            }
        });
        self.register_terminal(
            terminal.clone(),
            vec![terminal_subscription, title_subscription],
        );
    }

    fn handle_terminal_event(
        &mut self,
        source: &Entity<TerminalView>,
        event: &TerminalViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalViewEvent::Closed => self.remove_terminal(source.entity_id(), cx),
            TerminalViewEvent::Focused => self.active_terminal = Some(source.entity_id()),
            TerminalViewEvent::OpenJmsTerminal(params, context) => {
                cx.emit(TerminalViewEvent::OpenJmsTerminal(
                    params.clone(),
                    context.clone(),
                ));
            }
            TerminalViewEvent::SplitPaneRight => {
                self.split_terminal(source.clone(), gpui_component::Placement::Right, window, cx);
            }
            TerminalViewEvent::SplitPaneDown => {
                self.split_terminal(
                    source.clone(),
                    gpui_component::Placement::Bottom,
                    window,
                    cx,
                );
            }
            TerminalViewEvent::ClosePane => self.close_terminal(source.clone(), window, cx),
            TerminalViewEvent::TogglePaneZoom => {
                self.active_terminal = Some(source.entity_id());
                self.toggle_zoom(window, cx);
            }
            TerminalViewEvent::RequestSplitPane(request) => {
                if let Some(on_request_split) = self.on_request_split.clone() {
                    let self_entity = cx.entity();
                    on_request_split(request.clone(), window, cx, &self_entity);
                } else {
                    cx.emit(TerminalViewEvent::RequestSplitPane(request.clone()));
                }
            }
        }
    }

    /// Split the currently active pane horizontally.
    pub fn split_active_pane_right(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(source) = self.active_terminal_view(cx) {
            self.split_terminal(source, gpui_component::Placement::Right, window, cx);
        }
    }

    /// Split the currently active pane vertically.
    pub fn split_active_pane_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(source) = self.active_terminal_view(cx) {
            self.split_terminal(source, gpui_component::Placement::Bottom, window, cx);
        }
    }

    /// Close the currently active pane.
    pub fn close_active_pane_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(source) = self.active_terminal_view(cx) {
            self.close_terminal(source, window, cx);
        }
    }

    /// Toggle zoom for the currently active pane.
    pub fn toggle_active_pane_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_zoom(window, cx);
    }

    /// 获取 DockArea 实体，供外部进行高级分屏操作。
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Restore the complete Dock layout when the serialized state is valid.
    /// Returns false for incompatible or malformed state so callers can use
    /// the pane-level compatibility fallback.
    pub fn load_dock_state(
        &mut self,
        value: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Ok(state) = serde_json::from_value::<DockAreaState>(value.clone()) else {
            return false;
        };
        if !dock_state_contains_terminal(&state.center) {
            tracing::warn!(
                target: "terminal_residue",
                "terminal dock restore rejected: layout has no terminal panels"
            );
            return false;
        }

        let restored = self.dock_area.update(cx, |dock, cx| {
            if dock.load(state, window, cx).is_err() {
                return None;
            }

            let terminals = dock
                .center_panels_named("Terminal", cx)
                .into_iter()
                .filter_map(|panel| panel.view().downcast::<TerminalView>().ok())
                .collect::<Vec<_>>();
            Some(terminals)
        });
        let Some(restored_terminals) = restored else {
            return false;
        };

        let active_terminal = self
            .dock_area
            .read(cx)
            .active_center_panel_named("Terminal", cx);
        self.replace_restored_terminals(restored_terminals, active_terminal, window, cx);
        true
    }

    fn replace_restored_terminals(
        &mut self,
        restored_terminals: Vec<Entity<TerminalView>>,
        active_terminal: Option<Arc<dyn PanelView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_terminals = std::mem::take(&mut self.terminals);
        self._subscriptions.clear();
        self.active_terminal = None;
        self.pending_splits.clear();
        self.inflight_splits = 0;

        for terminal in old_terminals.into_values() {
            let _ = terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
        }

        for terminal in restored_terminals {
            self.subscribe_new_terminal(&terminal, window, cx);
        }
        self.active_terminal = active_terminal
            .and_then(|panel| panel.view().downcast::<TerminalView>().ok())
            .map(|terminal| terminal.entity_id())
            .filter(|terminal_id| self.terminals.contains_key(terminal_id))
            .or_else(|| self.active_terminal);
        self.next_pane_index = self.terminals.len().max(1);
        cx.notify();
    }
}

fn dock_state_contains_terminal(state: &PanelState) -> bool {
    state.panel_name == "Terminal" || state.children.iter().any(dock_state_contains_terminal)
}

fn terminal_pane_states(terminals: Vec<Entity<TerminalView>>, cx: &App) -> Vec<serde_json::Value> {
    terminals
        .into_iter()
        .map(|view| {
            let view = view.read(cx);
            match view.connection_kind(cx) {
                TerminalConnectionKind::Ssh => serde_json::json!({
                    "kind": "ssh",
                    "connection_id": view.connection_id(cx),
                    "working_dir": view.current_working_dir(cx),
                }),
                TerminalConnectionKind::Local => serde_json::json!({
                    "kind": "local",
                    "config": view.local_config().cloned().unwrap_or_default(),
                }),
                _ => serde_json::json!({ "kind": "local" }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::dock::PanelInfo;

    fn panel(name: &str, children: Vec<PanelState>) -> PanelState {
        PanelState {
            panel_name: name.into(),
            children,
            info: PanelInfo::Panel(serde_json::Value::Null),
        }
    }

    #[test]
    fn dock_state_detects_terminal_at_any_depth() {
        let state = panel(
            "StackPanel",
            vec![panel("TabPanel", vec![panel("Terminal", Vec::new())])],
        );

        assert!(dock_state_contains_terminal(&state));
    }

    #[test]
    fn dock_state_rejects_layout_without_terminal() {
        let state = panel("StackPanel", vec![panel("TabPanel", Vec::new())]);

        assert!(!dock_state_contains_terminal(&state));
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        tracing::trace!(
            target: "terminal_residue",
            panes = self.terminals.len(),
            inflight_splits = self.inflight_splits,
            "terminal pane diagnostics"
        );
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

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::SquareTerminal.mono())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn dump(&self, cx: &App) -> serde_json::Value {
        // Persist durable pane configuration only. Live sessions and terminal
        // buffers are intentionally recreated during restore.
        let panes = terminal_pane_states(self.collect_terminal_views(cx), cx);

        let dock_state = serde_json::to_value(self.dock_area.read(cx).dump(cx)).ok();
        serde_json::json!({ "version": 3, "panes": panes, "dock_state": dock_state })
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
            self.shutdown_all_terminals(cx);
            return Task::ready(true);
        }

        // 有本地终端正在运行命令/TUI，弹出确认对话框
        let confirmed = crate::dialog_helper::confirm_local_terminal_close_dialog(window, cx);
        let entity = cx.entity();
        cx.spawn(async move |_, cx| {
            let confirmed = confirmed.await;
            if confirmed {
                let _ = entity.update(cx, |this, cx| this.shutdown_all_terminals(cx));
            }
            confirmed
        })
    }
}

impl TerminalPaneArea {
    /// Explicitly stop every session before the pane area is removed.
    fn shutdown_all_terminals(&self, cx: &mut App) {
        for terminal in self.collect_terminal_views(cx) {
            let _ = terminal.update(cx, |view, cx| view.shutdown_terminal(cx));
        }
    }
}
