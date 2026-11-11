//! Text normalisation, content hashing and chunking.
//!
//! Chunks are paragraph- and heading-aware slices of an item body of roughly
//! [`CHUNK_MIN`]..[`CHUNK_MAX`] bytes (aiming at [`CHUNK_TARGET`]), with exact byte spans into
//! the body so that hits can cite their source. Chunk ids are derived deterministically from
//! `(item id, ord, text)` so replaying the event log yields the same chunks, and embeddings
//! (keyed by the chunk text hash) survive rebuilds and unchanged chunks across edits.

use crate::canon::sha256_hex;
use crate::model::{Chunk, Id};

/// Below this a chunk is not closed at a heading.
pub const CHUNK_MIN: usize = 400;
/// Chunks are closed once they reach this size.
pub const CHUNK_TARGET: usize = 1000;
/// Hard upper bound (except for a single unbreakable run of text without whitespace).
pub const CHUNK_MAX: usize = 1200;

/// Normalise a body for content hashing: unify line endings, strip trailing whitespace on
/// each line, collapse runs of blank lines and trim the whole.
pub fn normalize_body(body: &str) -> String {
    let unified = body.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len());
    let mut blank_run = 0;
    for line in unified.split('\n') {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// sha256 hex of the normalised body: the item's `content_hash`.
pub fn content_hash(body: &str) -> String {
    sha256_hex(normalize_body(body).as_bytes())
}

/// sha256 hex of a chunk's exact text: the key under which embeddings are stored.
pub fn text_hash(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

/// Deterministic chunk id: a ULID-shaped string built from a hash of item id, ord and text.
pub fn chunk_id(item_id: &Id, ord: u32, text_hash: &str) -> Id {
    let digest = sha256_hex(format!("{}\u{0}{}\u{0}{}", item_id, ord, text_hash).as_bytes());
    let bytes = hex::decode(&digest[..32]).expect("hex");
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes);
    Id(ulid::Ulid::from_bytes(arr).to_string())
}

#[derive(Debug, Clone, Copy)]
struct Block {
    start: usize,
    end: usize,
    heading: bool,
}

fn is_heading(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return false;
    }
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes)
        && t[hashes..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

/// Split into blocks: runs of non-blank lines, with headings starting a new block.
fn blocks(body: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;
    let mut pos = 0;
    for raw in body.split_inclusive('\n') {
        let line_start = pos;
        pos += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            if let Some(b) = cur.take() {
                out.push(b);
            }
            continue;
        }
        let lead = line.len() - line.trim_start().len();
        let s = line_start + lead;
        let e = line_start + line.trim_end().len();
        let heading = is_heading(line);
        match cur.as_mut() {
            Some(b) if !heading => b.end = e,
            _ => {
                if let Some(b) = cur.take() {
                    out.push(b);
                }
                cur = Some(Block {
                    start: s,
                    end: e,
                    heading,
                });
            }
        }
    }
    if let Some(b) = cur {
        out.push(b);
    }
    out
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Split a block longer than CHUNK_MAX at sentence ends or whitespace.
fn split_long(body: &str, b: Block) -> Vec<Block> {
    let mut out = Vec::new();
    let mut s = b.start;
    while b.end - s > CHUNK_MAX {
        let lo = floor_boundary(body, s + CHUNK_TARGET);
        let hi = floor_boundary(body, s + CHUNK_MAX);
        let window = &body[lo..hi];
        let cut = window
            .rfind(['\n'])
            .or_else(|| {
                window
                    .rfind(". ")
                    .or(window.rfind("? "))
                    .or(window.rfind("! "))
                    .map(|i| i + 1)
            })
            .map(|i| lo + i)
            .or_else(|| {
                body[s..hi]
                    .rfind(char::is_whitespace)
                    .map(|i| s + i)
                    .filter(|&i| i > s)
            })
            .unwrap_or(hi);
        let piece_end = s + body[s..cut].trim_end().len();
        if piece_end > s {
            out.push(Block {
                start: s,
                end: piece_end,
                heading: out.is_empty() && b.heading,
            });
        }
        let rest = &body[cut..b.end];
        s = cut + (rest.len() - rest.trim_start().len());
        if cut == s && rest.is_empty() {
            break;
        }
    }
    if b.end > s {
        out.push(Block {
            start: s,
            end: b.end,
            heading: out.is_empty() && b.heading,
        });
    }
    out
}

/// Byte spans `(start, end)` of the chunks of `body`. Empty bodies yield no spans.
pub fn chunk_spans(body: &str) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut cur: Option<(usize, usize)> = None;
    for b in blocks(body).into_iter().flat_map(|b| split_long(body, b)) {
        cur = match cur {
            None => Some((b.start, b.end)),
            Some((s, e)) => {
                let cur_len = e - s;
                let merged = b.end - s;
                if (b.heading && cur_len >= CHUNK_MIN)
                    || cur_len >= CHUNK_TARGET
                    || merged > CHUNK_MAX
                {
                    spans.push((s, e));
                    Some((b.start, b.end))
                } else {
                    Some((s, b.end))
                }
            }
        };
    }
    if let Some((s, e)) = cur {
        // fold a tiny tail into the previous chunk when it fits
        if let Some(last) = spans.last_mut()
            && e - s < CHUNK_MIN / 2
            && e - last.0 <= CHUNK_MAX
        {
            last.1 = e;
        } else {
            spans.push((s, e));
        }
    }
    spans
}

