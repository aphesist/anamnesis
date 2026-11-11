//! Run the rule-based thread extractor over a real brain and print counts and a
//! random sample (a tuning aid for precision).
//!
//! ```sh
//! cargo run --release -p anamnesis-mind --example threads -- brain.sqlite [sample size] [seed]
//! ```
//!
//! Opens the store at the given path (use a copy: opening may migrate it).
//! Only owner-written items are read, as the server does; items whose
//! frontmatter declares another origin are skipped too.

use std::collections::BTreeMap;

use anamnesis_core::{ItemFilter, Origin, Store};
use anamnesis_mind::rng::Rng;
use anamnesis_mind::threads::extract_rules;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .ok_or("usage: threads <brain.sqlite> [sample] [seed]")?;
    let sample: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let store = Store::open(path)?;
    let items = store.list_items(&ItemFilter::default())?;
    let mut all = Vec::new();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut read = 0;
    for it in &items {
        if it.deleted || !it.origin.is_owner() {
            continue;
        }
        let declared = it
            .meta
            .get("frontmatter")
            .and_then(|f| f.get("origin"))
            .and_then(|o| serde_json::from_value::<Origin>(o.clone()).ok());
        if declared.is_some_and(|o| !o.is_owner()) {
            continue;
        }
        read += 1;
        for t in extract_rules(&it.id, &it.body, it.created_at) {
            *by_kind
                .entry(format!("{:?}", t.kind).to_lowercase())
                .or_default() += 1;
            all.push((it.title.clone(), t));
        }
    }
    println!(
        "items {}, owner items read {read}, threads {}",
        items.len(),
        all.len()
    );
    for (k, n) in &by_kind {
        println!("  {k:<14} {n}");
    }
    let mut rng = Rng::new(seed);
    let mut idx: Vec<usize> = (0..all.len()).collect();
    for i in (1..idx.len()).rev() {
        let j = rng.below(i + 1);
        idx.swap(i, j);
    }
    println!("\nsample of {}:", sample.min(all.len()));
    for (n, &i) in idx.iter().take(sample).enumerate() {
        let (title, t) = &all[i];
        println!(
            "{:>2}. [{:?}] {} — {}",
            n + 1,
            t.kind,
            title,
            t.quote.replace('\n', " ")
        );
    }
    if std::env::var("ALL").is_ok() {
        println!("\nall:");
        for (title, t) in &all {
            println!("[{:?}] {} — {}", t.kind, title, t.quote.replace('\n', " "));
        }
    }
    Ok(())
}
