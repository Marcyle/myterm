//! ZMODEM（`rz` / `sz`）文件传输支持。
//!
//! 终端输出中出现 `**` `ZDLE` `B` 开头的 ZMODEM 十六进制帧头时，判定对端启动了
//! `rz`（远端接收 = 上传）或 `sz`（远端发送 = 下载），随后把该会话期间的字节流
//! 从「终端渲染」通道切到「协议」通道，直到会话结束。
//!
//! 使用方式：
//! - 数据通路（本地 PTY reader / SSH recv 循环 / Koko recv 循环）持有一个
//!   [`ZmodemScanner`]，每批字节先 `scan` 再交给 alacritty 的 parser；
//! - 通过 [`ZmodemState`] 向协议线程回写字节（写回 PTY / SSH channel）；
//! - 协议线程把进度、完成、失败等事件通过事件回调投递给 UI。

use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver as MpscReceiver, RecvTimeoutError, Sender as MpscSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use zmodem2::{Receiver as ZmodemReceiver, ReceiverEvent, Sender as ZmodemSender, SenderEvent};

/// ZMODEM 帧头前缀：`*` `*` ZDLE。
const START_PREFIX: &[u8] = b"**\x18";
/// 帧头标记字节（十六进制帧）。
const HEX_FRAME_MARK: u8 = b'B';
/// 协议线程轮询间隔。
const TICK: Duration = Duration::from_millis(20);
/// 传输过程中最长无数据时间。
const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// 等待用户在界面上选择上传文件的最长时间。
const UPLOAD_PICK_TIMEOUT: Duration = Duration::from_secs(120);
/// 进度事件节流间隔。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);
/// 取消时发送的 CAN 序列（与 lrzsz 的 `canit()` 一致）。
const ABORT_SEQUENCE: [u8; 20] = [
    0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
    0x08, 0x08, 0x08, 0x08,
];

/// 传输方向（站在本机的角度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmodemDirection {
    /// 远端 `sz`，本机接收（下载）。
    Download,
    /// 远端 `rz`，本机发送（上传）。
    Upload,
}

impl ZmodemDirection {
    pub fn is_download(self) -> bool {
        matches!(self, ZmodemDirection::Download)
    }
}

/// 上传文件选择的一次性应答通道。
type UploadPickSender = oneshot::Sender<Option<Vec<PathBuf>>>;
/// 供事件跨线程携带、可重复持有的应答槽位。
type SharedUploadPickSender = Arc<Mutex<Option<UploadPickSender>>>;

/// UI 侧对上传请求的应答句柄。
#[derive(Clone)]
pub struct ZmodemUploadRequest {
    pub session: u64,
    responder: SharedUploadPickSender,
}

impl ZmodemUploadRequest {
    fn new(session: u64, sender: UploadPickSender) -> Self {
        Self {
            session,
            responder: Arc::new(Mutex::new(Some(sender))),
        }
    }

    /// 回传用户选择的文件；`None` 表示取消上传。
    pub fn respond(&self, files: Option<Vec<PathBuf>>) {
        if let Ok(mut guard) = self.responder.lock()
            && let Some(sender) = guard.take()
        {
            let _ = sender.send(files);
        }
    }
}

impl fmt::Debug for ZmodemUploadRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZmodemUploadRequest")
            .field("session", &self.session)
            .finish()
    }
}

/// ZMODEM 会话事件（协议线程 -> UI）。
#[derive(Debug, Clone)]
pub enum ZmodemEvent {
    /// 远端 `rz` 等待接收，需要界面选择上传文件。
    UploadRequest(ZmodemUploadRequest),
    /// 开始传输一个文件。
    Started {
        session: u64,
        direction: ZmodemDirection,
        file_name: String,
        file_size: Option<u64>,
    },
    /// 传输进度。
    Progress {
        session: u64,
        direction: ZmodemDirection,
        file_name: String,
        transferred: u64,
        total: Option<u64>,
    },
    /// 会话成功结束。
    Finished {
        session: u64,
        direction: ZmodemDirection,
        files: usize,
        /// 下载时的保存目录。
        location: Option<PathBuf>,
    },
    /// 会话失败或被取消。
    Failed {
        session: u64,
        direction: ZmodemDirection,
        message: String,
    },
}

/// 向终端回写协议字节的回调。
pub type ZmodemWriteFn = Arc<dyn Fn(&[u8]) + Send + Sync + 'static>;
/// 事件回调。
pub type ZmodemEventFn = Arc<dyn Fn(ZmodemEvent) + Send + Sync + 'static>;

/// 默认下载目录。
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// ZMODEM 运行期共享状态：数据通路与协议线程之间唯一的交接点。
pub struct ZmodemState {
    write: ZmodemWriteFn,
    emit: ZmodemEventFn,
    download_dir: PathBuf,
    feed: Mutex<Option<MpscSender<Vec<u8>>>>,
    cancel_flag: Mutex<Option<Arc<AtomicBool>>>,
    session_seq: AtomicU64,
    active: AtomicBool,
}

