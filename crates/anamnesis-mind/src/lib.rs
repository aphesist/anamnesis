//! anamnesis-mind: the growing brain.
//!
//! Pure computation on in-memory data — no storage, no network, no model
//! calls (prompts for labelling and extraction are returned as data for the
//! harness to run).
//!
//! - [`space`]: normalised vectors, PCA reduction and an exact kNN graph,
//!   shared by clustering and layout.
//! - [`regions`]: HDBSCAN regions with stable identity across runs
//!   (Hungarian matching on Jaccard overlap, MONIC transitions), assignment
//!   of new items between runs, drift and cohesion.
//! - [`labels`]: c-TF-IDF keywords, representative items, fallback labels,
//!   a labelling prompt + schema.
//! - [`anatomy`]: poetic, stable brain-area names for regions.
//! - [`layout`]: UMAP-like 3D layout, incremental placement, Procrustes
//!   alignment, and the `layout` / `anatomy` / `spheres` views.
//! - [`links`]: link suggestion and the seeded hold-out.
//! - [`threads`]: decisions / open questions / plans / rejections (rules and
//!   validated model extraction).
//! - [`resurface`]: FSRS-6 scheduling and the daily review mix with a
//!   micro-randomised trial.
//! - [`pipeline`]: the full recompute job in one call.

// index loops read better than iterator chains in the numeric code
#![allow(clippy::needless_range_loop)]

pub mod anatomy;
pub mod hdbscan;
pub mod knn;
pub mod labels;
pub mod layout;
pub mod linalg;
pub mod links;
pub mod pipeline;
pub mod regions;
pub mod resurface;
pub mod rng;
pub mod space;
pub mod threads;

pub use layout::{Layout, LayoutParams, LayoutViews, layout_full, place_new, views};
pub use links::{ItemMeta, LinkParams, LinkSuggester, LinkSuggestion, split_for_holdout};
pub use pipeline::{FullRun, MindParams, MindState, full_run};
pub use regions::{RegionParams, RegionRun, RegionState, recompute_regions};
pub use resurface::{Scheduler, SrsCard, daily_review};
pub use space::{Space, SpaceParams};
pub use threads::{extract_rules, extraction_prompt, validate_extraction};
