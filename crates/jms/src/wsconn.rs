//! Koko WebSocket 公共连接建立逻辑（终端 / SFTP 共用）
//!
//! 负责：URL 拼装、子协议与认证头、TCP 直连 / SOCKS5 / HTTP CONNECT 隧道、
//! rustls TLS、WebSocket 握手。上层只需要给出端点路径和额外查询参数。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream, client_async_tls_with_config};

use crate::JmsError;
use crate::koko::{KokoConnectParams, KokoProxy};

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// 建立好的 Koko WebSocket 连接
pub(crate) type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 打开一个 Koko WebSocket 连接。
///
/// - `path`：端点路径，例如 `/koko/ws/terminal/` 或 `/koko/ws/sftp/`
/// - `extra_query`：额外查询参数（值会做 URL 编码）
pub(crate) async fn open_koko_socket(
    params: &KokoConnectParams,
    path: &str,
    extra_query: &[(&str, String)],
) -> Result<WsStream, JmsError> {
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
    let mut query = format!("_={timestamp}");
    if !params.token_id.trim().is_empty() {
        query.push_str(&format!("&token={}", urlencoding::encode(&params.token_id)));
    }
    if let Some(ref ticket) = params.ticket
        && !ticket.trim().is_empty()
    {
        query.push_str(&format!("&ticket={}", urlencoding::encode(ticket)));
    }
    for (key, value) in extra_query {
        query.push_str(&format!("&{}={}", key, urlencoding::encode(value)));
    }
    let port_suffix = if port == 443 {
        String::new()
    } else {
        format!(":{port}")
    };
    let ws_url = format!("wss://{host}{port_suffix}{path}?{query}");

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
        if let Some(ref ticket) = params.ticket
            && let Ok(v) = HeaderValue::from_str(ticket)
        {
            headers.insert("X-Koko-Connect-Ticket", v);
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
        None => tokio::time::timeout(connect_timeout, TcpStream::connect((host.as_str(), port)))
            .await
            .map_err(|_| JmsError::NetworkError("TCP 连接超时".to_string()))?
            .map_err(|e| JmsError::NetworkError(format!("TCP 连接失败: {e}")))?,
        Some(KokoProxy::Socks5 {
            host: phost,
            port: pport,
            username,
            password,
        }) => {
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
                    _ => tokio_socks::tcp::Socks5Stream::connect((phost.as_str(), *pport), target)
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
                http_connect(phost, *pport, &host, port, auth),
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
    .map_err(|e| {
        let text = e.to_string();
        // Django 在会话失效时会 302 到登录页，直接暴露成鉴权错误更好排查
        if text.contains("HTTP error: 302") || text.contains("HTTP error: 301") {
            JmsError::AuthError(format!(
                "服务端重定向到登录页（{text}），JMS 会话可能已过期或未携带会话 Cookie"
            ))
        } else if text.contains("HTTP error: 401") || text.contains("HTTP error: 403") {
            JmsError::AuthError(format!("鉴权失败（{text}）"))
        } else {
            JmsError::NetworkError(format!("WebSocket 握手失败: {text}"))
        }
    })?;

    Ok(stream)
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
            "HTTP 代理 CONNECT 失败: {status_line}"
        )));
    }

    Ok(stream)
}
