use std::{fs::File, io::Read, collections::{HashSet, HashMap}, ops::Deref, rc::Rc, num::NonZeroU64};

use tracing_report::{Report, ReportPayload};
use tracing_serde_structured as tss;
use tss::CowString;

#[derive(Clone)]
struct Element {
    rpt: Rc<Report<'static>>,
}

impl Deref for Element {
    type Target = Report<'static>;

    fn deref(&self) -> &Self::Target {
        &self.rpt
    }
}

struct Elements {
    rpts: Vec<Element>,
}

struct TlSpans {
    spans: Vec<Span>,
    events: Vec<tss::SerializeEvent<'static>>,
}

struct Span {
    start: u128,
    end: u128,
    spans: Vec<Span>,
    events: Vec<tss::SerializeEvent<'static>>,
    attrs: tss::SerializeAttributes<'static>,
}

impl Span {
    fn count_events_rec(&self) -> (usize, usize) {
        let own_events = self.events.len();
        let mut child_events = 0;
        self.spans.iter().for_each(|s| {
            let (own, child) = s.count_events_rec();
            child_events += own;
            child_events += child;
        });
        (own_events, child_events)
    }

    fn print_spans_rec(&self, depth: usize, indent: usize) {
        if depth == 0 {
            return;
        }

        for span in self.spans.iter() {
            for _ in 0..indent {
                print!("-");
            }
            print!("> ");

            let (oevt, cevt) = span.count_events_rec();
            println!(
                "[SPAN | {}ns | {}:{}] ({} events, {} child events)",
                span.end - span.start,
                span.attrs.metadata.file.as_deref().unwrap_or("???"),
                span.attrs.metadata.line.unwrap_or(0),
                oevt,
                cevt,
            );
            span.print_spans_rec(depth - 1, indent + 2);
        }
    }
}

fn capture_span(
    map: &HashMap<NonZeroU64, tss::SerializeAttributes<'static>>,
    stack: &mut Vec<Element>,
    id_span: NonZeroU64,
    start: u128,
) -> Span {
    let mut spans = vec![];
    let mut events = vec![];
    loop {
        let pop = stack.pop();
        let Report { tick, payload, .. } = pop.as_deref().unwrap();
        match payload {
            ReportPayload::OnEvent { event } => {
                events.push(event.to_owned());
            },
            ReportPayload::OnEnter { span } => {
                spans.push(capture_span(map, stack, span.id, *tick));
            },
            ReportPayload::OnExit { span } => {
                assert_eq!(span.id, id_span);
                return Span {
                    start,
                    end: *tick,
                    spans,
                    events,
                    attrs: map.get(&id_span).unwrap().to_owned(),
                };
            },
            _ => continue,
        }
    }
}

impl Elements {
    fn spanner(&self) -> TlSpans {
        let mut map = HashMap::new();
        self.rpts.iter().for_each(|rpt| {
            if let ReportPayload::OnNewSpan { attrs, id } = &rpt.payload {
                map.insert(
                    id.id,
                    attrs.to_owned(),
                );
            }
        });

        // Reverse, so we can pop off the end.
        let mut stack = self.rpts.iter().rev().cloned().collect::<Vec<Element>>();
        let mut spans = vec![];
        let mut events = vec![];

        loop {
            let pop = stack.pop();
            let Report { tick, payload, .. } = if let Some(rpt) = pop.as_deref() {
                rpt
            } else {
                break;
            };

            match payload {
                ReportPayload::OnEvent { event } => {
                    events.push(event.to_owned());
                },
                ReportPayload::OnEnter { span } => {
                    spans.push(capture_span(&map, &mut stack, span.id, *tick));
                },
                ReportPayload::OnExit { .. } => {
                    panic!()
                },
                _ => continue,
            }
        }

        TlSpans { spans, events }
    }



    fn split_by_thread_id(&self) -> Vec<(u64, Elements)> {
        let mut chunky = HashMap::new();

        self.rpts.iter().for_each(|rpt| {
            if !chunky.contains_key(&rpt.thread_id) {
                chunky.insert(rpt.thread_id, vec![]);
            }
            chunky.get_mut(&rpt.thread_id).unwrap().push(rpt.clone());
        });

        chunky
            .drain()
            .map(|(id, vr)| (id, Elements { rpts: vr }))
            .collect()
    }

    fn events_by_location(&self) -> Vec<(String, Vec<tss::SerializeRecordFields<'static>>)> {
        let mut chunky = HashMap::new();

        self.rpts.iter().for_each(|rpt| {
            if let ReportPayload::OnEvent { ref event } = &rpt.rpt.payload {
                let key = format!(
                    "{}:{}",
                    event.metadata.file.as_deref().unwrap_or("???"),
                    event.metadata.line.unwrap_or(0),
                );

                if !chunky.contains_key(&key) {
                    chunky.insert(key.clone(), vec![]);
                }
                chunky.get_mut(&key).unwrap().push(event.fields.to_owned());
            }


        });

        chunky
            .drain()
            .collect()
    }
}

