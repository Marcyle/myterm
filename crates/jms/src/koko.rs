//! JumpServer Koko WebSocket 终端隧道
//!
//! 通过 Koko 组件的 WebSocket 接口建立字符终端会话:
//! - 端点:`wss://<host>/koko/ws/terminal/?token=<connection_token_id>`
//! - 子协议:`JMS-KOKO`
//! - 认证:Cookie(`jms_sessionid` + `X-JMS-ORG`)
//!
//! 消息信封为 JSON `{"id","type","data"}`,终端字节流(TERMINAL_DATA)的 data 字段为 base64。

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream, client_async_tls_with_config};

use crate::JmsError;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

// Koko 消息类型常量
pub const MSG_PING: &str = "PING";
pub const MSG_PONG: &str = "PONG";
pub const MSG_CONNECT: &str = "CONNECT";
pub const MSG_CLOSE: &str = "CLOSE";
pub const MSG_TERMINAL_INIT: &str = "TERMINAL_INIT";
pub const MSG_TERMINAL_DATA: &str = "TERMINAL_DATA";
pub const MSG_TERMINAL_RESIZE: &str = "TERMINAL_RESIZE";
pub const MSG_TERMINAL_SESSION: &str = "TERMINAL_SESSION";
pub const MSG_TERMINAL_ERROR: &str = "TERMINAL_ERROR";
pub const MSG_MESSAGE_NOTIFY: &str = "MESSAGE_NOTIFY";

// Koko v5 二进制信封协议常量
const ENVELOPE_VERSION: u8 = 0x01;
const ENVELOPE_TERMINAL_INPUT: u8 = 0x01;
const ENVELOPE_TERMINAL_OUTPUT: u8 = 0x02;
const ENVELOPE_TERMINAL_COMMAND: u8 = 0x03;
const ENVELOPE_ERROR: u8 = 0x04;
const ENVELOPE_TERMINAL_CREATE: u8 = 0x06;
const ENVELOPE_TERMINAL_CLOSE: u8 = 0x07;
const ENVELOPE_HEADER_SIZE: usize = 6;
const ENVELOPE_TERMINAL_ID_SIZE: usize = 4;

/// 构建二进制信封帧：[version(1B), type(1B), length(4B big-endian), payload...]
fn build_envelope(msg_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ENVELOPE_HEADER_SIZE + payload.len());
    buf.push(ENVELOPE_VERSION);
    buf.push(msg_type);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// 构建终端输入二进制信封 (0x01)：[terminalId(4B big-endian), raw_data...]
fn build_terminal_input_envelope(terminal_id: u32, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(ENVELOPE_TERMINAL_ID_SIZE + data.len());
    payload.extend_from_slice(&terminal_id.to_be_bytes());
    payload.extend_from_slice(data);
    build_envelope(ENVELOPE_TERMINAL_INPUT, &payload)
}

/// 构建终端创建信封 (0x06)
fn build_terminal_create_envelope(request_id: &str, cols: u16, rows: u16) -> Vec<u8> {
    let json_val = serde_json::json!({
        "requestId": request_id,
        "params": {
            "type": "primary",
            "cols": cols,
            "rows": rows
        }
    });
    let payload = serde_json::to_vec(&json_val).unwrap_or_default();
    build_envelope(ENVELOPE_TERMINAL_CREATE, &payload)
}

/// 构建终端命令信封 (0x03)
fn build_terminal_command_envelope(
    terminal_id: u32,
    command: &str,
    params: serde_json::Value,
    request_id: &str,
) -> Vec<u8> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let json_val = serde_json::json!({
        "terminalId": terminal_id,
        "command": command,
        "params": params,
        "requestId": request_id,
        "timestamp": timestamp,
    });
    let payload = serde_json::to_vec(&json_val).unwrap_or_default();
    build_envelope(ENVELOPE_TERMINAL_COMMAND, &payload)
}

struct ParsedEnvelope<'a> {
    msg_type: u8,
    payload: &'a [u8],
}

