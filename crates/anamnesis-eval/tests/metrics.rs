//! Metrics on synthetic logs with known ground truth.

use std::collections::HashMap;

use anamnesis_core::{
    Actor, Event, EventKind, Id, Item, ItemKind, Link, LinkKind, Origin, Privacy, Region,
    RegionTransition, Timestamp,
};
use anamnesis_eval::links::{LinkConfig, link_metrics};
use anamnesis_eval::resurfacing::{ResurfacingConfig, resurfacing_metrics};
use anamnesis_eval::retrieval::{RetrievalConfig, retrieval_metrics};
use anamnesis_eval::synthetic::{self, SyntheticConfig, SyntheticSearch, chain};
use anamnesis_eval::*;
use chrono::{Duration, TimeZone, Utc};

fn t0() -> Timestamp {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap()
}
fn day(d: f64) -> Timestamp {
    t0() + Duration::minutes((d * 1440.0) as i64)
}
fn id(s: &str) -> Id {
    Id(s.to_string())
}

type Raw = Vec<(Timestamp, Actor, EventKind)>;

fn suggest(raw: &mut Raw, at: Timestamp, sid: &str, from: &str, to: &str, rank: u32, shown: bool) {
    raw.push((
        at,
        Actor::System("mind:suggest".into()),
        EventKind::SuggestionMade {
            suggestion_id: id(sid),
            from: id(from),
            to: id(to),
            score: 0.5,
            rank,
            shown,
            model: "test".into(),
        },
    ));
}

fn link(raw: &mut Raw, at: Timestamp, from: &str, to: &str, kind: LinkKind) {
    raw.push((
        at,
        Actor::User,
        EventKind::LinkAdded {
            link: Link {
                from: id(from),
                to: id(to),
                kind,
                created_at: at,
            },
        },
    ));
}

fn item(raw: &mut Raw, at: Timestamp, name: &str, origin: Origin) {
    let it = Item {
        id: id(name),
        source_id: id("src"),
        external_id: name.into(),
        uri: None,
        kind: ItemKind::Note,
        title: name.into(),
        body: String::new(),
        origin,
        privacy: Privacy::CloudOk,
        created_at: at,
        captured_at: at,
        modified_at: at,
        content_hash: String::new(),
        tags: vec![],
        meta: serde_json::Value::Null,
        deleted: false,
    };
    raw.push((at, Actor::User, EventKind::ItemCreated { item: it }));
}

/// 100 predicted pairs, 20 held out; `held_linked` of the held-out and `shown_linked`
/// of the shown pairs are later linked by hand, plus `unpredicted` other hand links.
fn latent_log(held_linked: usize, shown_linked: usize, unpredicted: usize) -> Vec<Event> {
    let mut raw = Raw::new();
    for i in 0..100 {
        let shown = i >= 20;
        suggest(
            &mut raw,
            day(0.0),
            &format!("s{i}"),
            &format!("a{i}"),
            &format!("b{i}"),
            1,
            shown,
        );
        let linked = if shown {
            i - 20 < shown_linked
        } else {
            i < held_linked
        };
        if linked {
            // reversed direction on purpose: pairs are unordered
            link(
                &mut raw,
                day(10.0),
                &format!("b{i}"),
                &format!("a{i}"),
                LinkKind::Manual,
            );
        }
    }
    for j in 0..unpredicted {
        link(
            &mut raw,
            day(12.0),
            &format!("x{j}"),
            &format!("y{j}"),
            LinkKind::Written,
        );
    }
    chain(raw)
}

fn link_cfg() -> LinkConfig {
    LinkConfig {
        as_of: Some(day(200.0)),
        ..LinkConfig::default()
    }
}

