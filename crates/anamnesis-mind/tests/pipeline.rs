mod support;

use anamnesis_core::RegionTransition;
use anamnesis_mind::layout::{LayoutParams, layout_full};
use anamnesis_mind::regions::drift_between;
use anamnesis_mind::rng::Rng;
use anamnesis_mind::space::{Space, SpaceParams};
use anamnesis_mind::{MindParams, MindState, full_run};
use chrono::{Duration, Utc};
use support::*;

#[test]
fn full_run_twice_keeps_identity() {
    let mut r = Rng::new(21);
    let centers: Vec<Vec<f32>> = (0..4).map(|_| unit(&mut r, 24)).collect();
    let mut vs = vec![];
    let mut texts = vec![];
    for (k, c) in centers.iter().enumerate() {
        vs.extend(blob(&mut r, c, 50, 0.08));
        texts.extend((0..50).map(|i| format!("topic{k} word{k} note {i}")));
    }
    let ids = ids(vs.len(), "i");
    let t = Utc::now();
    let p = MindParams::default();
    let a = full_run(&ids, &vs, Some(&texts), &MindState::default(), t, &p);
    assert_eq!(a.placements.len(), 200);
    assert_eq!(a.regions.regions.len(), 4);
    assert!(a.regions.regions.iter().all(|x| x.anatomy.is_some()));
    assert_eq!(a.views.anatomy.len(), 200);

    // serialise state as the server would, then run again with a few new items
    let state: MindState = serde_json::from_str(&serde_json::to_string(&a.state).unwrap()).unwrap();
    let mut vs2 = vs.clone();
    let mut ids2 = ids.clone();
    let mut texts2 = texts.clone();
    vs2.extend(blob(&mut r, &centers[0], 5, 0.08));
    ids2.extend(support::ids(5, "n"));
    texts2.extend((0..5).map(|i| format!("topic0 new {i}")));
    let b = full_run(
        &ids2,
        &vs2,
        Some(&texts2),
        &state,
        t + Duration::days(7),
        &p,
    );
    assert!(
        b.regions
            .transitions
            .iter()
            .all(|x| matches!(x, RegionTransition::Survive { .. })),
        "{:?}",
        b.regions.transitions
    );
    assert!(b.procrustes_residual.unwrap() < 0.1);
    for (ra, rb) in a.regions.regions.iter().zip(&b.regions.regions) {
        let other = b.regions.regions.iter().find(|x| x.id == ra.id).unwrap();
        assert_eq!(other.anatomy, ra.anatomy);
        let _ = rb;
    }
    // drift: tiny for unchanged regions
    let drift = drift_between(&a.state.regions, &b.state.regions);
    assert_eq!(drift.len(), 4);
    assert!(drift.iter().all(|d| d.drift < 0.01), "{drift:?}");
}

#[test]
fn drift_detects_moving_region() {
    let mut r = Rng::new(4);
    let c1 = unit(&mut r, 24);
    let c2 = unit(&mut r, 24);
    let c3 = unit(&mut r, 24);
    let mut vs: Vec<Vec<f32>> = blob(&mut r, &c1, 60, 0.05);
    vs.extend(blob(&mut r, &c2, 60, 0.05));
    let ids = ids(120, "i");
    let p = MindParams::default();
    let a = full_run(&ids, &vs, None, &MindState::default(), Utc::now(), &p);
    // region 1 drifts halfway towards c3 and gets looser
    let mid: Vec<f32> = c1.iter().zip(&c3).map(|(a, b)| a + 0.5 * b).collect();
    let mut vs2 = blob(&mut r, &mid, 60, 0.09);
    vs2.extend(vs[60..].iter().cloned());
    let b = full_run(&ids, &vs2, None, &a.state, Utc::now(), &p);
    let d = &b.regions.drift;
    assert_eq!(d.len(), 2, "{:?}", b.regions.transitions);
    let moved = d.iter().max_by(|x, y| x.drift.total_cmp(&y.drift)).unwrap();
    let still = d.iter().min_by(|x, y| x.drift.total_cmp(&y.drift)).unwrap();
    assert!(
        moved.drift > 0.05 && still.drift < 0.01,
        "{moved:?} {still:?}"
    );
    assert!(moved.cohesion_after < moved.cohesion_before);
}

#[test]
fn layout_is_deterministic_for_a_seed() {
    let mut r = Rng::new(2);
    let c = unit(&mut r, 16);
    let vs = blob(&mut r, &c, 120, 0.2);
    let ids = ids(120, "i");
    let space = Space::new(&vs, SpaceParams::default());
    let a = layout_full(&ids, &space, None, &LayoutParams::default());
    let b = layout_full(&ids, &space, None, &LayoutParams::default());
    assert_eq!(a.layout, b.layout);
}

/// Sentence embeddings are anisotropic: a shared direction dominates and every
/// pair has a high cosine. Topics must still come out as separate regions.
#[test]
fn anisotropic_topics_become_regions() {
    let mut r = Rng::new(31);
    let d = 64;
    let common = unit(&mut r, d);
    let topics = 12;
    let mut vs = vec![];
    for _ in 0..topics {
        let c = unit(&mut r, d);
        for v in blob(&mut r, &c, 60, 0.12) {
            // baseline cosine ≈ 0.8 between any two items
            let mut w: Vec<f32> = v
                .iter()
                .zip(&common)
                .map(|(a, b)| 0.5 * a + 1.0 * b)
                .collect();
            let n = w.iter().map(|x| x * x).sum::<f32>().sqrt();
            w.iter_mut().for_each(|x| *x /= n);
            vs.push(w);
        }
    }
    let ids = ids(vs.len(), "i");
    let a = full_run(
        &ids,
        &vs,
        None,
        &MindState::default(),
        Utc::now(),
        &MindParams::default(),
    );
    let unsorted = a.regions.assignment.iter().filter(|x| x.is_none()).count();
    let largest = a.regions.regions.iter().map(|x| x.size).max().unwrap_or(0);
    assert!(
        (11..=16).contains(&a.regions.regions.len()),
        "{} regions",
        a.regions.regions.len()
    );
    assert!(unsorted * 100 / vs.len() < 35, "unsorted {unsorted}");
    assert!(largest <= 90, "largest region {largest}");
}
