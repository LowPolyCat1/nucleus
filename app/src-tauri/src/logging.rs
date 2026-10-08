//! Logging: stderr, a daily rotating file in `<data dir>/logs`, and an in-memory ring buffer the
//! UI reads through the `recent_logs` command.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Milliseconds since the unix epoch.
    pub time: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Bounded buffer of the most recent log entries.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity: capacity.max(1),
        }
    }

    pub fn push(&self, entry: LogEntry) {
        let mut q = self.inner.lock().unwrap();
        if q.len() == self.capacity {
            q.pop_front();
        }
        q.push_back(entry);
    }

    /// Entries at or above `min_level` (`error` > `warn` > `info` > `debug` > `trace`), oldest first.
    pub fn entries(&self, min_level: Option<&str>) -> Vec<LogEntry> {
        let min = min_level.map(rank).unwrap_or(0);
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter(|e| rank(&e.level) >= min)
            .cloned()
            .collect()
    }
}

fn rank(level: &str) -> u8 {
    match level.to_ascii_lowercase().as_str() {
        "error" => 4,
        "warn" => 3,
        "info" => 2,
        "debug" => 1,
        _ => 0,
    }
}

/// A tracing layer that records events into a [`LogBuffer`].
pub struct BufferLayer(pub LogBuffer);

struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = MessageVisitor {
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut v);
        let mut message = v.message;
        if !v.fields.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&v.fields.join(" "));
        }
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.0.push(LogEntry {
            time,
            level: level_name(*event.metadata().level()).into(),
            target: event.metadata().target().into(),
            message,
        });
    }
}

fn level_name(l: Level) -> &'static str {
    match l {
        Level::ERROR => "error",
        Level::WARN => "warn",
        Level::INFO => "info",
        Level::DEBUG => "debug",
        Level::TRACE => "trace",
    }
}

/// Install the global subscriber. `RUST_LOG` overrides the default filter
/// (`info` for nucleus crates, `warn` elsewhere). Returns the guard that flushes the file.
pub fn init(data_dir: &Path, buffer: LogBuffer) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = || {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,nucleus=info,nucleus_app=info,nucleus_harness=info,nucleus_sandbox=info,nucleus_vcs=info,nucleus_core=info"))
    };
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("nucleus")
        .filename_suffix("log")
        .max_log_files(7)
        .build(data_dir.join("logs"))
        .ok();
    let (file_layer, guard) = match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(writer)
                        .with_filter(filter()),
                ),
                Some(guard),
            )
        }
        None => (None, None),
    };
    let result = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(filter()),
        )
        .with(file_layer)
        .with(BufferLayer(buffer).with_filter(filter()))
        .try_init();
    if result.is_err() {
        // Already initialised (tests); keep going without the file guard.
        return None;
    }
    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_bounded_and_filters_by_level() {
        let b = LogBuffer::new(3);
        for (i, level) in ["debug", "info", "warn", "error"].iter().enumerate() {
            b.push(LogEntry {
                time: i as i64,
                level: level.to_string(),
                target: "t".into(),
                message: format!("m{i}"),
            });
        }
        let all = b.entries(None);
        assert_eq!(
            all.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(),
            ["m1", "m2", "m3"]
        );
        assert_eq!(b.entries(Some("warn")).len(), 2);
        assert_eq!(b.entries(Some("ERROR")).len(), 1);
        assert_eq!(b.entries(Some("bogus")).len(), 3);
        assert_eq!(LogBuffer::new(0).capacity, 1);
    }

    #[test]
    fn layer_captures_message_and_fields() {
        let b = LogBuffer::new(10);
        let subscriber = tracing_subscriber::registry().with(BufferLayer(b.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(container = "c1", "orphan cleanup failed");
            tracing::info!("plain");
        });
        let e = b.entries(None);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].level, "warn");
        assert_eq!(e[0].message, "orphan cleanup failed container=c1");
        assert_eq!(e[1].message, "plain");
    }

    #[test]
    fn init_writes_a_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let guard = init(dir.path(), LogBuffer::new(10));
        tracing::info!(target: "nucleus_app", "hello file");
        drop(guard);
        let logs: Vec<_> = std::fs::read_dir(dir.path().join("logs")).unwrap().flatten().collect();
        assert_eq!(logs.len(), 1);
        let text = std::fs::read_to_string(logs[0].path()).unwrap();
        assert!(text.contains("hello file"), "{text}");
    }
}
