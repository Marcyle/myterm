use crate::home_tab::{HomePage, NewConnectionShortcut, OpenConnectionQuickOpen};
use gpui::{
    App, AppContext, Context, Entity, IntoElement, KeyBinding, ParentElement, Render, Styled,
    Window, actions, div,
};
use gpui_component::WindowExt;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

actions!(
    myterm_app,
    [
        ActivateTab1,
        ActivateTab2,
        ActivateTab3,
        ActivateTab4,
        ActivateTab5,
        ActivateTab6,
        ActivateTab7,
        ActivateTab8,
        ActivateTab9,
        ToggleFullscreen,
        DuplicateTab,
        QuitApp,
    ]
);

#[derive(Clone)]
pub struct GlobalTabContainer {
    pub tab_container: Entity<TabContainer>,
}

impl gpui::Global for GlobalTabContainer {}

#[derive(Clone)]
pub struct GlobalHomePage {
    pub home_page: Entity<HomePage>,
}

impl gpui::Global for GlobalHomePage {}

#[cfg(target_os = "macos")]
use gpui::px;

use gpui_component::dock::{ClosePanel, ToggleZoom};
use gpui_component::{ActiveTheme, Root};
use one_core::storage::manager::get_config_dir;
use one_core::tab_container::{TabContainer, TabContentRegistry, TabItem};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use terminal::LocalConfig;
use terminal_view::TerminalPaneArea;

use crate::setting_tab;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, GlobalStorageState};

const MAX_RESTORED_TERMINAL_PANES: usize = 8;

fn restored_panes(data: &serde_json::Value) -> &[serde_json::Value] {
    data.get("panes")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn restored_pane_kind(data: &serde_json::Value) -> &str {
    match data.get("kind").and_then(serde_json::Value::as_str) {
        Some("ssh") => "ssh",
        _ => "local",
    }
}

fn activate_tab_by_number(number: usize, cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    let Some(container) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = container.tab_container.clone();

    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            container.update(cx, |tc, cx| {
                if number == 1 && tc.has_pinned_tab() {
                    tc.activate_pinned_tab(window, cx);
                    return;
                }

                let index = if tc.has_pinned_tab() {
                    number.saturating_sub(2)
                } else {
                    number.saturating_sub(1)
                };

                if index < tc.tabs().len() {
                    tc.set_active_index(index, window, cx);
                }
            });
        });
    });
}

fn toggle_fullscreen(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, _| {
            window.toggle_fullscreen();
        });
    });
}

fn duplicate_tab(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    let Some(home) = cx.try_global::<GlobalHomePage>() else {
        return;
    };
    let home_page = home.home_page.clone();

    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            home_page.update(cx, |hp, cx| {
                hp.duplicate_active_tab(window, cx);
            });
        });
    });
}

fn quit_app(cx: &mut App) {
    cx.quit();
}

pub(crate) fn configured_log_file_path(value: &str) -> anyhow::Result<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Ok(default_log_file_path()?)
    } else {
        Ok(PathBuf::from(trimmed))
    }
}

fn default_log_file_path() -> anyhow::Result<PathBuf> {
    Ok(get_config_dir()?.join("logs").join("myterm.log"))
}

