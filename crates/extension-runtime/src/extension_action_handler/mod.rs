#![allow(dead_code)]

use std::path::PathBuf;

use gpui::{App, Window};
use gpui_component::{WindowExt, notification::Notification};

use crate::extension::{ExtensionKind, extensions_root};

mod popup;

pub fn register_db_tree_extension_action_handler(_cx: &mut App) {
    // Database support removed
}

fn push_error(window: &mut Window, message: impl Into<String>, cx: &mut App) {
    window.push_notification(Notification::error(message.into()).autohide(true), cx);
}

fn composite_root() -> Option<PathBuf> {
    extensions_root().map(|root| root.join(ExtensionKind::Composite.dir_name()))
}