#[test]
fn latent_share_recovers_known_truth() {
    let events = latent_log(10, 40, 50);
    let r = link_metrics(&events, &link_cfg());
    let l = &r.all.latent;
    assert_eq!(l.hand_links, 100);
    assert_eq!(l.held_out.links_predicted, 10);
    assert_eq!(l.held_out.links_unpredicted, 50);
    assert!((l.held_out.propensity.unwrap() - 0.2).abs() < 1e-12);
    // (10 / 0.2) / (10 / 0.2 + 50) = 0.5
    let v = l.held_out.estimate.value.unwrap();
    assert!((v - 0.5).abs() < 1e-12, "{v}");
    let ci = l.held_out.estimate.interval.as_ref().unwrap();
    assert!(ci.contains(0.5) && ci.lo > 0.2 && ci.hi < 0.8, "{ci:?}");
    // no exposure effect in this log: the shown arm agrees
    assert!((l.shown.estimate.value.unwrap() - 0.5).abs() < 1e-12);
    assert!(l.exposure_lift.unwrap().abs() < 1e-12);
    // raw "predicted before" share: 50 of 100 hand links
    assert!((l.predicted_before_raw.value.unwrap() - 0.5).abs() < 1e-12);

    // hit rate of the held-out arm: half of the 20 never-shown suggestions became links
    let h = &r.all.hit_rate.held_out;
    assert_eq!((h.pairs, h.linked, h.pending), (20, 10, 0));
    let w = h.estimate.interval.as_ref().unwrap();
    assert!(
        (w.lo - 0.299).abs() < 1e-3 && (w.hi - 0.701).abs() < 1e-3,
        "{w:?}"
    );
    assert_eq!(r.all.hit_rate.shown.pairs, 80);
    // 100 hand links but only 20 held-out pairs (< 30): still measuring
    assert_eq!(r.status, Status::Measuring);
}

#[test]
fn exposure_inflates_shown_arm_only() {
    let events = latent_log(10, 60, 50);
    let l = link_metrics(&events, &link_cfg()).all.latent;
    assert!((l.held_out.estimate.value.unwrap() - 0.5).abs() < 1e-12);
    // (60 / 0.8) / (75 + 50) = 0.6
    assert!((l.shown.estimate.value.unwrap() - 0.6).abs() < 1e-12);
    assert!((l.exposure_lift.unwrap() - 0.1).abs() < 1e-12);
}

#[test]
fn links_before_the_prediction_are_unpredicted() {
    let mut raw = Raw::new();
    link(&mut raw, day(1.0), "a", "b", LinkKind::Manual);
    suggest(&mut raw, day(2.0), "s", "a", "b", 1, false);
    // rank beyond k is not a prediction
    suggest(&mut raw, day(2.0), "s2", "c", "d", 9, false);
    link(&mut raw, day(3.0), "c", "d", LinkKind::Manual);
    let r = link_metrics(&chain(raw), &link_cfg());
    assert_eq!(r.all.latent.held_out.links_predicted, 0);
    assert_eq!(r.all.latent.held_out.links_unpredicted, 2);
    assert_eq!(r.predicted_pairs, 1);
}