pub(crate) fn log_file_appender(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    setting_tab::init_settings(cx);
    one_core::init(cx);
    terminal_view::init(cx);
    crate::home_tab::init(cx);
    cx.bind_keys(init_keybindings(cx));
    init_action_handlers(cx);

    // 注册内置 JetBrains Mono 字体族（Regular, Light, ExtraLight）
    let mut font_bytes = Vec::new();
    for font_file in [
        "fonts/JetBrainsMono-Regular.ttf",
        "fonts/JetBrainsMono-Light.ttf",
        "fonts/JetBrainsMono-ExtraLight.ttf",
    ] {
        if let Some(font_data) = gpui_component_assets::Assets::get(font_file) {
            font_bytes.push(font_data.data);
        }
    }
    if !font_bytes.is_empty() {
        if let Err(err) = cx.text_system().add_fonts(font_bytes) {
            tracing::warn!("Failed to add embedded JetBrains Mono fonts: {:?}", err);
        } else {
            tracing::info!("Successfully loaded embedded JetBrains Mono font family");
        }
    }

    let mut registry = TabContentRegistry::new();
    registry.register_fn("Terminal".into(), |_state, window, cx| {
        // 兼容旧状态：用默认本地终端包裹为 TerminalPaneArea
        let pane_area =
            cx.new(|cx| TerminalPaneArea::new_local(LocalConfig::default(), None, window, cx));
        Some(Arc::new(pane_area) as Arc<dyn one_core::tab_container::TabContentView>)
    });
    registry.register_fn("TerminalPaneArea".into(), |state, window, cx| {
        let data = &state.data;
        // Version 2 stores every pane. Older states contain a single pane at
        // the top level and remain supported below.
        let panes = restored_panes(data);
        let pane_data = panes.first().unwrap_or(data);
        let kind = restored_pane_kind(pane_data);
        let pane_area = if kind == "ssh" {
            let id = pane_data.get("connection_id").and_then(|v| v.as_i64());
            let conn = id.and_then(|id| {
                cx.global::<GlobalStorageState>()
                    .storage
                    .get::<ConnectionRepository>()
                    .and_then(|repo| repo.get(id).ok().flatten())
            });
            conn.map(|conn| {
                let working_dir = pane_data
                    .get("working_dir")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                cx.new(|cx| TerminalPaneArea::new_ssh(conn, None, working_dir, true, window, cx))
            })
        } else {
            let config = pane_data
                .get("config")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default();
            Some(cx.new(|cx| TerminalPaneArea::new_local(config, None, window, cx)))
        };
        let pane_area = pane_area.unwrap_or_else(|| {
            cx.new(|cx| TerminalPaneArea::new_local(LocalConfig::default(), None, window, cx))
        });

        let dock_restored = data
            .get("dock_state")
            .filter(|value| value.is_object())
            .map(|dock_state| {
                pane_area.update(cx, |area, cx| area.load_dock_state(dock_state, window, cx))
            })
            .unwrap_or(false);

        // Recreate additional panes from v2 state. Dock geometry is currently
        // restored as a stable rightward sequence; invalid panes are skipped.
        if !dock_restored && !panes.is_empty() {
            for pane_data in panes.iter().skip(1).take(MAX_RESTORED_TERMINAL_PANES - 1) {
                let kind = restored_pane_kind(pane_data);
                let terminal = if kind == "ssh" {
                    let id = pane_data.get("connection_id").and_then(|v| v.as_i64());
                    id.and_then(|id| {
                        cx.global::<GlobalStorageState>()
                            .storage
                            .get::<ConnectionRepository>()
                            .and_then(|repo| repo.get(id).ok().flatten())
                            .map(|conn| {
                                let working_dir = pane_data
                                    .get("working_dir")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_owned);
                                cx.new(|cx| {
                                    terminal_view::TerminalView::new_ssh_with_index(
                                        conn,
                                        None,
                                        window,
                                        cx,
                                        working_dir.as_deref(),
                                        true,
                                    )
                                })
                            })
                    })
                } else {
                    let config = pane_data
                        .get("config")
                        .and_then(|value| serde_json::from_value(value.clone()).ok())
                        .unwrap_or_default();
                    Some(cx.new(|cx| terminal_view::TerminalView::new(config, window, cx)))
                };

                if let Some(terminal) = terminal {
                    pane_area.update(cx, |area, cx| {
                        if let Some(source) = area.active_terminal_view(cx) {
                            area.split_with_terminal(
                                &source,
                                terminal,
                                gpui_component::Placement::Right,
                                window,
                                cx,
                            );
                        }
                    });
                }
            }
        }
        Some(Arc::new(pane_area) as Arc<dyn one_core::tab_container::TabContentView>)
    });
    cx.set_global(registry);

    cx.activate(true);
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
    crate::home_tab::refresh_keybindings(cx);
    terminal_view::refresh_keybindings(cx);
}

/// 应用级快捷键清单：action_id → 默认键。
/// init 与 refresh 两条注册路径共用同一份清单，避免默认值漂移。
fn app_shortcut_specs() -> Vec<(&'static str, Vec<&'static str>)> {
    use one_core::keybindings::platform_shortcut as platform;
    vec![
        (action_id::WINDOW_TOGGLE_ZOOM, vec!["shift-escape"]),
        (action_id::WINDOW_CLOSE_PANEL, vec!["ctrl-w"]),
        (
            action_id::WINDOW_TOGGLE_FULLSCREEN,
            vec![platform("ctrl-cmd-f", "alt-enter")],
        ),
        (
            action_id::APP_DUPLICATE_TAB,
            vec![platform("cmd-shift-t", "alt-shift-t")],
        ),
        (action_id::APP_QUIT, vec![platform("cmd-q", "alt-f4")]),
    ]
}

