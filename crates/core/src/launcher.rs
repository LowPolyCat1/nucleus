use std::collections::BTreeMap;

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::BoxStream;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchSpec {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutput {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

pub struct LaunchedProcess {
    pub output: BoxStream<'static, crate::Result<ProcessOutput>>,
    /// Resolves to the exit code once the output is drained.
    pub exit: BoxFuture<'static, crate::Result<Option<i64>>>,
}

/// Starts processes somewhere: inside a sandbox container in the harness, or on the host in
/// tests. Keeps `core` independent from the sandbox crate.
#[async_trait]
pub trait ProcessLauncher: Send + Sync {
    async fn launch(&self, spec: LaunchSpec) -> crate::Result<LaunchedProcess>;
}

#[cfg(feature = "local-launcher")]
pub use local::LocalLauncher;

#[cfg(feature = "local-launcher")]
mod local {
    use super::*;
    use futures::{FutureExt, StreamExt};
    use tokio::io::AsyncReadExt;

    /// Runs processes on the host. Never use this for agent work.
    pub struct LocalLauncher;

    #[async_trait]
    impl ProcessLauncher for LocalLauncher {
        async fn launch(&self, spec: LaunchSpec) -> crate::Result<LaunchedProcess> {
            let (prog, args) = spec
                .argv
                .split_first()
                .ok_or_else(|| anyhow::anyhow!("empty argv"))?;
            let mut cmd = tokio::process::Command::new(prog);
            cmd.args(args)
                .envs(&spec.env)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            if let Some(dir) = &spec.workdir {
                cmd.current_dir(dir);
            }
            let mut child = cmd.spawn()?;
            let (tx, rx) = futures::channel::mpsc::unbounded();
            for (mut pipe, is_err) in [
                (
                    Box::new(child.stdout.take().unwrap())
                        as Box<dyn tokio::io::AsyncRead + Send + Unpin>,
                    false,
                ),
                (Box::new(child.stderr.take().unwrap()), true),
            ] {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    loop {
                        match pipe.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let chunk = buf[..n].to_vec();
                                let item = if is_err {
                                    ProcessOutput::Stderr(chunk)
                                } else {
                                    ProcessOutput::Stdout(chunk)
                                };
                                if tx.unbounded_send(Ok(item)).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
            drop(tx);
            let exit = async move { Ok(child.wait().await?.code().map(i64::from)) }.boxed();
            Ok(LaunchedProcess {
                output: rx.boxed(),
                exit,
            })
        }
    }
}
