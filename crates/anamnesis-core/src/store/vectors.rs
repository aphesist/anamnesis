//! Chunk embeddings and brute-force cosine kNN.
//!
//! Choice: vectors are stored as little-endian f32 blobs keyed by `(chunk text hash, model id)`
//! and searched by brute force over a contiguous, row-normalised `Vec<f32>` matrix cached per
//! model. At 100k chunks × 384 dims that is ~150 MB and a full scan takes a few tens of
//! milliseconds on one core, which is fine for a personal brain; it needs no native extension
//! (sqlite-vec would add a loadable C extension and a second storage format). Keying by text
//! hash means identical chunks share a vector, unchanged chunks keep theirs across edits, and
//! vectors survive `rebuild_projections`. The cache is invalidated whenever items or
//! embeddings change.

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Store;
use crate::error::{Error, Result};
use crate::model::{Chunk, Id};

/// One kNN result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorHit {
    pub chunk_id: Id,
    pub item_id: Id,
    /// Cosine similarity in [-1, 1].
    pub score: f32,
}

/// An embedding model present in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingModelInfo {
    pub model_id: String,
    pub dim: usize,
    pub vectors: u64,
}

/// Cached, row-normalised matrix of one model's chunk vectors.
pub(crate) struct VecIndex {
    pub(crate) dim: usize,
    pub(crate) chunk_ids: Vec<Id>,
    pub(crate) item_ids: Vec<Id>,
    pub(crate) data: Vec<f32>,
}

pub(crate) fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub(crate) fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    // 4 accumulators let the compiler vectorise
    let mut acc = [0f32; 4];
    let chunks = a.len() / 4;
    for i in 0..chunks {
        let j = i * 4;
        acc[0] += a[j] * b[j];
        acc[1] += a[j + 1] * b[j + 1];
        acc[2] += a[j + 2] * b[j + 2];
        acc[3] += a[j + 3] * b[j + 3];
    }
    let mut s = acc[0] + acc[1] + acc[2] + acc[3];
    for j in chunks * 4..a.len() {
        s += a[j] * b[j];
    }
    s
}

