use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc, Mutex,
    },
};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id},
    Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use crate::progress::client::{LspClient, ProgressMessage, CLIENT};

pub struct LspProgressLayer {
    span_ids: Mutex<HashMap<Id, i32>>,
    next: AtomicI32,
}

impl LspProgressLayer {
    pub fn new() -> Self {
        Self {
            span_ids: Mutex::new(HashMap::new()),
            next: AtomicI32::new(1),
        }
    }

    pub fn next_id(&self) -> i32 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for LspProgressLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for LspProgressLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        if *attrs.metadata().level() > tracing::Level::INFO {
            return;
        }

        with_client(|client| {
            let pid = self.next_id();
            if let Ok(mut ids) = self.span_ids.lock() {
                ids.insert(id.clone(), pid);
            }
            client.send(&ProgressMessage::Begin {
                id: pid,
                title: attrs.metadata().name().to_string(),
            });
        });
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if *event.metadata().level() > tracing::Level::INFO {
            return;
        }

        let pid = ctx
            .event_span(event)
            .and_then(|span_ref| self.span_ids.lock().ok()?.get(&span_ref.id()).copied());

        if let Some(pid) = pid {
            let mut visitor = MessageVisitor { message: None };
            event.record(&mut visitor);

            if let Some(message) = visitor.message {
                with_client(|client| {
                    client.send(&ProgressMessage::Report { id: pid, message });
                });
            }
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        if let Some(pid) = self
            .span_ids
            .lock()
            .ok()
            .and_then(|mut ids| ids.remove(&id))
        {
            with_client(|client| {
                client.send(&ProgressMessage::End { id: pid });
            });
        }
    }
}

struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

fn with_client<F>(f: F)
where
    F: FnOnce(&LspClient),
{
    let client = CLIENT
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().map(Arc::clone));
    if let Some(ref c) = client {
        f(c);
    }
}