fn activate_tab_keybindings() -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    for n in 1..=9 {
        #[cfg(target_os = "macos")]
        let keystroke = format!("cmd-{n}");
        #[cfg(not(target_os = "macos"))]
        let keystroke = format!("alt-{n}");
        let binding = match n {
            1 => KeyBinding::new(&keystroke, ActivateTab1, None),
            2 => KeyBinding::new(&keystroke, ActivateTab2, None),
            3 => KeyBinding::new(&keystroke, ActivateTab3, None),
            4 => KeyBinding::new(&keystroke, ActivateTab4, None),
            5 => KeyBinding::new(&keystroke, ActivateTab5, None),
            6 => KeyBinding::new(&keystroke, ActivateTab6, None),
            7 => KeyBinding::new(&keystroke, ActivateTab7, None),
            8 => KeyBinding::new(&keystroke, ActivateTab8, None),
            _ => KeyBinding::new(&keystroke, ActivateTab9, None),
        };
        keybindings.push(binding);
    }
    keybindings
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = vec![];
    keybindings.extend(app_shortcut_specs().into_iter().flat_map(|(id, defaults)| {
        shortcuts_for(cx, id, &defaults)
            .into_iter()
            .map(move |key| match id {
                action_id::WINDOW_TOGGLE_ZOOM => KeyBinding::new(&key, ToggleZoom, None),
                action_id::WINDOW_CLOSE_PANEL => KeyBinding::new(&key, ClosePanel, None),
                action_id::WINDOW_TOGGLE_FULLSCREEN => {
                    KeyBinding::new(&key, ToggleFullscreen, None)
                }
                action_id::APP_DUPLICATE_TAB => KeyBinding::new(&key, DuplicateTab, None),
                action_id::APP_QUIT => KeyBinding::new(&key, QuitApp, None),
                _ => unreachable!("app_shortcut_specs 只包含已知 action_id"),
            })
    }));
    keybindings.extend(activate_tab_keybindings());

    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    for (id, defaults) in app_shortcut_specs() {
        let rebound = match id {
            action_id::WINDOW_TOGGLE_ZOOM => {
                rebind_keybindings(cx, id, &defaults, None, ToggleZoom)
            }
            action_id::WINDOW_CLOSE_PANEL => {
                rebind_keybindings(cx, id, &defaults, None, ClosePanel)
            }
            action_id::WINDOW_TOGGLE_FULLSCREEN => {
                rebind_keybindings(cx, id, &defaults, None, ToggleFullscreen)
            }
            action_id::APP_DUPLICATE_TAB => {
                rebind_keybindings(cx, id, &defaults, None, DuplicateTab)
            }
            action_id::APP_QUIT => rebind_keybindings(cx, id, &defaults, None, QuitApp),
            _ => unreachable!("app_shortcut_specs 只包含已知 action_id"),
        };
        keybindings.extend(rebound);
    }
    keybindings
}

fn init_action_handlers(cx: &mut App) {
    cx.on_action(|_: &ActivateTab1, cx| activate_tab_by_number(1, cx));
    cx.on_action(|_: &ActivateTab2, cx| activate_tab_by_number(2, cx));
    cx.on_action(|_: &ActivateTab3, cx| activate_tab_by_number(3, cx));
    cx.on_action(|_: &ActivateTab4, cx| activate_tab_by_number(4, cx));
    cx.on_action(|_: &ActivateTab5, cx| activate_tab_by_number(5, cx));
    cx.on_action(|_: &ActivateTab6, cx| activate_tab_by_number(6, cx));
    cx.on_action(|_: &ActivateTab7, cx| activate_tab_by_number(7, cx));
    cx.on_action(|_: &ActivateTab8, cx| activate_tab_by_number(8, cx));
    cx.on_action(|_: &ActivateTab9, cx| activate_tab_by_number(9, cx));
    cx.on_action(|_: &ToggleFullscreen, cx| toggle_fullscreen(cx));
    cx.on_action(|_: &DuplicateTab, cx| duplicate_tab(cx));
    cx.on_action(|_: &QuitApp, cx| quit_app(cx));
    cx.on_action(|_: &OpenConnectionQuickOpen, cx| {
        let Some(active_window) = cx.active_window() else {
            return;
        };
        let Some(home) = cx.try_global::<GlobalHomePage>() else {
            return;
        };
        let home_page = home.home_page.clone();
        cx.defer(move |cx| {
            _ = active_window.update(cx, |_, window, cx| {
                if window.has_active_dialog(cx) {
                    window.close_all_dialogs(cx);
                }
                home_page.update(cx, |hp, cx| {
                    hp.show_connection_quick_open(window, cx);
                });
            });
        });
    });
    cx.on_action(|_: &NewConnectionShortcut, cx| {
        let Some(active_window) = cx.active_window() else {
            return;
        };
        let Some(home) = cx.try_global::<GlobalHomePage>() else {
            return;
        };
        let home_page = home.home_page.clone();
        cx.defer(move |cx| {
            _ = active_window.update(cx, |_, window, cx| {
                if window.has_active_dialog(cx) {
                    window.close_all_dialogs(cx);
                }
                home_page.update(cx, |hp, cx| {
                    hp.show_new_connection_dialog(window, cx);
                });
            });
        });
    });
}