fn parse_envelope(data: &[u8]) -> Option<ParsedEnvelope<'_>> {
    if data.len() < ENVELOPE_HEADER_SIZE {
        return None;
    }
    if data[0] != ENVELOPE_VERSION {
        return None;
    }
    let msg_type = data[1];
    let payload_len = u32::from_be_bytes([data[2], data[3], data[4], data[5]]) as usize;
    if data.len() < ENVELOPE_HEADER_SIZE + payload_len {
        return None;
    }
    Some(ParsedEnvelope {
        msg_type,
        payload: &data[ENVELOPE_HEADER_SIZE..ENVELOPE_HEADER_SIZE + payload_len],
    })
}

fn parse_terminal_output(payload: &[u8]) -> Option<(u32, &[u8])> {
    if payload.len() < ENVELOPE_TERMINAL_ID_SIZE {
        return None;
    }
    let terminal_id = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    Some((terminal_id, &payload[ENVELOPE_TERMINAL_ID_SIZE..]))
}

/// Koko WebSocket 消息信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KokoMessage {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub data: String,
}

/// 终端尺寸负载(TERMINAL_INIT / TERMINAL_RESIZE 的 data 字段)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TerminalSizePayload {
    cols: u16,
    rows: u16,
    /// TERMINAL_INIT 需要携带空 code 字段(与浏览器行为保持一致)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    code: String,
}

