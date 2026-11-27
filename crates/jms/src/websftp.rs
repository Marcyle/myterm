//! Koko Web SFTP 通道（`/koko/ws/sftp/`）。
//!
//! 依据 koko（v4.10 / v5.x）源码实现：
//! - 端点：`wss://<host>/koko/ws/sftp/?token=<connect_token_id>`（无 token 时可用 `?asset=<id>`）
//! - 鉴权：与终端相同（Cookie `jms_sessionid` + `X-JMS-ORG` + `X-Koko-Connect-Ticket`）
//! - 文本帧：JSON `{id,type,cmd,data,err,error_code,current_path}`
//! - 二进制帧：`[0x01][u32 BE jsonLen][json header][raw]`，`raw` 上限 2 MiB
//! - 命令：`list` / `transfer_read` / `upload`(chunk+merge) / `save` / `rm` / `rename` / `mkdir`
//!
//! 上行的二进制数据必须以 **WebSocket 二进制消息**发送整帧；服务端把任意二进制
//! 消息当作 SFTP 帧解析（内层 header 决定 cmd）。下行时 `SFTP_TRANSFER_BINARY`
//! 会以二进制帧下发，而普通 `SFTP_BINARY` 走 JSON（raw 为 base64），因此这里下载
//! 统一走 `transfer_read` + `binary=true`，避免 base64 膨胀。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

use crate::JmsError;
use crate::koko::KokoConnectParams;
use crate::wsconn::{WsStream, open_koko_socket};

/// SFTP 二进制帧版本号
const SFTP_FRAME_VERSION: u8 = 0x01;
/// 帧头长度：[version(1)][jsonLen(4 BE)]
const SFTP_FRAME_HEADER_SIZE: usize = 5;
/// 单帧原始数据上限（与服务端 `transferChunkMaxSize` 一致）
pub const TRANSFER_CHUNK_MAX: usize = 2 * 1024 * 1024;

pub const MSG_SFTP_DATA: &str = "SFTP_DATA";
/// 服务端在 JSON 帧里携带 `raw`（base64）时使用；本项目下载统一走二进制帧。
#[allow(dead_code)]
pub const MSG_SFTP_BINARY: &str = "SFTP_BINARY";

/// 等待单个响应的最长时间
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// SFTP 文本消息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SftpMessage {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub msg_type: String,
    #[serde(default)]
    pub cmd: String,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub err: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(default)]
    pub current_path: String,
}

/// `list` 返回的目录项（对应 koko 的 `sftpFile`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SftpFileEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "de_size")]
    pub size: u64,
    #[serde(default)]
    pub perm: String,
    #[serde(default, deserialize_with = "de_size")]
    pub mod_time: u64,
    #[serde(rename = "type", default)]
    pub file_type: String,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub version: String,
}

/// koko 把 `size` / `mod_time` 以字符串下发（`json:",string"`），这里同时容忍数字。
fn de_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(u64),
        Text(String),
    }
    Ok(match Option::<Raw>::deserialize(deserializer)? {
        Some(Raw::Num(value)) => value,
        Some(Raw::Text(text)) => text.trim().parse().unwrap_or(0),
        None => 0,
    })
}

/// `transfer_read` 的块元数据
#[derive(Debug, Clone, Default, Deserialize)]
struct TransferChunkMeta {
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    eof: bool,
}

/// 下行帧
enum Incoming {
    Text(SftpMessage),
    Binary { header: SftpMessage, raw: Vec<u8> },
}

/// Koko Web SFTP 通道
pub struct WebSftpChannel {
    stream: WsStream,
    next_id: u64,
    /// 分片上传使用的数值会话 id（服务端要求 `id` 可被 strconv.Atoi 解析）
    chunk_session: i64,
}

