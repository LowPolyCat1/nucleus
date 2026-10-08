use anyhow::bail;
use nucleus_sandbox::Engine;

/// The agent image definition, also shipped as `images/agent/Containerfile`.
pub const AGENT_CONTAINERFILE: &str = include_str!("../../../images/agent/Containerfile");

/// File name of the optional build-time CA in the build context. It is always present (empty
/// when unused) so Containerfiles can `COPY` it unconditionally.
pub const BUILD_CA_FILE: &str = "nucleus-build-ca.crt";

/// Build the agent image with the engine's CLI (`podman build` or `docker build`).
pub async fn build_agent_image(engine: Engine, tag: &str) -> anyhow::Result<String> {
    build_image(engine, tag, AGENT_CONTAINERFILE).await
}

/// Build an image from Containerfile text.
///
/// Builds behind a proxy work out of the box: `HTTP(S)_PROXY`/`NO_PROXY` from the environment
/// are passed as build arguments, the build uses the host network when the proxy listens on
/// loopback, and `NUCLEUS_BUILD_CA` names a CA bundle that is placed in the context as
/// [`BUILD_CA_FILE`] for TLS-intercepting proxies.
pub async fn build_image(engine: Engine, tag: &str, containerfile: &str) -> anyhow::Result<String> {
    let dir = std::env::temp_dir().join(format!("nucleus-image-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir)?;
    let result = async {
        std::fs::write(dir.join("Containerfile"), containerfile)?;
        let ca = match std::env::var_os("NUCLEUS_BUILD_CA") {
            Some(p) => std::fs::read(p)?,
            None => Vec::new(),
        };
        std::fs::write(dir.join(BUILD_CA_FILE), ca)?;
        let cli = match engine {
            Engine::Podman => "podman",
            Engine::Docker => "docker",
        };
        let mut cmd = tokio::process::Command::new(cli);
        cmd.args(["build", "-t", tag, "-f"]).arg(dir.join("Containerfile"));
        cmd.args(build_args(&|k| std::env::var(k).ok()));
        cmd.arg(&dir);
        let out = cmd.output().await?;
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            bail!("{cli} build failed:\n{log}");
        }
        Ok(log)
    }
    .await;
    std::fs::remove_dir_all(&dir).ok();
    result
}

/// Proxy-related build arguments for the current environment.
pub fn build_args(env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut args = Vec::new();
    let mut loopback = false;
    for key in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY"] {
        let value = env(key).or_else(|| env(&key.to_lowercase())).filter(|v| !v.is_empty());
        if let Some(v) = value {
            if key != "NO_PROXY" && (v.contains("127.0.0.1") || v.contains("localhost") || v.contains("[::1]")) {
                loopback = true;
            }
            args.push("--build-arg".into());
            args.push(format!("{key}={v}"));
            args.push("--build-arg".into());
            args.push(format!("{}={v}", key.to_lowercase()));
        }
    }
    if loopback {
        args.push("--network".into());
        args.push("host".into());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_build_args() {
        assert!(build_args(&|_| None).is_empty());
        let a = build_args(&|k| (k == "HTTPS_PROXY").then(|| "http://127.0.0.1:3128".to_string()));
        assert_eq!(
            a,
            [
                "--build-arg",
                "HTTPS_PROXY=http://127.0.0.1:3128",
                "--build-arg",
                "https_proxy=http://127.0.0.1:3128",
                "--network",
                "host"
            ]
        );
        let a = build_args(&|k| (k == "http_proxy").then(|| "http://proxy.corp:8080".to_string()));
        assert_eq!(
            a,
            [
                "--build-arg",
                "HTTP_PROXY=http://proxy.corp:8080",
                "--build-arg",
                "http_proxy=http://proxy.corp:8080"
            ]
        );
    }
}
