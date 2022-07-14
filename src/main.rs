use std::{fs::File, io::Read, collections::{HashSet, HashMap}, ops::Deref};

use tracing_report::{Report, ReportPayload};

fn main() {
    let mut file = File::open("report.bin").unwrap();
    let mut contents = vec![];
    file.read_to_end(&mut contents).unwrap();
    let mut data: Vec<Report<'static>> = contents
        .split_mut(|&b| b == 0)
        .map(postcard::from_bytes_cobs::<tracing_report::Report>)
        .filter_map(Result::ok)
        .map(|rpt| rpt.to_owned())
        .collect();

    let mut chunky = HashMap::new();

    data.drain(..).for_each(|rpt| {
        if !chunky.contains_key(&rpt.thread_id) {
            chunky.insert(rpt.thread_id, vec![]);
        }
        chunky.get_mut(&rpt.thread_id).unwrap().push(rpt);
    });

    for (thread_id, reports) in chunky.into_iter() {
        let mut spans = HashMap::new();
        reports.iter().for_each(|rpt| {
            if let ReportPayload::OnNewSpan { attrs, id } = &rpt.payload {
                spans.insert(
                    id.id,
                    format!(
                        "[SPAN | {}:{}]",
                        attrs.metadata.file.as_deref().unwrap_or("???"),
                        attrs.metadata.line.unwrap_or(0),
                    )
                );
            }
        });

        let mut indent = 0;

        println!("============================================================");
        println!("THREAD {}", thread_id);
        println!("============================================================");

        for report in reports.iter() {
            match &report.payload {
                ReportPayload::OnEvent { event } => {
                    print!(" {:016} |", report.tick);
                    for _ in 0..indent {
                        print!(" ");
                    }
                    println!(
                        "[EVENT | {}:{}]",
                        event.metadata.file.as_deref().unwrap_or("???"),
                        event.metadata.line.unwrap_or(0),
                    );
                },
                ReportPayload::OnEnter { span } => {
                    print!(" {:016} |", report.tick);
                    for _ in 0..indent {
                        print!(" ");
                    }
                    if spans.contains_key(&span.id) {
                        println!("{}", spans[&span.id]);
                    } else {
                        println!("[SPAN | ???]");
                    }
                    indent += 2;
                },
                ReportPayload::OnExit { .. } => {
                    print!(" {:016} |", report.tick);
                    indent -= 2;
                    for _ in 0..indent {
                        print!(" ");
                    }
                    println!("<-");
                },
                _ => {},
            }
        }

        for _ in 0..5 {
            println!();
        }
    }
}