impl<'a> From<Report<'a>> for Element {
    fn from(other: Report<'a>) -> Self {
        Self {
            rpt: Rc::new(other.to_owned()),
        }
    }
}

fn main() {
    let mut file = File::open("report.bin").unwrap();
    let mut contents = vec![];
    file.read_to_end(&mut contents).unwrap();
    let data: Vec<Element> = contents
        .split_mut(|&b| b == 0)
        .map(postcard::from_bytes_cobs::<tracing_report::Report>)
        .filter_map(Result::ok)
        .map(Into::into)
        .collect();

    let elements = Elements { rpts: data };

    let by_thread = elements.split_by_thread_id();

    for (thread_id, elements) in by_thread.iter() {
        println!("THREAD {}", thread_id);
        println!();

        let tl_span = elements.spanner();
        for span in tl_span.spans.iter() {
            let (oevt, cevt) = span.count_events_rec();
            println!(
                "[SPAN | {}ns | {}:{}] ({} events, {} child events)",
                span.end - span.start,
                span.attrs.metadata.file.as_deref().unwrap_or("???"),
                span.attrs.metadata.line.unwrap_or(0),
                oevt,
                cevt,
            );
            span.print_spans_rec(4, 2);
        }
    }

    // for (thread_id, elements) in by_thread.iter() {
    //     println!("THREAD {}", thread_id);
    //     let mut ebl = elements.events_by_location();
    //     ebl.sort_unstable_by_key(|(key, _evts)| key.clone());
    //     for (key, events) in ebl.iter() {
    //         println!("{} | {} | {} INSTANCES", thread_id, key, events.len());
    //         for rec in events.iter() {
    //             print!("    |> ");
    //             match rec {
    //                 tss::SerializeRecordFields::Ser(_) => panic!(),
    //                 tss::SerializeRecordFields::De(fields) => {
    //                     let mut data: Vec<(&CowString, &tss::SerializeValue)> = fields.iter().collect();
    //                     data.sort_unstable_by_key(|(key, _val)| key.as_str());
    //                     for (key, val) in data.iter() {
    //                         print!("{} = ", key.as_str());
    //                         match val {
    //                             tss::SerializeValue::Debug(tss::DebugRecord::De(x)) => print!("{}, ", x.as_str()),
    //                             tss::SerializeValue::Str(x) => print!("{}, ", x.as_str()),
    //                             tss::SerializeValue::F64(x) => print!("{}, ", x),
    //                             tss::SerializeValue::I64(x) => print!("{}, ", x),
    //                             tss::SerializeValue::U64(x) => print!("{}, ", x),
    //                             tss::SerializeValue::Bool(x) => print!("{}, ", x),
    //                             _ => print!("???, "),
    //                         }
    //                     }
    //                 },
    //             }
    //             println!("|");
    //         }
    //     }
    //     for _ in 0..3 {
    //         println!();
    //     }
    // }



    // for (thread_id, reports) in by_thread.iter() {
    //     let mut spans = HashMap::new();
    //     reports.rpts.iter().for_each(|rpt| {
    //         if let ReportPayload::OnNewSpan { attrs, id } = &rpt.payload {
    //             spans.insert(
    //                 id.id,
    //                 format!(
    //                     "[SPAN | {}:{}]",
    //                     attrs.metadata.file.as_deref().unwrap_or("???"),
    //                     attrs.metadata.line.unwrap_or(0),
    //                 )
    //             );
    //         }
    //     });

    //     let mut indent = 0;

    //     println!("============================================================");
    //     println!("THREAD {}", thread_id);
    //     println!("============================================================");

    //     for report in reports.rpts.iter() {
    //         match &report.payload {
    //             ReportPayload::OnEvent { event } => {
    //                 print!(" {:016} |", report.tick);
    //                 for _ in 0..indent {
    //                     print!(" ");
    //                 }
    //                 println!(
    //                     "[EVENT | {}:{}]",
    //                     event.metadata.file.as_deref().unwrap_or("???"),
    //                     event.metadata.line.unwrap_or(0),
    //                 );
    //             },
    //             ReportPayload::OnEnter { span } => {
    //                 print!(" {:016} |", report.tick);
    //                 for _ in 0..indent {
    //                     print!(" ");
    //                 }
    //                 if spans.contains_key(&span.id) {
    //                     println!("{}", spans[&span.id]);
    //                 } else {
    //                     println!("[SPAN | ???]");
    //                 }
    //                 indent += 2;
    //             },
    //             ReportPayload::OnExit { .. } => {
    //                 print!(" {:016} |", report.tick);
    //                 indent -= 2;
    //                 for _ in 0..indent {
    //                     print!(" ");
    //                 }
    //                 println!("<-");
    //             },
    //             _ => {},
    //         }
    //     }

    //     for _ in 0..5 {
    //         println!();
    //     }
    // }
}
