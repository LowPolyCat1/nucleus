use anyhow::bail;
use nucleus_sandbox::Engine;

/// The agent image definition, also shipped as `images/agent/Containerfile`.
pub const AGENT_CONTAINERFILE: &str = include_str!("../../../images/agent/Containerfile");

/// Build the agent image with the engine's CLI (`podman build` or `docker build`).
pub async fn build_agent_image(engine: Engine, tag: &str) -> anyhow::Result<String> {
    let dir = tempfile_dir()?;
    std::fs::write(dir.join("Containerfile"), AGENT_CONTAINERFILE)?;
    let cli = match engine {
        Engine::Podman => "podman",
        Engine::Docker => "docker",
    };
    let out = tokio::process::Command::new(cli)
        .args(["build", "-t", tag, "-f"])
        .arg(dir.join("Containerfile"))
        .arg(&dir)
        .output()
        .await?;
    std::fs::remove_dir_all(&dir).ok();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        bail!("{cli} build failed:\n{log}");
    }
    Ok(log)
}

fn tempfile_dir() -> std::io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("nucleus-image-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
