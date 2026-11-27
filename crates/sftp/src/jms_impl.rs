//! JumpServer（Koko Web SFTP）实现的 `SftpClient`。
//!
//! 走 koko 的 `/koko/ws/sftp/` WebSocket 通道（见 `jms::WebSftpChannel`），
//! 能力受服务端限制：
//! - 不支持 `chmod`（服务端没有该命令）
//! - 不支持跨目录移动（`rename` 只接受同目录下的新名字）
//! - 下载/上传按 2 MiB 分块，可取消
//!
//! 目录列表条目自带大小与修改时间，无需额外的 stat。

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use ssh::SshConnectConfig;
use tokio::io::AsyncWriteExt;

use jms::{JmsError, KokoConnectParams, WebSftpChannel};

use crate::{FileEntry, ProgressCallback, SftpClient, TransferCancelled, TransferProgress};

/// JMS/Koko 的 SFTP 客户端
pub struct JmsSftpClient {
    channel: WebSftpChannel,
}

/// 把远端时间戳归一化为「秒」。
///
/// koko 各版本/不同后端返回的单位并不统一（秒 / 毫秒 / 纳秒），
/// 落到合理区间（1970..2100）之外的值一律当作未知（0），避免 UI 显示天文数字。
fn normalize_unix_seconds(value: u64) -> u64 {
    const MAX_PLAUSIBLE: u64 = 4_102_444_800; // 2100-01-01

    if value == 0 || value <= MAX_PLAUSIBLE {
        return value;
    }

    // 按数量级判断单位，避免「微秒值除以 1e9」这类误判
    let divisor = if value >= 100_000_000_000_000_000 {
        1_000_000_000 // 纳秒
    } else if value >= 100_000_000_000_000 {
        1_000_000 // 微秒
    } else if value >= 100_000_000_000 {
        1_000 // 毫秒
    } else {
        return 0;
    };

    let seconds = value / divisor;
    if (1..=MAX_PLAUSIBLE).contains(&seconds) {
        seconds
    } else {
        0
    }
}

fn map_err(context: &str, err: JmsError) -> anyhow::Error {
    match err {
        JmsError::Cancelled => anyhow::Error::new(TransferCancelled),
        other => anyhow!("{context}: {other}"),
    }
}

/// 把 `ls -l` 风格的权限串（`drwxr-xr-x`）转成 mode 位。
fn parse_perm(perm: &str) -> u32 {
    let bytes = perm.as_bytes();
    if bytes.len() < 10 {
        return 0;
    }
    let mut mode = 0u32;
    for (index, bit) in [
        0o400u32, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
    ]
    .into_iter()
    .enumerate()
    {
        if bytes[index + 1] != b'-' {
            mode |= bit;
        }
    }
    mode
}

