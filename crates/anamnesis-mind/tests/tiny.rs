//! Tiny corpora (fewer items than the kNN needs, down to zero) must give finite
//! coordinates and sensible output everywhere, with no panics.

mod support;

use anamnesis_core::{Id, Link, LinkKind};
use anamnesis_mind::layout::{LayoutParams, layout_full, place_new, views};
use anamnesis_mind::links::{ItemMeta, LinkParams, LinkSuggester};
use anamnesis_mind::rng::Rng;
use anamnesis_mind::space::{Space, SpaceParams};
use anamnesis_mind::{MindParams, MindState, full_run};
use chrono::{Duration, Utc};
use support::*;

const D: usize = 16;

fn finite(ps: &[[f32; 3]], what: &str, n: usize) {
    for (i, p) in ps.iter().enumerate() {
        assert!(
            p.iter().all(|c| c.is_finite()),
            "{what}: non-finite point {i} {p:?} at n = {n}"
        );
    }
}

/// Corpora of several shapes for a given size.
fn corpora(n: usize, seed: u64) -> Vec<(&'static str, Vec<Vec<f32>>)> {
    let mut r = Rng::new(seed);
    let c = unit(&mut r, D);
    let random: Vec<Vec<f32>> = (0..n).map(|_| unit(&mut r, D)).collect();
    let blob = blob(&mut r, &c, n, 0.05);
    let same: Vec<Vec<f32>> = vec![c.clone(); n];
    // one zero vector among real ones (an empty or failed embedding)
    let mut zero = random.clone();
    if let Some(v) = zero.first_mut() {
        v.iter_mut().for_each(|x| *x = 0.0);
    }
    vec![
        ("random", random),
        ("blob", blob),
        ("identical", same),
        ("zero", zero),
    ]
}

#[test]
fn full_run_is_finite_for_tiny_corpora() {
    let t = Utc::now();
    let p = MindParams::default();
    for n in 0..=40 {
        for (shape, vs) in corpora(n, n as u64 + 1) {
            let ids = ids(n, "i");
            let texts: Vec<String> = (0..n).map(|i| format!("note {i} about {shape}")).collect();
            let a = full_run(&ids, &vs, Some(&texts), &MindState::default(), t, &p);
            let tag = format!("{shape} full_run");
            assert_eq!(a.placements.len(), n, "{tag} n={n}");
            assert_eq!(a.regions.assignment.len(), n, "{tag} n={n}");
            for pl in &a.placements {
                assert!(
                    pl.xyz.iter().all(|c| c.is_finite()),
                    "{tag}: placement {pl:?} at n = {n}"
                );
            }
            finite(&a.views.layout, &format!("{tag} layout"), n);
            finite(&a.views.anatomy, &format!("{tag} anatomy"), n);
            finite(&a.views.spheres, &format!("{tag} spheres"), n);
            assert!(
                a.procrustes_residual.is_none_or(|r| r.is_finite()),
                "{tag} n={n}"
            );
            if n < p.regions.min_items {
                assert!(a.regions.unsorted, "{tag} n={n}");
                assert!(a.regions.regions.is_empty(), "{tag} n={n}");
                assert!(a.regions.assignment.iter().all(|x| x.is_none()));
            }
            for r in &a.state.regions.regions {
                assert!(r.centroid.iter().all(|c| c.is_finite()), "{tag} n={n}");
                assert!(r.cohesion.is_finite() && r.assign_threshold.is_finite());
            }
            // the state round-trips through json (NaN would not) and a second run works
            let s = serde_json::to_string(&a.state).expect("state serialises");
            let state: MindState = serde_json::from_str(&s).unwrap();
            let mut vs2 = vs.clone();
            let mut ids2 = ids.clone();
            vs2.push(unit(&mut Rng::new(99), D));
            ids2.push(Id("new".into()));
            let b = full_run(&ids2, &vs2, None, &state, t + Duration::days(1), &p);
            assert_eq!(b.placements.len(), n + 1);
            finite(&b.views.layout, &format!("{tag} second run"), n + 1);
            assert!(b.procrustes_residual.is_none_or(|r| r.is_finite()));
        }
    }
}

#[test]
fn layout_and_place_new_are_finite_for_tiny_corpora() {
    let lp = LayoutParams::default();
    for n in 0..=40 {
        for (shape, vs) in corpora(n, 1000 + n as u64) {
            let ids = ids(n, "i");
            let space = Space::new(&vs, SpaceParams::default());
            let run = layout_full(&ids, &space, None, &lp);
            assert_eq!(run.layout.xyz.len(), n);
            finite(&run.layout.xyz, &format!("{shape} layout_full"), n);
            // a fresh layout is centred and scaled to about unit radius
            for p in &run.layout.xyz {
                let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                assert!(r <= 3.0, "{shape}: radius {r} at n = {n}");
            }
            if shape == "random" && n >= 2 {
                let spread = run
                    .layout
                    .xyz
                    .iter()
                    .map(|p| (p[0] - run.layout.xyz[0][0]).abs())
                    .fold(0f32, f32::max);
                assert!(spread > 1e-3, "random points collapsed at n = {n}");
            }

            let v = views(&run.layout, &vec![None; n]);
            finite(&v.anatomy, &format!("{shape} views anatomy"), n);
            finite(&v.spheres, &format!("{shape} views spheres"), n);

            let mut layout = run.layout.clone();
            let mut r = Rng::new(n as u64);
            let new: Vec<Vec<f32>> = (0..3).map(|_| unit(&mut r, D)).collect();
            let new_ids = support::ids(3, "n");
            let placed = place_new(&mut layout, &vs, &new_ids, &new, &lp);
            assert_eq!(placed.len(), 3);
            finite(&placed, &format!("{shape} place_new"), n);
            assert_eq!(layout.xyz.len(), n + 3);
        }
    }
}

#[test]
fn link_suggestions_are_sane_for_tiny_corpora() {
    let lp = LinkParams::default();
    for n in 0..=40 {
        for (shape, vs) in corpora(n, 2000 + n as u64) {
            let ids = ids(n, "i");
            let links: Vec<Link> = if n >= 2 {
                vec![Link {
                    from: ids[0].clone(),
                    to: ids[1].clone(),
                    kind: LinkKind::Manual,
                    created_at: Utc::now(),
                }]
            } else {
                vec![]
            };
            let s = LinkSuggester::new(&ids, &vs, &links, &[] as &[ItemMeta]);
            for id in &ids {
                let out = s.suggest(id, &lp);
                assert!(out.len() <= lp.top_k.min(n.saturating_sub(1)));
                for x in &out {
                    assert_ne!(&x.to, id, "{shape}: self suggestion at n = {n}");
                    assert!(
                        x.score.is_finite() && x.cosine.is_finite(),
                        "{shape} n={n} {x:?}"
                    );
                    assert!(x.adamic_adar.is_finite());
                }
                for (_, c) in s.near_duplicates(id, 0.97) {
                    assert!(c.is_finite());
                }
            }
        }
    }
}
