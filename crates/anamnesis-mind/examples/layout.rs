//! Lay out a corpus at growing snapshots and print the lab's placements json.
//!
//! ```sh
//! cargo run --release -p anamnesis-mind --example layout -- vectors.jsonl > placements.json
//! cargo run --release -p anamnesis-mind --example layout -- vectors.npy [keys.txt] > placements.json
//! ```
//!
//! - `.jsonl`: one item per line, `{"id": "...", "key": "<note basename>", "vector": [...]}`
//!   (`id` defaults to the line number, `key` is optional), in time order.
//! - `.npy`: a 2-d float32 or float64 array (C order), one row per item in time
//!   order; ids are row numbers. An optional text file gives one key per line.
//!
//! `SNAPSHOTS` (default 5) sets how many growing snapshots, from 50% to 100%.

use std::path::Path;

use anamnesis_core::Id;
use anamnesis_mind::lab::{export_placements_json, layout_snapshots, snapshot_sizes};
use anamnesis_mind::{LayoutParams, SpaceParams};

type Corpus = (Vec<Id>, Vec<Vec<f32>>, Option<Vec<String>>);

fn read_jsonl(path: &Path) -> Result<Corpus, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (mut ids, mut vs, mut keys) = (vec![], vec![], vec![]);
    let mut any_key = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 1))?;
        let vec: Vec<f32> = serde_json::from_value(v["vector"].clone())
            .map_err(|e| format!("line {}: vector: {e}", i + 1))?;
        ids.push(Id(v["id"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| i.to_string())));
        let key = v["key"].as_str().map(str::to_string);
        any_key |= key.is_some();
        keys.push(key.unwrap_or_default());
        vs.push(vec);
    }
    Ok((ids, vs, any_key.then_some(keys)))
}

/// Minimal reader for 2-d little-endian `<f4` / `<f8` C-order arrays.
fn read_npy(path: &Path) -> Result<(Vec<Id>, Vec<Vec<f32>>), String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    if b.len() < 10 || &b[..6] != b"\x93NUMPY" {
        return Err("not an .npy file".into());
    }
    let (hlen, hstart) = if b[6] == 1 {
        (u16::from_le_bytes([b[8], b[9]]) as usize, 10)
    } else {
        (u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize, 12)
    };
    let header = std::str::from_utf8(&b[hstart..hstart + hlen]).map_err(|e| e.to_string())?;
    if header.contains("'fortran_order': True") {
        return Err("fortran-order arrays are not supported".into());
    }
    let width = if header.contains("'<f4'") {
        4
    } else if header.contains("'<f8'") {
        8
    } else {
        return Err(format!("unsupported dtype in header {header}"));
    };
    let shape = header
        .split("'shape':")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .ok_or("no shape")?;
    let dims: Vec<usize> = shape
        .trim_start_matches([' ', '('])
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let [n, d] = dims[..] else {
        return Err(format!("expected a 2-d array, got shape {dims:?}"));
    };
    let data = &b[hstart + hlen..];
    if data.len() < n * d * width {
        return Err("truncated .npy data".into());
    }
    let vs = (0..n)
        .map(|i| {
            (0..d)
                .map(|j| {
                    let o = (i * d + j) * width;
                    if width == 4 {
                        f32::from_le_bytes(data[o..o + 4].try_into().unwrap())
                    } else {
                        f64::from_le_bytes(data[o..o + 8].try_into().unwrap()) as f32
                    }
                })
                .collect()
        })
        .collect();
    Ok(((0..n).map(|i| Id(i.to_string())).collect(), vs))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        return Err("usage: layout <vectors.npy|vectors.jsonl> [keys.txt]".into());
    };
    let path = Path::new(input);
    let (ids, vectors, keys) = if path.extension().is_some_and(|e| e == "npy") {
        let (ids, vs) = read_npy(path)?;
        let keys = match args.get(1) {
            Some(k) => Some(
                std::fs::read_to_string(k)
                    .map_err(|e| e.to_string())?
                    .lines()
                    .map(str::to_string)
                    .collect(),
            ),
            None => None,
        };
        (ids, vs, keys)
    } else {
        read_jsonl(path)?
    };
    let count = std::env::var("SNAPSHOTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let sizes = snapshot_sizes(ids.len(), count, 0.5);
    eprintln!("{} items, snapshots {sizes:?}", ids.len());
    let snaps = layout_snapshots(
        &ids,
        &vectors,
        keys.as_deref(),
        &sizes,
        SpaceParams::default(),
        &LayoutParams::default(),
    );
    println!("{}", export_placements_json(&snaps));
    Ok(())
}