impl ZmodemState {
    pub fn new(write: ZmodemWriteFn, emit: ZmodemEventFn, download_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            write,
            emit,
            download_dir,
            feed: Mutex::new(None),
            cancel_flag: Mutex::new(None),
            session_seq: AtomicU64::new(0),
            active: AtomicBool::new(false),
        })
    }

    /// 当前是否处于 ZMODEM 会话中。
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn download_dir(&self) -> &Path {
        &self.download_dir
    }

    /// 请求取消当前会话（用户输入任意按键时调用）。
    pub fn cancel(&self) {
        if let Ok(guard) = self.cancel_flag.lock()
            && let Some(flag) = guard.as_ref()
        {
            flag.store(true, Ordering::Release);
        }
    }

    /// 数据通路把捕获到的协议字节送进会话。
    pub fn feed(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if let Ok(guard) = self.feed.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(data.to_vec());
        }
    }

    fn begin(self: &Arc<Self>, direction: ZmodemDirection, initial: Vec<u8>) -> bool {
        let mut guard = match self.feed.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.is_some() {
            return false;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        *guard = Some(tx.clone());
        let id = self.session_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut flag) = self.cancel_flag.lock() {
            *flag = Some(cancel.clone());
        }
        self.active.store(true, Ordering::Release);
        drop(guard);

        // 会话线程需要按构建顺序拿到 state；这里先喂初始字节，线程启动后即可读到。
        let _ = tx.send(initial);

        let write = self.write.clone();
        let emit = self.emit.clone();
        let download_dir = self.download_dir.clone();
        let state_weak = Arc::downgrade(self);

        tracing::info!(
            target: "terminal.zmodem",
            session = id,
            direction = ?direction,
            "zmodem session starting"
        );

        let spawn_result = std::thread::Builder::new()
            .name(format!("zmodem-{id}"))
            .spawn(move || {
                let started = Instant::now();
                let result = match direction {
                    ZmodemDirection::Download => {
                        run_download(&write, &emit, id, &download_dir, rx, cancel.clone())
                    }
                    ZmodemDirection::Upload => run_upload(&write, &emit, id, rx, cancel.clone()),
                };
                match result {
                    Ok(outcome) => {
                        let elapsed = started.elapsed();
                        let mbps = if elapsed.as_secs_f64() > 0.0 {
                            outcome.bytes as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64()
                        } else {
                            0.0
                        };
                        tracing::info!(
                            target: "terminal.zmodem",
                            session = id,
                            direction = ?direction,
                            files = outcome.files,
                            bytes = outcome.bytes,
                            elapsed_ms = elapsed.as_millis() as u64,
                            mb_per_s = format!("{mbps:.2}"),
                            "zmodem session finished"
                        );
                        emit(ZmodemEvent::Finished {
                            session: id,
                            direction,
                            files: outcome.files,
                            location: outcome.location,
                        });
                    }
                    Err(message) => {
                        tracing::warn!(
                            target: "terminal.zmodem",
                            session = id,
                            error = %message,
                            "zmodem session failed"
                        );
                        write(&ABORT_SEQUENCE);
                        emit(ZmodemEvent::Failed {
                            session: id,
                            direction,
                            message,
                        });
                    }
                }
                if let Some(state) = state_weak.upgrade() {
                    state.end();
                }
            });

        if let Err(err) = spawn_result {
            // 线程起不来时必须回滚，否则后续输出会被永久吞掉。
            tracing::error!(
                target: "terminal.zmodem",
                session = id,
                error = %err,
                "failed to spawn zmodem session thread"
            );
            if let Ok(mut guard) = self.feed.lock() {
                *guard = None;
            }
            if let Ok(mut flag) = self.cancel_flag.lock() {
                *flag = None;
            }
            self.active.store(false, Ordering::Release);
            return false;
        }

        true
    }

    fn end(&self) {
        if let Ok(mut guard) = self.feed.lock() {
            *guard = None;
        }
        if let Ok(mut flag) = self.cancel_flag.lock() {
            *flag = None;
        }
        self.active.store(false, Ordering::Release);
    }
}

impl fmt::Debug for ZmodemState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZmodemState")
            .field("active", &self.is_active())
            .field("download_dir", &self.download_dir)
            .finish()
    }
}

/// 数据通路侧的 ZMODEM 探测/截流器。
#[derive(Default)]
pub struct ZmodemScanner {
    /// 可能是半个帧头、暂缓交给终端渲染的字节。
    tail: Vec<u8>,
}

impl ZmodemScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// 扫描一批终端输出，返回应交给终端渲染的字节。
    ///
    /// 命中 ZMODEM 帧头后，其后的字节全部转交给会话线程。
    pub fn scan(&mut self, state: &Arc<ZmodemState>, chunk: &[u8]) -> Vec<u8> {
        if chunk.is_empty() {
            return Vec::new();
        }

        if state.is_active() {
            state.feed(chunk);
            return Vec::new();
        }

        // 快速路径：没有 `*` 就不可能命中帧头，直接放行（正常输出占绝大多数）。
        if self.tail.is_empty() && !chunk.contains(&b'*') {
            return chunk.to_vec();
        }

        let mut data = std::mem::take(&mut self.tail);
        data.extend_from_slice(chunk);

        match locate_start(&data) {
            Locate::Found(pos) => {
                let direction = frame_direction(&data[pos + START_PREFIX.len() + 1..]);
                let captured = data.split_off(pos);
                // 余下字节（含帧头）属于协议会话。
                let started = match direction {
                    Some(direction) => state.begin(direction, captured.clone()),
                    None => false,
                };
                if !started {
                    // 识别不出方向或会话已在运行：原样交给终端。
                    let mut passthrough = data;
                    passthrough.extend_from_slice(&captured);
                    passthrough
                } else {
                    data
                }
            }
            Locate::HoldFrom(pos) => {
                self.tail = data.split_off(pos);
                data
            }
            Locate::Pass => {
                let keep = partial_prefix_len(&data);
                if keep > 0 {
                    self.tail = data.split_off(data.len() - keep);
                }
                data
            }
        }
    }

    /// 连接结束时取出暂缓的字节，避免内容丢失。
    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.tail)
    }
}

enum Locate {
    /// 完整帧头起始位置。
    Found(usize),
    /// 从该位置起可能是被截断的帧头。
    HoldFrom(usize),
    /// 未发现。
    Pass,
}

fn locate_start(data: &[u8]) -> Locate {
    let mut hold: Option<usize> = None;
    let mut idx = 0usize;

    while idx < data.len() {
        let Some(rel) = find_subslice(&data[idx..], START_PREFIX) else {
            break;
        };
        let pos = idx + rel;
        let rest = &data[pos + START_PREFIX.len()..];

        if rest.is_empty() {
            hold = Some(hold.map_or(pos, |held: usize| held.min(pos)));
            break;
        }

        if rest[0] != HEX_FRAME_MARK {
            idx = pos + 1;
            continue;
        }

        let hex_tail = &rest[1..];
        let mut hex = 0usize;
        for &byte in hex_tail.iter().take(4) {
            if byte.is_ascii_hexdigit() {
                hex += 1;
            } else {
                break;
            }
        }

        if hex == 4 {
            return Locate::Found(pos);
        }
        if hex == hex_tail.len() && hex_tail.len() < 4 {
            // 帧头被截断，等下一批数据再判断。
            hold = Some(hold.map_or(pos, |held: usize| held.min(pos)));
        }
        idx = pos + 1;
    }

    // 数据尾部可能是 START_PREFIX 的前缀。
    let max_keep = START_PREFIX.len().saturating_sub(1).min(data.len());
    for keep in (1..=max_keep).rev() {
        if data[data.len() - keep..] == START_PREFIX[..keep] {
            let pos = data.len() - keep;
            hold = Some(hold.map_or(pos, |held: usize| held.min(pos)));
            break;
        }
    }

    match hold {
        Some(pos) => Locate::HoldFrom(pos),
        None => Locate::Pass,
    }
}

