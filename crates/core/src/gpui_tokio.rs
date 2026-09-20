use std::future::Future;

use gpui::{App, AppContext, Global, ReadGlobal, Task};
use util::defer;

pub use tokio::task::JoinError;

const DEFAULT_WORKER_THREADS: usize = 4;
const MIN_WORKER_THREADS: usize = 1;
const MAX_WORKER_THREADS: usize = 32;
const WORKER_THREADS_ENV: &str = "MYTERM_TOKIO_WORKER_THREADS";

fn configured_worker_threads() -> usize {
    std::env::var(WORKER_THREADS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|threads| threads.clamp(MIN_WORKER_THREADS, MAX_WORKER_THREADS))
        .unwrap_or(DEFAULT_WORKER_THREADS)
}

/// Initializes the Tokio wrapper using a new Tokio runtime.
pub fn init(cx: &mut App) {
    let worker_threads = configured_worker_threads();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio");

    tracing::info!(
        worker_threads,
        env = WORKER_THREADS_ENV,
        "Initialized Tokio runtime"
    );

    cx.set_global(GlobalTokio::new(RuntimeHolder::Owned(runtime)));
}

enum RuntimeHolder {
    Owned(tokio::runtime::Runtime),
}

impl RuntimeHolder {
    pub fn handle(&self) -> &tokio::runtime::Handle {
        match self {
            RuntimeHolder::Owned(runtime) => runtime.handle(),
        }
    }
}

struct GlobalTokio {
    runtime: RuntimeHolder,
}

impl Global for GlobalTokio {}

impl GlobalTokio {
    fn new(runtime: RuntimeHolder) -> Self {
        Self { runtime }
    }
}

pub struct Tokio {}

impl Tokio {
    /// Spawns the given future on Tokio's thread pool, and returns it via a GPUI task
    /// Note that the Tokio task will be cancelled if the GPUI task is dropped
    pub fn spawn<C, Fut, R>(cx: &C, f: Fut) -> Task<Result<R, JoinError>>
    where
        C: AppContext,
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        cx.read_global(|tokio: &GlobalTokio, cx| {
            let join_handle = tokio.runtime.handle().spawn(f);
            let abort_handle = join_handle.abort_handle();
            let cancel = defer(move || {
                abort_handle.abort();
            });
            cx.background_spawn(async move {
                let result = join_handle.await;
                drop(cancel);
                result
            })
        })
    }

    /// Spawns the given future on Tokio's thread pool, and returns it via a GPUI task
    /// Note that the Tokio task will be cancelled if the GPUI task is dropped
    pub fn spawn_result<C, Fut, R>(cx: &C, f: Fut) -> Task<anyhow::Result<R>>
    where
        C: AppContext,
        Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        cx.read_global(|tokio: &GlobalTokio, cx| {
            let join_handle = tokio.runtime.handle().spawn(f);
            let abort_handle = join_handle.abort_handle();
            let cancel = defer(move || {
                abort_handle.abort();
            });
            cx.background_spawn(async move {
                let result = join_handle.await?;
                drop(cancel);
                result
            })
        })
    }

    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        GlobalTokio::global(cx).runtime.handle().clone()
    }
}
