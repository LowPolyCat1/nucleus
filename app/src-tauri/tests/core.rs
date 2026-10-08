use std::sync::Arc;

use nucleus_app::core::{AppCore, Connector};
use nucleus_app::settings::SECRET_MASK;
use nucleus_harness::Settings;
use nucleus_sandbox::fake::FakeBackend;
use nucleus_sandbox::{Engine, SandboxBackend};

fn connector(fail_first: bool) -> Connector {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Box::new(move |support| {
        let n = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            if fail_first && n == 0 {
                anyhow::bail!("no Podman or Docker socket found");
            }
            Ok(Arc::new(FakeBackend::new(Engine::Podman, support)) as Arc<dyn SandboxBackend>)
        })
    })
}

#[tokio::test]
async fn init_reports_errors_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::new(dir.path().into(), connector(true), Arc::new(|_| {}));
    assert!(core.state().await.unwrap_err().contains("not connected"));
    let info = core.init().await;
    assert!(!info.ready);
    assert!(info.error.unwrap().contains("socket"));
    let info = core.init().await;
    assert!(info.ready, "{info:?}");
    assert_eq!(info.engine, Some(Engine::Podman));
    // Idempotent.
    assert!(core.init().await.ready);
    assert!(core.state().await.unwrap().workspaces.is_empty());
}

#[tokio::test]
async fn concurrent_init_opens_once() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(AppCore::new(
        dir.path().into(),
        connector(false),
        Arc::new(|_| {}),
    ));
    let (a, b) = tokio::join!(core.init(), core.init());
    assert!(a.ready && b.ready);
}

#[tokio::test]
async fn secrets_never_leave_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::new(dir.path().into(), connector(false), Arc::new(|_| {}));
    core.init().await;
    let mut s = Settings::default();
    s.provider_env
        .insert("ANTHROPIC_API_KEY".into(), "sk-real".into());
    core.update_settings(s).await.unwrap();
    let state = core.state().await.unwrap();
    assert_eq!(
        state.settings.provider_env["ANTHROPIC_API_KEY"],
        SECRET_MASK
    );
    // Round-tripping the masked settings keeps the real value.
    core.update_settings(state.settings.clone()).await.unwrap();
    let h = core.harness().await.unwrap();
    assert_eq!(
        h.settings().await.provider_env["ANTHROPIC_API_KEY"],
        "sk-real"
    );
    // JSON shape matches what the frontend expects.
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        json["settings"]["default_network"],
        serde_json::json!({"mode": "none"})
    );
    assert!(
        core.update_settings(Settings {
            image: "".into(),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .contains("image")
    );
}