fn join_path(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(index) => &path[..index],
        None => ".",
    }
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl JmsSftpClient {
    /// 用已有的 Koko 连接参数建立 Web SFTP 会话。
    pub async fn connect_jms(params: KokoConnectParams) -> Result<Self> {
        let channel = WebSftpChannel::connect(&params)
            .await
            .map_err(|err| anyhow!("建立 JMS SFTP 通道失败: {err}"))?;
        Ok(Self { channel })
    }

    /// 递归列出目录（供下载目录/删除目录使用）。
    async fn walk_dir(
        &mut self,
        path: &str,
        cancelled: &Arc<AtomicBool>,
        entries: &mut Vec<FileEntry>,
        visited: &mut usize,
    ) -> Result<()> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(TransferCancelled));
        }
        let children = self
            .list_dir(path)
            .await
            .map_err(|err| anyhow!("列出 {} 失败: {err}", path))?;

        let dirs: Vec<FileEntry> = children
            .iter()
            .filter(|entry| entry.is_dir)
            .cloned()
            .collect();
        entries.extend(children);
        *visited += 1;
        if *visited > 20_000 {
            return Err(anyhow!("目录层级过深或文件过多，已停止递归"));
        }

        for dir in dirs {
            Box::pin(self.walk_dir(&dir.path, cancelled, entries, visited)).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl SftpClient for JmsSftpClient {
    async fn connect(_ssh_config: SshConnectConfig) -> Result<Self> {
        Err(anyhow!("JmsSftpClient 不支持 SSH 直连，请使用 connect_jms"))
    }

    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>> {
        let (current_path, entries) = self
            .channel
            .list(path)
            .await
            .map_err(|err| map_err("列出目录失败", err))?;

        Ok(entries
            .into_iter()
            .filter(|entry| entry.name != "." && entry.name != "..")
            .map(|entry| {
                let seconds = normalize_unix_seconds(entry.mod_time);
                if seconds != entry.mod_time {
                    tracing::debug!(
                        target: "sftp.jms",
                        name = %entry.name,
                        raw_mod_time = entry.mod_time,
                        normalized = seconds,
                        "JMS 返回的修改时间单位异常，已归一化"
                    );
                }
                FileEntry {
                    path: join_path(&current_path, &entry.name),
                    name: entry.name,
                    size: entry.size,
                    modified: UNIX_EPOCH + Duration::from_secs(seconds),
                    is_dir: entry.is_dir,
                    permissions: parse_perm(&entry.perm),
                }
            })
            .collect())
    }

    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        if let Some(parent) = Path::new(local_path).parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        // 远端大小：通过父目录列表获取，用于进度显示
        let total = self
            .list_dir(parent_of(remote_path))
            .await
            .ok()
            .and_then(|entries| {
                entries
                    .into_iter()
                    .find(|entry| entry.name == base_name(remote_path))
                    .map(|entry| entry.size)
            })
            .unwrap_or(0);

        let mut file = tokio::fs::File::create(local_path)
            .await
            .map_err(|err| anyhow!("创建本地文件失败 {}: {err}", local_path))?;

        let started = Instant::now();
        let name = base_name(remote_path).to_string();
        let mut last_report = Instant::now() - Duration::from_secs(1);
        let writer = &mut file;
        let path_for_log = remote_path.to_string();
        let result = self
            .channel
            .download_to_writer(path_for_log.as_str(), writer, &cancelled, |done| {
                if last_report.elapsed() < Duration::from_millis(100) {
                    return;
                }
                last_report = Instant::now();
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                progress(TransferProgress {
                    transferred: done,
                    total,
                    speed: done as f64 / elapsed,
                    current_file: Some(name.clone()),
                    current_file_transferred: done,
                    current_file_total: total,
                });
            })
            .await
            .map_err(|err| map_err("下载失败", err))?;

        file.flush()
            .await
            .map_err(|err| anyhow!("刷新本地文件失败 {}: {err}", local_path))?;
        progress(TransferProgress {
            transferred: result,
            total: total.max(result),
            speed: 0.0,
            current_file: Some(base_name(remote_path).to_string()),
            current_file_transferred: result,
            current_file_total: total.max(result),
        });
        Ok(())
    }

    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let mut file = tokio::fs::File::open(local_path)
            .await
            .map_err(|err| anyhow!("打开本地文件失败 {}: {err}", local_path))?;
        let size = file
            .metadata()
            .await
            .map_err(|err| anyhow!("读取本地文件信息失败 {}: {err}", local_path))?
            .len();

        let started = Instant::now();
        let name = base_name(local_path).to_string();
        let mut last_report = Instant::now() - Duration::from_secs(1);
        let uploaded = self
            .channel
            .upload_from_reader(remote_path, size, &mut file, &cancelled, |done| {
                if last_report.elapsed() < Duration::from_millis(100) {
                    return;
                }
                last_report = Instant::now();
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                progress(TransferProgress {
                    transferred: done,
                    total: size,
                    speed: done as f64 / elapsed,
                    current_file: Some(name.clone()),
                    current_file_transferred: done,
                    current_file_total: size,
                });
            })
            .await
            .map_err(|err| map_err("上传失败", err))?;

        progress(TransferProgress {
            transferred: uploaded,
            total: size,
            speed: 0.0,
            current_file: Some(base_name(local_path).to_string()),
            current_file_transferred: uploaded,
            current_file_total: size,
        });
        Ok(())
    }

    async fn delete(&mut self, path: &str, _is_dir: bool) -> Result<()> {
        self.channel
            .remove(path)
            .await
            .map_err(|err| map_err("删除失败", err))
    }

    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        // 服务端 `rm` 对目录是递归删除的，这里先统计条目数用于进度展示
        let mut entries = Vec::new();
        let mut visited = 0usize;
        if let Ok(()) = self
            .walk_dir(path, &cancelled, &mut entries, &mut visited)
            .await
        {
            progress(TransferProgress {
                transferred: entries.len() as u64,
                total: entries.len() as u64,
                speed: 0.0,
                current_file: Some(path.to_string()),
                current_file_transferred: entries.len() as u64,
                current_file_total: entries.len() as u64,
            });
        }
        self.channel
            .remove(path)
            .await
            .map_err(|err| map_err("删除目录失败", err))
    }

    async fn mkdir(&mut self, path: &str) -> Result<()> {
        self.channel
            .mkdir(path)
            .await
            .map_err(|err| map_err("创建目录失败", err))
    }

    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        if parent_of(old_path) != parent_of(new_path) {
            return Err(anyhow!(
                "JMS Web SFTP 不支持跨目录移动（{} → {}）",
                old_path,
                new_path
            ));
        }
        self.channel
            .rename(old_path, base_name(new_path))
            .await
            .map_err(|err| map_err("重命名失败", err))
    }

    async fn chmod(&mut self, _path: &str, _mode: u32) -> Result<()> {
        Err(anyhow!("JMS Web SFTP 不支持修改文件权限"))
    }

    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.channel
            .read_file(path, max_bytes)
            .await
            .map_err(|err| map_err("读取文件失败", err))
    }

    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()> {
        self.channel
            .write_file(path, content)
            .await
            .map_err(|err| map_err("写入文件失败", err))
    }

    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>> {
        let mut entries = Vec::new();
        let mut visited = 0usize;
        self.walk_dir(path, &cancelled, &mut entries, &mut visited)
            .await?;
        Ok(entries)
    }

    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let entries = self
            .list_dir_recursive(remote_path, cancelled.clone())
            .await?;
        let files: Vec<FileEntry> = entries.into_iter().filter(|entry| !entry.is_dir).collect();
        let progress: Arc<dyn Fn(TransferProgress) + Send + Sync> = Arc::from(progress);
        let total: u64 = files.iter().map(|entry| entry.size).sum();
        let mut transferred = 0u64;

        for entry in files {
            if cancelled.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(TransferCancelled));
            }
            let relative = entry
                .path
                .strip_prefix(remote_path)
                .unwrap_or(&entry.path)
                .trim_start_matches('/')
                .to_string();
            let target = Path::new(local_path).join(&relative);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            let done_before = transferred;
            let total_sum = total;
            let name = entry.name.clone();
            let cancelled_inner = cancelled.clone();
            let progress_for_file = progress.clone();
            self.download_with_progress(
                &entry.path,
                &target.to_string_lossy(),
                cancelled_inner,
                Box::new(move |item| {
                    progress_for_file(TransferProgress {
                        transferred: done_before + item.current_file_transferred,
                        total: total_sum,
                        speed: item.speed,
                        current_file: Some(name.clone()),
                        current_file_transferred: item.current_file_transferred,
                        current_file_total: item.current_file_total,
                    });
                }),
            )
            .await?;
            transferred += entry.size;
        }
        Ok(())
    }

    async fn upload_dir_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let base = Path::new(local_path).to_path_buf();
        let progress: Arc<dyn Fn(TransferProgress) + Send + Sync> = Arc::from(progress);
        let mut queue: VecDeque<(std::path::PathBuf, String)> = VecDeque::new();
        queue.push_back((base.clone(), remote_path.to_string()));

        while let Some((dir, remote_dir)) = queue.pop_front() {
            if cancelled.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(TransferCancelled));
            }
            if dir != base {
                let _ = self.channel.mkdir(&remote_dir).await;
            }

            let mut reader = tokio::fs::read_dir(&dir)
                .await
                .map_err(|err| anyhow!("读取本地目录失败 {}: {err}", dir.display()))?;
            while let Some(entry) = reader
                .next_entry()
                .await
                .map_err(|err| anyhow!("遍历本地目录失败: {err}"))?
            {
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|err| anyhow!("读取文件类型失败: {err}"))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let remote_child = join_path(&remote_dir, &name);
                if file_type.is_dir() {
                    queue.push_back((entry.path(), remote_child));
                    continue;
                }

                let progress_for_file = progress.clone();
                self.upload_with_progress(
                    &entry.path().to_string_lossy(),
                    &remote_child,
                    cancelled.clone(),
                    Box::new(move |item| progress_for_file(item)),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.channel.close().await;
        Ok(())
    }

    async fn realpath(&mut self, path: &str) -> Result<String> {
        self.channel
            .realpath(path)
            .await
            .map_err(|err| map_err("解析路径失败", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perm_string_maps_to_mode_bits() {
        assert_eq!(parse_perm("drwxr-xr-x"), 0o755);
        assert_eq!(parse_perm("-rw-r--r--"), 0o644);
        assert_eq!(parse_perm("-rw-------"), 0o600);
        assert_eq!(parse_perm(""), 0);
    }

    #[test]
    fn normalize_unix_seconds_handles_common_units() {
        // 秒：保持原样
        assert_eq!(normalize_unix_seconds(1_700_000_000), 1_700_000_000);
        // 毫秒 / 微秒 / 纳秒：换算成秒
        assert_eq!(normalize_unix_seconds(1_700_000_000_000), 1_700_000_000);
        assert_eq!(normalize_unix_seconds(1_700_000_000_000_000), 1_700_000_000);
        assert_eq!(
            normalize_unix_seconds(1_700_000_000_000_000_000),
            1_700_000_000
        );
        // 无法解释的值：当作未知
        assert_eq!(normalize_unix_seconds(6_582_955_728_264_977_243), 0);
        assert_eq!(normalize_unix_seconds(0), 0);
    }

    #[test]
    fn path_helpers_handle_root_and_nested() {
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("/var/log", "app.log"), "/var/log/app.log");
        assert_eq!(parent_of("/var/log/app.log"), "/var/log");
        assert_eq!(parent_of("/var"), "/");
        assert_eq!(base_name("/var/log/app.log"), "app.log");
    }
}