#[test]
fn ranking_metrics_hand_computed() {
    let mut raw = Raw::new();
    for (r, t) in ["t1", "t2", "t3", "t4", "t5"].iter().enumerate() {
        suggest(
            &mut raw,
            day(0.0),
            &format!("s{r}"),
            "x",
            t,
            r as u32 + 1,
            *t != "t2",
        );
    }
    raw.push((
        day(1.0),
        Actor::User,
        EventKind::SuggestionAccepted {
            suggestion_id: id("s3"),
        },
    ));
    raw.push((
        day(1.0),
        Actor::User,
        EventKind::SuggestionDismissed {
            suggestion_id: id("s0"),
        },
    ));
    link(&mut raw, day(2.0), "x", "t4", LinkKind::AcceptedSuggestion);
    link(&mut raw, day(3.0), "z", "x", LinkKind::Manual);
    link(&mut raw, day(5.0), "x", "t2", LinkKind::Manual);
    // outside the 90-day window: ignored by ranking
    link(&mut raw, day(120.0), "x", "t5", LinkKind::Manual);
    let r = link_metrics(&chain(raw), &link_cfg());

    let all = &r.all.ranking.all;
    assert_eq!(
        (all.queries, all.queries_with_links, all.pending),
        (1, 1, 0)
    );
    assert!((all.precision_at_k.value.unwrap() - 0.4).abs() < 1e-12);
    assert!((all.recall_at_k.value.unwrap() - 2.0 / 3.0).abs() < 1e-12);
    assert!((all.mrr.value.unwrap() - 0.5).abs() < 1e-12);

    let held = &r.all.ranking.held_out;
    assert!((held.precision_at_k.value.unwrap() - 1.0).abs() < 1e-12);
    assert!((held.recall_at_k.value.unwrap() - 0.5).abs() < 1e-12);
    assert!((held.mrr.value.unwrap() - 1.0).abs() < 1e-12);

    let a = &r.all.acceptance;
    assert_eq!((a.shown, a.accepted, a.dismissed, a.ignored), (4, 1, 1, 2));
    assert!((a.rate.value.unwrap() - 0.25).abs() < 1e-12);
    assert!(a.note.contains("not a quality measure"));

    // time to link: held-out t2 after 5 days; shown t4 (2 d) and t5 (120 d)
    let ttl = &r.all.time_to_link;
    assert_eq!(ttl.held_out.n, 1);
    assert!((ttl.held_out.median.unwrap() - 5.0).abs() < 1e-9);
    assert_eq!(ttl.shown.n, 2);
    assert_eq!(
        ttl.shown
            .buckets
            .iter()
            .find(|b| b.label == "≥ 90 days")
            .unwrap()
            .count,
        1
    );
}

#[test]
fn open_windows_are_censored() {
    let mut raw = Raw::new();
    suggest(&mut raw, day(0.0), "s", "a", "b", 1, false);
    let cfg = LinkConfig {
        as_of: Some(day(30.0)),
        ..LinkConfig::default()
    };
    let r = link_metrics(&chain(raw), &cfg);
    assert_eq!(r.all.hit_rate.held_out.pending, 1);
    assert_eq!(r.all.hit_rate.held_out.pairs, 0);
    assert_eq!(r.all.ranking.all.pending, 1);
}

#[test]
fn link_novelty_split_partitions_links() {
    let mut raw = Raw::new();
    suggest(&mut raw, day(0.0), "s", "a", "b", 1, false);
    link(&mut raw, day(5.0), "p", "q", LinkKind::Manual);
    link(&mut raw, day(40.0), "r", "s", LinkKind::Manual);
    link(&mut raw, day(50.0), "a", "b", LinkKind::Manual);
    let r = link_metrics(&chain(raw), &link_cfg());
    let n = &r.novelty;
    assert_eq!(n.feature_start, Some(day(0.0)));
    assert_eq!(n.first_days.latent.hand_links, 1);
    assert_eq!(n.after.latent.hand_links, 2);
    assert_eq!(n.after.latent.held_out.links_predicted, 1);
}

// ---------------------------------------------------------------------------
// retrieval

fn label(raw: &mut Raw, at: Timestamp, q: &str, item: &str, grade: u8) {
    raw.push((
        at,
        Actor::User,
        EventKind::RetrievalLabeled {
            query: q.into(),
            item_id: id(item),
            grade,
        },
    ));
}

#[test]
fn retrieval_hand_computed() {
    let mut raw = Raw::new();
    for (it, g) in [("a", 3), ("b", 2), ("c", 0), ("d", 1)] {
        label(&mut raw, day(0.0), "Walking  Notes", it, g);
    }
    let events = chain(raw);
    let r = retrieval_metrics(
        &events,
        |_q| vec![id("c"), id("a"), id("d"), id("b")],
        &RetrievalConfig::default(),
    );
    assert_eq!(r.all.n, 1);
    assert!((r.all.ndcg_at_10.value.unwrap() - 0.660_989_805_785).abs() < 1e-9);
    assert!((r.all.precision_at_5.value.unwrap() - 0.6).abs() < 1e-12);
    assert!((r.all.mrr.value.unwrap() - 0.5).abs() < 1e-12);
    assert_eq!(r.queries[0].query, "walking notes");
    assert_eq!(r.all.status, Status::Measuring);

    // the latest label wins, and a perfect ranking gives ndcg 1
    let mut raw = Raw::new();
    label(&mut raw, day(0.0), "q", "a", 0);
    label(&mut raw, day(1.0), "q", "a", 2);
    let r = retrieval_metrics(&chain(raw), |_q| vec![id("a")], &RetrievalConfig::default());
    assert!((r.all.ndcg_at_10.value.unwrap() - 1.0).abs() < 1e-12);
}

