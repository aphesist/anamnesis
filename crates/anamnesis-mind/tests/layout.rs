mod support;

use anamnesis_mind::layout::{LayoutParams, layout_full, neighbourhood_preservation, place_new, views};
use anamnesis_mind::rng::Rng;
use anamnesis_mind::space::{Space, SpaceParams};
use support::*;

const D: usize = 32;

fn blobs(r: &mut Rng, k: usize, n: usize) -> (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<usize>) {
    let centers: Vec<Vec<f32>> = (0..k).map(|_| unit(r, D)).collect();
    let mut vs = vec![];
    let mut label = vec![];
    for (b, c) in centers.iter().enumerate() {
        vs.extend(blob(r, c, n, 0.08));
        label.extend(std::iter::repeat_n(b, n));
    }
    (centers, vs, label)
}

fn centroids(xyz: &[[f32; 3]], label: &[usize], k: usize) -> Vec<[f32; 3]> {
    let mut c = vec![[0f32; 3]; k];
    let mut n = vec![0f32; k];
    for (p, &l) in xyz.iter().zip(label) {
        for d in 0..3 {
            c[l][d] += p[d];
        }
        n[l] += 1.0;
    }
    for (c, n) in c.iter_mut().zip(n) {
        *c = c.map(|v| v / n);
    }
    c
}

fn nearest(p: &[f32; 3], cs: &[[f32; 3]]) -> usize {
    let mut best = (f32::MAX, 0);
    for (i, c) in cs.iter().enumerate() {
        let d = (0..3).map(|k| (p[k] - c[k]).powi(2)).sum::<f32>();
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

#[test]
fn layout_separates_and_is_stable() {
    let mut r = Rng::new(7);
    let (centers, vs, label) = blobs(&mut r, 5, 80);
    let ids = ids(vs.len(), "i");
    let space = Space::new(&vs, SpaceParams::default());
    let params = LayoutParams::default();
    let run1 = layout_full(&ids, &space, None, &params);
    let xyz = &run1.layout.xyz;
    assert_eq!(xyz.len(), 400);
    // normalised to a unit-ish box
    let maxabs = xyz.iter().flat_map(|p| p.iter()).fold(0f32, |m, v| m.max(v.abs()));
    assert!(maxabs < 2.0 && maxabs > 0.3, "{maxabs}");
    // blobs are separated
    let cs = centroids(xyz, &label, 5);
    let correct = xyz.iter().zip(&label).filter(|(p, l)| nearest(p, &cs) == **l).count();
    assert!(correct >= 396, "{correct}");
    let np = neighbourhood_preservation(&space, xyz, 10, 100);
    assert!(np > 0.3, "neighbourhood preservation {np}");

    // full re-layout from previous positions on unchanged data: small residual
    let run2 = layout_full(&ids, &space, Some(&run1.layout), &params);
    let res = run2.procrustes_residual.unwrap();
    assert!(res < 0.05, "procrustes residual {res}");
    assert_eq!(run2.shared, 400);

    // incremental placement: new points land near their own blob, nothing else moves
    let mut layout = run1.layout.clone();
    let new_vs: Vec<Vec<f32>> = (0..10).map(|i| around(&mut r, &centers[i % 5], 0.08)).collect();
    let new_ids = support::ids(10, "n");
    let placed = place_new(&mut layout, &vs, &new_ids, &new_vs, &params);
    assert_eq!(layout.xyz.len(), 410);
    assert_eq!(&layout.xyz[..400], &run1.layout.xyz[..]);
    for (i, p) in placed.iter().enumerate() {
        assert_eq!(nearest(p, &cs), i % 5, "new point {i} at {p:?}");
    }

    // re-layout with the new items included keeps the old ones aligned
    let mut all = vs.clone();
    all.extend(new_vs.iter().cloned());
    let mut all_ids = ids.clone();
    all_ids.extend(new_ids.iter().cloned());
    let space3 = Space::new(&all, SpaceParams::default());
    let run3 = layout_full(&all_ids, &space3, Some(&layout), &params);
    assert!(run3.procrustes_residual.unwrap() < 0.08, "{:?}", run3.procrustes_residual);
}

#[test]
fn views_have_brain_and_spheres() {
    let mut r = Rng::new(8);
    let (_c, vs, label) = blobs(&mut r, 4, 60);
    let ids = ids(vs.len(), "i");
    let space = Space::new(&vs, SpaceParams::default());
    let run = layout_full(&ids, &space, None, &LayoutParams::default());
    let mut assignment: Vec<Option<anamnesis_core::Id>> = label.iter().map(|&l| Some(anamnesis_core::Id(format!("r{l}")))).collect();
    assignment[0] = None; // one unsorted item
    let v = views(&run.layout, &assignment);
    assert_eq!(v.anatomy.len(), 240);
    for p in &v.anatomy {
        assert!(p[0].abs() < 0.65 && p[1].abs() < 0.6 && p[2].abs() < 0.8, "{p:?}");
    }
    // hemispheres: both sides populated
    assert!(v.anatomy.iter().any(|p| p[0] < -0.1) && v.anatomy.iter().any(|p| p[0] > 0.1));
    // spheres: members of one region are tight, regions are apart
    let cs = centroids(&v.spheres, &label, 4);
    let correct = v.spheres.iter().zip(&label).skip(1).filter(|(p, l)| nearest(p, &cs) == **l).count();
    assert_eq!(correct, 239);
    assert!(v.mode("spheres").is_some() && v.mode("nope").is_none());
    // anatomy keeps neighbourhoods roughly
    let np = neighbourhood_preservation(&space, &v.anatomy, 10, 60);
    assert!(np > 0.2, "{np}");
}
