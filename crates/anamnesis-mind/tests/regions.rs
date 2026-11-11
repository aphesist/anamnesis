mod support;

use std::collections::HashMap;

use anamnesis_core::{Id, RegionTransition};
use anamnesis_mind::regions::{RegionExtras, RegionParams, RegionState, recompute_regions};
use anamnesis_mind::rng::Rng;
use anamnesis_mind::space::{Space, SpaceParams};
use chrono::{TimeZone, Utc};
use support::*;

const D: usize = 32;
const S: f32 = 0.06;

#[test]
fn small_corpus_is_unsorted() {
    let mut r = Rng::new(1);
    let c = unit(&mut r, D);
    let vs = blob(&mut r, &c, 20, S);
    let ids = ids(20, "i");
    let space = Space::new(&vs, SpaceParams::default());
    let run = recompute_regions(
        &ids,
        &space,
        &RegionState::default(),
        Utc::now(),
        &RegionParams::default(),
        RegionExtras::default(),
    );
    assert!(run.unsorted);
    assert!(run.regions.is_empty());
    assert!(run.assignment.iter().all(|a| a.is_none()));
}

#[test]
fn transitions_and_stable_ids() {
    let mut r = Rng::new(42);
    let centers: Vec<Vec<f32>> = (0..8).map(|_| unit(&mut r, D)).collect();
    let (ca, cb, cc, cd, ce, cf, ca2, cbc) = (
        &centers[0],
        &centers[1],
        &centers[2],
        &centers[3],
        &centers[4],
        &centers[5],
        &centers[6],
        &centers[7],
    );
    let n = 60;
    let ga = ids(n, "a");
    let gb = ids(n, "b");
    let gc = ids(n, "c");
    let gd = ids(n, "d");
    let ge = ids(n, "e");
    let gf = ids(n, "f");
    let texts_for = |prefix: &str, words: &str, n: usize| -> Vec<String> {
        (0..n)
            .map(|i| format!("{prefix} note {i}: {words}"))
            .collect()
    };

    // run 1: five blobs
    let mut ids1 = vec![];
    let mut vs1 = vec![];
    let mut texts1 = vec![];
    for (g, c, w) in [
        (&ga, ca, "attention ethics"),
        (&gb, cb, "bread sourdough"),
        (&gc, cc, "pão fermentação"),
        (&gd, cd, "rust compiler"),
        (&ge, ce, "garden tomatoes"),
    ] {
        ids1.extend(g.iter().cloned());
        vs1.extend(blob(&mut r, c, n, S));
        texts1.extend(texts_for("x", w, n));
    }
    let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let space1 = Space::new(&vs1, SpaceParams::default());
    let run1 = recompute_regions(
        &ids1,
        &space1,
        &RegionState::default(),
        t1,
        &RegionParams::default(),
        RegionExtras {
            texts: Some(&texts1),
            anatomy_xyz: None,
            ..Default::default()
        },
    );
    assert!(!run1.unsorted);
    assert_eq!(
        run1.regions.len(),
        5,
        "{:?}",
        run1.regions.iter().map(|r| r.size).collect::<Vec<_>>()
    );
    assert!(
        run1.transitions
            .iter()
            .all(|t| matches!(t, RegionTransition::Birth { .. }))
    );
    let region_of = |run: &anamnesis_mind::regions::RegionRun, ids: &[Id], id: &Id| -> Option<Id> {
        let k = ids.iter().position(|x| x == id).unwrap();
        run.assignment[k].clone()
    };
    let ra = region_of(&run1, &ids1, &ga[0]).unwrap();
    let rb = region_of(&run1, &ids1, &gb[0]).unwrap();
    let rc = region_of(&run1, &ids1, &gc[0]).unwrap();
    let rd = region_of(&run1, &ids1, &gd[0]).unwrap();
    let re = region_of(&run1, &ids1, &ge[0]).unwrap();
    // every region got a lowercase fallback label from its keywords
    let label_d = &run1.regions.iter().find(|x| x.id == rd).unwrap().label;
    assert!(
        label_d.contains("rust") || label_d.contains("compiler"),
        "{label_d}"
    );
    let anat_d = run1
        .regions
        .iter()
        .find(|x| x.id == rd)
        .unwrap()
        .anatomy
        .clone()
        .unwrap();

    // run 2: A splits, B+C merge, D survives (same vectors), E dies, F is born
    let mut ids2 = vec![];
    let mut vs2 = vec![];
    ids2.extend(ga.iter().cloned());
    vs2.extend(blob(&mut r, ca, n / 2, S));
    vs2.extend(blob(&mut r, ca2, n / 2, S));
    ids2.extend(gb.iter().cloned());
    ids2.extend(gc.iter().cloned());
    vs2.extend(blob(&mut r, cbc, 2 * n, S));
    ids2.extend(gd.iter().cloned());
    vs2.extend(vs1[3 * n..4 * n].iter().cloned());
    ids2.extend(gf.iter().cloned());
    vs2.extend(blob(&mut r, cf, n, S));
    let t2 = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let space2 = Space::new(&vs2, SpaceParams::default());
    let run2 = recompute_regions(
        &ids2,
        &space2,
        &run1.state,
        t2,
        &RegionParams::default(),
        RegionExtras::default(),
    );

    let mut kinds: HashMap<&str, usize> = HashMap::new();
    for t in &run2.transitions {
        let k = match t {
            RegionTransition::Birth { .. } => "birth",
            RegionTransition::Survive { .. } => "survive",
            RegionTransition::Split { .. } => "split",
            RegionTransition::Merge { .. } => "merge",
            RegionTransition::Death { .. } => "death",
        };
        *kinds.entry(k).or_default() += 1;
    }
    assert_eq!(kinds.get("split"), Some(&1), "{:?}", run2.transitions);
    assert_eq!(kinds.get("merge"), Some(&1), "{:?}", run2.transitions);
    assert_eq!(kinds.get("survive"), Some(&1), "{:?}", run2.transitions);
    assert_eq!(kinds.get("death"), Some(&1), "{:?}", run2.transitions);
    assert_eq!(kinds.get("birth"), Some(&1), "{:?}", run2.transitions);
    for t in &run2.transitions {
        match t {
            RegionTransition::Split { from, into } => {
                assert_eq!(from, &ra);
                assert_eq!(into.len(), 2);
            }
            RegionTransition::Merge { from, .. } => {
                let mut f = from.clone();
                f.sort();
                let mut want = vec![rb.clone(), rc.clone()];
                want.sort();
                assert_eq!(f, want);
            }
            RegionTransition::Survive { region } => assert_eq!(region, &rd),
            RegionTransition::Death { region } => assert_eq!(region, &re),
            RegionTransition::Birth { region } => {
                assert_eq!(region_of(&run2, &ids2, &gf[0]).as_ref(), Some(region))
            }
        }
    }
    // the survivor kept its id, label, anatomy name and birth time
    let d2 = run2.regions.iter().find(|x| x.id == rd).unwrap();
    assert_eq!(&d2.label, label_d);
    assert_eq!(d2.anatomy.as_ref(), Some(&anat_d));
    assert_eq!(d2.born_at, t1);
    // retired regions carry died_at
    assert_eq!(run2.retired.len(), 4); // a, b, c, e
    assert!(run2.retired.iter().all(|x| x.died_at == Some(t2)));
    // the larger split child inherits the parent's anatomy name
    let anat_a = run1
        .regions
        .iter()
        .find(|x| x.id == ra)
        .unwrap()
        .anatomy
        .clone();
    assert!(
        run2.regions
            .iter()
            .any(|x| x.parent_ids == vec![ra.clone()] && x.anatomy == anat_a)
    );
    // drift for the unchanged survivor is ~0
    let dd = run2.drift.iter().find(|x| x.region == rd).unwrap();
    assert!(dd.drift < 1e-3, "{}", dd.drift);
    assert!((dd.jaccard - 1.0).abs() < 1e-6);

    // run 3: nothing changes → everything survives with the same ids
    let space3 = Space::new(&vs2, SpaceParams::default());
    let run3 = recompute_regions(
        &ids2,
        &space3,
        &run2.state,
        t2,
        &RegionParams::default(),
        RegionExtras::default(),
    );
    assert!(
        run3.transitions
            .iter()
            .all(|t| matches!(t, RegionTransition::Survive { .. })),
        "{:?}",
        run3.transitions
    );
    let mut a: Vec<_> = run2.regions.iter().map(|x| x.id.clone()).collect();
    let mut b: Vec<_> = run3.regions.iter().map(|x| x.id.clone()).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);

    // assign between runs: new points near F go to F's region, a far point is unsorted
    let rf = region_of(&run2, &ids2, &gf[0]).unwrap();
    let near = blob(&mut r, cf, 3, S);
    let far = vec![unit(&mut r, D)];
    let got = run3.state.assign(&near);
    assert!(got.iter().all(|g| g.as_ref() == Some(&rf)));
    assert_eq!(run3.state.assign(&far), vec![None]);
}
