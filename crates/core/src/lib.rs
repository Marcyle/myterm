use gpui::App;

rust_i18n::i18n!("locales", fallback = "zh-CN");

pub mod app_paths;
pub mod connection_notifier;
pub mod crypto;
pub mod gpui_tokio;
pub mod key_storage;
pub mod keybindings;
pub mod layout;
pub mod popup_window;
pub mod storage;
pub mod tab_container;
pub mod settings;
pub mod themes;
pub mod utils;

pub fn init(cx: &mut App) {
    gpui_tokio::init(cx);
    themes::init(cx);
    storage::init(cx);
    connection_notifier::init(cx);
}