impl WebSftpChannel {
    /// 建立 SFTP WebSocket 连接。
    ///
    /// `params.token_id` 为空时改用 `?asset=<asset_id>`（由服务端按当前账号权限自动选账号）。
    pub async fn connect(params: &KokoConnectParams) -> Result<Self, JmsError> {
        let mut extra: Vec<(&str, String)> = Vec::new();
        if params.token_id.trim().is_empty()
            && let Some(asset) = params.asset_id.as_ref().filter(|id| !id.trim().is_empty())
        {
            extra.push(("asset", asset.clone()));
        }

        let stream = open_koko_socket(params, "/koko/ws/sftp/", &extra).await?;
        tracing::info!(
            target: "jms.websftp",
            host = %params.base_url,
            has_token = !params.token_id.trim().is_empty(),
            "Web SFTP 通道已连接"
        );
        Ok(Self {
            stream,
            next_id: 1,
            chunk_session: 1,
        })
    }

    fn take_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id.to_string()
    }

    async fn send_text(&mut self, msg: &SftpMessage) -> Result<(), JmsError> {
        let text = serde_json::to_string(msg)
            .map_err(|e| JmsError::ParseError(format!("序列化 SFTP 消息失败: {e}")))?;
        self.stream
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| JmsError::NetworkError(format!("发送 SFTP 消息失败: {e}")))
    }

    async fn send_binary(&mut self, header: &SftpMessage, raw: &[u8]) -> Result<(), JmsError> {
        if raw.len() > TRANSFER_CHUNK_MAX {
            return Err(JmsError::ApiError(format!(
                "SFTP 单帧数据过大: {} > {}",
                raw.len(),
                TRANSFER_CHUNK_MAX
            )));
        }
        let json = serde_json::to_vec(header)
            .map_err(|e| JmsError::ParseError(format!("序列化 SFTP 帧头失败: {e}")))?;
        let mut frame = Vec::with_capacity(SFTP_FRAME_HEADER_SIZE + json.len() + raw.len());
        frame.push(SFTP_FRAME_VERSION);
        frame.extend_from_slice(&(json.len() as u32).to_be_bytes());
        frame.extend_from_slice(&json);
        frame.extend_from_slice(raw);
        self.stream
            .send(Message::Binary(frame.into()))
            .await
            .map_err(|e| JmsError::NetworkError(format!("发送 SFTP 二进制帧失败: {e}")))
    }

    async fn next_frame(&mut self) -> Result<Incoming, JmsError> {
        loop {
            let message = self
                .stream
                .next()
                .await
                .ok_or_else(|| JmsError::NetworkError("SFTP 连接已关闭".to_string()))?
                .map_err(|e| JmsError::NetworkError(format!("接收 SFTP 消息失败: {e}")))?;

            match message {
                Message::Text(text) => {
                    let msg: SftpMessage = serde_json::from_str(&text).map_err(|e| {
                        JmsError::ParseError(format!("解析 SFTP 消息失败: {e}: {text}"))
                    })?;
                    match msg.msg_type.as_str() {
                        "PING" => {
                            let pong = SftpMessage {
                                id: msg.id.clone(),
                                msg_type: "PONG".to_string(),
                                data: msg.data.clone(),
                                ..Default::default()
                            };
                            self.send_text(&pong).await?;
                            continue;
                        }
                        "PONG" => continue,
                        "CLOSE" => {
                            let reason = if msg.err.is_empty() {
                                msg.data.clone()
                            } else {
                                msg.err.clone()
                            };
                            return Err(JmsError::ApiError(format!("SFTP 会话已关闭: {reason}")));
                        }
                        _ => return Ok(Incoming::Text(msg)),
                    }
                }
                Message::Binary(bytes) => {
                    if bytes.first() != Some(&SFTP_FRAME_VERSION) {
                        tracing::debug!(
                            target: "jms.websftp",
                            len = bytes.len(),
                            "忽略非 SFTP 二进制帧"
                        );
                        continue;
                    }
                    let (header, payload) = parse_binary_frame(&bytes)?;
                    return Ok(Incoming::Binary {
                        header,
                        raw: payload,
                    });
                }
                Message::Ping(payload) => {
                    self.stream
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| JmsError::NetworkError(format!("回送 Pong 失败: {e}")))?;
                    continue;
                }
                Message::Pong(_) | Message::Frame(_) => continue,
                Message::Close(reason) => {
                    return Err(JmsError::NetworkError(format!(
                        "SFTP 连接被服务端关闭: {reason:?}"
                    )));
                }
            }
        }
    }

    /// 发送一条文本请求并等待 `SFTP_DATA` 响应。
    async fn call(
        &mut self,
        cmd: &str,
        request: serde_json::Value,
    ) -> Result<SftpMessage, JmsError> {
        let id = self.take_id();
        let msg = SftpMessage {
            id: id.clone(),
            msg_type: MSG_SFTP_DATA.to_string(),
            cmd: cmd.to_string(),
            data: request.to_string(),
            ..Default::default()
        };
        self.send_text(&msg).await?;

        let deadline = tokio::time::Instant::now() + RESPONSE_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(JmsError::NetworkError(format!("SFTP 命令 {cmd} 响应超时")));
            }
            let frame = tokio::time::timeout(remaining, self.next_frame())
                .await
                .map_err(|_| JmsError::NetworkError(format!("SFTP 命令 {cmd} 响应超时")))??;
            match frame {
                Incoming::Text(resp) => {
                    if resp.id != id && !resp.id.is_empty() {
                        continue;
                    }
                    return ensure_ok(resp);
                }
                Incoming::Binary { header, .. } => {
                    if header.id != id && !header.id.is_empty() {
                        continue;
                    }
                    return ensure_ok(header);
                }
            }
        }
    }

    /// 列目录，返回 (服务端解析出的当前路径, 目录项)。
    pub async fn list(&mut self, path: &str) -> Result<(String, Vec<SftpFileEntry>), JmsError> {
        let resp = self.call("list", json!({ "path": path })).await?;
        let entries: Vec<SftpFileEntry> = serde_json::from_str(&resp.data)
            .map_err(|e| JmsError::ParseError(format!("解析目录列表失败: {e}: {}", resp.data)))?;
        let current_path = if resp.current_path.is_empty() {
            path.to_string()
        } else {
            resp.current_path.clone()
        };
        Ok((current_path, entries))
    }

    /// 解析路径的真实绝对路径（koko 没有独立 realpath 命令，用 list 的 current_path）。
    pub async fn realpath(&mut self, path: &str) -> Result<String, JmsError> {
        let (current_path, _) = self.list(path).await?;
        Ok(current_path)
    }

    pub async fn mkdir(&mut self, path: &str) -> Result<(), JmsError> {
        self.call("mkdir", json!({ "path": path })).await?;
        Ok(())
    }

    /// 删除（目录由服务端递归删除）。
    pub async fn remove(&mut self, path: &str) -> Result<(), JmsError> {
        self.call("rm", json!({ "path": path })).await?;
        Ok(())
    }

    /// 改名（服务端接收的是新名字，不是新路径）。
    pub async fn rename(&mut self, path: &str, new_name: &str) -> Result<(), JmsError> {
        self.call("rename", json!({ "path": path, "new_name": new_name }))
            .await?;
        Ok(())
    }

    /// 流式下载：按 `transfer_read` 分块拉取并写入 `writer`。
    pub async fn download_to_writer<W>(
        &mut self,
        path: &str,
        writer: &mut W,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(u64),
    ) -> Result<u64, JmsError>
    where
        W: AsyncWrite + Unpin,
    {
        let id = self.take_id();
        let mut offset: i64 = 0;
        let mut total: u64 = 0;

        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(JmsError::Cancelled);
            }

            let request = json!({
                "path": path,
                "offset": offset,
                "length": TRANSFER_CHUNK_MAX,
                "binary": true,
            });
            let msg = SftpMessage {
                id: id.clone(),
                msg_type: MSG_SFTP_DATA.to_string(),
                cmd: "transfer_read".to_string(),
                data: request.to_string(),
                ..Default::default()
            };
            self.send_text(&msg).await?;

            // 等待这一块的二进制帧（同一 id）
            let (raw, eof, meta_offset) = loop {
                let frame = tokio::time::timeout(RESPONSE_TIMEOUT, self.next_frame())
                    .await
                    .map_err(|_| JmsError::NetworkError("SFTP 下载响应超时".to_string()))??;
                match frame {
                    Incoming::Binary { header, raw } => {
                        if header.id != id && !header.id.is_empty() {
                            continue;
                        }
                        ensure_ok(header.clone())?;
                        let meta: TransferChunkMeta =
                            serde_json::from_str(&header.data).unwrap_or_default();
                        break (raw, meta.eof, meta.offset);
                    }
                    Incoming::Text(resp) => {
                        if resp.id != id && !resp.id.is_empty() {
                            continue;
                        }
                        let resp = ensure_ok(resp)?;
                        return Err(JmsError::ApiError(format!(
                            "服务端未按二进制分块返回下载数据: {}",
                            resp.msg_type
                        )));
                    }
                }
            };

            if !raw.is_empty() {
                writer
                    .write_all(&raw)
                    .await
                    .map_err(|e| JmsError::NetworkError(format!("写入本地文件失败: {e}")))?;
                // 服务端会回传本次读取的起始偏移，用它推进以容忍长度被裁剪
                offset = meta_offset + raw.len() as i64;
                total += raw.len() as u64;
                progress(total);
            }
            if eof {
                break;
            }
        }

        writer
            .flush()
            .await
            .map_err(|e| JmsError::NetworkError(format!("刷新本地文件失败: {e}")))?;
        Ok(total)
    }

    /// 流式上传：`upload(chunk=true)` 分片 + `upload(merge=true)` 收尾。
    pub async fn upload_from_reader<R>(
        &mut self,
        path: &str,
        size: u64,
        reader: &mut R,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(u64),
    ) -> Result<u64, JmsError>
    where
        R: AsyncRead + Unpin,
    {
        self.chunk_session += 1;
        let session_id = self.chunk_session;
        let id = session_id.to_string();

        let mut buffer = vec![0u8; TRANSFER_CHUNK_MAX];
        let mut offset: u64 = 0;

        while offset < size {
            if cancelled.load(Ordering::Relaxed) {
                return Err(JmsError::Cancelled);
            }
            let want = TRANSFER_CHUNK_MAX.min((size - offset) as usize);
            reader
                .read_exact(&mut buffer[..want])
                .await
                .map_err(|e| JmsError::NetworkError(format!("读取本地文件失败: {e}")))?;

            let header = SftpMessage {
                id: id.clone(),
                msg_type: MSG_SFTP_DATA.to_string(),
                cmd: "upload".to_string(),
                data: json!({
                    "path": path,
                    "chunk": true,
                    "offset": offset,
                    "size": size,
                })
                .to_string(),
                ..Default::default()
            };
            self.send_binary(&header, &buffer[..want]).await?;
            self.await_response(&id).await?;

            offset += want as u64;
            progress(offset);
        }

        // 收尾合并
        let header = SftpMessage {
            id: id.clone(),
            msg_type: MSG_SFTP_DATA.to_string(),
            cmd: "upload".to_string(),
            data: json!({ "path": path, "merge": true }).to_string(),
            ..Default::default()
        };
        self.send_text(&header).await?;
        self.await_response(&id).await?;

        Ok(offset)
    }

    /// 等待指定 id 的响应（忽略心跳与其他消息）。
    async fn await_response(&mut self, id: &str) -> Result<SftpMessage, JmsError> {
        let deadline = tokio::time::Instant::now() + RESPONSE_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(JmsError::NetworkError("SFTP 上传响应超时".to_string()));
            }
            let frame = tokio::time::timeout(remaining, self.next_frame())
                .await
                .map_err(|_| JmsError::NetworkError("SFTP 上传响应超时".to_string()))??;
            match frame {
                Incoming::Text(resp) => {
                    if resp.id != id && !resp.id.is_empty() {
                        continue;
                    }
                    return ensure_ok(resp);
                }
                Incoming::Binary { header, .. } => {
                    if header.id != id && !header.id.is_empty() {
                        continue;
                    }
                    return ensure_ok(header);
                }
            }
        }
    }

    /// 读取整个文件内容（超过 `max_bytes` 直接报错，避免误读大文件）。
    pub async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, JmsError> {
        let (_, entries) = self.list(parent_dir(path)).await?;
        let base = file_name(path);
        let entry = entries
            .iter()
            .find(|entry| entry.name == base)
            .ok_or_else(|| JmsError::ApiError(format!("远端文件不存在: {path}")))?;
        if entry.size > max_bytes as u64 {
            return Err(JmsError::ApiError(format!(
                "远端文件 {} 大小 {} 超过上限 {}",
                path, entry.size, max_bytes
            )));
        }

        let mut sink = Vec::with_capacity(entry.size as usize);
        let cancelled = AtomicBool::new(false);
        self.download_to_writer(path, &mut sink, &cancelled, |_| {})
            .await?;
        Ok(sink)
    }

    /// 写入整个文件（`save` 命令，带版本冲突检测）。
    pub async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<(), JmsError> {
        if content.len() > TRANSFER_CHUNK_MAX {
            // 大文件走分片上传
            let mut cursor = std::io::Cursor::new(content.to_vec());
            let cancelled = AtomicBool::new(false);
            let size = content.len() as u64;
            self.upload_from_reader(path, size, &mut cursor, &cancelled, |_| {})
                .await?;
            return Ok(());
        }

        let header = SftpMessage {
            id: self.take_id(),
            msg_type: MSG_SFTP_DATA.to_string(),
            cmd: "save".to_string(),
            data: json!({ "path": path, "size": content.len(), "force": true }).to_string(),
            ..Default::default()
        };
        self.send_binary(&header, content).await?;
        self.await_response(&header.id).await?;
        Ok(())
    }

    /// 关闭连接（发送 CLOSE 后断开，失败可忽略）。
    pub async fn close(&mut self) {
        let msg = SftpMessage {
            id: self.take_id(),
            msg_type: "CLOSE".to_string(),
            ..Default::default()
        };
        let _ = self.send_text(&msg).await;
        let _ = self.stream.close(None).await;
    }
}

