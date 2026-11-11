//! One call for the server's periodic "recompute the brain" job:
//! space → layout (continuing the previous one) → regions (named with the
//! anatomy view) → the three views → placements.

use anamnesis_core::{Id, Placement, Timestamp};
use serde::{Deserialize, Serialize};

use crate::layout::{AnatomyWarp, Layout, LayoutParams, LayoutViews, layout_full, views};
use crate::regions::{RegionExtras, RegionParams, RegionRun, RegionState, recompute_regions};
use crate::space::{Space, SpaceParams};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct MindParams {
    pub space: SpaceParams,
    pub layout: LayoutParams,
    pub regions: RegionParams,
}

/// Everything the next full run needs; persist as json.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MindState {
    pub layout: Layout,
    pub regions: RegionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullRun {
    pub regions: RegionRun,
    pub views: LayoutViews,
    pub placements: Vec<Placement>,
    pub procrustes_residual: Option<f32>,
    /// Pass to the next run.
    pub state: MindState,
}

/// Recompute regions and layout for the whole corpus. `texts` (title + body
/// per item) feed region keywords; pass `None` to skip them.
pub fn full_run(
    ids: &[Id],
    vectors: &[Vec<f32>],
    texts: Option<&[String]>,
    previous: &MindState,
    now: Timestamp,
    params: &MindParams,
) -> FullRun {
    let space = Space::new(vectors, params.space);
    let prev_layout = if previous.layout.ids.is_empty() {
        None
    } else {
        Some(&previous.layout)
    };
    let lay = layout_full(ids, &space, prev_layout, &params.layout);
    let warp = AnatomyWarp::fit(&lay.layout.xyz);
    let anatomy_xyz: Vec<[f32; 3]> = lay.layout.xyz.iter().map(|p| warp.apply(*p)).collect();
    let regions = recompute_regions(
        ids,
        &space,
        &previous.regions,
        now,
        &params.regions,
        RegionExtras {
            texts,
            anatomy_xyz: Some(&anatomy_xyz),
            layout_xyz: Some(&lay.layout.xyz),
        },
    );
    let views = views(&lay.layout, &regions.assignment);
    let placements = lay.layout.placements(&regions.assignment);
    let state = MindState {
        layout: lay.layout,
        regions: regions.state.clone(),
    };
    FullRun {
        regions,
        views,
        placements,
        procrustes_residual: lay.procrustes_residual,
        state,
    }
}
