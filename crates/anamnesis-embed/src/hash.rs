//! Deterministic feature-hashing embedder.
//!
//! No model, no network, no state: word unigrams plus character 3- and
//! 4-grams (per word, with boundary markers), hashed with FNV-1a into a
//! fixed number of buckets with a sign bit (the "hashing trick"), sublinear
//! term frequency, then L2-normalised. Text is lowercased and common Latin
//! diacritics are folded, so "memória" and "memoria" share most features.
//!
//! It captures lexical overlap only (no synonyms, no cross-lingual meaning),
//! which is enough for tests, near-duplicate detection, and as an offline
//! fallback so the brain still forms regions without any model installed.

use std::collections::HashMap;

use anyhow::Result;

use crate::{Embedder, l2_normalize};

/// Default output dimension.
pub const DEFAULT_DIM: usize = 256;

const WORD_WEIGHT: f32 = 1.0;
const GRAM_WEIGHT: f32 = 0.4;

#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dim: usize,
    id: String,
}

impl HashEmbedder {
    /// `dim` must be at least 8; smaller values are raised to 8.
    pub fn new(dim: usize) -> Self {
        let dim = dim.max(8);
        Self {
            dim,
            id: format!("hash@{dim}"),
        }
    }

    /// Embed one text (never fails).
    pub fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut counts: HashMap<u64, f32> = HashMap::new();
        let folded = fold(text);
        let mut any = false;
        for word in folded
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            any = true;
            *counts.entry(fnv1a(b'w', word.as_bytes())).or_default() += WORD_WEIGHT;
            let padded: Vec<char> = std::iter::once('^')
                .chain(word.chars())
                .chain(std::iter::once('$'))
                .collect();
            for n in [3usize, 4] {
                if padded.len() < n {
                    continue;
                }
                for g in padded.windows(n) {
                    let s: String = g.iter().collect();
                    *counts
                        .entry(fnv1a(b'0' + n as u8, s.as_bytes()))
                        .or_default() += GRAM_WEIGHT;
                }
            }
        }
        if !any {
            // Empty or symbol-only text: hash the raw trimmed string so the
            // result is still a deterministic unit vector.
            *counts
                .entry(fnv1a(b'e', text.trim().as_bytes()))
                .or_default() += 1.0;
        }
        let mut v = vec![0.0f32; self.dim];
        for (h, tf) in counts {
            let bucket = (h % self.dim as u64) as usize;
            let sign = if h >> 63 == 1 { -1.0 } else { 1.0 };
            // Sublinear tf: repeated words matter, but not linearly.
            v[bucket] += sign * (1.0 + tf.ln().max(0.0)) * tf.min(1.0);
        }
        l2_normalize(&mut v);
        if v.iter().all(|x| *x == 0.0) {
            // All features cancelled out (vanishingly rare): fall back to a
            // basis vector so the output is still unit length.
            v[0] = 1.0;
        }
        v
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self::new(DEFAULT_DIM)
    }
}

impl Embedder for HashEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
    fn is_local(&self) -> bool {
        true
    }
}

/// FNV-1a 64-bit with a namespace byte, stable across platforms and Rust
/// versions (unlike `std::hash`).
fn fnv1a(ns: u8, bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in std::iter::once(&ns).chain(bytes) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Final avalanche so the low bits (bucket) and high bit (sign) are mixed.
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}

/// Lowercase and fold common Latin diacritics.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => 'u',
            'ý' | 'ÿ' => 'y',
            'ś' | 'š' => 's',
            'ź' | 'ż' | 'ž' => 'z',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dot;

    fn e(text: &str) -> Vec<f32> {
        HashEmbedder::default().embed_one(text)
    }

    #[test]
    fn unit_length_and_dimension() {
        for t in [
            "",
            "   ",
            "!!!",
            "a",
            "hello world",
            &"long text ".repeat(500),
            "記憶は再構成",
        ] {
            let v = e(t);
            assert_eq!(v.len(), DEFAULT_DIM);
            assert!((dot(&v, &v) - 1.0).abs() < 1e-5, "{t:?}");
        }
    }

    #[test]
    fn deterministic() {
        let a = e("is learning recollection?");
        let b = HashEmbedder::new(256).embed_one("is learning recollection?");
        assert_eq!(a, b);
        // Pin a few values so a change to the hashing scheme is noticed:
        // vectors stored under the old scheme would silently stop matching.
        // If you change the scheme on purpose, change the id too.
        for (i, x) in [
            (2usize, -0.131_590_35f32),
            (3, 0.131_590_35),
            (5, 0.131_590_35),
        ] {
            assert!((a[i] - x).abs() < 1e-6, "component {i}: {}", a[i]);
        }
    }

    #[test]
    fn similar_beats_dissimilar() {
        let base = e("the meeting about the garden project is moved to thursday");
        let near = e("the meeting about the garden project moved to thursday afternoon");
        let typo = e("the meting about the gardn project is moved to thursday");
        let far = e("quantum chromodynamics describes the strong interaction");
        let s_near = dot(&base, &near);
        let s_typo = dot(&base, &typo);
        let s_far = dot(&base, &far);
        assert!(s_near > 0.7, "near {s_near}");
        assert!(s_typo > 0.5, "typo {s_typo}");
        assert!(s_far < 0.25, "far {s_far}");
        assert!(s_near > s_far && s_typo > s_far);
    }

    #[test]
    fn diacritics_and_case_fold() {
        let a = e("A memória é uma reconstrução");
        let b = e("a memoria e uma reconstrucao");
        assert!(dot(&a, &b) > 0.99);
    }

    #[test]
    fn trait_batch() {
        let h = HashEmbedder::default();
        let out = h.embed(&["a".into(), "b".into(), "a".into()]).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], out[2]);
        assert_eq!(h.embed_query(&["a".into()]).unwrap()[0], out[0]);
        assert!(h.is_local());
        assert_eq!(h.id(), "hash@256");
    }
}