#[test]
fn retrieval_readiness_and_skips() {
    let mut raw = Raw::new();
    for i in 0..25 {
        label(
            &mut raw,
            day(i as f64 * 2.0),
            &format!("q{i}"),
            &format!("i{i}"),
            2,
        );
    }
    label(&mut raw, day(1.0), "nothing relevant", "z", 0);
    let events = chain(raw);
    let r = retrieval_metrics(
        &events,
        |q| vec![id(&format!("i{}", &q[1..]))],
        &RetrievalConfig::default(),
    );
    assert_eq!(r.labelled_queries, 26);
    assert_eq!(r.skipped_queries, 1);
    assert_eq!(r.all.n, 25);
    assert_eq!(r.all.status, Status::EnoughData);
    // 14 queries labelled in the first 28 days (days 0, 2, ..., 26), 11 after
    assert_eq!(r.novelty.first_days.n, 14);
    assert_eq!(r.novelty.after.n, 11);
    assert_eq!(r.novelty.first_days.status, Status::Measuring);
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["status"], "enough data");
}

#[test]
fn retrieval_bootstrap_interval_covers_mean() {
    let mut raw = Raw::new();
    for i in 0..30 {
        label(&mut raw, day(0.0), &format!("q{i}"), "hit", 3);
    }
    // half of the queries find it first, half second
    let r = retrieval_metrics(
        &chain(raw),
        |q| {
            let n: usize = q[1..].parse().unwrap();
            if n.is_multiple_of(2) {
                vec![id("hit")]
            } else {
                vec![id("miss"), id("hit")]
            }
        },
        &RetrievalConfig::default(),
    );
    let m = r.all.mrr.value.unwrap();
    assert!((m - 0.75).abs() < 1e-12);
    let ci = r.all.mrr.interval.unwrap();
    assert!(ci.contains(0.75) && ci.lo > 0.6 && ci.hi < 0.9, "{ci:?}");
}

// ---------------------------------------------------------------------------
// resurfacing

fn mrt_log(algo: (usize, usize), random: (usize, usize), start_day: f64) -> Raw {
    let mut raw = Raw::new();
    let mut k = 0;
    for (arm, (useful, n)) in [("algorithm", algo), ("random", random)] {
        for i in 0..n {
            let rid = format!("r{start_day}-{k}");
            k += 1;
            let at = day(start_day + i as f64 * 0.1);
            raw.push((
                at,
                Actor::System("mind".into()),
                EventKind::ResurfaceShown {
                    resurface_id: id(&rid),
                    item_id: id("it"),
                    arm: arm.into(),
                },
            ));
            raw.push((
                at + Duration::minutes(1),
                Actor::User,
                EventKind::ResurfaceRated {
                    resurface_id: id(&rid),
                    useful: i < useful,
                },
            ));
        }
    }
    raw
}

