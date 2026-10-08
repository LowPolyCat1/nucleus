mod common;

use std::sync::{Arc, Mutex};

use common::*;
use nucleus_harness::DeleteMode;
use nucleus_sandbox::Engine;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

struct V(String);
impl Visit for V {
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        if f.name() == "message" {
            self.0 = format!("{v:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut v = V(String::new());
        e.record(&mut v);
        self.0.lock().unwrap().push(format!("{} {}", e.metadata().level(), v.0));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn key_operations_are_logged() {
    let cap = Capture::default();
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(cap.clone()));
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.mode("propose");
    f.harness.send_message(&conv.id, "x").await.unwrap();
    f.harness
        .delete_conversation(&conv.id, DeleteMode::Discard)
        .await
        .unwrap();
    f.backend.fail_next_create("down");
    f.harness.create_conversation(&ws.id, "main", "t").await.unwrap_err();
    let logs = cap.0.lock().unwrap().join("\n");
    for expected in [
        "INFO conversation created",
        "INFO turn started",
        "INFO turn finished",
        "INFO committed agent changes",
        "INFO proposal created",
        "WARN proposal rejected automatically",
        "INFO conversation deleted",
        "WARN creating conversation failed",
    ] {
        assert!(logs.contains(expected), "missing {expected:?} in\n{logs}");
    }
}