pub struct MyApp {
    tab_container: Entity<TabContainer>,
}

impl MyApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tab_container = cx.new(|cx| {
            let theme = cx.theme();
            let tab_bar = theme.tab_bar;
            let tab = theme.tab;
            let tab_active = theme.tab_active;
            let tab_active_foreground = theme.tab_active_foreground;
            let tab_foreground = theme.tab_foreground;
            let mut container = TabContainer::new(window, cx)
                .with_tab_bar_colors(Some(tab_bar), Some(tab))
                .with_tab_item_colors(Some(tab_active), Some(tab))
                .with_inactive_tab_bg_color(Some(tab))
                .with_tab_content_colors(Some(tab_active_foreground), Some(tab_foreground));

            #[cfg(target_os = "macos")]
            {
                container = container
                    .with_left_padding(px(80.0))
                    .with_top_padding(px(4.0))
            }

            #[cfg(not(target_os = "macos"))]
            {
                container = container
                    .with_window_controls(true)
                    .on_split_horizontal(move |window, cx| {
                        split_active_terminal_pane(gpui_component::Placement::Right, window, cx);
                    })
                    .on_split_vertical(move |window, cx| {
                        split_active_terminal_pane(gpui_component::Placement::Bottom, window, cx);
                    })
            }

            container
        });

        cx.set_global(GlobalTabContainer {
            tab_container: tab_container.clone(),
        });
        // Set HomePage as the pinned tab (always visible, not scrollable)
        {
            let tab_container_clone = tab_container.clone();
            tab_container.update(cx, |tc, cx| {
                let home_page = cx.new(|cx| HomePage::new(tab_container_clone, window, cx));
                cx.set_global(GlobalHomePage {
                    home_page: home_page.clone(),
                });
                let home_tab = TabItem::new("home", "app", home_page);
                tc.set_pinned_tab(home_tab, cx);
                tc.activate_pinned_tab(window, cx);
            });
        }

        Self { tab_container }
    }
}

#[cfg(test)]
mod tests {
    use super::{configured_log_file_path, default_log_file_path, log_file_appender};
    use std::io::Write;

    #[test]
    fn configured_log_file_path_uses_default_for_empty_value() {
        let default_path = default_log_file_path().expect("应返回默认日志路径");

        assert_eq!(configured_log_file_path("").unwrap(), default_path);
        assert_eq!(configured_log_file_path("   ").unwrap(), default_path);
    }

    #[test]
    fn configured_log_file_path_trims_value() {
        let path = configured_log_file_path("  /tmp/myterm.log  ").expect("应返回日志路径");
        assert_eq!(path, std::path::PathBuf::from("/tmp/myterm.log"));
    }

    #[test]
    fn log_file_appender_creates_parent_directories_and_appends() {
        let path = std::env::temp_dir()
            .join(format!("myterm-log-test-{}", std::process::id()))
            .join("nested")
            .join("app.log");

        {
            let mut file = log_file_appender(&path).expect("应创建日志文件");
            writeln!(file, "first").expect("应写入第一行");
        }
        {
            let mut file = log_file_appender(&path).expect("应重新打开日志文件");
            writeln!(file, "second").expect("应追加第二行");
        }

        let content = std::fs::read_to_string(&path).expect("应读取日志文件");
        assert_eq!(content, "first\nsecond\n");

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn log_file_appender_creates_private_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir()
            .join(format!("myterm-log-permission-test-{}", std::process::id()))
            .join("app.log");
        let _file = log_file_appender(&path).expect("应创建日志文件");

        let mode = std::fs::metadata(&path)
            .expect("应读取日志文件元数据")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

fn split_active_terminal_pane(
    placement: gpui_component::Placement,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(tab_container) = cx
        .try_global::<GlobalTabContainer>()
        .map(|g| g.tab_container.clone())
    else {
        return;
    };

    let Some(pane_area) = tab_container.read(cx).active_tab().and_then(|tab| {
        if tab.content().content_key(cx) == "TerminalPaneArea" {
            tab.content().view().downcast::<TerminalPaneArea>().ok()
        } else {
            None
        }
    }) else {
        return;
    };

    pane_area.update(cx, |pane_area, cx| match placement {
        gpui_component::Placement::Right => pane_area.split_active_pane_right(window, cx),
        gpui_component::Placement::Bottom => pane_area.split_active_pane_down(window, cx),
        _ => {}
    });
}

impl Render for MyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        div()
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .child(div().size_full().child(self.tab_container.clone()))
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}