#[test]
fn mrt_posterior_sanity() {
    let r = resurfacing_metrics(
        &chain(mrt_log((60, 100), (40, 100), 0.0)),
        &ResurfacingConfig::default(),
    );
    let a = &r.all;
    assert_eq!(
        (a.algorithm.n, a.algorithm.useful, a.random.n),
        (100, 60, 100)
    );
    assert!((a.algorithm.posterior.value.unwrap() - 60.5 / 101.0).abs() < 1e-12);
    let d = &a.difference;
    assert!((d.mean.unwrap() - 0.198).abs() < 0.01, "{d:?}");
    let ci = d.interval.as_ref().unwrap();
    assert!(ci.lo > 0.0 && ci.hi < 0.4, "{ci:?}");
    // normal approximation: z = 0.2 / 0.0693 ≈ 2.89 → P ≈ 0.998
    assert!(d.p_algorithm_better.unwrap() > 0.99);
    assert_eq!(a.readiness.n_smaller_arm, 100);
    assert_eq!(a.readiness.detectable, "moderate (20 points)");
    assert_eq!(a.status, Status::EnoughData);
    assert_eq!(a.readiness.thresholds[0].n_per_arm, 40);
    assert_eq!(a.readiness.thresholds[1].n_per_arm, 95);

    // equal arms → P(better) ≈ ½, interval straddles 0
    let r = resurfacing_metrics(
        &chain(mrt_log((25, 50), (25, 50), 0.0)),
        &ResurfacingConfig::default(),
    );
    let p = r.all.difference.p_algorithm_better.unwrap();
    assert!((p - 0.5).abs() < 0.03, "{p}");
    assert!(r.all.difference.interval.as_ref().unwrap().contains(0.0));
    assert_eq!(r.all.readiness.detectable, "large (30 points)");
    assert_eq!(r.all.status, Status::Measuring);

    // deterministic
    let again = resurfacing_metrics(
        &chain(mrt_log((25, 50), (25, 50), 0.0)),
        &ResurfacingConfig::default(),
    );
    assert_eq!(r, again);
}

#[test]
fn mrt_novelty_split() {
    // first four weeks: algorithm looks great; afterwards it is no better than random
    let mut raw = mrt_log((18, 20), (8, 20), 0.0);
    raw.extend(mrt_log((20, 40), (20, 40), 40.0));
    let r = resurfacing_metrics(&chain(raw), &ResurfacingConfig::default());
    let n = &r.novelty;
    assert_eq!(n.first_days.algorithm.n, 20);
    assert_eq!(n.after.algorithm.n, 40);
    assert_eq!(n.first_days.n + n.after.n, r.all.n);
    assert!(n.first_days.difference.p_algorithm_better.unwrap() > 0.99);
    assert!((n.after.difference.p_algorithm_better.unwrap() - 0.5).abs() < 0.05);
    assert!(n.first_days.difference.mean.unwrap() > n.after.difference.mean.unwrap() + 0.3);
}

#[test]
fn unrated_and_unknown_arms_are_counted() {
    let raw: Raw = vec![
        (
            day(0.0),
            Actor::User,
            EventKind::ResurfaceShown {
                resurface_id: id("a"),
                item_id: id("i"),
                arm: "algorithm".into(),
            },
        ),
        (
            day(0.0),
            Actor::User,
            EventKind::ResurfaceShown {
                resurface_id: id("b"),
                item_id: id("i"),
                arm: "recency".into(),
            },
        ),
    ];
    let r = resurfacing_metrics(&chain(raw), &ResurfacingConfig::default());
    assert_eq!(r.all.algorithm.unrated, 1);
    assert_eq!(r.all.algorithm.n, 0);
    assert_eq!(r.other_arm, 1);
}

// ---------------------------------------------------------------------------
// growth

