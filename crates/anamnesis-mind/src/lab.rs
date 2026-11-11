//! Export layouts in the research lab's placements format, so the Rust layout
//! can be compared with UMAP / PaCMAP (`anamnesis-lab bench layout
//! --rust-placements FILE`, see `lab/README.md`):
//!
//! ```json
//! {"snapshots": [{"label": "50%", "placements": [{"item_id": "01J…", "xyz": [0.1, 0.2, 0.3], "key": "<note basename>"}]}]}
//! ```
//!
//! `key` ties a placement to a bundle note so the lab can look up its
//! embedding; drift between snapshots needs only stable `item_id`s.

use anamnesis_core::Id;
use serde::{Deserialize, Serialize};

use crate::layout::{Layout, LayoutParams, layout_full};
use crate::space::{Space, SpaceParams};

/// One placement as the lab reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabPlacement {
    pub item_id: Id,
    pub xyz: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// One layout snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabSnapshot {
    pub label: String,
    pub placements: Vec<LabPlacement>,
}

/// The whole placements file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LabPlacements {
    pub snapshots: Vec<LabSnapshot>,
}

impl LabSnapshot {
    /// A snapshot from a layout. `keys`, when given, is aligned with
    /// `layout.ids` (missing trailing keys are left out).
    pub fn from_layout(label: impl Into<String>, layout: &Layout, keys: Option<&[String]>) -> Self {
        LabSnapshot {
            label: label.into(),
            placements: layout
                .ids
                .iter()
                .zip(&layout.xyz)
                .enumerate()
                .map(|(i, (id, xyz))| LabPlacement {
                    item_id: id.clone(),
                    xyz: *xyz,
                    key: keys.and_then(|k| k.get(i).cloned()),
                })
                .collect(),
        }
    }
}

/// Serialise snapshots as the lab's placements json.
pub fn export_placements_json(snapshots: &[LabSnapshot]) -> String {
    serde_json::to_string(&LabPlacements {
        snapshots: snapshots.to_vec(),
    })
    .expect("placements serialise")
}

/// Snapshot sizes as the lab computes them: `count` sizes evenly spaced from
/// `start · n` to `n` (rounded half to even, like python).
pub fn snapshot_sizes(n: usize, count: usize, start: f64) -> Vec<usize> {
    match count {
        0 => vec![],
        1 => vec![n],
        _ => (0..count)
            .map(|i| {
                let f = start + (1.0 - start) * i as f64 / (count - 1) as f64;
                // python's round() rounds half to even
                (n as f64 * f).round_ties_even() as usize
            })
            .collect(),
    }
}

/// Lay out growing prefixes of the corpus (items in time order), each
/// snapshot continuing the previous one as the app's periodic full run does.
/// Labels are the share of the corpus, e.g. `"50%"`.
pub fn layout_snapshots(
    ids: &[Id],
    vectors: &[Vec<f32>],
    keys: Option<&[String]>,
    sizes: &[usize],
    space: SpaceParams,
    params: &LayoutParams,
) -> Vec<LabSnapshot> {
    assert_eq!(ids.len(), vectors.len());
    let n = ids.len();
    let mut previous: Option<Layout> = None;
    let mut out = Vec::with_capacity(sizes.len());
    for &m in sizes {
        let m = m.min(n);
        let sp = Space::new(&vectors[..m], space);
        let run = layout_full(&ids[..m], &sp, previous.as_ref(), params);
        let pct = if n == 0 {
            100.0
        } else {
            100.0 * m as f64 / n as f64
        };
        out.push(LabSnapshot::from_layout(
            format!("{pct:.0}%"),
            &run.layout,
            keys.map(|k| &k[..m.min(k.len())]),
        ));
        previous = Some(run.layout);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_lab() {
        // numpy: [int(round(n * f)) for f in np.linspace(0.5, 1.0, 5)] with n = 101
        assert_eq!(snapshot_sizes(101, 5, 0.5), vec![50, 63, 76, 88, 101]);
        assert_eq!(snapshot_sizes(10, 1, 0.5), vec![10]);
        assert!(snapshot_sizes(10, 0, 0.5).is_empty());
    }

    #[test]
    fn json_has_the_documented_shape() {
        let layout = Layout {
            ids: vec![Id("a".into()), Id("b".into())],
            xyz: vec![[0.1, 0.2, 0.3], [0.0, 0.0, 1.0]],
        };
        let keys = vec!["note-a".to_string()];
        let s = export_placements_json(&[LabSnapshot::from_layout("50%", &layout, Some(&keys))]);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        let snap = &v["snapshots"][0];
        assert_eq!(snap["label"], "50%");
        let p0 = &snap["placements"][0];
        assert_eq!(p0["item_id"], "a");
        assert_eq!(p0["key"], "note-a");
        assert_eq!(p0["xyz"].as_array().unwrap().len(), 3);
        // no key: field left out, as the lab's `p.get("key")` expects
        assert!(snap["placements"][1].get("key").is_none());
        let back: LabPlacements = serde_json::from_str(&s).unwrap();
        assert_eq!(back.snapshots[0].placements.len(), 2);
    }
}
