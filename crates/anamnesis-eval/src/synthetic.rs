//! A seeded, deterministic event log for a simulated user: notes and clips arriving
//! over months, link suggestions with a random holdout, hand links (some latent in
//! the text, some not), accepted and dismissed suggestions, weekly clustering runs
//! with births / splits / merges / a death, a resurfacing trial and a few labelled
//! retrieval queries. Used by the ui mock, the lab and this crate's tests.
//!
//! The simulated user has a known ground truth: a same-topic suggestion becomes a
//! hand link with probability `p_latent_same_topic` whether or not it is shown, and
//! showing adds `p_exposure_link` on top (so the shown arm is inflated, as in life).
//!
//! Hashes are sha256(prev_hash ‖ json of the event without `hash`), genesis
//! prev_hash = 64 zeros. Core owns the canonical chain format; synthetic logs are
//! for display and evaluation, not for chain verification.

use std::collections::HashMap;

use anamnesis_core::{
    Actor, Event, EventKind, Id, Item, ItemKind, Link, LinkKind, Origin, Privacy, Region,
    RegionTransition, Source, Timestamp,
};
use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::stats::Rng;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticConfig {
    pub seed: u64,
    pub days: u32,
    pub start: Timestamp,
    pub items_per_day: f64,
    /// Day the link suggester is switched on.
    pub suggestion_start_day: u32,
    pub suggestions_per_item: u32,
    /// Share of suggestions randomly held out (never shown).
    pub holdout: f64,
    /// Chance a same-topic suggestion is later linked by hand regardless of exposure.
    pub p_latent_same_topic: f64,
    pub p_latent_other_topic: f64,
    /// Extra chance of a hand link caused by having seen the suggestion.
    pub p_exposure_link: f64,
    pub p_accept: f64,
    pub p_dismiss: f64,
    /// Added to acceptance and usefulness during a feature's first 28 days.
    pub novelty_boost: f64,
    pub unpredicted_links_per_day: f64,
    pub resurface_start_day: u32,
    pub resurfaces_per_day: f64,
    pub p_useful_algorithm: f64,
    pub p_useful_random: f64,
    pub p_rated: f64,
    pub retrieval_start_day: u32,
    pub labelled_queries_per_week: f64,
}

impl Default for SyntheticConfig {
    fn default() -> Self {
        SyntheticConfig {
            seed: 10,
            days: 180,
            start: Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap(),
            items_per_day: 3.0,
            suggestion_start_day: 14,
            suggestions_per_item: 5,
            holdout: 0.2,
            p_latent_same_topic: 0.22,
            p_latent_other_topic: 0.02,
            p_exposure_link: 0.08,
            p_accept: 0.12,
            p_dismiss: 0.35,
            novelty_boost: 0.12,
            unpredicted_links_per_day: 2.0,
            resurface_start_day: 7,
            resurfaces_per_day: 2.0,
            p_useful_algorithm: 0.6,
            p_useful_random: 0.4,
            p_rated: 0.85,
            retrieval_start_day: 21,
            labelled_queries_per_week: 2.0,
        }
    }
}

pub const TOPICS: [&str; 8] = [
    "ethics of attention",
    "memory and recollection",
    "walking",
    "gardens",
    "typography",
    "music theory",
    "sleep",
    "cities at night",
];

const ANATOMY: [&str; 10] = [
    "angular gyrus",
    "hippocampus",
    "precuneus",
    "insula",
    "cingulate",
    "fusiform gyrus",
    "thalamus",
    "entorhinal cortex",
    "temporal pole",
    "cuneus",
];

/// Turns raw (time, actor, kind) triples into a seq-numbered, hash-chained log
/// (stable-sorted by time).
pub fn chain(mut raw: Vec<(Timestamp, Actor, EventKind)>) -> Vec<Event> {
    // stable sort keeps causal order for equal timestamps
    raw.sort_by_key(|r| r.0);
    let mut prev = "0".repeat(64);
    let mut out = Vec::with_capacity(raw.len());
    for (i, (at, actor, kind)) in raw.into_iter().enumerate() {
        let seq = i as u64 + 1;
        let body = serde_json::json!({ "seq": seq, "at": at, "actor": actor, "kind": kind, "prev_hash": prev });
        let mut h = Sha256::new();
        h.update(prev.as_bytes());
        h.update(serde_json::to_vec(&body).unwrap_or_default());
        let hash = hex::encode(h.finalize());
        out.push(Event {
            seq,
            at,
            actor,
            kind,
            prev_hash: prev.clone(),
            hash: hash.clone(),
        });
        prev = hash;
    }
    out
}

