//! Container sandbox. Everything the agent produces runs here, never on the host.
//!
//! [`SandboxBackend`] is the abstraction; [`BollardBackend`] implements it for both Podman and
//! Docker, which differ only in the socket path and a few capabilities (overlay mounts).

mod bollard_backend;
pub mod caches;
pub mod endpoint;
#[cfg(feature = "fake")]
pub mod fake;
mod network;
mod types;

pub use bollard_backend::BollardBackend;
pub use endpoint::{Endpoint, detect as detect_endpoint};
pub use network::EgressPlan;
pub use types::*;

use async_trait::async_trait;
use futures::StreamExt;

/// `uid:gid` of the harness process, so files containers write into mounted directories stay
/// owned by the user. `None` on Windows, where the engine runs in a VM with its own users.
pub fn current_user() -> Option<String> {
    #[cfg(unix)]
    {
        // SAFETY: getuid and getgid cannot fail and have no preconditions.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        Some(format!("{uid}:{gid}"))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Name of the egress proxy sidecar of `container`.
pub fn egress_name(container: &str) -> String {
    format!("{container}-egress")
}

/// One decision of the egress proxy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EgressEntry {
    /// Milliseconds since the unix epoch.
    pub time: i64,
    /// `allow` or `deny`.
    pub verdict: String,
    /// `host:port` for tunnels, `host` for plain HTTP.
    pub target: String,
}

/// Parse the JSON lines the egress proxy writes; other lines (and its startup line) are skipped.
pub fn parse_egress_log(text: &str) -> Vec<EgressEntry> {
    text.lines()
        .filter_map(|l| {
            let start = l.find('{')?;
            let v: serde_json::Value = serde_json::from_str(&l[start..]).ok()?;
            let verdict = v.get("verdict")?.as_str()?;
            if verdict != "allow" && verdict != "deny" {
                return None;
            }
            Some(EgressEntry {
                time: v.get("t").and_then(|t| t.as_i64()).unwrap_or(0),
                verdict: verdict.to_string(),
                target: v.get("target")?.as_str()?.to_string(),
            })
        })
        .collect()
}

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

/// Label set on every container, network and volume the harness creates.
pub const MANAGED_LABEL: &str = "nucleus.managed";

#[async_trait]
pub trait SandboxBackend: Send + Sync {
    fn engine(&self) -> Engine;

    /// Make sure `image` is available locally, pulling it if needed. Returns the image id, which
    /// identifies the exact image content (used for template staleness checks).
    async fn ensure_image(&self, image: &str) -> Result<String>;

    /// Environment baked into an image (`KEY=value`), e.g. its `PATH`.
    async fn image_env(&self, image: &str) -> Result<Vec<String>>;

    /// Create and start a long-running container, including its network setup.
    async fn create(&self, spec: &ContainerSpec) -> Result<ContainerInfo>;

    /// Start a process inside a running container.
    async fn exec(&self, container: &str, spec: &ExecSpec) -> Result<ExecHandle>;

    /// Stop and remove a container together with any network resources created for it.
    /// Removing a container that does not exist is not an error.
    async fn remove(&self, name: &str) -> Result<()>;

    /// Containers created by the harness, optionally filtered by a label value.
    async fn list(&self, label: Option<(&str, &str)>) -> Result<Vec<ContainerInfo>>;

    /// Output a container's main process wrote (stdout and stderr), at most `tail` lines.
    async fn logs(&self, name: &str, tail: usize) -> Result<String>;

    /// Decisions of the egress proxy for a container: which hosts it allowed and denied.
    /// Empty when the container has no proxy (no network at all, or full access).
    async fn egress_log(&self, container: &str, tail: usize) -> Result<Vec<EgressEntry>> {
        match self.logs(&egress_name(container), tail).await {
            Ok(text) => Ok(parse_egress_log(&text)),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// Create a named volume if it does not exist.
    async fn ensure_volume(&self, name: &str) -> Result<()>;

    /// Run a process to completion and collect its output.
    async fn exec_collect(&self, container: &str, spec: &ExecSpec) -> Result<ExecResult> {
        let mut handle = self.exec(container, spec).await?;
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        while let Some(chunk) = handle.output.next().await {
            match chunk? {
                ExecChunk::Stdout(b) => stdout.extend(b),
                ExecChunk::Stderr(b) => stderr.extend(b),
            }
        }
        let exit_code = handle.wait().await?;
        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }
}

#[cfg(test)]
mod egress_tests {
    use super::*;

    #[test]
    fn parses_proxy_output() {
        let text = "{\"t\":1,\"verdict\":\"listening\",\"target\":\"3128\"}\n\
                    {\"t\":2,\"verdict\":\"allow\",\"target\":\"api.anthropic.com:443\"}\n\
                    garbage\n\
                    2026-01-01T00:00:00Z {\"t\":3,\"verdict\":\"deny\",\"target\":\"evil.example:443\"}\n\
                    {\"t\":4,\"verdict\":\"deny\"}\n";
        let e = parse_egress_log(text);
        assert_eq!(e.len(), 2);
        assert_eq!(
            e[0],
            EgressEntry {
                time: 2,
                verdict: "allow".into(),
                target: "api.anthropic.com:443".into()
            }
        );
        assert_eq!(e[1].verdict, "deny");
        assert!(parse_egress_log("").is_empty());
        assert_eq!(egress_name("c"), "c-egress");
    }
}
