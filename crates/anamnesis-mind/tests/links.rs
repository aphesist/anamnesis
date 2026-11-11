mod support;

use anamnesis_core::{Id, Link, LinkKind};
use anamnesis_mind::links::{ItemMeta, LinkParams, LinkSuggester, SuggestionFlag, split_for_holdout};
use anamnesis_mind::rng::Rng;
use chrono::{Duration, TimeZone, Utc};
use support::*;

fn link(a: &Id, b: &Id) -> Link {
    Link { from: a.clone(), to: b.clone(), kind: LinkKind::Manual, created_at: Utc::now() }
}

/// item 0 = the query; 1 = near duplicate; 2..=4 similar; 5 already linked
/// and similar; 6 similar but same session; 7..=9 shares neighbours with 0;
/// 10.. unrelated.
fn fixture() -> (Vec<Id>, Vec<Vec<f32>>, Vec<Link>, Vec<ItemMeta>) {
    let mut r = Rng::new(3);
    let d = 32;
    let c = unit(&mut r, d);
    let ids = ids(30, "i");
    let mut vs = vec![c.clone()];
    vs.push(around(&mut r, &c, 0.01)); // near duplicate
    for _ in 2..=6 {
        vs.push(around(&mut r, &c, 0.15));
    }
    for _ in 7..30 {
        vs.push(unit(&mut r, d));
    }
    // 7 is moderately similar to 0 and shares two neighbours (20, 21) with it
    vs[7] = around(&mut r, &c, 0.3);
    let links = vec![link(&ids[0], &ids[5]), link(&ids[0], &ids[20]), link(&ids[0], &ids[21]), link(&ids[7], &ids[20]), link(&ids[21], &ids[7])];
    let t0 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
    let src = Id("src".into());
    let mut meta: Vec<ItemMeta> = (0..30)
        .map(|i| ItemMeta { source_id: Some(src.clone()), session: None, created_at: Some(t0 + Duration::days(i)), region: None })
        .collect();
    meta[6].created_at = Some(t0 + Duration::minutes(5)); // same source, close in time
    (ids, vs, links, meta)
}

#[test]
fn penalties_and_graph_signal() {
    let (ids, vs, links, meta) = fixture();
    let s = LinkSuggester::new(&ids, &vs, &links, &meta);
    let out = s.suggest(&ids[0], &LinkParams::default());
    assert!(!out.is_empty());
    assert_eq!(out[0].rank, 1);
    // never suggests itself or already-linked items
    assert!(out.iter().all(|x| x.to != ids[0] && x.to != ids[5] && x.to != ids[20]));
    // near duplicate flagged and pushed below the plain similar items
    let dup = out.iter().find(|x| x.to == ids[1]).expect("duplicate still listed, penalised");
    assert!(dup.flags.contains(&SuggestionFlag::NearDuplicate));
    assert!(dup.cosine > 0.95);
    let plain = out.iter().find(|x| x.to == ids[2]).unwrap();
    assert!(plain.score > dup.score);
    // same session penalised relative to its cosine
    let sess = out.iter().find(|x| x.to == ids[6]).unwrap();
    assert!(sess.flags.contains(&SuggestionFlag::SameSession));
    assert!((sess.score - (sess.cosine - 0.15)).abs() < 1e-5);
    // adamic-adar lifts item 7 above its raw cosine
    let n7 = out.iter().find(|x| x.to == ids[7]).unwrap();
    assert!(n7.adamic_adar > 0.0 && n7.flags.contains(&SuggestionFlag::SharesNeighbours));
    assert!(n7.score > n7.cosine);
    // sorted by score
    assert!(out.windows(2).all(|w| w[0].score >= w[1].score));
    assert_eq!(s.near_duplicates(&ids[0], 0.95).first().map(|x| x.0.clone()), Some(ids[1].clone()));
}

#[test]
fn holdout_is_reproducible() {
    let (ids, vs, links, meta) = fixture();
    let s = LinkSuggester::new(&ids, &vs, &links, &meta);
    let p = LinkParams { top_k: 25, candidates: 30, ..Default::default() };
    let mut all = vec![];
    for id in &ids {
        all.extend(s.suggest(id, &p));
    }
    let a = split_for_holdout(all.clone(), 0.2, 99);
    let b = split_for_holdout(all.iter().rev().cloned().collect(), 0.2, 99);
    // same arms regardless of order
    for x in &a {
        let y = b.iter().find(|y| y.suggestion.from == x.suggestion.from && y.suggestion.to == x.suggestion.to).unwrap();
        assert_eq!(x.shown, y.shown);
    }
    let held = a.iter().filter(|x| !x.shown).count() as f64 / a.len() as f64;
    assert!((held - 0.2).abs() < 0.07, "held share {held} of {}", a.len());
    // another seed gives another split
    let c = split_for_holdout(all.clone(), 0.2, 100);
    assert!(a.iter().zip(&c).any(|(x, y)| x.shown != y.shown));
    // rate 0 shows everything, rate 1 holds everything
    assert!(split_for_holdout(all.clone(), 0.0, 1).iter().all(|x| x.shown));
    assert!(split_for_holdout(all, 1.0, 1).iter().all(|x| !x.shown));
}
