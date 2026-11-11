mod support;

use anamnesis_core::Id;
use anamnesis_mind::resurface::{Arm, Engagement, Rating, ReviewInfo, ReviewParams, ReviewReason, Scheduler, SrsCard, daily_review};
use anamnesis_mind::rng::Rng;
use chrono::{Duration, TimeZone, Utc};
use support::*;

#[test]
fn fsrs_intervals_grow_and_lapse() {
    let s = Scheduler::default();
    // R(S, S) = 0.9 and the interval at 90% retention equals S
    assert!((s.retrievability(10.0, 10.0) - 0.9).abs() < 1e-9);
    assert!((s.interval_days(10.0) - 10.0).abs() < 1e-6);
    let mut now = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap();
    let mut card = SrsCard::new(now);
    let mut intervals = vec![];
    for _ in 0..6 {
        card = s.review(&card, Rating::Good, now);
        let iv = (card.due - now).num_seconds() as f64 / 86_400.0;
        intervals.push(iv);
        now = card.due;
    }
    assert!(intervals.windows(2).all(|w| w[1] > w[0]), "{intervals:?}");
    assert!(intervals[0] >= 1.0 && intervals[5] > 30.0, "{intervals:?}");
    // a lapse shrinks stability and counts
    let before = card.stability;
    let lapsed = s.review(&card, Rating::Again, now);
    assert!(lapsed.stability < before);
    assert_eq!(lapsed.lapses, 1);
    // easy grows faster than good, hard slower
    let good = s.review(&card, Rating::Good, now).stability;
    let easy = s.review(&card, Rating::Easy, now).stability;
    let hard = s.review(&card, Rating::Hard, now).stability;
    assert!(easy > good && good > hard && hard > before * 0.99, "{hard} {good} {easy} vs {before}");
    // lower desired retention → longer intervals
    let loose = Scheduler { desired_retention: 0.8, ..Default::default() };
    assert!(loose.interval_days(10.0) > 10.0);
    // "never" suspends
    assert!(s.engage(&card, Engagement::Never, now).suspended);
    assert_eq!(Engagement::from_useful(true).rating(), Some(Rating::Good));
}

fn pool(now: chrono::DateTime<Utc>) -> (Vec<Id>, Vec<Vec<f32>>, Vec<ReviewInfo>, Vec<Vec<f32>>) {
    let mut r = Rng::new(5);
    let d = 16;
    let topic = unit(&mut r, d);
    let ids = ids(60, "i");
    let mut vs = vec![];
    let mut info = vec![];
    for i in 0..60 {
        let near_topic = i < 10;
        vs.push(if near_topic { around(&mut r, &topic, 0.3) } else { unit(&mut r, d) });
        let card = if (10..14).contains(&i) {
            // seeds: due in the past
            let mut c = SrsCard::new(now - Duration::days(30));
            c = Scheduler::default().review(&c, Rating::Good, now - Duration::days(30 - i as i64 + 10));
            Some(c)
        } else {
            None
        };
        info.push(ReviewInfo {
            created_at: now - Duration::days(10 + 5 * i as i64),
            last_seen: None,
            times_shown: (i % 3) as u32,
            card,
            is_open_question: i % 7 == 0,
            centrality: Some(0.8),
        });
    }
    let recent = vec![around(&mut r, &topic, 0.1)];
    (ids, vs, info, recent)
}

#[test]
fn review_mix_is_reproducible_and_sensible() {
    let now = Utc.with_ymd_and_hms(2026, 6, 1, 8, 0, 0).unwrap();
    let (ids, vs, info, recent) = pool(now);
    let sched = Scheduler::default();
    let all_algo = ReviewParams { random_arm_probability: 0.0, ..Default::default() };
    let a = daily_review(&ids, &vs, &info, &recent, &[], now, &all_algo, &sched, 1);
    let b = daily_review(&ids, &vs, &info, &recent, &[], now, &all_algo, &sched, 1);
    assert_eq!(a, b);
    assert_eq!(a.len(), 7);
    // due slots are filled by the seeds
    for p in a.iter().filter(|p| p.slot == ReviewReason::Due) {
        assert_eq!(p.reason, ReviewReason::Due);
        let k = ids.iter().position(|x| x == &p.item_id).unwrap();
        assert!((10..14).contains(&k), "{k}");
    }
    // contextual picks come from the recent topic
    for p in a.iter().filter(|p| p.reason == ReviewReason::Contextual) {
        let k = ids.iter().position(|x| x == &p.item_id).unwrap();
        assert!(k < 10, "{k}");
    }
    // serendipity picks are old
    for p in a.iter().filter(|p| p.reason == ReviewReason::Serendipity) {
        let k = ids.iter().position(|x| x == &p.item_id).unwrap();
        assert!((now - info[k].created_at).num_days() >= 30);
    }
    // no duplicates
    let set: std::collections::HashSet<_> = a.iter().map(|p| &p.item_id).collect();
    assert_eq!(set.len(), a.len());

    // micro-randomised arms: ~50% random over many days, reproducible per seed
    let p = ReviewParams::default();
    let mut random = 0;
    let mut total = 0;
    for day in 0..60 {
        let t = now + Duration::days(day);
        let x = daily_review(&ids, &vs, &info, &recent, &[], t, &p, &sched, 7);
        let y = daily_review(&ids, &vs, &info, &recent, &[], t, &p, &sched, 7);
        assert_eq!(x, y);
        random += x.iter().filter(|q| q.arm == Arm::Random).count();
        total += x.len();
        assert!(x.iter().all(|q| (q.arm == Arm::Random) == (q.reason == ReviewReason::Random)));
    }
    let share = random as f64 / total as f64;
    assert!((share - 0.5).abs() < 0.1, "{share}");
    assert_eq!(Arm::Random.as_str(), "random");
    // recently touched items are excluded
    let ex = daily_review(&ids, &vs, &info, &recent, &ids[..30], now, &all_algo, &sched, 1);
    assert!(ex.iter().all(|q| !ids[..30].contains(&q.item_id)));
}
