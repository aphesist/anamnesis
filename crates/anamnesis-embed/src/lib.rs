//! anamnesis-embed: embedding backends behind one small, synchronous trait.
//!
//! Backends:
//! - [`local`] (cargo feature `local`, on by default): on-device ONNX
//!   inference through `fastembed`. Weights download on first use into a
//!   cache directory (see [`default_cache_dir`]).
//! - [`ollama`]: a local Ollama server's `/api/embed`.
//! - [`openai`]: any OpenAI-compatible `/v1/embeddings` endpoint (llama.cpp
//!   server, LM Studio, vLLM, OpenAI, OpenRouter, ...).
//! - [`hash`]: deterministic feature hashing; offline fallback and tests.
//!
//! Construct a backend from a serialisable [`EmbedderSpec`] with [`build`].
//! All vectors returned by [`Embedder::embed`] and [`Embedder::embed_query`]
//! are L2-normalised, so cosine similarity is a dot product.
//!
//! The trait is synchronous; async callers should use `spawn_blocking`.

pub mod hash;
mod http;
#[cfg(feature = "local")]
pub mod local;
pub mod ollama;
pub mod openai;
pub mod spec;
pub mod text;

pub use hash::HashEmbedder;
pub use http::{HttpOptions, RetryPolicy, is_loopback_url};
pub use ollama::OllamaEmbedder;
pub use openai::OpenAiCompatibleEmbedder;
pub use spec::{EmbedderSpec, build, build_with_secret, default_cache_dir};
pub use text::{Prompts, approx_tokens, truncate_to_tokens};

#[cfg(feature = "local")]
pub use local::LocalEmbedder;

use anyhow::Result;

/// An embedding backend.
///
/// `embed` is for documents (the things stored in the brain); `embed_query`
/// is for search queries. Symmetric models use the same path for both;
/// instruction-tuned models (E5, Qwen3-Embedding, EmbeddingGemma, Nomic)
/// prefix queries and documents differently.
pub trait Embedder: Send + Sync {
    /// Stable identifier of backend + model + output dimension, e.g.
    /// `local:multilingual-e5-small@384`. Vectors from embedders with
    /// different ids are not comparable; store the id next to each vector.
    fn id(&self) -> &str;

    /// Output dimension. HTTP backends with no configured dimension learn it
    /// from the server on first use (and probe once if asked before that);
    /// they return 0 if the server cannot be reached.
    fn dim(&self) -> usize;

    /// Embed documents. One L2-normalised vector per input, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Embed search queries. Defaults to [`Embedder::embed`].
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed(texts)
    }

    /// True when text never leaves this machine (on-device inference, or an
    /// HTTP server on a loopback address). The egress gate uses this to
    /// decide whether an `Egress` event must be logged. Defaults to `false`
    /// so an implementation that forgets to say is treated as remote.
    fn is_local(&self) -> bool {
        false
    }
}

impl<E: Embedder + ?Sized> Embedder for Box<E> {
    fn id(&self) -> &str {
        (**self).id()
    }
    fn dim(&self) -> usize {
        (**self).dim()
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        (**self).embed(texts)
    }
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        (**self).embed_query(texts)
    }
    fn is_local(&self) -> bool {
        (**self).is_local()
    }
}

/// Embed a single query string.
pub fn embed_one_query(e: &dyn Embedder, text: &str) -> Result<Vec<f32>> {
    e.embed_query(&[text.to_owned()])?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("embedder returned no vector"))
}

/// L2-normalise in place. A zero vector is left as is.
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 && norm.is_finite() {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Dot product; equals cosine similarity for L2-normalised vectors.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Cosine similarity for arbitrary (not necessarily normalised) vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (na, nb) = (dot(a, a).sqrt(), dot(b, b).sqrt());
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot(a, b) / (na * nb)
    }
}

/// Matryoshka truncation: keep the first `dim` components and renormalise.
///
/// Only meaningful for models trained with Matryoshka representation
/// learning (EmbeddingGemma: 768/512/256/128; Qwen3-Embedding: any size
/// from 32 up; OpenAI text-embedding-3; nomic-embed-text-v1.5). For other
/// models, truncation degrades quality sharply. `dim >= len` only
/// renormalises.
pub fn matryoshka_truncate(v: &[f32], dim: usize) -> Vec<f32> {
    let mut out = v[..dim.min(v.len())].to_vec();
    l2_normalize(&mut out);
    out
}

/// Wraps another embedder and applies [`matryoshka_truncate`] to every
/// output vector.
pub struct Matryoshka<E> {
    inner: E,
    dim: usize,
    id: String,
}

impl<E: Embedder> Matryoshka<E> {
    pub fn new(inner: E, dim: usize) -> Self {
        let base = inner.id();
        let base = base.rsplit_once('@').map_or(base, |(b, _)| b);
        let id = format!("{base}@{dim}");
        Self { inner, dim, id }
    }
}

impl<E: Embedder> Embedder for Matryoshka<E> {
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .inner
            .embed(texts)?
            .iter()
            .map(|v| matryoshka_truncate(v, self.dim))
            .collect())
    }
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .inner
            .embed_query(texts)?
            .iter()
            .map(|v| matryoshka_truncate(v, self.dim))
            .collect())
    }
    fn is_local(&self) -> bool {
        self.inner.is_local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_cosine() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        let mut z = vec![0.0; 3];
        l2_normalize(&mut z);
        assert_eq!(z, vec![0.0; 3]);
        assert!((cosine(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[0.0], &[1.0]), 0.0);
    }

    #[test]
    fn matryoshka_truncates_and_renormalises() {
        let v = vec![0.5, 0.5, 0.5, 0.5];
        let t = matryoshka_truncate(&v, 2);
        assert_eq!(t.len(), 2);
        assert!((dot(&t, &t) - 1.0).abs() < 1e-6);
        assert_eq!(matryoshka_truncate(&v, 10).len(), 4);
    }

    #[test]
    fn matryoshka_wrapper() {
        let e = Matryoshka::new(HashEmbedder::new(256), 64);
        assert_eq!(e.dim(), 64);
        assert_eq!(e.id(), "hash@64");
        let v = e.embed(&["hello world".into()]).unwrap();
        assert_eq!(v[0].len(), 64);
        assert!((dot(&v[0], &v[0]) - 1.0).abs() < 1e-5);
        assert!(e.is_local());
    }
}