impl VecIndex {
    pub(crate) fn knn(&self, query: &[f32], k: usize) -> Vec<(usize, f32)> {
        let n = self.chunk_ids.len();
        if n == 0 || k == 0 {
            return Vec::new();
        }
        let mut scored: Vec<(usize, f32)> = (0..n)
            .map(|i| (i, dot(&self.data[i * self.dim..(i + 1) * self.dim], query)))
            .collect();
        let k = k.min(n);
        if k < n {
            scored.select_nth_unstable_by(k - 1, |a, b| b.1.total_cmp(&a.1));
            scored.truncate(k);
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored
    }
}

impl Store {
    fn model_dim(&self, model_id: &str) -> Result<Option<usize>> {
        Ok(self
            .conn
            .query_row(
                "SELECT dim FROM embeddings WHERE model_id = ?1 LIMIT 1",
                params![model_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .map(|d| d as usize))
    }

    /// Store embeddings for chunks (by chunk id) under `model_id`. All vectors of one model
    /// must have the same dimension. Embeddings are a cache, not events. Returns how many were
    /// stored (unknown chunk ids are skipped).
    pub fn put_embeddings(&self, model_id: &str, entries: &[(Id, Vec<f32>)]) -> Result<usize> {
        let mut dim = self.model_dim(model_id)?;
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0;
        {
            let mut find = tx.prepare_cached("SELECT text_hash FROM chunks WHERE id = ?1")?;
            let mut ins = tx.prepare_cached(
                "INSERT OR REPLACE INTO embeddings (text_hash, model_id, dim, vec) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (chunk_id, v) in entries {
                if v.is_empty() {
                    return Err(Error::Invalid("empty embedding".into()));
                }
                match dim {
                    Some(d) if d != v.len() => {
                        return Err(Error::Invalid(format!(
                            "embedding dim {} does not match model {model_id} dim {d}",
                            v.len()
                        )));
                    }
                    None => dim = Some(v.len()),
                    _ => {}
                }
                let Some(h): Option<String> = find
                    .query_row(params![chunk_id.0], |r| r.get(0))
                    .optional()?
                else {
                    continue;
                };
                ins.execute(params![h, model_id, v.len() as i64, to_blob(v)])?;
                n += 1;
            }
        }
        tx.commit()?;
        self.invalidate_vectors();
        Ok(n)
    }

    /// Live, non-empty chunks that have no embedding for `model_id` yet (embed these next).
    pub fn chunks_missing_embedding(&self, model_id: &str, limit: usize) -> Result<Vec<Chunk>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.item_id, c.ord, c.text, c.span_start, c.span_end FROM chunks c
             WHERE c.text != '' AND NOT EXISTS
               (SELECT 1 FROM embeddings e WHERE e.text_hash = c.text_hash AND e.model_id = ?1)
             ORDER BY c.rowid LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![model_id, limit as i64], row_to_chunk)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The chunks of an item, in order.
    pub fn chunks_for_item(&self, item_id: &Id) -> Result<Vec<Chunk>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_id, ord, text, span_start, span_end FROM chunks WHERE item_id = ?1 ORDER BY ord",
        )?;
        let rows = stmt
            .query_map(params![item_id.0], row_to_chunk)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// One chunk by id.
    pub fn get_chunk(&self, chunk_id: &Id) -> Result<Option<Chunk>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, item_id, ord, text, span_start, span_end FROM chunks WHERE id = ?1",
                params![chunk_id.0],
                row_to_chunk,
            )
            .optional()?)
    }

    /// Models with stored vectors.
    pub fn embedding_models(&self) -> Result<Vec<EmbeddingModelInfo>> {
        let mut stmt =
            self.conn.prepare("SELECT model_id, MAX(dim), COUNT(*) FROM embeddings GROUP BY model_id ORDER BY model_id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(EmbeddingModelInfo {
                    model_id: r.get(0)?,
                    dim: r.get::<_, i64>(1)? as usize,
                    vectors: r.get::<_, i64>(2)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Delete embeddings whose text no longer appears in any chunk. Returns rows removed.
    pub fn prune_embeddings(&self) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM embeddings WHERE text_hash NOT IN (SELECT text_hash FROM chunks)",
            [],
        )?;
        Ok(n)
    }

    pub(crate) fn vec_index(&self, model_id: &str) -> Result<Arc<VecIndex>> {
        if let Some(ix) = self
            .vec_cache
            .lock()
            .ok()
            .and_then(|c| c.get(model_id).cloned())
        {
            return Ok(ix);
        }
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.item_id, e.dim, e.vec FROM chunks c
             JOIN embeddings e ON e.text_hash = c.text_hash AND e.model_id = ?1
             ORDER BY c.rowid",
        )?;
        let mut rows = stmt.query(params![model_id])?;
        let mut ix = VecIndex {
            dim: 0,
            chunk_ids: Vec::new(),
            item_ids: Vec::new(),
            data: Vec::new(),
        };
        while let Some(r) = rows.next()? {
            let dim = r.get::<_, i64>(2)? as usize;
            if ix.dim == 0 {
                ix.dim = dim;
            } else if dim != ix.dim {
                continue;
            }
            let blob: Vec<u8> = r.get(3)?;
            let mut v = from_blob(&blob);
            if v.len() != dim {
                continue;
            }
            normalize(&mut v);
            ix.chunk_ids.push(Id(r.get(0)?));
            ix.item_ids.push(Id(r.get(1)?));
            ix.data.extend_from_slice(&v);
        }
        let ix = Arc::new(ix);
        if let Ok(mut c) = self.vec_cache.lock() {
            c.insert(model_id.to_string(), ix.clone());
        }
        Ok(ix)
    }

    /// The `k` chunks most cosine-similar to `query` under `model_id`.
    pub fn knn(&self, model_id: &str, query: &[f32], k: usize) -> Result<Vec<VectorHit>> {
        let ix = self.vec_index(model_id)?;
        if ix.chunk_ids.is_empty() {
            return Ok(Vec::new());
        }
        if query.len() != ix.dim {
            return Err(Error::Invalid(format!(
                "query dim {} != model dim {}",
                query.len(),
                ix.dim
            )));
        }
        let mut q = query.to_vec();
        normalize(&mut q);
        Ok(ix
            .knn(&q, k)
            .into_iter()
            .map(|(i, s)| VectorHit {
                chunk_id: ix.chunk_ids[i].clone(),
                item_id: ix.item_ids[i].clone(),
                score: s,
            })
            .collect())
    }

    /// One l2-normalised vector per live item (mean of its chunk vectors), ordered by item id.
    /// This is the input `mind` clusters and lays out.
    pub fn item_vectors(&self, model_id: &str) -> Result<(Vec<Id>, Vec<Vec<f32>>)> {
        let ix = self.vec_index(model_id)?;
        let mut acc: HashMap<&Id, (Vec<f32>, usize)> = HashMap::new();
        for (i, item) in ix.item_ids.iter().enumerate() {
            let row = &ix.data[i * ix.dim..(i + 1) * ix.dim];
            let e = acc.entry(item).or_insert_with(|| (vec![0.0; ix.dim], 0));
            e.0.iter_mut().zip(row).for_each(|(a, b)| *a += b);
            e.1 += 1;
        }
        let mut items: Vec<(&Id, (Vec<f32>, usize))> = acc.into_iter().collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        let mut ids = Vec::with_capacity(items.len());
        let mut vecs = Vec::with_capacity(items.len());
        for (id, (mut v, _)) in items {
            normalize(&mut v);
            ids.push(id.clone());
            vecs.push(v);
        }
        Ok((ids, vecs))
    }
}

fn row_to_chunk(r: &rusqlite::Row<'_>) -> rusqlite::Result<Chunk> {
    Ok(Chunk {
        id: Id(r.get(0)?),
        item_id: Id(r.get(1)?),
        ord: r.get::<_, i64>(2)? as u32,
        text: r.get(3)?,
        start: r.get::<_, i64>(4)? as usize,
        end: r.get::<_, i64>(5)? as usize,
    })
}