/// Deterministic ULID-shaped ids.
pub struct IdGen {
    rng: Rng,
}

impl IdGen {
    pub fn new(seed: u64) -> Self {
        IdGen {
            rng: Rng::new(seed ^ 0x1d_1d),
        }
    }
    pub fn at(&mut self, t: Timestamp) -> Id {
        let r = ((self.rng.next_u64() as u128) << 64) | self.rng.next_u64() as u128;
        Id(ulid::Ulid::from_parts(t.timestamp_millis().max(0) as u64, r).to_string())
    }
}

struct SimItem {
    id: Id,
    topic: usize,
    at: Timestamp,
}

struct SimRegion {
    id: Id,
    label: String,
    anatomy: Option<String>,
    topics: Vec<usize>,
    born_at: Timestamp,
    parents: Vec<Id>,
    died_at: Option<Timestamp>,
}

/// Generate a plausible event log. Same config, same log.
pub fn generate(cfg: &SyntheticConfig) -> Vec<Event> {
    let mut rng = Rng::new(cfg.seed);
    let mut ids = IdGen::new(cfg.seed);
    let end = cfg.start + Duration::days(cfg.days as i64);
    let mut raw: Vec<(Timestamp, Actor, EventKind)> = Vec::new();
    let push =
        |raw: &mut Vec<(Timestamp, Actor, EventKind)>, t: Timestamp, a: Actor, k: EventKind| {
            if t < end {
                raw.push((t, a, k));
            }
        };

    // sources
    let mk_source = |ids: &mut IdGen, connector: &str, name: &str, origin: Origin| Source {
        id: ids.at(cfg.start),
        connector: connector.into(),
        name: name.into(),
        config: serde_json::json!({}),
        default_origin: origin,
        default_privacy: Privacy::CloudOk,
        cursor: None,
        created_at: cfg.start,
    };
    let editor = mk_source(&mut ids, "editor", "editor", Origin::OwnerWritten);
    let web = mk_source(&mut ids, "web", "web clips", Origin::External);
    let chat = mk_source(&mut ids, "chat", "chats", Origin::OwnerInChat);
    for s in [&editor, &web, &chat] {
        push(
            &mut raw,
            cfg.start,
            Actor::User,
            EventKind::SourceAdded { source: s.clone() },
        );
    }

    let mut items: Vec<SimItem> = Vec::new();
    let mut by_topic: Vec<Vec<usize>> = vec![Vec::new(); TOPICS.len()];
    let mut linked: std::collections::HashSet<(String, String)> = Default::default();
    let pair_key = |a: &Id, b: &Id| {
        if a.0 < b.0 {
            (a.0.clone(), b.0.clone())
        } else {
            (b.0.clone(), a.0.clone())
        }
    };
    let suggest_start = cfg.start + Duration::days(cfg.suggestion_start_day as i64);
    let resurf_start = cfg.start + Duration::days(cfg.resurface_start_day as i64);
    let mut regions: Vec<SimRegion> = Vec::new();
    let mut n_runs = 0u32;

    for day in 0..cfg.days {
        let day0 = cfg.start + Duration::days(day as i64);

        // --- items
        let n_items = rng.poisson(cfg.items_per_day);
        let mut today: Vec<(Timestamp, Origin)> = (0..n_items)
            .map(|_| {
                let t = day0 + Duration::seconds(rng.below(15 * 3600) as i64);
                let r = rng.f64();
                let origin = if r < 0.5 {
                    Origin::OwnerWritten
                } else if r < 0.6 {
                    Origin::OwnerInChat
                } else if r < 0.72 {
                    Origin::AiGenerated
                } else {
                    Origin::External
                };
                (t, origin)
            })
            .collect();
        today.sort_by_key(|x| x.0);
        for (t, origin) in today {
            // "sleep" falls silent after day 110, so its region eventually dies
            let mut topic = weighted_topic(&mut rng, day);
            if topic == 6 && day > 110 {
                topic = 1;
            }
            let (kind, source) = match origin {
                Origin::OwnerWritten => (
                    if rng.bernoulli(0.25) {
                        ItemKind::Journal
                    } else {
                        ItemKind::Note
                    },
                    &editor,
                ),
                Origin::OwnerInChat | Origin::AiGenerated => (ItemKind::Chat, &chat),
                _ => (
                    [
                        ItemKind::Article,
                        ItemKind::Quote,
                        ItemKind::Clip,
                        ItemKind::Pdf,
                    ][rng.below(4)],
                    &web,
                ),
            };
            let id = ids.at(t);
            let n = items.len() + 1;
            let title = format!("{} {}", TOPICS[topic], n);
            let body = format!("synthetic {} about {} (#{n}).", snake(&kind), TOPICS[topic]);
            let item = Item {
                id: id.clone(),
                source_id: source.id.clone(),
                external_id: format!("synthetic-{n}"),
                uri: None,
                kind,
                title,
                body: body.clone(),
                origin,
                privacy: Privacy::CloudOk,
                created_at: t,
                captured_at: t,
                modified_at: t,
                content_hash: hex::encode(Sha256::digest(body.as_bytes())),
                tags: vec![TOPICS[topic].to_string()],
                meta: serde_json::json!({ "synthetic": true, "topic": topic }),
                deleted: false,
            };
            let actor = match origin {
                Origin::External => Actor::Connector("connector:web".into()),
                Origin::AiGenerated => Actor::Model("model:local/synthetic".into()),
                _ => Actor::User,
            };
            push(&mut raw, t, actor, EventKind::ItemCreated { item });
            if origin == Origin::AiGenerated && rng.bernoulli(0.5) {
                push(
                    &mut raw,
                    t,
                    Actor::System("harness:egress".into()),
                    EventKind::Egress {
                        provider: "anthropic".into(),
                        model: "claude-haiku".into(),
                        purpose: "chat".into(),
                        items: vec![id.clone()],
                        chars: 400 + rng.below(4000) as u64,
                    },
                );
            }

            // written link in the text to an earlier same-topic note
            if origin == Origin::OwnerWritten && !by_topic[topic].is_empty() && rng.bernoulli(0.3) {
                let other = &items[by_topic[topic][rng.below(by_topic[topic].len())]];
                if linked.insert(pair_key(&id, &other.id)) {
                    let link = Link {
                        from: id.clone(),
                        to: other.id.clone(),
                        kind: LinkKind::Written,
                        created_at: t,
                    };
                    push(
                        &mut raw,
                        t + Duration::seconds(1),
                        Actor::User,
                        EventKind::LinkAdded { link },
                    );
                }
            }

            // --- suggestions for the new item
            if t >= suggest_start && items.len() > 10 {
                let ts = t + Duration::minutes(5);
                let feature_young = ts < suggest_start + Duration::days(28);
                let mut cands: Vec<usize> = by_topic[topic].clone();
                rng.shuffle(&mut cands);
                cands.truncate(cfg.suggestions_per_item.saturating_sub(1) as usize);
                // one off-topic candidate
                cands.push(rng.below(items.len()));
                cands.dedup();
                for (r, ci) in cands.iter().enumerate() {
                    let other = &items[*ci];
                    if other.id == id || linked.contains(&pair_key(&id, &other.id)) {
                        continue;
                    }
                    let same = other.topic == topic;
                    let sid = ids.at(ts);
                    let shown = !rng.bernoulli(cfg.holdout);
                    push(
                        &mut raw,
                        ts,
                        Actor::System("mind:suggest".into()),
                        EventKind::SuggestionMade {
                            suggestion_id: sid.clone(),
                            from: id.clone(),
                            to: other.id.clone(),
                            score: (0.9 - 0.08 * r as f64 + 0.03 * rng.normal()).clamp(0.0, 1.0)
                                as f32,
                            rank: r as u32 + 1,
                            shown,
                            model: "synthetic-embed".into(),
                        },
                    );
                    let p_latent = if same {
                        cfg.p_latent_same_topic
                    } else {
                        cfg.p_latent_other_topic
                    };
                    let latent = rng.bernoulli(p_latent);
                    let latent_delay =
                        Duration::minutes((rng.gamma(1.2) * 18.0 * 1440.0) as i64 + 30);
                    let boost = if feature_young {
                        cfg.novelty_boost
                    } else {
                        0.0
                    };
                    let mut done = false;
                    if shown {
                        let at = ts + Duration::minutes(10 + rng.below(3 * 1440) as i64);
                        if rng.bernoulli(cfg.p_accept + boost + if same { 0.05 } else { 0.0 }) {
                            push(
                                &mut raw,
                                at,
                                Actor::User,
                                EventKind::SuggestionAccepted {
                                    suggestion_id: sid.clone(),
                                },
                            );
                            let link = Link {
                                from: id.clone(),
                                to: other.id.clone(),
                                kind: LinkKind::AcceptedSuggestion,
                                created_at: at,
                            };
                            if at < end {
                                linked.insert(pair_key(&id, &other.id));
                            }
                            push(
                                &mut raw,
                                at + Duration::seconds(1),
                                Actor::User,
                                EventKind::LinkAdded { link },
                            );
                            done = true;
                        } else if rng.bernoulli(cfg.p_dismiss) {
                            push(
                                &mut raw,
                                at,
                                Actor::User,
                                EventKind::SuggestionDismissed {
                                    suggestion_id: sid.clone(),
                                },
                            );
                        }
                    }
                    let exposure = shown && rng.bernoulli(cfg.p_exposure_link);
                    if !done && (latent || exposure) {
                        let at = ts + latent_delay;
                        if at < end {
                            linked.insert(pair_key(&id, &other.id));
                        }
                        let link = Link {
                            from: id.clone(),
                            to: other.id.clone(),
                            kind: LinkKind::Manual,
                            created_at: at,
                        };
                        push(&mut raw, at, Actor::User, EventKind::LinkAdded { link });
                    }
                }
            }

            by_topic[topic].push(items.len());
            items.push(SimItem { id, topic, at: t });
        }

        // --- hand links nobody predicted
        for _ in 0..rng.poisson(cfg.unpredicted_links_per_day) {
            if items.len() < 4 {
                break;
            }
            let a = &items[rng.below(items.len())];
            let pool = &by_topic[a.topic];
            let b = &items[pool[rng.below(pool.len())]];
            if a.id == b.id || !linked.insert(pair_key(&a.id, &b.id)) {
                continue;
            }
            let t = day0 + Duration::seconds(rng.below(15 * 3600) as i64);
            let t = t.max(a.at.max(b.at) + Duration::minutes(1));
            let link = Link {
                from: a.id.clone(),
                to: b.id.clone(),
                kind: LinkKind::Manual,
                created_at: t,
            };
            push(&mut raw, t, Actor::User, EventKind::LinkAdded { link });
        }

        // --- resurfacing trial
        if day0 >= resurf_start && !items.is_empty() {
            let young = day0 < resurf_start + Duration::days(28);
            for _ in 0..rng.poisson(cfg.resurfaces_per_day) {
                let t = day0 + Duration::seconds(rng.below(15 * 3600) as i64);
                let algo = rng.bernoulli(0.5);
                let item = &items[rng.below(items.len())];
                let rid = ids.at(t);
                push(
                    &mut raw,
                    t,
                    Actor::System("mind:resurface".into()),
                    EventKind::ResurfaceShown {
                        resurface_id: rid.clone(),
                        item_id: item.id.clone(),
                        arm: if algo {
                            "algorithm".into()
                        } else {
                            "random".into()
                        },
                    },
                );
                if rng.bernoulli(cfg.p_rated) {
                    let p = if algo {
                        cfg.p_useful_algorithm
                    } else {
                        cfg.p_useful_random
                    } + if young { cfg.novelty_boost } else { 0.0 };
                    let useful = rng.bernoulli(p.min(1.0));
                    push(
                        &mut raw,
                        t + Duration::minutes(1 + rng.below(120) as i64),
                        Actor::User,
                        EventKind::ResurfaceRated {
                            resurface_id: rid,
                            useful,
                        },
                    );
                }
            }
        }

        // --- labelled retrieval queries (spread over the week)
        if day >= cfg.retrieval_start_day && rng.bernoulli(cfg.labelled_queries_per_week / 7.0) {
            let topic = rng.below(TOPICS.len());
            let pool = &by_topic[topic];
            if pool.len() >= 4 {
                let t = day0 + Duration::hours(20);
                let phrase = [
                    "what did i write about",
                    "notes on",
                    "where was i on",
                    "everything about",
                ][rng.below(4)];
                let query = format!("{phrase} {} ({day})", TOPICS[topic]);
                let mut pick: Vec<usize> = pool.clone();
                rng.shuffle(&mut pick);
                for (j, ix) in pick.iter().take(5).enumerate() {
                    let grade = if j == 0 { 3 } else { 1 + rng.below(3) as u8 };
                    push(
                        &mut raw,
                        t + Duration::seconds(j as i64),
                        Actor::User,
                        EventKind::RetrievalLabeled {
                            query: query.clone(),
                            item_id: items[*ix].id.clone(),
                            grade,
                        },
                    );
                }
                for j in 0..2 {
                    let ix = rng.below(items.len());
                    if items[ix].topic != topic {
                        push(
                            &mut raw,
                            t + Duration::seconds(10 + j),
                            Actor::User,
                            EventKind::RetrievalLabeled {
                                query: query.clone(),
                                item_id: items[ix].id.clone(),
                                grade: 0,
                            },
                        );
                    }
                }
            }
        }

        // --- weekly clustering run
        if day % 7 == 6 {
            n_runs += 1;
            let t = day0 + Duration::hours(23);
            let mut transitions = Vec::new();
            let topic_counts: Vec<usize> = by_topic.iter().map(|v| v.len()).collect();
            // births
            for (topic, c) in topic_counts.iter().enumerate() {
                let covered = regions
                    .iter()
                    .any(|r| r.died_at.is_none() && r.topics.contains(&topic));
                let ever = regions.iter().any(|r| r.topics.contains(&topic));
                if *c >= 6 && !covered && !ever {
                    let id = ids.at(t);
                    transitions.push(RegionTransition::Birth { region: id.clone() });
                    let anatomy = Some(ANATOMY[regions.len() % ANATOMY.len()].to_string());
                    regions.push(SimRegion {
                        id,
                        label: TOPICS[topic].into(),
                        anatomy,
                        topics: vec![topic],
                        born_at: t,
                        parents: vec![],
                        died_at: None,
                    });
                }
            }
            let mut touched: Vec<Id> = transitions
                .iter()
                .filter_map(|tr| {
                    if let RegionTransition::Birth { region } = tr {
                        Some(region.clone())
                    } else {
                        None
                    }
                })
                .collect();
            // a split around week 10: "memory and recollection" divides in two
            if n_runs == 10
                && let Some(ix) = regions
                    .iter()
                    .position(|r| r.died_at.is_none() && r.topics == vec![1])
            {
                regions[ix].died_at = Some(t);
                let parent = regions[ix].id.clone();
                let a = ids.at(t);
                let b = ids.at(t);
                for (id, label, anat) in [
                    (&a, "recollection", "hippocampus"),
                    (&b, "forgetting", "entorhinal cortex"),
                ] {
                    regions.push(SimRegion {
                        id: id.clone(),
                        label: label.into(),
                        anatomy: Some(anat.into()),
                        topics: vec![1],
                        born_at: t,
                        parents: vec![parent.clone()],
                        died_at: None,
                    });
                }
                transitions.push(RegionTransition::Split {
                    from: parent.clone(),
                    into: vec![a.clone(), b.clone()],
                });
                touched.extend([parent, a, b]);
            }
            // a merge around week 16: "walking" and "cities at night" become one region
            if n_runs == 16 {
                let w = regions
                    .iter()
                    .position(|r| r.died_at.is_none() && r.topics == vec![2]);
                let c = regions
                    .iter()
                    .position(|r| r.died_at.is_none() && r.topics == vec![7]);
                if let (Some(w), Some(c)) = (w, c) {
                    regions[w].died_at = Some(t);
                    regions[c].died_at = Some(t);
                    let from = vec![regions[w].id.clone(), regions[c].id.clone()];
                    let into = ids.at(t);
                    regions.push(SimRegion {
                        id: into.clone(),
                        label: "night walks".into(),
                        anatomy: Some("precuneus".into()),
                        topics: vec![2, 7],
                        born_at: t,
                        parents: from.clone(),
                        died_at: None,
                    });
                    transitions.push(RegionTransition::Merge {
                        from: from.clone(),
                        into: into.clone(),
                    });
                    touched.extend(from);
                    touched.push(into);
                }
            }
            // a death around week 22: "sleep" has gone quiet
            if n_runs == 22
                && let Some(ix) = regions
                    .iter()
                    .position(|r| r.died_at.is_none() && r.topics == vec![6])
            {
                regions[ix].died_at = Some(t);
                transitions.push(RegionTransition::Death {
                    region: regions[ix].id.clone(),
                });
                touched.push(regions[ix].id.clone());
            }
            for r in regions
                .iter()
                .filter(|r| r.died_at.is_none() && !touched.contains(&r.id))
            {
                transitions.push(RegionTransition::Survive {
                    region: r.id.clone(),
                });
            }
            let out: Vec<Region> = regions
                .iter()
                .filter(|r| r.died_at.is_none() || r.died_at == Some(t))
                .map(|r| {
                    let siblings = regions
                        .iter()
                        .filter(|o| o.died_at.is_none() && o.topics == r.topics)
                        .count()
                        .max(1);
                    let size: usize =
                        r.topics.iter().map(|tp| topic_counts[*tp]).sum::<usize>() / siblings;
                    Region {
                        id: r.id.clone(),
                        label: r.label.clone(),
                        anatomy: r.anatomy.clone(),
                        size: size as u32,
                        born_at: r.born_at,
                        died_at: r.died_at,
                        parent_ids: r.parents.clone(),
                    }
                })
                .collect();
            if !out.is_empty() {
                let run_id = ids.at(t);
                push(
                    &mut raw,
                    t,
                    Actor::System("mind:cluster".into()),
                    EventKind::RegionsRecomputed {
                        run_id: run_id.clone(),
                        regions: out,
                        transitions,
                    },
                );
                push(
                    &mut raw,
                    t + Duration::seconds(5),
                    Actor::System("mind:layout".into()),
                    EventKind::LayoutUpdated {
                        run_id,
                        placements_hash: hex::encode(Sha256::digest(
                            format!("{}-{}", cfg.seed, n_runs).as_bytes(),
                        )),
                        count: items.len() as u32,
                    },
                );
            }
        }
    }
    chain(raw)
}

