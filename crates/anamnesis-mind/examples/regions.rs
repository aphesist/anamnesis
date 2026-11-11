//! Cluster a real brain and print region statistics (a tuning aid).
//!
//! ```sh
//! cargo run --release -p anamnesis-mind --example regions -- brain.sqlite local:multilingual-e5-small@384
//! ```
//!
//! Opens the store at the given path (use a copy: opening may migrate it).
//! Environment overrides for experiments: `MCS` (min cluster size), `MS`
//! (min samples), `COH` (min relative cohesion), `MIN_ITEMS`, `K` (kNN),
//! `REDUCE` (PCA dims), `WHITEN` (0/1), `MAXSHARE` (max region share),
//! `LEAF` (0/1), `ON_LAYOUT` (0/1), `LMCS`, `LMS`, `LLEAF`, `LMAX`, `VOTE` (layout clustering).

use std::time::Instant;

use anamnesis_core::Store;
use anamnesis_mind::{MindParams, MindState, RegionParams, SpaceParams, full_run};

fn env<T: std::str::FromStr>(k: &str) -> Option<T> {
    std::env::var(k).ok().and_then(|v| v.parse().ok())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .ok_or("usage: regions <brain.sqlite> <model id>")?;
    let model = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("local:multilingual-e5-small@384");
    let store = Store::open(path)?;
    let (ids, vectors) = store.item_vectors(model)?;
    let mut texts = Vec::with_capacity(ids.len());
    for id in &ids {
        let it = store.get_item(id)?;
        texts.push(
            it.map(|i| format!("{}\n{}", i.title, i.body))
                .unwrap_or_default(),
        );
    }
    let mut sp = SpaceParams::default();
    if let Some(k) = env("K") {
        sp.k = k;
    }
    if let Some(r) = env("REDUCE") {
        sp.reduce_to = r;
    }
    if let Some(w) = env::<u8>("WHITEN") {
        sp.whiten = w != 0;
    }
    let mut rp = RegionParams::default();
    if let Some(v) = env("MAXSHARE") {
        rp.max_cluster_share = Some(v);
    }
    if let Some(v) = env::<u8>("LEAF") {
        rp.leaf = v != 0;
    }
    if let Some(v) = env("MCS") {
        rp.min_cluster_size = Some(v);
    }
    if let Some(v) = env("MS") {
        rp.min_samples = v;
    }
    if let Some(v) = env("COH") {
        rp.min_relative_cohesion = v;
    }
    if let Some(v) = env("MIN_ITEMS") {
        rp.min_items = v;
    }
    if let Some(v) = env::<u8>("ON_LAYOUT") {
        rp.cluster_on_layout = v != 0;
    }
    if let Some(v) = env("LMCS") {
        rp.layout_min_cluster_size = Some(v);
    }
    if let Some(v) = env("LMS") {
        rp.layout_min_samples = v;
    }
    if let Some(v) = env::<u8>("LLEAF") {
        rp.layout_leaf = v != 0;
    }
    if let Some(v) = env::<f32>("LMAX") {
        rp.layout_max_share = (v > 0.0).then_some(v);
    }
    if let Some(v) = env("VOTE") {
        rp.layout_vote = v;
    }
    let params = MindParams {
        space: sp,
        regions: rp,
        ..MindParams::default()
    };
    let t = Instant::now();
    let full = full_run(
        &ids,
        &vectors,
        Some(&texts),
        &MindState::default(),
        chrono::Utc::now(),
        &params,
    );
    let run = full.regions;
    let n = ids.len();
    let unsorted = run.assignment.iter().filter(|a| a.is_none()).count();
    let mut sizes: Vec<u32> = run.regions.iter().map(|r| r.size).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    println!(
        "items {n}, regions {}, unsorted {unsorted} ({:.1}%), {:.1}s",
        run.regions.len(),
        100.0 * unsorted as f64 / n.max(1) as f64,
        t.elapsed().as_secs_f64()
    );
    println!("sizes {sizes:?}");
    let mut regions = run.regions.clone();
    regions.sort_by_key(|r| std::cmp::Reverse(r.size));
    for r in &regions {
        println!("{:>6}  {}", r.size, r.label);
    }
    Ok(())
}
