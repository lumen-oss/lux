use indicatif::{MultiProgress, ProgressBar, ProgressFinish};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::span;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

pub struct ProgressLayer {
    multi: Arc<MultiProgress>,
    bars: Mutex<HashMap<span::Id, ProgressBar>>,
}

impl ProgressLayer {
    pub fn new() -> Self {
        Self {
            multi: Arc::new(MultiProgress::new()),
            bars: Mutex::new(HashMap::new()),
        }
    }

    pub fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.multi.suspend(f)
    }
}

impl<S> Layer<S> for ProgressLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, _attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span must exist");
        let bar = ProgressBar::new_spinner()
            .with_finish(ProgressFinish::AndClear)
            .with_message(span.name().to_string());
        let bar = self.multi.add(bar);
        self.bars.lock().unwrap().insert(id.clone(), bar);
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let span_id = match ctx.event_span(event) {
            Some(span_ref) => span_ref.id(),
            None => return,
        };
        if let Some(bar) = self.bars.lock().unwrap().get(&span_id) {
            let mut visitor = MessageVisitor::default();
            event.record(&mut visitor);
            if let Some(msg) = visitor.message {
                bar.set_message(msg);
            }
        }
    }

    fn on_close(&self, id: span::Id, _ctx: Context<'_, S>) {
        if let Some(bar) = self.bars.lock().unwrap().remove(&id) {
            bar.finish_and_clear();
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        }
    }

    fn record_str(&mut self, _field: &tracing::field::Field, _value: &str) {}
}