/// 代理配置(支持 SOCKS5 与 HTTP CONNECT)
#[derive(Clone, Debug)]
pub enum KokoProxy {
    Socks5 {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
    Http {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
}

/// Koko 连接参数
#[derive(Clone, Debug)]
pub struct KokoConnectParams {
    /// JMS 服务器基础地址(如 https://jms.example.com)
    pub base_url: String,
    /// 连接 token 的 id(UUID,用于 ?token= 查询参数)
    pub token_id: String,
    /// 会话 cookie(jms_sessionid)
    pub session_cookie: Option<String>,
    /// 组织 ID
    pub org_id: String,
    /// 代理(可选)
    pub proxy: Option<KokoProxy>,
    /// 终端显示名(用于 tab 标题)
    pub title: String,
    /// 资产 ID（用于复制标签页时重新申请连接 token）
    pub asset_id: Option<String>,
    /// 账号名（用于复制标签页时重新申请连接 token）
    pub account_name: Option<String>,
    /// 账号 ID（UUID，用于创建连接 token）
    pub account_id: Option<String>,
    /// Koko 临时连接票据（JumpServer v5 使用）
    pub ticket: Option<String>,
}

/// recv 返回的事件
#[derive(Debug)]
pub enum KokoEvent {
    /// 服务端 CONNECT 消息,携带其分配的终端 id
    Connect(String),
    /// 终端输出字节
    Output(Vec<u8>),
    /// 服务端通知/会话消息(已记录,无需处理)
    Notify(String),
    /// 连接关闭
    Closed,
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Koko WebSocket 通道
pub struct KokoChannel {
    stream: WsStream,
    /// 服务端 CONNECT 消息中分配的终端 id,后续上行消息必须复用该 id
    term_id: String,
    /// Koko 终端 ID（v5 为 u32，默认分配为 1）
    terminal_id: u32,
    /// 是否使用 v5 二进制信封协议
    is_envelope: bool,
}

impl KokoChannel {
    /// 建立 Koko WebSocket 连接
    pub async fn connect(params: &KokoConnectParams) -> Result<Self, JmsError> {
        // 依赖树中可能同时存在多个 rustls crypto provider,需显式安装进程级默认值,
        // 否则 tokio-tungstenite 的自动 TLS 路径会 panic。
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        }

        let base = params.base_url.trim_end_matches('/');
        let parsed = url::Url::parse(base)
            .map_err(|e| JmsError::NetworkError(format!("解析 JMS 地址失败: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| JmsError::NetworkError("JMS 地址缺少主机名".to_string()))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(443);

        // 构建 wss URL(浏览器还会带 disableautohash 与 _ 时间戳,补上避免被拦截)
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut query = format!("disableautohash=false&token={}&_={}", params.token_id, timestamp);
        if let Some(ref ticket) = params.ticket {
            query.push_str(&format!("&ticket={}", ticket));
        }
        let ws_url = format!(
            "wss://{host}{port_suffix}/koko/ws/terminal/?{query}",
            host = host,
            port_suffix = if port == 443 {
                String::new()
            } else {
                format!(":{port}")
            },
        );

        // 构建握手请求,附加子协议、Cookie、Origin
        let mut request = ws_url
            .as_str()
            .into_client_request()
            .map_err(|e| JmsError::NetworkError(format!("构建 WS 请求失败: {e}")))?;
        {
            let headers = request.headers_mut();
            headers.insert(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_static("JMS-KOKO"),
            );
            headers.insert(
                "Sec-WebSocket-Extensions",
                HeaderValue::from_static("permessage-deflate; client_max_window_bits"),
            );
            let origin = format!("https://{host}");
            if let Ok(v) = HeaderValue::from_str(&origin) {
                headers.insert("Origin", v);
            }
            if let Some(ref ticket) = params.ticket {
                if let Ok(v) = HeaderValue::from_str(ticket) {
                    headers.insert("X-Koko-Connect-Ticket", v);
                }
            }
            // Koko WS 端点双写 sessionid / jms_sessionid，确保通过 Django 认证
            if let Some(sid) = &params.session_cookie {
                let cookie = format!(
                    "jms_sessionid={sid}; sessionid={sid}; X-JMS-ORG={org}; X-JMS-LUNA-ORG={org}",
                    sid = sid,
                    org = params.org_id
                );
                if let Ok(v) = HeaderValue::from_str(&cookie) {
                    headers.insert("Cookie", v);
                }
            }
        }

        // 建立 TCP(直连 / SOCKS5 / HTTP CONNECT),三种情况最终都得到 tokio TcpStream
        let connect_timeout = Duration::from_secs(15);
        let tcp = match &params.proxy {
            None => {
                tokio::time::timeout(connect_timeout, TcpStream::connect((host.as_str(), port)))
                    .await
                    .map_err(|_| JmsError::NetworkError("TCP 连接超时".to_string()))?
                    .map_err(|e| JmsError::NetworkError(format!("TCP 连接失败: {e}")))?
            }
            Some(KokoProxy::Socks5 {
                host: phost,
                port: pport,
                username,
                password,
            }) => {
                let target = (host.as_str(), port);
                let fut = async {
                    match (username, password) {
                        (Some(u), Some(p)) => {
                            tokio_socks::tcp::Socks5Stream::connect_with_password(
                                (phost.as_str(), *pport),
                                target,
                                u,
                                p,
                            )
                            .await
                            .map(|s| s.into_inner())
                        }
                        _ => tokio_socks::tcp::Socks5Stream::connect(
                            (phost.as_str(), *pport),
                            target,
                        )
                        .await
                        .map(|s| s.into_inner()),
                    }
                };
                tokio::time::timeout(connect_timeout, fut)
                    .await
                    .map_err(|_| JmsError::NetworkError("SOCKS5 连接超时".to_string()))?
                    .map_err(|e| JmsError::NetworkError(format!("SOCKS5 连接失败: {e}")))?
            }
            Some(KokoProxy::Http {
                host: phost,
                port: pport,
                username,
                password,
            }) => {
                let auth = match (username, password) {
                    (Some(u), Some(p)) => Some((u.clone(), p.clone())),
                    _ => None,
                };
                tokio::time::timeout(
                    connect_timeout,
                    Self::http_connect(phost, *pport, &host, port, auth),
                )
                .await
                .map_err(|_| JmsError::NetworkError("HTTP 代理连接超时".to_string()))?
                .map_err(|e| JmsError::NetworkError(format!("HTTP 代理连接失败: {e}")))?
            }
        };

        // 显式构造 rustls TLS Connector,确保对 wss:// 真正执行 TLS
        // (自动 connector 在某些特性组合下不生效,会向 TLS 端口发明文导致挂死)
        let mut roots = rustls::RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            let _ = roots.add(cert);
        }
        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = Connector::Rustls(Arc::new(tls_config));

        // 在 TCP 之上完成 WS（含 TLS）握手
        let (stream, _resp) = tokio::time::timeout(
            connect_timeout,
            client_async_tls_with_config(request, tcp, None, Some(connector)),
        )
        .await
        .map_err(|_| JmsError::NetworkError("WebSocket 握手超时".to_string()))?
        .map_err(|e| JmsError::NetworkError(format!("WebSocket 握手失败: {e}")))?;

        // 终端 id 占位,收到服务端 CONNECT 消息后再更新为服务端分配的 id
        Ok(Self {
            stream,
            term_id: String::new(),
            terminal_id: 1,
            is_envelope: false,
        })
    }

    /// 通过 HTTP 代理建立到目标的 CONNECT 隧道,返回隧道 TcpStream
    async fn http_connect(
        proxy_host: &str,
        proxy_port: u16,
        target_host: &str,
        target_port: u16,
        auth: Option<(String, String)>,
    ) -> std::io::Result<TcpStream> {
        let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;

        let mut req = format!(
            "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n",
            host = target_host,
            port = target_port
        );
        if let Some((u, p)) = auth {
            let cred = B64.encode(format!("{u}:{p}"));
            req.push_str(&format!("Proxy-Authorization: Basic {cred}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await?;

        // 读取响应直到 \r\n\r\n
        let mut buf = Vec::with_capacity(256);
        let mut byte = [0u8; 1];
        loop {
            let n = stream.read(&mut byte).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP 代理在握手期间关闭连接",
                ));
            }
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
            if buf.len() > 8192 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "HTTP 代理响应过长",
                ));
            }
        }

        let head = String::from_utf8_lossy(&buf);
        let status_line = head.lines().next().unwrap_or("");
        // 期望 "HTTP/1.1 200 ..."
        let ok = status_line
            .split_whitespace()
            .nth(1)
            .map(|code| code == "200")
            .unwrap_or(false);
        if !ok {
            return Err(std::io::Error::other(format!(
                "HTTP 代理 CONNECT 失败: {}",
                status_line
            )));
        }

        Ok(stream)
    }

    /// 发送 TERMINAL_INIT,告知初始终端尺寸
    pub async fn send_init(&mut self, cols: u16, rows: u16) -> Result<(), JmsError> {
        if self.is_envelope {
            let req_id = format!("init-{}", self.term_id);
            // 1. 发送 TERMINAL_CREATE 信封 (0x06)，告知终端行列尺寸以分配 terminalId
            let create_frame = build_terminal_create_envelope(&req_id, cols, rows);
            self.stream
                .send(Message::Binary(create_frame.into()))
                .await
                .map_err(|e| JmsError::NetworkError(format!("发送 TERMINAL_CREATE 失败: {e}")))?;

            // 2. 同时发送 TERMINAL_INIT 命令信封 (0x03)，以防服务端等待该指令
            let data_str = serde_json::to_string(&TerminalSizePayload {
                cols,
                rows,
                code: String::new(),
            })
            .unwrap_or_default();
            let init_frame = build_terminal_command_envelope(
                0,
                MSG_TERMINAL_INIT,
                serde_json::json!({
                    "id": self.term_id,
                    "type": MSG_TERMINAL_INIT,
                    "data": data_str,
                }),
                &req_id,
            );
            let _ = self.stream.send(Message::Binary(init_frame.into())).await;
            Ok(())
        } else {
            if self.term_id.is_empty() {
                return Err(JmsError::ApiError(
                    "尚未收到 CONNECT 消息,无法发送 TERMINAL_INIT".to_string(),
                ));
            }
            let data = serde_json::to_string(&TerminalSizePayload {
                cols,
                rows,
                code: String::new(),
            })
            .map_err(|e| JmsError::ParseError(e.to_string()))?;
            self.send_message(MSG_TERMINAL_INIT, data).await
        }
    }

    /// 发送用户输入
    ///
    /// Koko v5 二进制信封模式使用 0x01 (envelopeTerminalInput)，携带 4 字节 terminalId + 原始输入。
    /// 旧版模式则发送 TERMINAL_DATA 文本消息。
    pub async fn send_input(&mut self, bytes: &[u8]) -> Result<(), JmsError> {
        if self.is_envelope {
            let frame = build_terminal_input_envelope(self.terminal_id, bytes);
            self.stream
                .send(Message::Binary(frame.into()))
                .await
                .map_err(|e| JmsError::NetworkError(format!("WS 发送二进制输入失败: {e}")))?;
            Ok(())
        } else {
            let data = String::from_utf8_lossy(bytes).to_string();
            self.send_message(MSG_TERMINAL_DATA, data).await
        }
    }

    /// 发送 resize
    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), JmsError> {
        if self.is_envelope {
            let data_str = serde_json::to_string(&TerminalSizePayload {
                cols,
                rows,
                code: String::new(),
            })
            .unwrap_or_default();
            let cmd_frame = build_terminal_command_envelope(
                self.terminal_id,
                MSG_TERMINAL_RESIZE,
                serde_json::json!({
                    "id": self.term_id,
                    "type": MSG_TERMINAL_RESIZE,
                    "data": data_str,
                    "terminalId": self.terminal_id,
                }),
                "resize",
            );
            self.stream
                .send(Message::Binary(cmd_frame.into()))
                .await
                .map_err(|e| JmsError::NetworkError(format!("WS 发送 resize 失败: {e}")))?;
            Ok(())
        } else {
            let data = serde_json::to_string(&TerminalSizePayload {
                cols,
                rows,
                code: String::new(),
            })
            .map_err(|e| JmsError::ParseError(e.to_string()))?;
            self.send_message(MSG_TERMINAL_RESIZE, data).await
        }
    }

