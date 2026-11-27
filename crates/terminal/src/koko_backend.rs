//! JumpServer Koko WebSocket 终端后端
//!
//! 与 `SshBackend` 同构:把 Koko WS 隧道的字节流喂进 alacritty Term grid,
//! 把用户输入/resize 通过 WS 上行。

use std::sync::Arc;
use std::time::Duration;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use tokio::sync::mpsc::{Sender, UnboundedSender, channel as mpsc_channel, unbounded_channel};
use tokio::time::interval;

use jms::{KokoChannel, KokoConnectParams, KokoEvent};

use crate::osc::{OscEvent, extract_osc_events};
use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::zmodem::{ZmodemEventFn, ZmodemScanner, ZmodemState, ZmodemWriteFn};
use crate::{TerminalBackend, TerminalSize};

enum KokoCommand {
    /// 用户输入（传输期间会转为取消请求）
    Write(Vec<u8>),
    /// 协议流字节（ZMODEM 等），不做取消判断
    WriteBinary(Vec<u8>),
    Resize(TerminalSize),
    Shutdown,
}

/// Keeps the server's first terminal geometry aligned with the rendered pane.
///
/// Koko assigns the terminal id in its `CONNECT` event. Resize commands sent before then have no
/// usable id, so retain only the latest geometry and send it as `TERMINAL_INIT` instead.
#[derive(Debug, Clone, Copy)]
struct KokoHandshakeState {
    pending_initial_size: Option<TerminalSize>,
}

impl KokoHandshakeState {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            pending_initial_size: Some(TerminalSize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            }),
        }
    }

    /// Returns a resize suitable for the established terminal, or defers it for `TERMINAL_INIT`.
    fn record_resize(&mut self, size: TerminalSize) -> Option<TerminalSize> {
        match self.pending_initial_size.as_mut() {
            Some(pending_size) => {
                *pending_size = size;
                None
            }
            None => Some(size),
        }
    }

    fn take_initial_size(&mut self) -> Option<TerminalSize> {
        self.pending_initial_size.take()
    }
}

/// Koko WebSocket 终端后端
pub struct KokoBackend {
    command_tx: Sender<KokoCommand>,
}

