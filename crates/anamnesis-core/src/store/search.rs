//! Hybrid search: FTS5 (bm25) + vector kNN, fused with reciprocal rank fusion.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Store;
use crate::error::Result;
use crate::model::{Hit, Id};

/// RRF constant (Cormack et al.).
const RRF_K: f32 = 60.0;
const SNIPPET_CHARS: usize = 240;

/// A query embedding, produced by the caller with the same model as the stored vectors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryVector {
    pub model_id: String,
    pub values: Vec<f32>,
}

/// Whether hits are chunks or items (best chunk per item).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    #[default]
    Item,
    Chunk,
}

/// A hybrid search request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub vector: Option<QueryVector>,
    pub k: usize,
    #[serde(default)]
    pub granularity: Granularity,
}

impl SearchQuery {
    /// Text-only item search.
    pub fn text(q: impl Into<String>, k: usize) -> Self {
        SearchQuery {
            text: q.into(),
            vector: None,
            k,
            granularity: Granularity::Item,
        }
    }
}

/// Build an FTS5 MATCH expression from free text: every word quoted (so operators and
/// punctuation in user input are inert), joined with `AND` or `OR`.
fn fts_expr(text: &str, op: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(&format!(" {op} ")))
}

struct Cand {
    item_id: Id,
    title: String,
    snippet: String,
    start: usize,
    end: usize,
}

fn cut_snippet(text: &str) -> String {
    if text.chars().count() <= SNIPPET_CHARS {
        return text.to_string();
    }
    let s: String = text.chars().take(SNIPPET_CHARS).collect();
    format!("{}…", s.trim_end())
}

impl Store {
    fn fts_hits(&self, text: &str, limit: usize) -> Result<Vec<(Id, Cand)>> {
        let mut out = Vec::new();
        for op in ["AND", "OR"] {
            let Some(expr) = fts_expr(text, op) else {
                return Ok(out);
            };
            let mut stmt = self.conn.prepare_cached(
                "SELECT c.id, c.item_id, i.title, snippet(chunks_fts, 1, '', '', '…', 32), c.text, c.span_start, c.span_end
                 FROM chunks_fts JOIN chunks c ON c.rowid = chunks_fts.rowid JOIN items i ON i.id = c.item_id
                 WHERE chunks_fts MATCH ?1 AND i.deleted = 0
                 ORDER BY bm25(chunks_fts, 2.0, 1.0) LIMIT ?2",
            )?;
            out = stmt
                .query_map(params![expr, limit as i64], |r| {
                    let snip: String = r.get(3)?;
                    let text: String = r.get(4)?;
                    Ok((
                        Id(r.get(0)?),
                        Cand {
                            item_id: Id(r.get(1)?),
                            title: r.get(2)?,
                            snippet: if snip.trim().is_empty() {
                                cut_snippet(&text)
                            } else {
                                snip
                            },
                            start: r.get::<_, i64>(5)? as usize,
                            end: r.get::<_, i64>(6)? as usize,
                        },
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !out.is_empty() || text.split_whitespace().nth(1).is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn chunk_cand(&self, chunk_id: &Id) -> Result<Option<Cand>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(
                "SELECT c.item_id, i.title, c.text, c.span_start, c.span_end FROM chunks c JOIN items i ON i.id = c.item_id
                 WHERE c.id = ?1 AND i.deleted = 0",
                params![chunk_id.0],
                |r| {
                    let text: String = r.get(2)?;
                    Ok(Cand {
                        item_id: Id(r.get(0)?),
                        title: r.get(1)?,
                        snippet: cut_snippet(&text),
                        start: r.get::<_, i64>(3)? as usize,
                        end: r.get::<_, i64>(4)? as usize,
                    })
                },
            )
            .optional()?)
    }

    /// Hybrid search. Text goes to FTS5 (unicode61 with diacritics removed, so `decisao`
    /// finds `decisão`), the optional vector to kNN; both ranked lists are fused with
    /// reciprocal rank fusion (k = 60). With [`Granularity::Item`], each item appears once with
    /// its best chunk's snippet and byte span.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Hit>> {
        if q.k == 0 {
            return Ok(Vec::new());
        }
        let pool = (q.k * 5).max(50);
        let mut fused: HashMap<Id, f32> = HashMap::new();
        let mut cands: HashMap<Id, Cand> = HashMap::new();
        for (rank, (cid, cand)) in self.fts_hits(&q.text, pool)?.into_iter().enumerate() {
            *fused.entry(cid.clone()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
            cands.entry(cid).or_insert(cand);
        }
        if let Some(v) = &q.vector {
            for (rank, h) in self
                .knn(&v.model_id, &v.values, pool)?
                .into_iter()
                .enumerate()
            {
                *fused.entry(h.chunk_id.clone()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
                if !cands.contains_key(&h.chunk_id)
                    && let Some(c) = self.chunk_cand(&h.chunk_id)?
                {
                    cands.insert(h.chunk_id, c);
                }
            }
        }
        let mut ranked: Vec<(Id, f32)> = fused
            .into_iter()
            .filter(|(id, _)| cands.contains_key(id))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut hits = Vec::new();
        let mut seen_items: std::collections::HashSet<Id> = Default::default();
        for (cid, score) in ranked {
            let c = cands.remove(&cid).expect("filtered");
            if q.granularity == Granularity::Item && !seen_items.insert(c.item_id.clone()) {
                continue;
            }
            hits.push(Hit {
                item_id: c.item_id,
                chunk_id: Some(cid),
                title: c.title,
                snippet: c.snippet,
                score,
                start: c.start,
                end: c.end,
            });
            if hits.len() == q.k {
                break;
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::fts_expr;

    #[test]
    fn fts_expr_quotes_words() {
        assert_eq!(
            fts_expr("decisão AND \"x\" -y", "AND").unwrap(),
            "\"decisão\" AND \"AND\" AND \"x\" AND \"y\""
        );
        assert!(fts_expr("  ...", "AND").is_none());
    }
}