    /// 发送应用层 PING 心跳，用于保持长连接不被中间件/防火墙因空闲而断开。
    pub async fn send_ping(&mut self) -> Result<(), JmsError> {
        if self.is_envelope {
            let cmd_frame = build_terminal_command_envelope(
                0,
                MSG_PING,
                serde_json::json!({
                    "id": self.term_id,
                    "type": MSG_PING,
                    "data": "",
                }),
                "ping",
            );
            let _ = self.stream.send(Message::Binary(cmd_frame.into())).await;
            let _ = self.stream.send(Message::Ping(vec![].into())).await;
            Ok(())
        } else {
            self.send_message(MSG_PING, String::new()).await
        }
    }

    /// 关闭连接
    pub async fn close(&mut self) -> Result<(), JmsError> {
        if self.is_envelope {
            let json_val = serde_json::json!({
                "terminalId": self.terminal_id,
                "requestId": "close",
                "reason": "client_close",
            });
            let payload = serde_json::to_vec(&json_val).unwrap_or_default();
            let frame = build_envelope(ENVELOPE_TERMINAL_CLOSE, &payload);
            let _ = self.stream.send(Message::Binary(frame.into())).await;
        }
        let _ = self.stream.send(Message::Close(None)).await;
        Ok(())
    }

    async fn send_message(&mut self, msg_type: &str, data: String) -> Result<(), JmsError> {
        let msg = KokoMessage {
            id: self.term_id.clone(),
            msg_type: msg_type.to_string(),
            data,
        };
        let txt = serde_json::to_string(&msg).map_err(|e| JmsError::ParseError(e.to_string()))?;
        self.stream
            .send(Message::Text(txt.into()))
            .await
            .map_err(|e| JmsError::NetworkError(format!("WS 发送失败: {e}")))?;
        Ok(())
    }

