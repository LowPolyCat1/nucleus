//! Command logic, independent of Tauri so it can be tested on the fake sandbox backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use nucleus_harness::{EventSink, Harness};
use nucleus_sandbox::{Engine, SandboxBackend};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::settings;

/// Errors cross IPC as their full message chain.
pub type CmdResult<T> = Result<T, String>;

pub fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub ready: bool,
    pub engine: Option<Engine>,
    pub data_dir: PathBuf,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub settings: nucleus_harness::Settings,
    pub workspaces: Vec<nucleus_harness::Workspace>,
    pub conversations: Vec<nucleus_harness::Conversation>,
}

pub type Connector = Box<
    dyn Fn(PathBuf) -> BoxFuture<'static, anyhow::Result<Arc<dyn SandboxBackend>>> + Send + Sync,
>;

/// Holds the harness once the container engine is reachable. Until then every command fails
/// with a clear message and the UI offers a retry.
pub struct AppCore {
    data_dir: PathBuf,
    connector: Connector,
    sink: EventSink,
    harness: RwLock<Option<Arc<Harness>>>,
    init_lock: Mutex<()>,
}

impl AppCore {
    pub fn new(data_dir: PathBuf, connector: Connector, sink: EventSink) -> Self {
        Self {
            data_dir,
            connector,
            sink,
            harness: RwLock::new(None),
            init_lock: Mutex::new(()),
        }
    }

    /// Connector for the real engine (Podman or Docker socket).
    pub fn engine_connector() -> Connector {
        Box::new(|support: PathBuf| {
            Box::pin(async move {
                let b = nucleus_sandbox::BollardBackend::connect_default(support).await?;
                Ok(Arc::new(b) as Arc<dyn SandboxBackend>)
            })
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Connect and open the harness. Idempotent; concurrent calls wait for the first.
    pub async fn init(&self) -> AppInfo {
        let _guard = self.init_lock.lock().await;
        if let Some(h) = self.harness.read().await.as_ref() {
            return self.info(Some(h.backend().engine()), None);
        }
        let result = async {
            let backend = (self.connector)(self.data_dir.join("engine-support")).await?;
            let harness =
                Arc::new(Harness::open(&self.data_dir, backend, self.sink.clone()).await?);
            Ok::<_, anyhow::Error>(harness)
        }
        .await;
        match result {
            Ok(h) => {
                let engine = h.backend().engine();
                // Remove leftovers from crashes in the background; failures are not fatal.
                let bg = h.clone();
                tokio::spawn(async move {
                    match bg.cleanup_orphans().await {
                        Ok(r)
                            if !(r.containers.is_empty()
                                && r.worktrees.is_empty()
                                && r.branches.is_empty()) =>
                        {
                            tracing::info!(?r, "removed orphaned resources")
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!("orphan cleanup failed: {e:#}"),
                    }
                });
                *self.harness.write().await = Some(h);
                self.info(Some(engine), None)
            }
            Err(e) => self.info(None, Some(err(e))),
        }
    }

    fn info(&self, engine: Option<Engine>, error: Option<String>) -> AppInfo {
        AppInfo {
            ready: error.is_none(),
            engine,
            data_dir: self.data_dir.clone(),
            error,
        }
    }

    pub async fn harness(&self) -> CmdResult<Arc<Harness>> {
        self.harness
            .read()
            .await
            .clone()
            .ok_or_else(|| "nucleus is not connected to a container engine yet".to_string())
    }

    pub async fn state(&self) -> CmdResult<AppState> {
        let h = self.harness().await?;
        let s = h.snapshot().await;
        Ok(AppState {
            settings: settings::masked(&s.settings),
            workspaces: s.workspaces,
            conversations: s.conversations,
        })
    }

    pub async fn update_settings(&self, incoming: nucleus_harness::Settings) -> CmdResult<()> {
        let h = self.harness().await?;
        let merged = settings::merge(&h.settings().await, incoming).map_err(err)?;
        h.update_settings(merged).await.map_err(err)
    }
}