fn weighted_topic(rng: &mut Rng, day: u32) -> usize {
    // interests drift: early weight on the first topics, later on the last ones
    let drift = day as f64 / 180.0;
    let weights: Vec<f64> = (0..TOPICS.len())
        .map(|i| {
            let x = i as f64 / (TOPICS.len() - 1) as f64;
            1.0 + 1.5 * (1.0 - (x - drift).abs())
        })
        .collect();
    let total: f64 = weights.iter().sum();
    let mut r = rng.f64() * total;
    for (i, w) in weights.iter().enumerate() {
        if r < *w {
            return i;
        }
        r -= w;
    }
    TOPICS.len() - 1
}

fn snake<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A deterministic stand-in for the app's search over a synthetic log: items of
/// the query's topic, ordered by a noisy relevance score that tends to follow the
/// user's own grades. Good enough to exercise the retrieval metrics and the ui.
pub struct SyntheticSearch {
    topic_items: Vec<Vec<Id>>,
    grades: HashMap<String, HashMap<String, u8>>,
    all: Vec<Id>,
    seed: u64,
    /// 0 = random ranking, larger = closer to the user's grades.
    pub quality: f64,
}

impl SyntheticSearch {
    pub fn new(events: &[Event], seed: u64) -> Self {
        let mut topic_items = vec![Vec::new(); TOPICS.len()];
        let mut all = Vec::new();
        for e in events {
            if let EventKind::ItemCreated { item } = &e.kind {
                all.push(item.id.clone());
                if let Some(t) = item
                    .tags
                    .first()
                    .and_then(|t| TOPICS.iter().position(|x| x == t))
                {
                    topic_items[t].push(item.id.clone());
                }
            }
        }
        let grades = crate::retrieval::collect_labels(events)
            .into_iter()
            .map(|(q, (_, g))| (q, g))
            .collect();
        SyntheticSearch {
            topic_items,
            grades,
            all,
            seed,
            quality: 1.2,
        }
    }

    pub fn search(&self, query: &str) -> Vec<Id> {
        let q = crate::retrieval::normalise_query(query);
        let mut h = Sha256::new();
        h.update(q.as_bytes());
        let d = h.finalize();
        let mut rng = Rng::new(self.seed ^ u64::from_le_bytes(d[..8].try_into().unwrap()));
        let topic = TOPICS.iter().position(|t| q.contains(t));
        let pool: &[Id] = match topic {
            Some(t) => &self.topic_items[t],
            None => &self.all,
        };
        let grades = self.grades.get(&q);
        let mut scored: Vec<(f64, &Id)> = pool
            .iter()
            .map(|id| {
                let g = grades.and_then(|g| g.get(&id.0)).copied().unwrap_or(1) as f64;
                (self.quality * g + rng.normal(), id)
            })
            .collect();
        // a little off-topic noise
        for _ in 0..3 {
            if !self.all.is_empty() {
                scored.push((rng.normal() - 0.5, &self.all[rng.below(self.all.len())]));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut seen = std::collections::HashSet::new();
        scored
            .into_iter()
            .filter(|(_, id)| seen.insert(id.0.clone()))
            .take(20)
            .map(|(_, id)| id.clone())
            .collect()
    }
}
