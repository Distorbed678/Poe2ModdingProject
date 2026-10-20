//! Live log: a `tracing` layer keeps the last N events in a ring buffer and broadcasts new
//! ones; the WebSocket handler filters them per client.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{Layer, layer::Context};
use utoipa::ToSchema;

/// Severity, ordered from most severe to most verbose (`Error < Warn < ... < Trace`), so
/// "at least `min_level`" is `level <= min_level`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl From<&tracing::Level> for LogLevel {
    fn from(l: &tracing::Level) -> Self {
        match *l {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::TRACE => LogLevel::Trace,
        }
    }
}

/// One log event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LogEvent {
    /// Increases by one per event since the server started.
    pub seq: u64,
    /// Unix time in milliseconds.
    pub ts_ms: u64,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    /// The event's other fields, formatted as text.
    pub fields: BTreeMap<String, String>,
}

/// What a client wants to see. Sent as JSON over the WebSocket, first and whenever it changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct LogFilter {
    /// Events at this severity or worse.
    pub min_level: LogLevel,
    /// Only targets starting with this (`poe_net`, `instance_server::exec`).
    pub target: Option<String>,
    /// Only events whose message or fields contain this text (case-insensitive).
    pub text: Option<String>,
    /// How many of the newest matching buffered events to send first.
    pub history: usize,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            min_level: LogLevel::Info,
            target: None,
            text: None,
            history: 200,
        }
    }
}

impl LogFilter {
    pub fn matches(&self, e: &LogEvent) -> bool {
        if e.level > self.min_level {
            return false;
        }
        if let Some(prefix) = self.target.as_deref().filter(|t| !t.is_empty())
            && !e.target.starts_with(prefix)
        {
            return false;
        }
        if let Some(text) = self.text.as_deref().filter(|t| !t.is_empty()) {
            let needle = text.to_lowercase();
            let hit = e.message.to_lowercase().contains(&needle)
                || e.fields.iter().any(|(k, v)| {
                    k.to_lowercase().contains(&needle) || v.to_lowercase().contains(&needle)
                });
            if !hit {
                return false;
            }
        }
        true
    }
}

struct Inner {
    events: VecDeque<Arc<LogEvent>>,
    next_seq: u64,
}

/// The ring buffer plus the broadcast of new events.
pub struct LogBuffer {
    cap: usize,
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Arc<LogEvent>>,
}

