#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "en");

mod app_init;
mod home;
mod home_tab;
mod jms_connection_window;
mod jms_session;
mod jms_session_badge;
mod myterm_app;
pub mod new_connection;
mod setting_tab;
mod settings;

use crate::myterm_app::MyApp;
use gpui::*;

use gpui_component::Root;
use gpui_component_assets::Assets;

/// 全局 panic 钩子：panic 信息同步追加写入 `myterm-panic.log`（与主日志同目录）。
///
/// tracing 的 non_blocking writer 在 abort 时可能丢失缓冲，这里用同步写保证
/// 崩溃现场（消息/位置/backtrace）不丢，便于事后定位异常退出原因。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "Box<dyn Any>".to_string());
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();

        // 同步写入独立 panic 日志（不依赖 tracing，abort 前必落盘）
        if let Ok(path) = crate::myterm_app::default_log_file_path() {
            let panic_log = path.with_file_name("myterm-panic.log");
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&panic_log)
            {
                use std::io::Write as _;
                let timestamp = chrono::Utc::now().to_rfc3339();
                let _ = writeln!(
                    file,
                    "\n=== PANIC @ {timestamp} ===\nthread: {thread_name}\nmessage: {payload}\nlocation: {location}\nbacktrace:\n{backtrace}\n"
                );
            }
        }

        // 保留默认行为（stderr 输出 + 可能的 WER 上报）
        default_hook(info);
    }));
}

fn main() {
    install_panic_hook();

    let app = Application::new()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::LastWindowClosed);

    app.run(move |cx| {
        myterm_app::init(cx);

        let mut window_size = size(px(1600.0), px(1200.0));
        if let Some(display) = cx.primary_display() {
            let display_size = display.bounds().size;
            window_size.width = window_size.width.min(display_size.width * 0.85);
            window_size.height = window_size.height.min(display_size.height * 0.85);
        }

        let window_bounds = Bounds::centered(None, window_size, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
            #[cfg(not(target_os = "linux"))]
            titlebar: Some(gpui_component::TitleBar::title_bar_options()),
            window_min_size: Some(Size {
                width: px(640.),
                height: px(480.),
            }),
            #[cfg(target_os = "linux")]
            window_background: gpui::WindowBackgroundAppearance::Transparent,
            #[cfg(target_os = "linux")]
            window_decorations: Some(gpui::WindowDecorations::Client),
            kind: WindowKind::Normal,
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                window.activate_window();
                app_init::init_window_systems(window, cx);
                let view = cx.new(|cx| MyApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
