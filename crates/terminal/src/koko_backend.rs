//! JumpServer Koko WebSocket 终端后端
//!
//! 与 `SshBackend` 同构:把 Koko WS 隧道的字节流喂进 alacritty Term grid,
//! 把用户输入/resize 通过 WS 上行。

use std::sync::Arc;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use jms::{KokoChannel, KokoConnectParams, KokoEvent};

use crate::osc::{OscEvent, extract_osc_events};
use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::{TerminalBackend, TerminalSize};

enum KokoCommand {
    Write(Vec<u8>),
    Resize(TerminalSize),
    Shutdown,
}

/// Koko WebSocket 终端后端
pub struct KokoBackend {
    command_tx: UnboundedSender<KokoCommand>,
}

impl KokoBackend {
    /// 建立 Koko 连接并启动读写循环
    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        params: KokoConnectParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        event_tx: UnboundedSender<TerminalEvent>,
        notify_tx: UnboundedSender<()>,
        on_disconnect: Option<UnboundedSender<()>>,
        init_cols: u16,
        init_rows: u16,
    ) -> anyhow::Result<Self> {
        let mut channel = KokoChannel::connect(&params)
            .await
            .map_err(|e| anyhow::anyhow!("Koko 连接失败: {e}"))?;

        // 注意:必须在收到服务端 CONNECT 消息、获取其分配的终端 id 后,
        // 才能发送 TERMINAL_INIT;过早发送会导致服务端无法路由。
        let mut pending_init: Option<(u16, u16)> = Some((init_cols, init_rows));

        let (command_tx, mut command_rx) = unbounded_channel::<KokoCommand>();

        // 创建回写通道,使终端响应(如 DA 查询)能通过 WS 写回
        let (pty_write_tx, mut pty_write_rx) = unbounded_channel::<Vec<u8>>();
        event_proxy.set_ssh_write_back(pty_write_tx);

        tokio::spawn(async move {
            let mut processor: Processor<StdSyncHandler> = Processor::new();

            loop {
                tokio::select! {
                    biased;
                    Some(cmd) = command_rx.recv() => {
                        match cmd {
                            KokoCommand::Write(data) => {
                                if channel.send_input(&data).await.is_err() {
                                    break;
                                }
                            }
                            KokoCommand::Resize(size) => {
                                let _ = channel.resize(size.cols, size.rows).await;
                            }
                            KokoCommand::Shutdown => {
                                let _ = channel.close().await;
                                break;
                            }
                        }
                    }
                    Some(data) = pty_write_rx.recv() => {
                        if channel.send_input(&data).await.is_err() {
                            break;
                        }
                    }
                    event = channel.recv() => {
                        match event {
                            Some(KokoEvent::Connect(_id)) => {
                                if let Some((cols, rows)) = pending_init.take() {
                                    if channel.send_init(cols, rows).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Some(KokoEvent::Output(data)) => {
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
                                        OscEvent::CommandStart => {
                                            let _ = event_tx.send(TerminalEvent::CommandStart);
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
                                let _ = notify_tx.send(());
                            }
                            Some(KokoEvent::Notify(_)) => {
                                // 服务端会话/通知消息已在 channel 内记录
                            }
                            Some(KokoEvent::Closed) | None => {
                                break;
                            }
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
        let _ = self.command_tx.send(KokoCommand::Write(data));
    }

    fn resize(&self, size: TerminalSize) {
        let _ = self.command_tx.send(KokoCommand::Resize(size));
    }

    fn shutdown(&self) {
        let _ = self.command_tx.send(KokoCommand::Shutdown);
    }
}
