//! Print a synthetic event log or its evaluation report as json.
//!
//! cargo run -p anamnesis-eval --example synthetic -- report [seed]
//! cargo run -p anamnesis-eval --example synthetic -- events [seed]   (json lines)
//! cargo run -p anamnesis-eval --example synthetic -- honesty [seed]  (markdown)

use anamnesis_eval::synthetic::{self, SyntheticConfig, SyntheticSearch};
use anamnesis_eval::{EvalConfig, evaluate};

fn main() {
    let mut args = std::env::args().skip(1);
    let what = args.next().unwrap_or_else(|| "report".into());
    let seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let events = synthetic::generate(&SyntheticConfig {
        seed,
        ..SyntheticConfig::default()
    });
    match what.as_str() {
        "events" => {
            for e in &events {
                println!("{}", serde_json::to_string(e).unwrap());
            }
        }
        _ => {
            let search = SyntheticSearch::new(&events, seed);
            let mut f = |q: &str| search.search(q);
            let mut report = evaluate(&events, Some(&mut f), None, &EvalConfig::default());
            if what == "honesty" {
                print!("{}", report.honesty.to_markdown());
            } else {
                // per-query rows make the report long; keep it readable
                if let Some(r) = report.retrieval.as_mut() {
                    r.queries.truncate(5);
                }
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            }
        }
    }
}