    /// 接收下一个事件(过滤掉协议层心跳/通知)
    pub async fn recv(&mut self) -> Option<KokoEvent> {
        loop {
            let msg = match self.stream.next().await {
                Some(Ok(m)) => m,
                Some(Err(e)) => {
                    tracing::warn!("Koko WS 接收错误: {e}");
                    return Some(KokoEvent::Closed);
                }
                None => return Some(KokoEvent::Closed),
            };

            match msg {
                Message::Binary(bytes) => {
                    if let Some(env) = parse_envelope(&bytes) {
                        self.is_envelope = true;
                        match env.msg_type {
                            ENVELOPE_TERMINAL_OUTPUT => {
                                if let Some((term_id, output)) = parse_terminal_output(env.payload) {
                                    if term_id > 0 {
                                        self.terminal_id = term_id;
                                    }
                                    return Some(KokoEvent::Output(output.to_vec()));
                                }
                            }
                            ENVELOPE_TERMINAL_COMMAND => {
                                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(env.payload) {
                                    let cmd = val
                                        .get("command")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default();
                                    let term_id = val
                                        .get("terminalId")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                    if term_id > 0 {
                                        self.terminal_id = term_id;
                                    }
                                    match cmd {
                                        MSG_CONNECT => {
                                            let id = val
                                                .get("params")
                                                .and_then(|p| p.get("id"))
                                                .and_then(|v| v.as_str())
                                                .unwrap_or_default()
                                                .to_string();
                                            if !id.is_empty() {
                                                self.term_id = id;
                                            }
                                            return Some(KokoEvent::Connect(self.term_id.clone()));
                                        }
                                        "created" => {
                                            tracing::info!("Koko v5 终端分配成功: terminal_id={}", self.terminal_id);
                                        }
                                        MSG_CLOSE => {
                                            return Some(KokoEvent::Closed);
                                        }
                                        MSG_PING => {
                                            let _ = self.send_ping().await;
                                        }
                                        _ => {
                                            tracing::debug!("Koko 忽略命令消息: {}", cmd);
                                        }
                                    }
                                }
                            }
                            ENVELOPE_ERROR => {
                                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(env.payload) {
                                    let err_msg = val
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("终端错误");
                                    tracing::warn!("Koko 收到服务端错误: {}", err_msg);
                                    return Some(KokoEvent::Notify(err_msg.to_string()));
                                }
                            }
                            ENVELOPE_TERMINAL_CLOSE => {
                                return Some(KokoEvent::Closed);
                            }
                            _ => {}
                        }
                    } else {
                        return Some(KokoEvent::Output(bytes.to_vec()));
                    }
                }
                Message::Text(text) => {
                    let parsed: Result<KokoMessage, _> = serde_json::from_str(text.as_str());
                    let km = match parsed {
                        Ok(km) => km,
                        Err(_) => {
                            continue;
                        }
                    };

                    match km.msg_type.as_str() {
                        MSG_CONNECT => {
                            // 保存服务端分配的终端 id,后续上行消息必须复用
                            if !km.id.is_empty() {
                                self.term_id = km.id.clone();
                            }
                            return Some(KokoEvent::Connect(self.term_id.clone()));
                        }
                        MSG_TERMINAL_DATA => match B64.decode(km.data.as_bytes()) {
                            Ok(bytes) => return Some(KokoEvent::Output(bytes)),
                            Err(_) => {
                                // base64 解码失败 → 也许是裸文本,直接当字节返回(便于协议校正)
                                return Some(KokoEvent::Output(km.data.into_bytes()));
                            }
                        },
                        MSG_PING => {
                            continue;
                        }
                        MSG_PONG => continue,
                        MSG_CLOSE | MSG_TERMINAL_ERROR => {
                            return Some(KokoEvent::Closed);
                        }
                        MSG_TERMINAL_SESSION | MSG_MESSAGE_NOTIFY => {
                            return Some(KokoEvent::Notify(km.data));
                        }
                        _ => continue,
                    }
                }
                Message::Ping(payload) => {
                    let _ = self.stream.send(Message::Pong(payload)).await;
                    continue;
                }
                Message::Pong(_) => continue,
                Message::Close(_) => return Some(KokoEvent::Closed),
                Message::Frame(_) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_koko_message_roundtrip() {
        let msg = KokoMessage {
            id: "abc".to_string(),
            msg_type: MSG_TERMINAL_DATA.to_string(),
            data: "ZWNobyBoaQ==".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        // type 字段必须序列化为 "type"
        assert!(json.contains("\"type\":\"TERMINAL_DATA\""));
        let back: KokoMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "abc");
        assert_eq!(back.msg_type, MSG_TERMINAL_DATA);
        assert_eq!(back.data, "ZWNobyBoaQ==");
    }

    #[test]
    fn test_terminal_data_base64() {
        let input = b"echo hello\n";
        let encoded = B64.encode(input);
        let decoded = B64.decode(encoded.as_bytes()).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_size_payload_json() {
        let data = serde_json::to_string(&TerminalSizePayload {
            cols: 191,
            rows: 43,
            code: String::new(),
        })
        .unwrap();
        assert_eq!(data, "{\"cols\":191,\"rows\":43}");
    }

    #[test]
    fn test_parse_server_terminal_data() {
        let raw = r#"{"id":"t1","type":"TERMINAL_DATA","data":"aGVsbG8="}"#;
        let km: KokoMessage = serde_json::from_str(raw).unwrap();
        assert_eq!(km.msg_type, MSG_TERMINAL_DATA);
        let bytes = B64.decode(km.data.as_bytes()).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn test_envelope_roundtrip() {
        let payload = b"hello terminal";
        let frame = build_envelope(ENVELOPE_TERMINAL_OUTPUT, payload);
        assert_eq!(frame[0], 0x01);
        assert_eq!(frame[1], 0x02);
        assert_eq!(frame.len(), 6 + payload.len());

        let parsed = parse_envelope(&frame).expect("should parse envelope");
        assert_eq!(parsed.msg_type, 0x02);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn test_terminal_input_output_envelopes() {
        let input_data = b"ls -la\n";
        let input_frame = build_terminal_input_envelope(1, input_data);
        let parsed = parse_envelope(&input_frame).unwrap();
        assert_eq!(parsed.msg_type, ENVELOPE_TERMINAL_INPUT);
        assert_eq!(&parsed.payload[..4], &1u32.to_be_bytes());
        assert_eq!(&parsed.payload[4..], input_data);

        let output_data = b"total 40\ndrwxr-xr-x";
        let mut out_payload = Vec::new();
        out_payload.extend_from_slice(&2u32.to_be_bytes());
        out_payload.extend_from_slice(output_data);
        let (term_id, output) = parse_terminal_output(&out_payload).unwrap();
        assert_eq!(term_id, 2);
        assert_eq!(output, output_data);
    }

    #[test]
    fn test_terminal_create_and_command_envelopes() {
        let create_frame = build_terminal_create_envelope("test-req", 120, 40);
        let parsed = parse_envelope(&create_frame).unwrap();
        assert_eq!(parsed.msg_type, ENVELOPE_TERMINAL_CREATE);
        let json_val: serde_json::Value = serde_json::from_slice(parsed.payload).unwrap();
        assert_eq!(json_val["requestId"], "test-req");
        assert_eq!(json_val["params"]["cols"], 120);
        assert_eq!(json_val["params"]["rows"], 40);

        let cmd_frame = build_terminal_command_envelope(
            1,
            "TERMINAL_RESIZE",
            serde_json::json!({"cols": 100, "rows": 30}),
            "resize-1",
        );
        let parsed_cmd = parse_envelope(&cmd_frame).unwrap();
        assert_eq!(parsed_cmd.msg_type, ENVELOPE_TERMINAL_COMMAND);
        let cmd_val: serde_json::Value = serde_json::from_slice(parsed_cmd.payload).unwrap();
        assert_eq!(cmd_val["command"], "TERMINAL_RESIZE");
        assert_eq!(cmd_val["terminalId"], 1);
    }
}