impl LogBuffer {
    /// Keeps the last `cap` events (at least 1).
    pub fn new(cap: usize) -> Arc<Self> {
        let cap = cap.max(1);
        Arc::new(Self {
            cap,
            inner: Mutex::new(Inner {
                events: VecDeque::with_capacity(cap.min(8192)),
                next_seq: 1,
            }),
            tx: broadcast::channel(1024).0,
        })
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// The layer to add to the process's tracing subscriber.
    pub fn layer(self: &Arc<Self>) -> LogLayer {
        LogLayer {
            buffer: self.clone(),
        }
    }

    pub fn push(
        &self,
        level: LogLevel,
        target: &str,
        message: String,
        fields: BTreeMap<String, String>,
    ) {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        // The lock covers the broadcast too, so `subscribe` can take a consistent snapshot.
        let mut inner = self.inner.lock().unwrap();
        let event = Arc::new(LogEvent {
            seq: inner.next_seq,
            ts_ms,
            level,
            target: target.to_owned(),
            message,
            fields,
        });
        inner.next_seq += 1;
        if inner.events.len() == self.cap {
            inner.events.pop_front();
        }
        inner.events.push_back(event.clone());
        // No receivers is fine.
        let _ = self.tx.send(event);
    }

    /// The newest `filter.history` buffered events that match, oldest first.
    pub fn history(&self, filter: &LogFilter) -> Vec<Arc<LogEvent>> {
        let inner = self.inner.lock().unwrap();
        Self::pick(&inner, filter)
    }

    /// [`history`](Self::history) plus a receiver of every event after it (not filtered).
    pub fn subscribe(
        &self,
        filter: &LogFilter,
    ) -> (Vec<Arc<LogEvent>>, broadcast::Receiver<Arc<LogEvent>>) {
        let inner = self.inner.lock().unwrap();
        let rx = self.tx.subscribe();
        (Self::pick(&inner, filter), rx)
    }

    fn pick(inner: &Inner, filter: &LogFilter) -> Vec<Arc<LogEvent>> {
        let mut out: Vec<Arc<LogEvent>> = inner
            .events
            .iter()
            .rev()
            .filter(|e| filter.matches(e))
            .take(filter.history)
            .cloned()
            .collect();
        out.reverse();
        out
    }
}

/// `tracing_subscriber::Layer` feeding a [`LogBuffer`].
pub struct LogLayer {
    buffer: Arc<LogBuffer>,
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }
    }
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = FieldVisitor::default();
        event.record(&mut v);
        let meta = event.metadata();
        self.buffer
            .push(meta.level().into(), meta.target(), v.message, v.fields);
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    fn push(b: &LogBuffer, level: LogLevel, target: &str, msg: &str) {
        b.push(level, target, msg.into(), BTreeMap::new());
    }

    #[test]
    fn ring_buffer_keeps_the_newest() {
        let b = LogBuffer::new(3);
        for i in 0..5 {
            push(&b, LogLevel::Info, "t", &format!("m{i}"));
        }
        let all = LogFilter {
            history: 100,
            ..Default::default()
        };
        let got: Vec<_> = b
            .history(&all)
            .iter()
            .map(|e| (e.seq, e.message.clone()))
            .collect();
        assert_eq!(
            got,
            vec![(3, "m2".to_string()), (4, "m3".into()), (5, "m4".into())]
        );
        // `history` limits to the newest matching, still oldest first.
        let two = LogFilter {
            history: 2,
            ..Default::default()
        };
        let got: Vec<_> = b.history(&two).iter().map(|e| e.message.clone()).collect();
        assert_eq!(got, ["m3", "m4"]);
    }

    #[test]
    fn filters_by_level_target_and_text() {
        let b = LogBuffer::new(10);
        push(&b, LogLevel::Debug, "poe_net::conn", "bytes read");
        push(&b, LogLevel::Info, "instance_server", "player entered");
        push(&b, LogLevel::Warn, "instance_server::exec", "Slow frame");
        push(&b, LogLevel::Error, "login_server", "bad PLAYER packet");
        let msgs = |f: LogFilter| -> Vec<String> {
            b.history(&f).iter().map(|e| e.message.clone()).collect()
        };
        let base = LogFilter {
            history: 100,
            ..Default::default()
        };
        // Default: info and worse.
        assert_eq!(
            msgs(base.clone()),
            ["player entered", "Slow frame", "bad PLAYER packet"]
        );
        assert_eq!(
            msgs(LogFilter {
                min_level: LogLevel::Trace,
                ..base.clone()
            })
            .len(),
            4
        );
        assert_eq!(
            msgs(LogFilter {
                min_level: LogLevel::Warn,
                ..base.clone()
            }),
            ["Slow frame", "bad PLAYER packet"]
        );
        assert_eq!(
            msgs(LogFilter {
                target: Some("instance_server".into()),
                ..base.clone()
            }),
            ["player entered", "Slow frame"]
        );
        assert_eq!(
            msgs(LogFilter {
                text: Some("player".into()),
                ..base.clone()
            }),
            ["player entered", "bad PLAYER packet"]
        );
        assert_eq!(
            msgs(LogFilter {
                min_level: LogLevel::Warn,
                target: Some("instance_server".into()),
                text: Some("slow".into()),
                ..base
            }),
            ["Slow frame"]
        );
    }

    #[tokio::test]
    async fn subscribers_get_history_then_new_events() {
        let b = LogBuffer::new(10);
        push(&b, LogLevel::Info, "t", "old");
        let (history, mut rx) = b.subscribe(&LogFilter::default());
        assert_eq!(history.len(), 1);
        push(&b, LogLevel::Info, "t", "new");
        let e = rx.recv().await.unwrap();
        assert_eq!((e.seq, e.message.as_str()), (2, "new"));
    }

    #[test]
    fn layer_records_tracing_events() {
        let b = LogBuffer::new(10);
        let subscriber = tracing_subscriber::registry().with(b.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(account = "alice", ok = true, "login");
            tracing::debug!(target: "poe_net", "raw");
        });
        let got = b.history(&LogFilter {
            min_level: LogLevel::Trace,
            history: 10,
            ..Default::default()
        });
        assert_eq!(got.len(), 2);
        assert_eq!(
            (got[0].message.as_str(), got[0].level),
            ("login", LogLevel::Info)
        );
        assert_eq!(got[0].fields["account"], "alice");
        assert_eq!(got[0].fields["ok"], "true");
        assert_eq!(
            (got[1].target.as_str(), got[1].level),
            ("poe_net", LogLevel::Debug)
        );
    }
}