fn partial_prefix_len(data: &[u8]) -> usize {
    let max_keep = START_PREFIX.len().saturating_sub(1).min(data.len());
    for keep in (1..=max_keep).rev() {
        if data[data.len() - keep..] == START_PREFIX[..keep] {
            return keep;
        }
    }
    0
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 由帧类型判断方向：`ZRQINIT`(0) = 远端发送，`ZRINIT`(1) = 远端接收。
fn frame_direction(frame: &[u8]) -> Option<ZmodemDirection> {
    if frame.len() < 2 {
        return None;
    }
    let hi = (frame[0] as char).to_digit(16)?;
    let lo = (frame[1] as char).to_digit(16)?;
    match (hi << 4) | lo {
        0 => Some(ZmodemDirection::Download),
        1 => Some(ZmodemDirection::Upload),
        _ => None,
    }
}

// ===========================================================================
// 协议线程
// ===========================================================================

struct SessionOutcome {
    files: usize,
    bytes: u64,
    location: Option<PathBuf>,
}

struct DownloadFile {
    path: PathBuf,
    name: String,
    file: std::io::BufWriter<File>,
    total: u64,
    written: u64,
}

struct UploadFile {
    path: PathBuf,
    name: String,
    file: File,
    size: u64,
    sent: u64,
    /// 当前文件读指针，避免每个子包都 seek
    position: u64,
}

fn emit_progress(
    emit: &ZmodemEventFn,
    session: u64,
    direction: ZmodemDirection,
    name: &str,
    transferred: u64,
    total: Option<u64>,
) {
    emit(ZmodemEvent::Progress {
        session,
        direction,
        file_name: name.to_string(),
        transferred,
        total,
    });
}

/// 下载：远端 `sz`。
fn run_download(
    write: &ZmodemWriteFn,
    emit: &ZmodemEventFn,
    session: u64,
    download_dir: &Path,
    rx: MpscReceiver<Vec<u8>>,
    cancel: Arc<AtomicBool>,
) -> Result<SessionOutcome, String> {
    std::fs::create_dir_all(download_dir)
        .map_err(|err| format!("无法创建下载目录 {}: {err}", download_dir.display()))?;

    let mut machine = ZmodemReceiver::new().map_err(|err| format!("初始化接收端失败: {err}"))?;
    let mut pending: Vec<u8> = Vec::new();
    let mut pending_pos = 0usize;
    let mut current: Option<DownloadFile> = None;
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut last_activity = Instant::now();
    let mut last_emit = Instant::now();

    loop {
        // 阶段 1：把能推进的工作一次做完。
        // 少了这一层，每处理完一个 1KB 子包就会阻塞等到下一个 TICK（20ms），
        // 吞吐会被压到 ~50KB/s —— 这正是之前「很慢」的主因。
        loop {
            let mut progressed = false;

            if cancel.load(Ordering::Acquire) {
                cleanup(&mut current);
                return Err("已取消".to_string());
            }

            // 1) 回写协议应答（ZRINIT 会被改写成流式声明）
            let outgoing_len = {
                let outgoing = machine.drain_outgoing();
                if outgoing.is_empty() {
                    0
                } else {
                    let len = outgoing.len();
                    let mut frame = outgoing.to_vec();
                    patch_zrinit_streaming(&mut frame);
                    write(&frame);
                    len
                }
            };
            if outgoing_len > 0 {
                machine.advance_outgoing(outgoing_len);
                progressed = true;
            }

            // 2) 落盘
            let chunk_len = {
                let chunk = machine.drain_file();
                let len = chunk.len();
                if len > 0
                    && let Some(file) = current.as_mut()
                {
                    file.file
                        .write_all(chunk)
                        .map_err(|err| format!("写入失败 {}: {err}", file.path.display()))?;
                    file.written += len as u64;
                }
                len
            };
            if chunk_len > 0 {
                machine
                    .advance_file(chunk_len)
                    .map_err(|err| format!("接收数据失败: {err}"))?;
                progressed = true;

                if last_emit.elapsed() >= PROGRESS_INTERVAL
                    && let Some(file) = current.as_ref()
                {
                    last_emit = Instant::now();
                    emit_progress(
                        emit,
                        session,
                        ZmodemDirection::Download,
                        &file.name,
                        file.written,
                        Some(file.total),
                    );
                }
            }

            // 3) 协议事件
            while let Some(event) = machine.poll_event() {
                progressed = true;
                match event {
                    ReceiverEvent::FileStart => {
                        let name = String::from_utf8_lossy(machine.file_name()).into_owned();
                        let total = machine.file_size() as u64;
                        let path = unique_download_path(download_dir, &name);
                        let file = File::create(&path)
                            .map_err(|err| format!("创建文件失败 {}: {err}", path.display()))?;
                        emit(ZmodemEvent::Started {
                            session,
                            direction: ZmodemDirection::Download,
                            file_name: name.clone(),
                            file_size: Some(total),
                        });
                        current = Some(DownloadFile {
                            path,
                            name,
                            file: std::io::BufWriter::with_capacity(256 * 1024, file),
                            total,
                            written: 0,
                        });
                    }
                    ReceiverEvent::FileComplete => {
                        if let Some(file) = current.take() {
                            let (name, written, total) =
                                (file.name.clone(), file.written, file.total);
                            finish_download_file(file)?;
                            bytes += written;
                            files += 1;
                            emit_progress(
                                emit,
                                session,
                                ZmodemDirection::Download,
                                &name,
                                written,
                                Some(total),
                            );
                        }
                    }
                    ReceiverEvent::SessionComplete => {
                        if let Some(file) = current.take() {
                            bytes += file.written;
                            finish_download_file(file)?;
                            files += 1;
                        }
                        return Ok(SessionOutcome {
                            files,
                            bytes,
                            location: Some(download_dir.to_path_buf()),
                        });
                    }
                }
            }

            // 4) 喂输入
            compact_pending(&mut pending, &mut pending_pos);
            if pending_pos < pending.len() {
                let consumed = machine
                    .feed_incoming(&pending[pending_pos..])
                    .map_err(|err| format!("接收数据失败: {err}"))?;
                if consumed > 0 {
                    pending_pos += consumed;
                    last_activity = Instant::now();
                    progressed = true;
                }
            }

            if !progressed {
                break;
            }
        }

        // 阶段 2：真正没有进展时才等待新数据。
        match rx.recv_timeout(TICK) {
            Ok(chunk) => {
                pending.extend_from_slice(&chunk);
                last_activity = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {
                if last_activity.elapsed() >= TRANSFER_IDLE_TIMEOUT {
                    cleanup(&mut current);
                    return Err("传输超时".to_string());
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                cleanup(&mut current);
                return Err("终端已断开".to_string());
            }
        }
    }
}

/// 上传：远端 `rz`。
fn run_upload(
    write: &ZmodemWriteFn,
    emit: &ZmodemEventFn,
    session: u64,
    rx: MpscReceiver<Vec<u8>>,
    cancel: Arc<AtomicBool>,
) -> Result<SessionOutcome, String> {
    let mut machine = ZmodemSender::new().map_err(|err| format!("初始化发送端失败: {err}"))?;

    let (picker_tx, mut picker_rx) = oneshot::channel::<Option<Vec<PathBuf>>>();
    emit(ZmodemEvent::UploadRequest(ZmodemUploadRequest::new(
        session, picker_tx,
    )));

    let mut queue: VecDeque<UploadFile> = VecDeque::new();
    let mut picks_ready = false;
    let mut current: Option<UploadFile> = None;
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut pending: Vec<u8> = Vec::new();
    let mut pending_pos = 0usize;
    let mut last_activity = Instant::now();
    let mut last_emit = Instant::now();
    let wait_started = Instant::now();
    let mut zrinit_logged = false;

    loop {
        // 阶段 1：能推进的全做完再阻塞，否则每个子包都会白等一个 TICK。
        loop {
            let mut progressed = false;

            if cancel.load(Ordering::Acquire) {
                return Err("已取消".to_string());
            }

            // 1) 回写协议应答
            let outgoing_len = {
                let outgoing = machine.drain_outgoing();
                if outgoing.is_empty() {
                    0
                } else {
                    write(outgoing);
                    outgoing.len()
                }
            };
            if outgoing_len > 0 {
                machine.advance_outgoing(outgoing_len);
                progressed = true;
            }

            // 2) 等待用户选择文件
            if !picks_ready {
                match picker_rx.try_recv() {
                    Ok(Some(paths)) => {
                        progressed = true;
                        if paths.is_empty() {
                            return Err("未选择文件".to_string());
                        }
                        for path in paths {
                            let meta = std::fs::metadata(&path)
                                .map_err(|err| format!("读取文件失败 {}: {err}", path.display()))?;
                            if !meta.is_file() {
                                continue;
                            }
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "upload.bin".to_string());
                            let file = File::open(&path)
                                .map_err(|err| format!("打开文件失败 {}: {err}", path.display()))?;
                            queue.push_back(UploadFile {
                                path,
                                name,
                                file,
                                size: meta.len(),
                                sent: 0,
                                position: 0,
                            });
                        }
                        if queue.is_empty() {
                            return Err("没有可上传的文件".to_string());
                        }
                        picks_ready = true;
                    }
                    Ok(None) => return Err("已取消上传".to_string()),
                    Err(oneshot::error::TryRecvError::Closed) => {
                        return Err("已取消上传".to_string());
                    }
                    Err(oneshot::error::TryRecvError::Empty) => {
                        if wait_started.elapsed() >= UPLOAD_PICK_TIMEOUT {
                            return Err("等待选择文件超时".to_string());
                        }
                    }
                }
            }

            // 3) 开始下一个文件
            if picks_ready
                && current.is_none()
                && let Some(file) = queue.pop_front()
            {
                progressed = true;
                machine
                    .start_file(file.name.as_bytes(), file.size.min(u32::MAX as u64) as u32)
                    .map_err(|err| format!("发送文件名失败: {err}"))?;
                emit(ZmodemEvent::Started {
                    session,
                    direction: ZmodemDirection::Upload,
                    file_name: file.name.clone(),
                    file_size: Some(file.size),
                });
                current = Some(file);
            }

            // 4) 满足数据请求
            if let Some(request) = machine.poll_file() {
                progressed = true;
                let file = current.as_mut().ok_or_else(|| "协议状态异常".to_string())?;
                let mut buf = vec![0u8; request.len];
                if file.position != u64::from(request.offset) {
                    file.file
                        .seek(SeekFrom::Start(u64::from(request.offset)))
                        .map_err(|err| format!("读取文件失败 {}: {err}", file.path.display()))?;
                    file.position = u64::from(request.offset);
                }
                let mut filled = 0usize;
                while filled < buf.len() {
                    let read = file
                        .file
                        .read(&mut buf[filled..])
                        .map_err(|err| format!("读取文件失败 {}: {err}", file.path.display()))?;
                    if read == 0 {
                        break;
                    }
                    filled += read;
                }
                if filled == 0 {
                    return Err(format!("文件已变化: {}", file.path.display()));
                }
                file.position += filled as u64;
                buf.truncate(filled);
                machine
                    .feed_file(&buf)
                    .map_err(|err| format!("发送数据失败: {err}"))?;
                file.sent += filled as u64;
                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    last_emit = Instant::now();
                    emit_progress(
                        emit,
                        session,
                        ZmodemDirection::Upload,
                        &file.name,
                        file.sent,
                        Some(file.size),
                    );
                }
            }

            // 5) 协议事件
            while let Some(event) = machine.poll_event() {
                progressed = true;
                match event {
                    SenderEvent::FileComplete => {
                        if let Some(file) = current.take() {
                            bytes += file.sent;
                            files += 1;
                            emit_progress(
                                emit,
                                session,
                                ZmodemDirection::Upload,
                                &file.name,
                                file.size,
                                Some(file.size),
                            );
                        }
                        if queue.is_empty() {
                            machine
                                .finish_session()
                                .map_err(|err| format!("结束会话失败: {err}"))?;
                        }
                    }
                    SenderEvent::SessionComplete => {
                        return Ok(SessionOutcome {
                            files,
                            bytes,
                            location: None,
                        });
                    }
                }
            }

            // 6) 输入
            compact_pending(&mut pending, &mut pending_pos);
            if pending_pos < pending.len() {
                if !zrinit_logged
                    && let Some((buffer_len, caps)) = find_remote_zrinit(&pending[pending_pos..])
                {
                    zrinit_logged = true;
                    tracing::info!(
                        target: "terminal.zmodem",
                        session,
                        buffer_len,
                        canovio = caps & 0x02 != 0,
                        canfdx = caps & 0x01 != 0,
                        "远端 rz 声明的接收窗口（决定上传分帧大小）"
                    );
                }
                let consumed = machine
                    .feed_incoming(&pending[pending_pos..])
                    .map_err(|err| format!("接收应答失败: {err}"))?;
                if consumed > 0 {
                    pending_pos += consumed;
                    last_activity = Instant::now();
                    progressed = true;
                }
            }

            if !progressed {
                break;
            }
        }

        // 阶段 2：确实没活干了才等新数据 / 等用户选文件
        if cancel.load(Ordering::Acquire) {
            return Err("已取消".to_string());
        }
        match rx.recv_timeout(TICK) {
            Ok(chunk) => {
                pending.extend_from_slice(&chunk);
                last_activity = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {
                if last_activity.elapsed() >= TRANSFER_IDLE_TIMEOUT {
                    return Err("传输超时".to_string());
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err("终端已断开".to_string());
            }
        }
    }
}

fn finish_download_file(file: DownloadFile) -> Result<(), String> {
    let path = file.path.clone();
    let DownloadFile { file, .. } = file;
    let mut file = file;
    file.flush()
        .map_err(|err| format!("写入失败 {}: {err}", path.display()))?;
    file.get_ref()
        .sync_all()
        .map_err(|err| format!("写入失败 {}: {err}", path.display()))?;
    Ok(())
}

fn cleanup(current: &mut Option<DownloadFile>) {
    if let Some(file) = current.take() {
        let path = file.path.clone();
        drop(file.file);
        let _ = std::fs::remove_file(&path);
    }
}

/// 丢弃已被消费的输入前缀，避免 `Vec` 反复移动内存。
fn compact_pending(pending: &mut Vec<u8>, pending_pos: &mut usize) {
    if *pending_pos == 0 {
        return;
    }
    if *pending_pos >= pending.len() {
        pending.clear();
        *pending_pos = 0;
    } else if *pending_pos >= 64 * 1024 {
        pending.drain(..*pending_pos);
        *pending_pos = 0;
    }
}

/// 把 ZRINIT 头改写为「无接收窗口 + CANOVIO」。
///
/// lrzsz 的 `sz` 只有在 ZRINIT 声明「buffer_len = 0（可以连续发送）且带 CANOVIO
/// （收发可重叠）」时才会连续灌数据；否则每发完一个 1KB 子包就等一次应答，
/// 在 30ms RTT 的链路上速度会被压到 ~30KB/s。zmodem2 0.5 默认只声明 1024 字节
/// 窗口且不带 CANOVIO，这里就地重写帧内的 4 字节参数并重算 CRC16。
fn patch_zrinit_streaming(frame: &mut [u8]) {
    /// ZRINIT 帧类型
    const FRAME_ZRINIT: u8 = 1;
    /// 收发可重叠
    const CANOVIO: u8 = 0x02;
    /// ZHEX 头：帧类型(1) + 参数(4) + CRC16(2) = 7 字节 = 14 个十六进制字符
    const HEX_LEN: usize = 14;

    let mut index = 0usize;
    while index + START_PREFIX.len() + 1 + HEX_LEN <= frame.len() {
        if &frame[index..index + START_PREFIX.len()] != START_PREFIX
            || frame[index + START_PREFIX.len()] != HEX_FRAME_MARK
        {
            index += 1;
            continue;
        }

        let hex_start = index + START_PREFIX.len() + 1;
        if let Some(mut data) = decode_hex_header(frame, hex_start)
            && data[0] == FRAME_ZRINIT
        {
            data[1] = 0;
            data[2] = 0;
            data[4] |= CANOVIO;
            let crc = crc16_xmodem(&data[..5]);
            data[5] = (crc >> 8) as u8;
            data[6] = crc as u8;
            for (slot, byte) in data.iter().enumerate() {
                const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
                frame[hex_start + slot * 2] = HEX_DIGITS[(byte >> 4) as usize];
                frame[hex_start + slot * 2 + 1] = HEX_DIGITS[(byte & 0x0f) as usize];
            }
        }

        index = hex_start + HEX_LEN;
    }
}

/// 解码位于 `hex_start` 的 7 字节十六进制帧头载荷（帧类型 + 4 参数 + CRC16）。
fn decode_hex_header(frame: &[u8], hex_start: usize) -> Option<[u8; 7]> {
    let hex = frame.get(hex_start..hex_start + 14)?;
    let mut data = [0u8; 7];
    for (slot, pair) in data.iter_mut().zip(hex.chunks(2)) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        *slot = ((hi as u8) << 4) | lo as u8;
    }
    Some(data)
}

/// 在输入流里找远端 `rz` 的 ZRINIT，返回 (接收窗口字节数, 能力位)。
///
/// 仅用于诊断上传速度：窗口为 0 表示对方允许连续发送，能力位 0x02 = CANOVIO。
fn find_remote_zrinit(data: &[u8]) -> Option<(u16, u8)> {
    let mut index = 0usize;
    while index + START_PREFIX.len() + 1 + 14 <= data.len() {
        if &data[index..index + START_PREFIX.len()] == START_PREFIX
            && data[index + START_PREFIX.len()] == HEX_FRAME_MARK
        {
            let hex_start = index + START_PREFIX.len() + 1;
            if let Some(header) = decode_hex_header(data, hex_start)
                && header[0] == 1
            {
                return Some((u16::from_le_bytes([header[1], header[2]]), header[4]));
            }
            index = hex_start + 14;
            continue;
        }
        index += 1;
    }
    None
}

/// CRC-16/XMODEM（poly 0x1021，初值 0），与 ZMODEM 十六进制帧头一致。
fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// 生成不覆盖已有文件的下载路径。
pub fn unique_download_path(dir: &Path, name: &str) -> PathBuf {
    let safe = sanitize_file_name(name);
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(&safe);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| safe.clone());
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());

    for index in 1..1000u32 {
        let candidate = match extension.as_deref() {
            Some(ext) => dir.join(format!("{stem} ({index}).{ext}")),
            None => dir.join(format!("{stem} ({index})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(format!("{stem}-{}.bin", std::process::id()))
}

/// 去掉路径分隔符与非法字符，避免远端文件名影响本地路径。
pub fn sanitize_file_name(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches('\0');
    let cleaned: String = base
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn state_for_test() -> (Arc<ZmodemState>, Arc<Mutex<Vec<u8>>>, Arc<AtomicUsize>) {
        let written: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(AtomicUsize::new(0));

        let write_sink = written.clone();
        let write: ZmodemWriteFn = Arc::new(move |data: &[u8]| {
            write_sink.lock().unwrap().extend_from_slice(data);
        });
        let event_counter = events.clone();
        let emit: ZmodemEventFn = Arc::new(move |_event| {
            event_counter.fetch_add(1, Ordering::Relaxed);
        });
        let state = ZmodemState::new(write, emit, std::env::temp_dir());
        (state, written, events)
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let (state, _, _) = state_for_test();
        let mut scanner = ZmodemScanner::new();

        let out = scanner.scan(&state, b"hello world\r\n");
        assert_eq!(out, b"hello world\r\n");
        assert!(!state.is_active());

        let out = scanner.scan(&state, b"second line\n");
        assert_eq!(out, b"second line\n");
    }

    #[test]
    fn partial_prefix_is_held_until_resolved() {
        let (state, _, _) = state_for_test();
        let mut scanner = ZmodemScanner::new();

        // 结尾的 "**" 可能是帧头前缀，先扣住
        let out = scanner.scan(&state, b"abc**");
        assert_eq!(out, b"abc");
        assert_eq!(scanner.flush(), b"**");

        // 后续不是帧头，则原样输出
        let mut scanner = ZmodemScanner::new();
        let out = scanner.scan(&state, b"abc*");
        assert_eq!(out, b"abc");
        let out = scanner.scan(&state, b"x");
        assert_eq!(out, b"*x");
    }

    #[test]
    fn rz_header_starts_upload_session() {
        let (state, written, _) = state_for_test();
        let mut scanner = ZmodemScanner::new();

        let out = scanner.scan(&state, b"before**\x18B01000000000000\r\n");
        assert_eq!(out, b"before");
        assert!(state.is_active());
        // rz 会话启动后，接收端会立刻回写 ZRINIT
        std::thread::sleep(Duration::from_millis(120));
        assert!(!written.lock().unwrap().is_empty());
        state.cancel();
        std::thread::sleep(Duration::from_millis(120));
        assert!(!state.is_active());
    }

    #[test]
    fn sz_header_starts_download_session() {
        let (state, _, _) = state_for_test();
        let mut scanner = ZmodemScanner::new();

        let out = scanner.scan(&state, b"**\x18B00000000000000\r\n");
        assert!(out.is_empty(), "帧头不应交给终端渲染");
        assert!(state.is_active());
        state.cancel();
        std::thread::sleep(Duration::from_millis(120));
        assert!(!state.is_active());
    }

    #[test]
    fn header_split_across_chunks_is_detected() {
        let (state, _, _) = state_for_test();
        let mut scanner = ZmodemScanner::new();

        let out = scanner.scan(&state, b"prefix**\x18");
        assert_eq!(out, b"prefix");
        let out = scanner.scan(&state, b"B01000000000000\r\n");
        assert!(out.is_empty());
        assert!(state.is_active());
        state.cancel();
        std::thread::sleep(Duration::from_millis(120));
    }

    #[test]
    fn frame_direction_maps_zrqinit_and_zrinit() {
        assert_eq!(
            frame_direction(b"00000000"),
            Some(ZmodemDirection::Download)
        );
        assert_eq!(frame_direction(b"01000000"), Some(ZmodemDirection::Upload));
        assert_eq!(frame_direction(b"04000000"), None);
    }

    #[test]
    fn crc16_matches_zmodem_reference() {
        // ZMODEM 规范给出的示例："123456789" 的 CRC-16/XMODEM 为 0x31C3
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    #[test]
    fn zrinit_is_rewritten_for_streaming_and_still_valid() {
        let receiver = ZmodemReceiver::new().expect("receiver");
        let mut frame = receiver.drain_outgoing().to_vec();
        let before = frame.clone();
        patch_zrinit_streaming(&mut frame);

        // 长度不变（原地改写十六进制字符）
        assert_eq!(frame.len(), before.len());
        assert_ne!(frame, before, "ZRINIT 参数应被改写");

        // 改写后的帧仍能被标准发送端解析（CRC 正确、参数被接受）
        let mut sender = ZmodemSender::new().expect("sender");
        let hello = sender.drain_outgoing().to_vec();
        sender.advance_outgoing(hello.len());
        sender.feed_incoming(&frame).expect("sender accepts zrinit");
        // 接收端声明可连续发送后，发送端应当立刻可以给出文件头
        sender.start_file(b"probe.bin", 1).expect("start file");
        assert!(!sender.drain_outgoing().is_empty(), "应已排队 ZFILE 帧");
    }

    #[test]
    fn patch_leaves_non_zrinit_frames_untouched() {
        let mut frame: Vec<u8> = vec![b'*', b'*', 0x18, HEX_FRAME_MARK];
        frame.extend_from_slice(b"00000000000000");
        let before = frame.clone();
        patch_zrinit_streaming(&mut frame);
        assert_eq!(frame, before, "ZRQINIT 不应被改写");
    }

    #[test]
    fn remote_zrinit_window_is_parsed() {
        // 远端 rz 声明 8192 字节窗口 + CANFDX|CANOVIO|CANFC32
        let caps = 0x01 | 0x02 | 0x20;
        let mut data = [0u8; 7];
        data[0] = 1;
        data[1] = 0x00;
        data[2] = 0x20; // 8192 little endian
        data[4] = caps;
        let crc = crc16_xmodem(&data[..5]);
        data[5] = (crc >> 8) as u8;
        data[6] = crc as u8;

        let mut frame: Vec<u8> = vec![b'*', b'*', 0x18, HEX_FRAME_MARK];
        for byte in data {
            const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
            frame.push(HEX_DIGITS[(byte >> 4) as usize]);
            frame.push(HEX_DIGITS[(byte & 0x0f) as usize]);
        }

        let parsed = find_remote_zrinit(&frame).expect("应能解析 ZRINIT");
        assert_eq!(parsed.0, 8192);
        assert_eq!(parsed.1 & 0x02, 0x02, "CANOVIO 应被识别");
    }

    #[test]
    fn sanitize_rejects_path_traversal() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\tmp\\a.txt"), "a.txt");
        assert_eq!(sanitize_file_name(".."), "download");
        assert_eq!(sanitize_file_name("a:b?.txt"), "a_b_.txt");
    }

    #[test]
    fn unique_download_path_avoids_overwrite() {
        let dir = std::env::temp_dir().join(format!("myterm-zmodem-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_download_path(&dir, "a.txt");
        assert_eq!(first.file_name().unwrap(), "a.txt");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_download_path(&dir, "a.txt");
        assert_eq!(second.file_name().unwrap(), "a (1).txt");
        std::fs::write(&second, b"x").unwrap();
        let third = unique_download_path(&dir, "a.txt");
        assert_eq!(third.file_name().unwrap(), "a (2).txt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "myterm-zmodem-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn take(buffer: &Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
        let mut guard = buffer.lock().unwrap();
        std::mem::take(&mut *guard)
    }

    fn take_terminal_event(events: &Arc<Mutex<Vec<ZmodemEvent>>>) -> Option<ZmodemEvent> {
        let mut guard = events.lock().unwrap();
        let index = guard.iter().position(|event| {
            matches!(
                event,
                ZmodemEvent::Finished { .. } | ZmodemEvent::Failed { .. }
            )
        });
        index.map(|index| guard.remove(index))
    }

    fn next_upload_request(events: &Arc<Mutex<Vec<ZmodemEvent>>>) -> ZmodemUploadRequest {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let found = {
                let mut guard = events.lock().unwrap();
                let index = guard
                    .iter()
                    .position(|event| matches!(event, ZmodemEvent::UploadRequest(_)));
                index.map(|index| guard.remove(index))
            };
            if let Some(ZmodemEvent::UploadRequest(request)) = found {
                return request;
            }
            assert!(Instant::now() < deadline, "等待上传请求超时");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// 模拟远端 `sz`：本机应作为接收端把文件落到下载目录。
    #[test]
    fn download_session_persists_file_from_simulated_sz() {
        let dir = temp_dir("download");
        let to_remote: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let events: Arc<Mutex<Vec<ZmodemEvent>>> = Arc::new(Mutex::new(Vec::new()));

        let mut sender = ZmodemSender::new().expect("sender");
        let initial = sender.drain_outgoing().to_vec();
        sender.advance_outgoing(initial.len());

        let payload: Vec<u8> = (0..300_000u32).map(|index| (index % 251) as u8).collect();
        let remote_file = dir.join("remote.txt");
        std::fs::write(&remote_file, &payload).unwrap();

        let write: ZmodemWriteFn = {
            let to_remote = to_remote.clone();
            Arc::new(move |data: &[u8]| to_remote.lock().unwrap().extend_from_slice(data))
        };
        let emit: ZmodemEventFn = {
            let events = events.clone();
            Arc::new(move |event| events.lock().unwrap().push(event))
        };
        let state = ZmodemState::new(write, emit, dir.to_path_buf());
        assert!(state.begin(ZmodemDirection::Download, initial));

        sender
            .start_file(b"remote.txt", payload.len() as u32)
            .expect("start file");

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let incoming = take(&to_remote);
            if !incoming.is_empty() {
                let mut pending = incoming;
                while !pending.is_empty() {
                    let consumed = sender.feed_incoming(&pending).expect("sender feed");
                    if consumed == 0 {
                        break;
                    }
                    pending.drain(..consumed);
                }
            }

            if let Some(request) = sender.poll_file() {
                let start = request.offset as usize;
                let end = (start + request.len).min(payload.len());
                sender
                    .feed_file(&payload[start..end])
                    .expect("sender feed_file");
            }

            while let Some(event) = sender.poll_event() {
                if event == SenderEvent::FileComplete {
                    sender.finish_session().expect("finish session");
                }
            }

            let outgoing = sender.drain_outgoing().to_vec();
            if !outgoing.is_empty() {
                sender.advance_outgoing(outgoing.len());
                state.feed(&outgoing);
            }

            if let Some(event) = take_terminal_event(&events) {
                match event {
                    ZmodemEvent::Finished { files, .. } => {
                        assert_eq!(files, 1);
                        break;
                    }
                    ZmodemEvent::Failed { message, .. } => panic!("下载失败: {message}"),
                    other => panic!("意外事件: {other:?}"),
                }
            }

            assert!(Instant::now() < deadline, "zmodem 下载会话超时");
            std::thread::sleep(Duration::from_millis(1));
        }

        let saved = std::fs::read(dir.join("remote.txt")).expect("saved file");
        assert_eq!(saved, payload);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 模拟远端 `rz`：本机作为发送端把用户选中的文件发出去。
    #[test]
    fn upload_session_sends_selected_file_to_simulated_rz() {
        let dir = temp_dir("upload");
        let to_remote: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let events: Arc<Mutex<Vec<ZmodemEvent>>> = Arc::new(Mutex::new(Vec::new()));

        let payload: Vec<u8> = (0..200_000u32).map(|index| (index % 241) as u8).collect();
        let local_file = dir.join("local.bin");
        std::fs::write(&local_file, &payload).unwrap();

        let mut receiver = ZmodemReceiver::new().expect("receiver");
        let initial = receiver.drain_outgoing().to_vec();
        receiver.advance_outgoing(initial.len());

        let write: ZmodemWriteFn = {
            let to_remote = to_remote.clone();
            Arc::new(move |data: &[u8]| to_remote.lock().unwrap().extend_from_slice(data))
        };
        let emit: ZmodemEventFn = {
            let events = events.clone();
            Arc::new(move |event| events.lock().unwrap().push(event))
        };
        let state = ZmodemState::new(write, emit, dir.clone());
        assert!(state.begin(ZmodemDirection::Upload, initial));

        let request = next_upload_request(&events);
        request.respond(Some(vec![local_file.clone()]));

        let mut received: Vec<u8> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let incoming = take(&to_remote);
            if !incoming.is_empty() {
                let mut pending = incoming;
                while !pending.is_empty() {
                    let consumed = receiver.feed_incoming(&pending).expect("receiver feed");
                    if consumed == 0 {
                        break;
                    }
                    pending.drain(..consumed);
                }
            }

            let chunk = receiver.drain_file().to_vec();
            if !chunk.is_empty() {
                received.extend_from_slice(&chunk);
                receiver.advance_file(chunk.len()).expect("advance file");
            }

            while receiver.poll_event().is_some() {}

            let outgoing = receiver.drain_outgoing().to_vec();
            if !outgoing.is_empty() {
                receiver.advance_outgoing(outgoing.len());
                state.feed(&outgoing);
            }

            if let Some(event) = take_terminal_event(&events) {
                match event {
                    ZmodemEvent::Finished { files, .. } => {
                        assert_eq!(files, 1);
                        break;
                    }
                    ZmodemEvent::Failed { message, .. } => panic!("上传失败: {message}"),
                    other => panic!("意外事件: {other:?}"),
                }
            }

            assert!(Instant::now() < deadline, "zmodem 上传会话超时");
            std::thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(received, payload);
        let _ = std::fs::remove_dir_all(&dir);
    }
    // ===== 性能基准（默认忽略） =====

    fn bench_trace(message: impl AsRef<str>) {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("myterm-bench-trace.log"))
        {
            let _ = writeln!(file, "{}", message.as_ref());
            let _ = file.flush();
        }
    }

    /// 用忙等模拟链路 RTT：Windows 的 sleep 粒度是 15.6ms，直接 sleep 会把基准带偏。
    fn spin_delay(duration: Duration) {
        if duration.is_zero() {
            return;
        }
        let until = Instant::now() + duration;
        while Instant::now() < until {
            std::hint::spin_loop();
        }
    }

    fn bench_payload(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    struct Loopback {
        to_remote: Arc<Mutex<Vec<u8>>>,
        events: Arc<Mutex<Vec<ZmodemEvent>>>,
        state: Arc<ZmodemState>,
    }

    fn loopback(dir: &Path, download: bool, initial: Vec<u8>) -> Loopback {
        let to_remote: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let events: Arc<Mutex<Vec<ZmodemEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let write: ZmodemWriteFn = {
            let to_remote = to_remote.clone();
            Arc::new(move |data: &[u8]| to_remote.lock().unwrap().extend_from_slice(data))
        };
        let emit: ZmodemEventFn = {
            let events = events.clone();
            Arc::new(move |event| events.lock().unwrap().push(event))
        };
        let state = ZmodemState::new(write, emit, dir.to_path_buf());
        let direction = if download {
            ZmodemDirection::Download
        } else {
            ZmodemDirection::Upload
        };
        assert!(state.begin(direction, initial));
        Loopback {
            to_remote,
            events,
            state,
        }
    }

    /// 回环下载吞吐（本机接收，模拟远端 `sz`）。
    fn measure_download_mbps(payload: &[u8], rtt: Duration) -> (f64, Duration) {
        let dir = temp_dir("bench-download");
        std::fs::write(dir.join("bench.bin"), payload).unwrap();

        let mut sender = ZmodemSender::new().expect("sender");
        let initial = sender.drain_outgoing().to_vec();
        sender.advance_outgoing(initial.len());
        let lb = loopback(&dir, true, initial);
        sender
            .start_file(b"bench.bin", payload.len() as u32)
            .expect("start file");

        let started = Instant::now();
        let deadline = started + Duration::from_secs(120);
        let mut pending: Vec<u8> = Vec::new();
        let mut steps: u64 = 0;
        loop {
            steps += 1;
            pending.extend_from_slice(&take(&lb.to_remote));
            while !pending.is_empty() {
                let consumed = sender.feed_incoming(&pending).expect("sender feed");
                if consumed == 0 {
                    break;
                }
                pending.drain(..consumed);
            }

            if let Some(request) = sender.poll_file() {
                let start = request.offset as usize;
                let end = (start + request.len).min(payload.len());
                sender
                    .feed_file(&payload[start..end])
                    .expect("sender feed_file");
            }

            while let Some(event) = sender.poll_event() {
                if event == SenderEvent::FileComplete {
                    sender.finish_session().expect("finish session");
                }
            }

            let outgoing = sender.drain_outgoing().to_vec();
            if !outgoing.is_empty() {
                sender.advance_outgoing(outgoing.len());
                lb.state.feed(&outgoing);
            }

            if let Some(event) = take_terminal_event(&lb.events) {
                match event {
                    ZmodemEvent::Finished { .. } => break,
                    ZmodemEvent::Failed { message, .. } => panic!("基准下载失败: {message}"),
                    other => panic!("意外事件: {other:?}"),
                }
            }

            assert!(Instant::now() < deadline, "基准下载超时");
            spin_delay(rtt);
        }

        let elapsed = started.elapsed();
        bench_trace(format!(
            "[bench-dl] rtt={rtt:?} steps={steps} elapsed={elapsed:?}"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (
            payload.len() as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64(),
            elapsed,
        )
    }

    /// 回环上传吞吐（本机发送，模拟远端 `rz`）。
    fn measure_upload_mbps(payload: &[u8], rtt: Duration) -> (f64, Duration) {
        let dir = temp_dir("bench-upload");
        let local_file = dir.join("bench.bin");
        std::fs::write(&local_file, payload).unwrap();

        let mut receiver = ZmodemReceiver::new().expect("receiver");
        let initial = receiver.drain_outgoing().to_vec();
        receiver.advance_outgoing(initial.len());
        let lb = loopback(&dir, false, initial);
        next_upload_request(&lb.events).respond(Some(vec![local_file]));

        let started = Instant::now();
        let deadline = started + Duration::from_secs(120);
        let mut pending: Vec<u8> = Vec::new();
        let mut steps: u64 = 0;
        loop {
            steps += 1;
            pending.extend_from_slice(&take(&lb.to_remote));
            while !pending.is_empty() {
                let consumed = receiver.feed_incoming(&pending).expect("receiver feed");
                if consumed == 0 {
                    break;
                }
                pending.drain(..consumed);
            }

            let chunk = receiver.drain_file().to_vec();
            if !chunk.is_empty() {
                receiver.advance_file(chunk.len()).expect("advance file");
            }

            while receiver.poll_event().is_some() {}

            let outgoing = receiver.drain_outgoing().to_vec();
            if !outgoing.is_empty() {
                receiver.advance_outgoing(outgoing.len());
                lb.state.feed(&outgoing);
            }

            if let Some(event) = take_terminal_event(&lb.events) {
                match event {
                    ZmodemEvent::Finished { .. } => break,
                    ZmodemEvent::Failed { message, .. } => panic!("基准上传失败: {message}"),
                    other => panic!("意外事件: {other:?}"),
                }
            }

            assert!(Instant::now() < deadline, "基准上传超时");
            spin_delay(rtt);
        }

        let elapsed = started.elapsed();
        bench_trace(format!(
            "[bench-ul] rtt={rtt:?} steps={steps} elapsed={elapsed:?}"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (
            payload.len() as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64(),
            elapsed,
        )
    }

    #[test]
    #[ignore = "性能基准：cargo test -p terminal benchmark_zmodem -- --ignored --nocapture"]
    fn benchmark_zmodem_loopback() {
        let size: usize = std::env::var("MYTERM_ZMODEM_BENCH_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1024 * 1024);
        let payload = bench_payload(size);
        for rtt_ms in [0u64, 10, 30] {
            let (mbps, elapsed) = measure_download_mbps(&payload, Duration::from_millis(rtt_ms));
            bench_trace(format!(
                "download rtt={rtt_ms}ms: {mbps:.2} MB/s ({elapsed:?})"
            ));
        }
        for rtt_ms in [0u64, 10, 30] {
            let (mbps, elapsed) = measure_upload_mbps(&payload, Duration::from_millis(rtt_ms));
            bench_trace(format!(
                "upload   rtt={rtt_ms}ms: {mbps:.2} MB/s ({elapsed:?})"
            ));
        }
    }
}
