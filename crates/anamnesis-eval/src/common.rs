//! Shared shapes: readiness status, estimates, time windows and the novelty split.

use anamnesis_core::{Event, Timestamp};
use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::stats::Interval;

/// Days at the start of a feature's life that are reported separately (novelty guard).
pub const NOVELTY_DAYS: i64 = 28;

/// Whether a metric has enough data to be read as a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[serde(rename = "measuring")]
    Measuring,
    #[serde(rename = "enough data")]
    EnoughData,
}

impl Status {
    pub fn from_bool(enough: bool) -> Self {
        if enough {
            Status::EnoughData
        } else {
            Status::Measuring
        }
    }
}

/// A point estimate with its sample size and interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub n: usize,
    pub value: Option<f64>,
    pub interval: Option<Interval>,
}

impl Estimate {
    pub fn empty() -> Self {
        Estimate {
            n: 0,
            value: None,
            interval: None,
        }
    }
}

/// Half-open time window `[from, to)`; `None` bounds are open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Window {
    pub from: Option<Timestamp>,
    pub to: Option<Timestamp>,
}

impl Window {
    pub const ALL: Window = Window {
        from: None,
        to: None,
    };
    /// Contains nothing.
    pub const EMPTY: Window = Window {
        from: Some(chrono::DateTime::<chrono::Utc>::MAX_UTC),
        to: None,
    };
    pub fn contains(&self, t: Timestamp) -> bool {
        self.from.is_none_or(|f| t >= f) && self.to.is_none_or(|e| t < e)
    }
}

/// The same metric computed over the feature's first `days` days and after them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Novelty<T> {
    /// When the feature first appears in the log (`None`: never used).
    pub feature_start: Option<Timestamp>,
    pub days: i64,
    pub first_days: T,
    pub after: T,
}

impl<T> Novelty<T> {
    /// Build both halves by calling `f` with each window.
    pub fn compute(
        feature_start: Option<Timestamp>,
        days: i64,
        mut f: impl FnMut(Window) -> T,
    ) -> Self {
        let (first, after) = match feature_start {
            Some(s) => {
                let cut = s + Duration::days(days);
                (
                    Window {
                        from: None,
                        to: Some(cut),
                    },
                    Window {
                        from: Some(cut),
                        to: None,
                    },
                )
            }
            // the feature was never used: both halves are empty
            None => (Window::EMPTY, Window::EMPTY),
        };
        Novelty {
            feature_start,
            days,
            first_days: f(first),
            after: f(after),
        }
    }
}

/// Timestamp of the first event matching `pred`.
pub fn first_at(events: &[Event], pred: impl Fn(&Event) -> bool) -> Option<Timestamp> {
    events.iter().filter(|e| pred(e)).map(|e| e.at).min()
}

/// The time the log is read "as of": the explicit value or the last event's time.
pub fn as_of(events: &[Event], explicit: Option<Timestamp>) -> Option<Timestamp> {
    explicit.or_else(|| events.iter().map(|e| e.at).max())
}

pub fn days_between(a: Timestamp, b: Timestamp) -> f64 {
    (b - a).num_milliseconds() as f64 / 86_400_000.0
}