#[test]
fn growth_counts() {
    let mut raw = Raw::new();
    item(&mut raw, day(0.0), "a", Origin::OwnerWritten);
    item(&mut raw, day(1.0), "b", Origin::AiGenerated);
    item(&mut raw, day(70.0), "c", Origin::External);
    raw.push((
        day(71.0),
        Actor::User,
        EventKind::ItemDeleted { item_id: id("b") },
    ));
    link(&mut raw, day(2.0), "a", "b", LinkKind::Manual);
    let reg = |s: &str| Region {
        id: id(s),
        label: s.into(),
        anatomy: None,
        size: 1,
        born_at: day(0.0),
        died_at: None,
        parent_ids: vec![],
    };
    raw.push((
        day(7.0),
        Actor::System("mind".into()),
        EventKind::RegionsRecomputed {
            run_id: id("run1"),
            regions: vec![reg("r1")],
            transitions: vec![RegionTransition::Birth { region: id("r1") }],
        },
    ));
    raw.push((
        day(14.0),
        Actor::System("mind".into()),
        EventKind::RegionsRecomputed {
            run_id: id("run2"),
            regions: vec![reg("r2"), reg("r3")],
            transitions: vec![RegionTransition::Split {
                from: id("r1"),
                into: vec![id("r2"), id("r3")],
            }],
        },
    ));
    let mut cohesion = HashMap::new();
    cohesion.insert(id("run1"), 0.6);
    cohesion.insert(id("run2"), 0.8);
    let g = growth_metrics(&chain(raw), Some(&cohesion));
    assert_eq!((g.n, g.items_created, g.items_deleted), (2, 3, 1));
    assert_eq!(
        g.by_month
            .iter()
            .map(|m| m.month.as_str())
            .collect::<Vec<_>>(),
        vec!["2026-03", "2026-04", "2026-05"]
    );
    assert_eq!(g.by_month.last().unwrap().cumulative, 2);
    assert_eq!(g.by_month[1].created, 0);
    assert_eq!(g.by_speaker.get("you"), Some(&1));
    assert_eq!(g.by_speaker.get("assistants"), None);
    assert_eq!(g.by_origin.get("external"), Some(&1));
    assert_eq!(g.links_by_kind.get("manual"), Some(&1));
    assert_eq!(g.regions_over_time.len(), 2);
    assert_eq!(g.regions_over_time[1].live_regions, 2);
    assert_eq!((g.transitions.births, g.transitions.splits), (1, 1));
    assert!((g.mean_cohesion.unwrap() - 0.7).abs() < 1e-12);
    assert_eq!(g.novelty.first_days.items_created, 2);
    assert_eq!(g.novelty.after.items_created, 1);
}

// ---------------------------------------------------------------------------
// synthetic + full report

#[test]
fn synthetic_is_deterministic_and_chained() {
    let cfg = SyntheticConfig::default();
    let a = synthetic::generate(&cfg);
    let b = synthetic::generate(&cfg);
    assert_eq!(a, b);
    assert!(a.len() > 1000, "{}", a.len());
    let c = synthetic::generate(&SyntheticConfig { seed: 11, ..cfg });
    assert_ne!(a, c);
    for w in a.windows(2) {
        assert_eq!(w[1].prev_hash, w[0].hash);
        assert_eq!(w[1].seq, w[0].seq + 1);
        assert!(w[1].at >= w[0].at);
    }
    // round-trips through json (the ui mock reads it)
    let json = serde_json::to_string(&a[..50]).unwrap();
    let back: Vec<Event> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, a[..50].to_vec());
}