impl KokoBackend {
    /// 建立 Koko 连接并启动读写循环
    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        params: KokoConnectParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        event_tx: UnboundedSender<TerminalEvent>,
        notify_tx: Sender<()>,
        on_disconnect: Option<UnboundedSender<()>>,
        init_cols: u16,
        init_rows: u16,
    ) -> anyhow::Result<Self> {
        let mut channel = KokoChannel::connect(&params)
            .await
            .map_err(|e| anyhow::anyhow!("Koko 连接失败: {e}"))?;

        // The server cannot route terminal messages until its CONNECT event assigns an id.
        // Preserve the latest layout while that handshake is pending; a split pane commonly
        // receives its real bounds before CONNECT arrives.
        let mut handshake = KokoHandshakeState::new(init_cols, init_rows);

        // Bound UI -> websocket commands so repeated resize/input events cannot
        // grow an unbounded queue while the connection is stalled.
        let (command_tx, mut command_rx) = mpsc_channel::<KokoCommand>(256);

        // 创建回写通道,使终端响应(如 DA 查询)能通过 WS 写回
        let (pty_write_tx, mut pty_write_rx) = unbounded_channel::<Vec<u8>>();
        event_proxy.set_ssh_write_back(pty_write_tx);

        // ZMODEM（rz/sz）：协议字节经同一条 WS 通道上行
        let zmodem = {
            let command_tx = command_tx.clone();
            let write_fn: ZmodemWriteFn = Arc::new(move |data: &[u8]| {
                let _ = command_tx.try_send(KokoCommand::WriteBinary(data.to_vec()));
            });
            let emit_fn: ZmodemEventFn = {
                let event_tx = event_tx.clone();
                Arc::new(move |event| {
                    let _ = event_tx.send(TerminalEvent::Zmodem(event));
                })
            };
            ZmodemState::new(write_fn, emit_fn, crate::zmodem::default_download_dir())
        };
        let zmodem_for_task = zmodem.clone();

        tokio::spawn(async move {
            let zmodem = zmodem_for_task;
            let mut zmodem_scanner = ZmodemScanner::new();
            let mut processor: Processor<StdSyncHandler> = Processor::new();
            // 每隔 30 秒发送一次应用层 PING 心跳，防止 NAT/防火墙/负载均衡因长时间
            // 空闲而静默断开 WebSocket，导致终端“有焦点但无响应”。
            let mut heartbeat_interval = interval(Duration::from_secs(30));

            loop {
                tokio::select! {
                    biased;
                    command = command_rx.recv() => {
                        let Some(cmd) = command else {
                            break;
                        };
                        match cmd {
                            KokoCommand::Write(data) => {
                                // 传输期间的按键会打断 ZMODEM 字节流，转为取消请求
                                if zmodem.is_active() {
                                    tracing::debug!(
                                        target: "terminal.zmodem",
                                        bytes = ?data,
                                        "zmodem 传输期间收到用户输入，取消本次传输"
                                    );
                                    zmodem.cancel();
                                    continue;
                                }
                                if channel.send_input(&data).await.is_err() {
                                    break;
                                }
                            }
                            KokoCommand::WriteBinary(data) => {
                                if channel.send_input_binary(&data).await.is_err() {
                                    break;
                                }
                            }
                            KokoCommand::Resize(size) => {
                                if let Some(size) = handshake.record_resize(size) {
                                    let _ = channel.resize(size.cols, size.rows).await;
                                }
                            }
                            KokoCommand::Shutdown => {
                                let _ = channel.close().await;
                                break;
                            }
                        }
                    }
                    Some(data) = pty_write_rx.recv() => {
                        if zmodem.is_active() {
                            continue;
                        }
                        if channel.send_input(&data).await.is_err() {
                            break;
                        }
                    }
                    event = channel.recv() => {
                        match event {
                            Some(KokoEvent::Connect(_id)) => {
                                if let Some(size) = handshake.take_initial_size()
                                    && channel.send_init(size.cols, size.rows).await.is_err()
                                {
                                    break;
                                }
                            }
                            Some(KokoEvent::Output(data)) => {
                                // ZMODEM 会话期间，输出字节交给协议线程，不再解析进终端
                                let data = zmodem_scanner.scan(&zmodem, &data);
                                if data.is_empty() {
                                    continue;
                                }
                                // 解析 OSC 事件(工作目录/prompt 生命周期等)
                                for osc_event in extract_osc_events(&data) {
                                    match osc_event {
                                        OscEvent::WorkingDirChanged(path) => {
                                            let _ = event_tx.send(TerminalEvent::WorkingDirChanged(path));
                                        }
                                        OscEvent::PromptStart => {
                                            let _ = event_tx.send(TerminalEvent::PromptStart);
                                        }
                                        OscEvent::InputStart => {
                                            let _ = event_tx.send(TerminalEvent::InputStart);
                                        }
                                        OscEvent::CommandStart { command } => {
                                            let _ = event_tx.send(TerminalEvent::CommandStart { command });
                                        }
                                        OscEvent::CommandFinished { exit_code } => {
                                            let _ = event_tx.send(TerminalEvent::CommandFinished { exit_code });
                                        }
                                        OscEvent::CommandRecorded(command) => {
                                            let _ = event_tx.send(TerminalEvent::CommandRecorded(command));
                                        }
                                    }
                                }

                                processor.advance(&mut *term.lock(), &data);
                                // Notification is only a render hint; coalesce it when
                                // the UI is already waiting for a frame.
                                let _ = notify_tx.try_send(());
                            }
                            Some(KokoEvent::Notify(_)) => {
                                // 服务端会话/通知消息已在 channel 内记录
                            }
                            Some(KokoEvent::Closed) | None => {
                                break;
                            }
                        }
                    }
                    _ = heartbeat_interval.tick() => {
                        if channel.send_ping().await.is_err() {
                            break;
                        }
                    }
                }
            }

            if let Some(tx) = on_disconnect {
                let _ = tx.send(());
            }
        });

        Ok(Self { command_tx })
    }
}

impl TerminalBackend for KokoBackend {
    fn write(&self, data: Vec<u8>) {
        if self.command_tx.try_send(KokoCommand::Write(data)).is_err() {
            tracing::warn!(target: "terminal_residue", "Koko command queue full; dropping input");
        }
    }

    fn resize(&self, size: TerminalSize) {
        if self.command_tx.try_send(KokoCommand::Resize(size)).is_err() {
            tracing::debug!(target: "terminal_residue", "Koko command queue full; dropping resize");
        }
    }

    fn shutdown(&self) {
        let _ = self.command_tx.try_send(KokoCommand::Shutdown);
    }
}
impl Drop for KokoBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::KokoHandshakeState;
    use crate::TerminalSize;

    fn size(cols: u16, rows: u16) -> TerminalSize {
        TerminalSize {
            cols,
            rows,
            pixel_width: cols * 8,
            pixel_height: rows * 20,
        }
    }

    #[test]
    fn handshake_initializes_with_latest_pre_connect_resize() {
        let mut handshake = KokoHandshakeState::new(80, 24);

        assert_eq!(handshake.record_resize(size(96, 40)), None);
        assert_eq!(handshake.record_resize(size(120, 50)), None);

        assert_eq!(handshake.take_initial_size(), Some(size(120, 50)));
        assert_eq!(handshake.take_initial_size(), None);
    }

    #[test]
    fn handshake_forwards_resizes_after_connect() {
        let mut handshake = KokoHandshakeState::new(80, 24);

        assert_eq!(
            handshake.take_initial_size(),
            Some(TerminalSize {
                cols: 80,
                rows: 24,
                pixel_width: 0,
                pixel_height: 0,
            })
        );
        assert_eq!(handshake.record_resize(size(96, 40)), Some(size(96, 40)));
    }
}
