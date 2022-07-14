use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::{
    fs::File,
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{sync_channel, RecvTimeoutError, SyncSender},
    },
    thread::spawn,
    thread_local,
    time::{Duration, Instant},
};
use tracing::{
    span::{Attributes, Id, Record},
    Subscriber,
};
use tracing_subscriber::layer::{Context, Layer};
use tss::AsSerde;

static START: Lazy<Instant> = Lazy::new(Instant::now);
static THREAD_ID: AtomicU64 = AtomicU64::new(1);
static PRODUCER: Lazy<SyncSender<Vec<u8>>> = Lazy::new(|| {
    let (tx, rx) = sync_channel::<Vec<u8>>(128);
    spawn(move || {
        let mut f = File::create("report.bin").unwrap();
        f.sync_all().unwrap();
        let mut last_flush = Instant::now();

        loop {
            if last_flush.elapsed() > Duration::from_millis(250) {
                f.sync_all().unwrap();
                last_flush = Instant::now();
            }

            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(msg) => f.write_all(&msg).unwrap(),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    f.sync_all().unwrap();
                    return;
                }
            }
        }
    });
    tx
});

pub struct ReportLayer;

impl ReportLayer {
    thread_local! {
        static LOCAL_METADATA: Lazy<u64> = Lazy::new(|| {
            THREAD_ID.fetch_add(1, Ordering::Relaxed)
        });
    }

    fn handle_message(&self, payload: ReportPayload<'_>) {
        let thread_id = Self::LOCAL_METADATA.with(|id| *id.deref());
        let msg = Report {
            tick: START.elapsed().as_nanos(),
            thread_id,
            payload,
        };
        let ser_msg = postcard::to_stdvec_cobs(&msg).unwrap();
        let _ = PRODUCER.send(ser_msg);
    }
}

use tracing_serde_structured as tss;

#[derive(Debug, Deserialize, Serialize)]
pub struct Report<'a> {
    pub tick: u128,
    pub thread_id: u64,
    #[serde(borrow)]
    pub payload: ReportPayload<'a>,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ReportPayload<'a> {
    OnNewSpan {
        #[serde(borrow)]
        attrs: tss::SerializeAttributes<'a>,
        id: tss::SerializeId,
    },
    OnRecord {
        span: tss::SerializeId,
        values: tss::SerializeRecord<'a>,
    },
    OnFollowsFrom {
        span: tss::SerializeId,
        follows: tss::SerializeId,
    },
    OnEvent {
        event: tss::SerializeEvent<'a>,
    },
    OnEnter {
        span: tss::SerializeId,
    },
    OnExit {
        span: tss::SerializeId,
    },
    OnIdChange {
        old: tss::SerializeId,
        new: tss::SerializeId,
    },
    OnClose {
        span: tss::SerializeId,
    },
}

impl<'a> Report<'a> {
    pub fn to_owned(&self) -> Report<'static> {
        Report {
            tick: self.tick,
            thread_id: self.thread_id,
            payload: self.payload.to_owned(),
        }
    }
}

impl<'a> ReportPayload<'a> {
    pub fn to_owned(&self) -> ReportPayload<'static> {
        match self {
            ReportPayload::OnNewSpan { attrs, id } => ReportPayload::OnNewSpan { attrs: attrs.to_owned(), id: id.to_owned() },
            ReportPayload::OnRecord { span, values } => ReportPayload::OnRecord { span: span.to_owned(), values: values.to_owned() },
            ReportPayload::OnFollowsFrom { span, follows } => ReportPayload::OnFollowsFrom { span: span.to_owned(), follows: follows.to_owned() },
            ReportPayload::OnEvent { event } => ReportPayload::OnEvent { event: event.to_owned() },
            ReportPayload::OnEnter { span } => ReportPayload::OnEnter { span: span.to_owned() },
            ReportPayload::OnExit { span } => ReportPayload::OnExit { span: span.to_owned() },
            ReportPayload::OnIdChange { old, new } => ReportPayload::OnIdChange { old: old.to_owned(), new: new.to_owned() },
            ReportPayload::OnClose { span } => ReportPayload::OnClose { span: span.to_owned() },
        }
    }
}

impl<S> Layer<S> for ReportLayer
where
    S: Subscriber,
{
    fn enabled(&self, _metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        // always enabled for all levels
        true
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnNewSpan {
            attrs: attrs.as_serde(),
            id: id.as_serde(),
        });
    }

    fn on_record(&self, span: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnRecord {
            span: span.as_serde(),
            values: values.as_serde(),
        })
    }

    fn on_follows_from(&self, span: &Id, follows: &Id, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnFollowsFrom {
            span: span.as_serde(),
            follows: follows.as_serde(),
        })
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnEvent {
            event: event.as_serde(),
        })
    }

    fn on_enter(&self, span: &Id, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnEnter {
            span: span.as_serde(),
        })
    }

    fn on_exit(&self, span: &Id, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnExit {
            span: span.as_serde(),
        })
    }

    fn on_id_change(&self, old: &Id, new: &Id, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnIdChange {
            old: old.as_serde(),
            new: new.as_serde(),
        })
    }

    fn on_close(&self, span: Id, _ctx: Context<'_, S>) {
        self.handle_message(ReportPayload::OnClose {
            span: span.as_serde(),
        })
    }
}
