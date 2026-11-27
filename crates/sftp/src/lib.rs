rust_i18n::i18n!("locales", fallback = "en");

mod jms_impl;
mod russh_impl;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use ssh::{RusshClient, SshConnectConfig};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;
use tokio::sync::Mutex;

pub use jms_impl::JmsSftpClient;
pub use russh_impl::RusshSftpClient;

/// SFTP 连接来源：SSH 直连，或 JumpServer（Koko Web SFTP）。
///
/// 侧边栏文件管理器只依赖这个枚举，不用关心底层是哪种通道。
pub enum SftpConnection {
    Ssh(RusshSftpClient),
    Jms(JmsSftpClient),
}

impl SftpConnection {
    /// 用 SSH 配置新建直连 SFTP（独立连接）。
    pub async fn connect_ssh(config: SshConnectConfig) -> Result<Self> {
        Ok(Self::Ssh(RusshSftpClient::connect(config).await?))
    }

    /// 复用已有的 SSH 会话打开 SFTP 通道。
    pub async fn from_shared_ssh(client: Arc<Mutex<RusshClient>>) -> Result<Self> {
        Ok(Self::Ssh(
            RusshSftpClient::connect_with_client(client).await?,
        ))
    }

    /// 建立 JumpServer Web SFTP 通道。
    pub async fn connect_jms(params: jms::KokoConnectParams) -> Result<Self> {
        Ok(Self::Jms(JmsSftpClient::connect_jms(params).await?))
    }

    pub fn is_jms(&self) -> bool {
        matches!(self, Self::Jms(_))
    }

    /// 是否支持修改权限（JMS Web SFTP 不支持）。
    pub fn supports_chmod(&self) -> bool {
        matches!(self, Self::Ssh(_))
    }
}

#[async_trait]
impl SftpClient for SftpConnection {
    async fn connect(_ssh_config: SshConnectConfig) -> Result<Self> {
        Err(anyhow!(
            "SftpConnection 请使用 connect_ssh / connect_jms / from_shared_ssh"
        ))
    }

    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>> {
        match self {
            Self::Ssh(client) => client.list_dir(path).await,
            Self::Jms(client) => client.list_dir(path).await,
        }
    }

    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        match self {
            Self::Ssh(client) => {
                client
                    .download_with_progress(remote_path, local_path, cancelled, progress)
                    .await
            }
            Self::Jms(client) => {
                client
                    .download_with_progress(remote_path, local_path, cancelled, progress)
                    .await
            }
        }
    }

    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        match self {
            Self::Ssh(client) => {
                client
                    .upload_with_progress(local_path, remote_path, cancelled, progress)
                    .await
            }
            Self::Jms(client) => {
                client
                    .upload_with_progress(local_path, remote_path, cancelled, progress)
                    .await
            }
        }
    }

    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()> {
        match self {
            Self::Ssh(client) => client.delete(path, is_dir).await,
            Self::Jms(client) => client.delete(path, is_dir).await,
        }
    }

    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        match self {
            Self::Ssh(client) => client.delete_recursive(path, cancelled, progress).await,
            Self::Jms(client) => client.delete_recursive(path, cancelled, progress).await,
        }
    }

    async fn mkdir(&mut self, path: &str) -> Result<()> {
        match self {
            Self::Ssh(client) => client.mkdir(path).await,
            Self::Jms(client) => client.mkdir(path).await,
        }
    }

    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        match self {
            Self::Ssh(client) => client.rename(old_path, new_path).await,
            Self::Jms(client) => client.rename(old_path, new_path).await,
        }
    }

    async fn chmod(&mut self, path: &str, mode: u32) -> Result<()> {
        match self {
            Self::Ssh(client) => client.chmod(path, mode).await,
            Self::Jms(client) => client.chmod(path, mode).await,
        }
    }

    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        match self {
            Self::Ssh(client) => client.read_file(path, max_bytes).await,
            Self::Jms(client) => client.read_file(path, max_bytes).await,
        }
    }

    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()> {
        match self {
            Self::Ssh(client) => client.write_file(path, content).await,
            Self::Jms(client) => client.write_file(path, content).await,
        }
    }

    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>> {
        match self {
            Self::Ssh(client) => client.list_dir_recursive(path, cancelled).await,
            Self::Jms(client) => client.list_dir_recursive(path, cancelled).await,
        }
    }

    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        match self {
            Self::Ssh(client) => {
                client
                    .download_dir_with_progress(remote_path, local_path, cancelled, progress)
                    .await
            }
            Self::Jms(client) => {
                client
                    .download_dir_with_progress(remote_path, local_path, cancelled, progress)
                    .await
            }
        }
    }

    async fn upload_dir_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        match self {
            Self::Ssh(client) => {
                client
                    .upload_dir_with_progress(local_path, remote_path, cancelled, progress)
                    .await
            }
            Self::Jms(client) => {
                client
                    .upload_dir_with_progress(local_path, remote_path, cancelled, progress)
                    .await
            }
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        match self {
            Self::Ssh(client) => client.disconnect().await,
            Self::Jms(client) => client.disconnect().await,
        }
    }

    async fn realpath(&mut self, path: &str) -> Result<String> {
        match self {
            Self::Ssh(client) => client.realpath(path).await,
            Self::Jms(client) => client.realpath(path).await,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: SystemTime,
    pub is_dir: bool,
    pub permissions: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TransferProgress {
    pub transferred: u64,
    pub total: u64,
    pub speed: f64,
    pub current_file: Option<String>,
    pub current_file_transferred: u64,
    pub current_file_total: u64,
}

#[derive(Debug)]
pub struct TransferCancelled;

impl std::fmt::Display for TransferCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cancelled")
    }
}

impl std::error::Error for TransferCancelled {}

pub type ProgressCallback = Box<dyn Fn(TransferProgress) + Send + Sync + 'static>;

pub(crate) fn validate_read_size(file_size: usize, max_bytes: usize) -> Result<()> {
    if file_size > max_bytes {
        return Err(anyhow!(
            "Remote file size {} exceeds max readable size {}",
            file_size,
            max_bytes
        ));
    }
    Ok(())
}

#[async_trait]
pub trait SftpClient: Send + Sync {
    async fn connect(ssh_config: SshConnectConfig) -> Result<Self>
    where
        Self: Sized;

    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>>;

    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()>;

    /// 递归删除目录及其所有内容，带进度回调
    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn mkdir(&mut self, path: &str) -> Result<()>;

    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()>;

    async fn chmod(&mut self, path: &str, mode: u32) -> Result<()>;

    /// 读取远程文件内容，超过 max_bytes 时返回错误。
    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>>;

    /// 写入文件内容（用于创建新文件或覆盖文件）
    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()>;

    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>>;

    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn upload_dir_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn disconnect(&mut self) -> Result<()>;

    /// 获取路径的真实绝对路径
    async fn realpath(&mut self, path: &str) -> Result<String>;
}

#[cfg(test)]
mod tests {
    use super::validate_read_size;

    #[test]
    fn validate_read_size_allows_files_within_limit() {
        assert!(validate_read_size(1024, 1024).is_ok());
        assert!(validate_read_size(512, 1024).is_ok());
    }

    #[test]
    fn validate_read_size_rejects_files_larger_than_limit() {
        let error = validate_read_size(1025, 1024).expect_err("应拒绝超限文件");
        assert!(error.to_string().contains("exceeds max readable size"));
    }
}
