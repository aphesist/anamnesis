//! Quick benchmark: `cargo test -p anamnesis-mind --release --test bench -- --ignored --nocapture`
mod support;

use std::time::Instant;

use anamnesis_mind::layout::{LayoutParams, layout_full, neighbourhood_preservation, place_new, views};
use anamnesis_mind::regions::{RegionExtras, RegionParams, RegionState, recompute_regions};
use anamnesis_mind::rng::Rng;
use anamnesis_mind::space::{Space, SpaceParams};
use chrono::Utc;
use support::*;

#[test]
#[ignore]
fn bench_20k_384d() {
    let n = 20_000;
    let d = 384;
    let mut r = Rng::new(1);
    let k = 60;
    let centers: Vec<Vec<f32>> = (0..k).map(|_| unit(&mut r, d)).collect();
    let vs: Vec<Vec<f32>> = (0..n)
        .map(|i| if i % 20 == 7 { unit(&mut r, d) } else { around(&mut r, &centers[i % k], 0.04 + 0.02 * ((i % 7) as f32 / 7.0)) })
        .collect();
    let ids = ids(n, "i");

    let t = Instant::now();
    let space = Space::new(&vs, SpaceParams::default());
    let t_space = t.elapsed();

    let t = Instant::now();
    let run = recompute_regions(&ids, &space, &RegionState::default(), Utc::now(), &RegionParams::default(), RegionExtras::default());
    let t_regions = t.elapsed();

    let t = Instant::now();
    let lay = layout_full(&ids, &space, None, &LayoutParams::default());
    let t_layout = t.elapsed();

    let t = Instant::now();
    let _seq = layout_full(&ids, &space, None, &LayoutParams { parallel_above: None, ..Default::default() });
    eprintln!("layout (fresh, sequential): {:?}", t.elapsed());

    let t = Instant::now();
    let lay2 = layout_full(&ids, &space, Some(&lay.layout), &LayoutParams::default());
    let t_relayout = t.elapsed();

    let t = Instant::now();
    let v = views(&lay.layout, &run.assignment);
    let t_views = t.elapsed();

    let t = Instant::now();
    let mut l = lay.layout.clone();
    let new_vs: Vec<Vec<f32>> = (0..100).map(|i| around(&mut r, &centers[i % k], 0.05)).collect();
    place_new(&mut l, &vs, &support::ids(100, "n"), &new_vs, &LayoutParams::default());
    let t_place = t.elapsed();

    let noise_assigned = (0..n).filter(|i| i % 20 == 7 && run.assignment[*i].is_some()).count();
    // purity: majority blob share per region
    let mut purity_hits = 0;
    let mut total = 0;
    for reg in &run.regions {
        let mut counts = std::collections::HashMap::new();
        for i in (0..n).filter(|i| i % 20 != 7 && run.assignment[*i].as_ref() == Some(&reg.id)) {
            *counts.entry(i % k).or_insert(0) += 1;
            total += 1;
        }
        purity_hits += counts.values().max().copied().unwrap_or(0);
    }
    eprintln!("random points assigned to a region: {noise_assigned}/{}, region purity {:.3}", n / 20, purity_hits as f32 / total.max(1) as f32);
    let np = neighbourhood_preservation(&space, &lay.layout.xyz, 10, 200);
    eprintln!("n={n} d={d}");
    eprintln!("space (pca + exact knn): {t_space:?}");
    eprintln!("regions: {t_regions:?} -> {} regions, {} unsorted", run.regions.len(), run.assignment.iter().filter(|a| a.is_none()).count());
    eprintln!("layout (fresh): {t_layout:?}, neighbourhood preservation@10 = {np:.3}");
    eprintln!("layout (refine + procrustes): {t_relayout:?}, residual = {:?}", lay2.procrustes_residual);
    eprintln!("views (anatomy + spheres): {t_views:?} ({} pts)", v.anatomy.len());
    eprintln!("place 100 new points: {t_place:?}");
}
