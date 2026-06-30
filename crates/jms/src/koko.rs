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
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream};

use crate::JmsError;

const B64: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::STANDARD;

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
        let ws_url = format!(
            "wss://{host}{port_suffix}/koko/ws/terminal/?disableautohash=false&token={token}&_={timestamp}",
            host = host,
            port_suffix = if port == 443 {
                String::new()
            } else {
                format!(":{port}")
            },
            token = params.token_id,
        );

        tracing::info!("Koko WS 连接: url={}", ws_url);

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
            // Koko WS 端点依赖 Django session cookie(jms_sessionid) 来识别已登录用户,
            // 即使 connect-token 已预授权。浏览器在访问 JMS 站点时已被种下该 cookie,
            // 因此握手必须携带它,否则会被 302 到登录页。
            if let Some(ref sid) = params.session_cookie {
                let cookie = format!(
                    "jms_sessionid={sid}; X-JMS-ORG={org}; X-JMS-LUNA-ORG={org}",
                    sid = sid,
                    org = params.org_id
                );
                tracing::info!(
                    "Koko 发送 Cookie: jms_sessionid 长度={}, org={}",
                    sid.len(),
                    params.org_id
                );
                if let Ok(v) = HeaderValue::from_str(&cookie) {
                    headers.insert("Cookie", v);
                }
            } else {
                tracing::warn!("Koko 未携带 session cookie(可能导致 302 重定向到登录)");
            }
        }

        // 建立 TCP(直连 / SOCKS5 / HTTP CONNECT),三种情况最终都得到 tokio TcpStream
        let connect_timeout = Duration::from_secs(15);
        let tcp = match &params.proxy {
            None => {
                tracing::info!("Koko TCP 直连: {}:{}", host, port);
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
                tracing::info!("Koko 经 SOCKS5 代理 {}:{} 连接 {}:{}", phost, pport, host, port);
                let target = (host.as_str(), port);
                let fut = async {
                    match (username, password) {
                        (Some(u), Some(p)) => tokio_socks::tcp::Socks5Stream::connect_with_password(
                            (phost.as_str(), *pport),
                            target,
                            u,
                            p,
                        )
                        .await
                        .map(|s| s.into_inner()),
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
                tracing::info!("Koko 经 HTTP 代理 {}:{} CONNECT {}:{}", phost, pport, host, port);
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

        tracing::info!("Koko TCP 已连接,开始 WS/TLS 握手");

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
        .map_err(|_| {
            tracing::error!("Koko WebSocket 握手超时");
            JmsError::NetworkError("WebSocket 握手超时".to_string())
        })?
        .map_err(|e| {
            if let tokio_tungstenite::tungstenite::Error::Http(resp) = &e {
                let status = resp.status();
                let headers = resp.headers();
                tracing::error!(
                    "Koko WebSocket 握手收到 HTTP 响应: status={}, headers={:?}",
                    status, headers
                );
                if let Some(body) = resp.body() {
                    tracing::error!(
                        "Koko WebSocket 握手响应体: {}",
                        String::from_utf8_lossy(body)
                    );
                }
            }
            tracing::error!("Koko WebSocket 握手失败: {e}");
            JmsError::NetworkError(format!("WebSocket 握手失败: {e}"))
        })?;

        tracing::info!("Koko WS 握手成功");

        // 终端 id 占位,收到服务端 CONNECT 消息后再更新为服务端分配的 id
        let term_id = String::new();
        Ok(Self { stream, term_id })
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
            return Err(std::io::Error::other(
                format!("HTTP 代理 CONNECT 失败: {}", status_line),
            ));
        }

        Ok(stream)
    }

    /// 发送 TERMINAL_INIT,告知初始终端尺寸
    pub async fn send_init(&mut self, cols: u16, rows: u16) -> Result<(), JmsError> {
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

    /// 发送用户输入
    ///
    /// Koko 服务端对 TERMINAL_DATA 的处理是 `conn.Write([]byte(msg.Data))`,
    /// 即直接把 data 字段当原始字符串写入伪终端,**不做 base64 解码**。
    /// 因此这里把输入字节按 UTF-8 放入 data(键盘输入含控制符也是合法 UTF-8)。
    pub async fn send_input(&mut self, bytes: &[u8]) -> Result<(), JmsError> {
        let data = String::from_utf8_lossy(bytes).to_string();
        self.send_message(MSG_TERMINAL_DATA, data).await
    }

    /// 发送 resize
    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), JmsError> {
        let data = serde_json::to_string(&TerminalSizePayload {
            cols,
            rows,
            code: String::new(),
        })
        .map_err(|e| JmsError::ParseError(e.to_string()))?;
        self.send_message(MSG_TERMINAL_RESIZE, data).await
    }

    /// 关闭连接
    pub async fn close(&mut self) -> Result<(), JmsError> {
        let _ = self.stream.send(Message::Close(None)).await;
        Ok(())
    }

    async fn send_message(&mut self, msg_type: &str, data: String) -> Result<(), JmsError> {
        if self.term_id.is_empty() {
            tracing::warn!("发送 {} 时 term_id 为空,可能 CONNECT 尚未到达", msg_type);
        }
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
                    tracing::warn!("Koko WS 读取错误: {}", e);
                    return Some(KokoEvent::Closed);
                }
                None => return Some(KokoEvent::Closed),
            };

            match msg {
                Message::Text(text) => {
                    let parsed: Result<KokoMessage, _> = serde_json::from_str(text.as_str());
                    let km = match parsed {
                        Ok(km) => km,
                        Err(e) => {
                            tracing::warn!(
                                "Koko 入站消息解析失败: {} | raw={}",
                                e,
                                &text.as_str()[..text.as_str().len().min(200)]
                            );
                            continue;
                        }
                    };

                    let preview = &km.data[..km.data.len().min(80)];
                    tracing::info!("Koko 入站: type={}, data_len={}, data_prefix={}", km.msg_type, km.data.len(), preview);

                    match km.msg_type.as_str() {
                        MSG_CONNECT => {
                            // 保存服务端分配的终端 id,后续上行消息必须复用
                            if !km.id.is_empty() {
                                self.term_id = km.id.clone();
                                tracing::info!("Koko 收到 CONNECT,服务端终端 id={}", self.term_id);
                            }
                            return Some(KokoEvent::Connect(self.term_id.clone()));
                        }
                        MSG_TERMINAL_DATA => match B64.decode(km.data.as_bytes()) {
                            Ok(bytes) => return Some(KokoEvent::Output(bytes)),
                            Err(e) => {
                                // base64 解码失败 → 也许是裸文本,直接当字节返回(便于协议校正)
                                tracing::warn!("Koko TERMINAL_DATA base64 解码失败: {},按裸文本处理", e);
                                return Some(KokoEvent::Output(km.data.into_bytes()));
                            }
                        },
                        MSG_PING => {
                            // 浏览器抓包显示:服务端 PING 后客户端无响应,双方各自定时发 PING。
                            // 因此这里不回 PONG,避免与应用层协议冲突。
                            tracing::debug!("Koko 收到服务端 PING,忽略");
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
                Message::Binary(b) => {
                    // Koko 服务端用 Binary 帧(opcode 2)直接发送原始终端字节流,
                    // 无需 base64 解码,直接喂进终端 grid。
                    return Some(KokoEvent::Output(b.to_vec()));
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
        let data = serde_json::to_string(&TerminalSizePayload { cols: 191, rows: 43, code: String::new() }).unwrap();
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
}
