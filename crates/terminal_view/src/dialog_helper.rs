//! Terminal 关闭确认对话框的公共工具函数
//!
//! 提取出重复的 `Arc<StdMutex<Option<oneshot::Sender<bool>>>>` +
//! `window.open_dialog` 模式，减少 view.rs 和 pane_area.rs 中的重复代码。

use gpui::*;
use gpui_component::WindowExt;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::v_flex;
use rust_i18n::t;
use std::sync::{Arc, Mutex as StdMutex};

/// 打开本地终端关闭确认对话框。
///
/// 当本地终端有运行中的命令或 TUI 应用时弹出，
/// 用户确认后才真正关闭终端。
///
/// 返回 `Task<bool>`：`true` 表示用户点击了"关闭"，`false` 表示取消。
pub fn confirm_local_terminal_close_dialog(window: &mut Window, cx: &mut App) -> gpui::Task<bool> {
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
                let tx = tx_ok.clone();
                move |_, _, _| {
                    if let Ok(mut guard) = tx.lock() {
                        if let Some(sender) = guard.take() {
                            let _ = sender.send(true);
                        }
                    }
                    true
                }
            })
            .on_cancel({
                let tx = tx_cancel.clone();
                move |_, _, _| {
                    if let Ok(mut guard) = tx.lock() {
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

    cx.spawn(async move |_| rx.await.unwrap_or(false))
}