fn ensure_ok(msg: SftpMessage) -> Result<SftpMessage, JmsError> {
    if msg.err.is_empty() {
        return Ok(msg);
    }
    let code = if msg.error_code.is_empty() {
        String::new()
    } else {
        format!(" [{}]", msg.error_code)
    };
    Err(JmsError::ApiError(format!(
        "SFTP 操作失败{code}: {}",
        msg.err
    )))
}

fn parse_binary_frame(bytes: &[u8]) -> Result<(SftpMessage, Vec<u8>), JmsError> {
    if bytes.len() < SFTP_FRAME_HEADER_SIZE {
        return Err(JmsError::ParseError("SFTP 二进制帧过短".to_string()));
    }
    let json_len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    let header_end = SFTP_FRAME_HEADER_SIZE + json_len;
    if header_end > bytes.len() {
        return Err(JmsError::ParseError("SFTP 二进制帧头长度越界".to_string()));
    }
    let header: SftpMessage = serde_json::from_slice(&bytes[SFTP_FRAME_HEADER_SIZE..header_end])
        .map_err(|e| JmsError::ParseError(format!("解析 SFTP 帧头失败: {e}")))?;
    Ok((header, bytes[header_end..].to_vec()))
}

fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(index) => &path[..index],
        None => ".",
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_frame_roundtrip() {
        let header = SftpMessage {
            id: "7".to_string(),
            msg_type: MSG_SFTP_BINARY.to_string(),
            cmd: "transfer_read".to_string(),
            data: r#"{"offset":0,"eof":false}"#.to_string(),
            ..Default::default()
        };
        let raw = b"hello world".to_vec();
        let json = serde_json::to_vec(&header).unwrap();
        let mut frame = Vec::new();
        frame.push(SFTP_FRAME_VERSION);
        frame.extend_from_slice(&(json.len() as u32).to_be_bytes());
        frame.extend_from_slice(&json);
        frame.extend_from_slice(&raw);

        let (parsed, payload) = parse_binary_frame(&frame).expect("应能解析");
        assert_eq!(parsed.id, "7");
        assert_eq!(parsed.cmd, "transfer_read");
        assert_eq!(payload, raw);
    }

    #[test]
    fn file_entry_accepts_string_numbers() {
        let json = r#"[{"name":"a.log","size":"1024","perm":"-rw-r--r--","mod_time":"1700000000","type":"file","is_dir":false}]"#;
        let entries: Vec<SftpFileEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries[0].name, "a.log");
        assert_eq!(entries[0].size, 1024);
        assert_eq!(entries[0].mod_time, 1_700_000_000);
        assert!(!entries[0].is_dir);
    }

    /// 真机探针：验证目标 JumpServer 的 `/koko/ws/sftp/` 可用。
    ///
    /// 需要环境变量（不设就会跳过）：
    /// - `JMS_PROBE_BASE_URL`     如 https://jms.example.com
    /// - `JMS_PROBE_COOKIE`       jms_sessionid 的值
    /// - `JMS_PROBE_ORG`          组织 ID（通常 default）
    /// - `JMS_PROBE_ASSET_ID`     资产 UUID
    ///
    /// 运行：`cargo test -p jms probe_websftp_list -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "需要真实 JumpServer 环境变量"]
    async fn probe_websftp_list() {
        let Ok(base_url) = std::env::var("JMS_PROBE_BASE_URL") else {
            eprintln!("跳过：未设置 JMS_PROBE_BASE_URL");
            return;
        };
        let cookie = std::env::var("JMS_PROBE_COOKIE").unwrap_or_default();
        let org = std::env::var("JMS_PROBE_ORG").unwrap_or_else(|_| "default".to_string());
        let asset_id = std::env::var("JMS_PROBE_ASSET_ID").unwrap_or_default();

        let params = crate::KokoConnectParams {
            base_url,
            token_id: String::new(),
            session_cookie: if cookie.is_empty() {
                None
            } else {
                Some(cookie)
            },
            org_id: org,
            proxy: None,
            title: "sftp-probe".to_string(),
            asset_id: if asset_id.is_empty() {
                None
            } else {
                Some(asset_id)
            },
            account_name: None,
            account_id: None,
            ticket: None,
        };

        let mut channel = WebSftpChannel::connect(&params)
            .await
            .expect("Web SFTP 连接失败（检查版本/路由/鉴权）");
        let (current_path, entries) = channel.list("/").await.expect("list 失败");
        eprintln!("cwd = {current_path}, entries = {}", entries.len());
        for entry in entries.iter().take(10) {
            eprintln!(
                "  {}  {}  {}  {}",
                if entry.is_dir { "d" } else { "-" },
                entry.size,
                entry.mod_time,
                entry.name
            );
        }
        channel.close().await;
    }

    #[test]
    fn path_helpers_work_for_root_and_nested() {
        assert_eq!(parent_dir("/a/b.txt"), "/a");
        assert_eq!(parent_dir("/a"), "/");
        assert_eq!(file_name("/a/b.txt"), "b.txt");
    }
}