#[test]
fn full_report_on_synthetic_user() {
    let events = synthetic::generate(&SyntheticConfig::default());
    let search = SyntheticSearch::new(&events, 1);
    let mut f = |q: &str| search.search(q);
    let report = evaluate(&events, Some(&mut f), None, &EvalConfig::default());

    // links: the held-out arm is a fair sample; the shown arm is inflated by exposure
    let l = &report.links.all;
    assert!(l.hit_rate.held_out.pairs > 50, "{:?}", l.hit_rate.held_out);
    let ho = l.hit_rate.held_out.estimate.value.unwrap();
    let sh = l.hit_rate.shown.estimate.value.unwrap();
    // truth: 4 same-topic candidates at 0.22 and 1 mostly off-topic at ~0.02 → ≈ 0.18
    assert!((0.08..0.30).contains(&ho), "held-out hit rate {ho}");
    assert!(sh > ho, "shown {sh} vs held-out {ho}");
    let latent = l.latent.held_out.estimate.value.unwrap();
    assert!((0.0..1.0).contains(&latent));
    assert!(l.latent.exposure_lift.unwrap() > 0.0);
    // acceptance: novelty boost shows in the first 28 days
    let nov = &report.links.novelty;
    assert!(
        nov.first_days.acceptance.rate.value.unwrap() > nov.after.acceptance.rate.value.unwrap()
    );

    // retrieval
    let rt = report.retrieval.as_ref().unwrap();
    assert!(rt.all.n >= 25, "{}", rt.all.n);
    assert!(rt.all.ndcg_at_10.value.unwrap() > 0.5);

    // resurfacing: algorithm 0.6 vs random 0.4
    let rs = &report.resurfacing.all;
    assert!(
        rs.difference.p_algorithm_better.unwrap() > 0.95,
        "{:?}",
        rs.difference
    );
    assert!(rs.n > 200);

    // growth
    let g = &report.growth;
    assert!(g.n > 400);
    assert!(
        g.transitions.splits >= 1 && g.transitions.merges >= 1 && g.transitions.deaths >= 1,
        "{:?}",
        g.transitions
    );
    // 180 days from 5 january: january to july
    assert_eq!(g.by_month.len(), 7);

    // honesty
    let keys: Vec<&str> = report
        .honesty
        .claims
        .iter()
        .map(|c| c.key.as_str())
        .collect();
    for k in [
        "latent_share",
        "held_out_hit_rate",
        "acceptance_rate",
        "retrieval_ndcg",
        "resurfacing_vs_random",
        "growth",
        "thinks_like_me",
    ] {
        assert!(keys.contains(&k), "missing {k}");
    }
    let think = report
        .honesty
        .claims
        .iter()
        .find(|c| c.key == "thinks_like_me")
        .unwrap();
    assert_eq!(think.class, ClaimClass::Hypothesis);
    let rsv = report
        .honesty
        .claims
        .iter()
        .find(|c| c.key == "resurfacing_vs_random")
        .unwrap();
    assert_eq!(rsv.class, ClaimClass::Measured);
    let md = report.honesty.to_markdown();
    assert!(md.contains("| the brain thinks like you | hypothesis |"));

    // serialises, with the contract's status strings
    let v = serde_json::to_value(&report).unwrap();
    assert!(v["links"]["latent"]["held_out"]["estimate"]["value"].is_number());
    assert!(matches!(
        v["resurfacing"]["status"].as_str(),
        Some("measuring") | Some("enough data")
    ));
}

#[test]
fn empty_log_is_all_hypothesis() {
    let report = evaluate(&[], None, None, &EvalConfig::default());
    assert!(report.retrieval.is_none());
    assert!(
        report
            .honesty
            .claims
            .iter()
            .all(|c| c.class == ClaimClass::Hypothesis)
    );
    assert_eq!(report.links.novelty.feature_start, None);
    assert_eq!(report.links.novelty.first_days.latent.hand_links, 0);
}

#[test]
fn growth_claim_follows_the_status_rule() {
    // A few days: items exist, but fewer than two clustering runs.
    let cfg = SyntheticConfig {
        days: 3,
        ..SyntheticConfig::default()
    };
    let events = synthetic::generate(&cfg);
    let report = evaluate(&events, None, None, &EvalConfig::default());
    assert!(report.growth.n > 0);
    assert!(report.growth.regions_over_time.len() < 2);
    let g = report
        .honesty
        .claims
        .iter()
        .find(|c| c.key == "growth")
        .unwrap();
    assert_eq!(g.status, Status::Measuring);
    assert_eq!(g.class, ClaimClass::Hypothesis);

    // Over a full log, no claim that is still measuring may be shown as measured or modelled.
    for days in [3, 30, 180] {
        let events = synthetic::generate(&SyntheticConfig {
            days,
            ..SyntheticConfig::default()
        });
        let report = evaluate(&events, None, None, &EvalConfig::default());
        for c in &report.honesty.claims {
            if c.status == Status::Measuring || c.n == 0 {
                assert_eq!(c.class, ClaimClass::Hypothesis, "{} at {days} days", c.key);
            }
        }
    }
}