/// Chunk an item body. An empty body yields one empty chunk so the title stays searchable.
pub fn chunk_item(item_id: &Id, body: &str) -> Vec<Chunk> {
    let spans = chunk_spans(body);
    if spans.is_empty() {
        return vec![Chunk {
            id: chunk_id(item_id, 0, &text_hash("")),
            item_id: item_id.clone(),
            ord: 0,
            text: String::new(),
            start: 0,
            end: 0,
        }];
    }
    spans
        .into_iter()
        .enumerate()
        .map(|(i, (s, e))| {
            let text = body[s..e].to_string();
            Chunk {
                id: chunk_id(item_id, i as u32, &text_hash(&text)),
                item_id: item_id.clone(),
                ord: i as u32,
                text,
                start: s,
                end: e,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_ignores_whitespace_noise() {
        assert_eq!(content_hash("a  \r\n\r\n\r\nb\n"), content_hash("a\n\nb"));
        assert_ne!(content_hash("a b"), content_hash("ab"));
    }

    #[test]
    fn spans_are_exact_and_bounded() {
        let para = "Uma decisão importante foi tomada aqui. ".repeat(12); // ~490 bytes
        let body = format!(
            "# Title\n\n{para}\n\n{para}\n\n## Second\n\n{para}\n\n{}",
            "x ".repeat(1500)
        );
        let spans = chunk_spans(&body);
        assert!(spans.len() >= 3, "{spans:?}");
        let mut prev_end = 0;
        for &(s, e) in &spans {
            assert!(s >= prev_end && e > s);
            assert!(body.is_char_boundary(s) && body.is_char_boundary(e));
            assert!(e - s <= CHUNK_MAX, "{}", e - s);
            assert_eq!(body[s..e].trim(), &body[s..e]);
            prev_end = e;
        }
        // heading starts a chunk once the current one is big enough
        assert!(
            spans
                .iter()
                .any(|&(s, _)| body[s..].starts_with("## Second"))
        );
    }

    #[test]
    fn small_note_is_one_chunk_and_ids_are_deterministic() {
        let id = Id("01J00000000000000000000000".into());
        let a = chunk_item(&id, "hello\n\nworld");
        let b = chunk_item(&id, "hello\n\nworld");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].text, "hello\n\nworld");
        assert_eq!(a, b);
        assert_eq!(chunk_item(&id, "").len(), 1);
    }
}
