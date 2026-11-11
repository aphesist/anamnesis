//! Ollama backend: `POST {base_url}/api/embed`.
//!
//! Request `{ model, input: [..], truncate: true, dimensions? }`, response
//! `{ embeddings: [[..]] }`. Good local models: `qwen3-embedding:0.6b`
//! (multilingual, Matryoshka, instruction-aware), `embeddinggemma`,
//! `bge-m3`, `nomic-embed-text`.

use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::http::{HttpClient, HttpOptions, finish, is_loopback_url, join};
use crate::text::{Prompts, prepare};
use crate::Embedder;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    prompts: Prompts,
    dimensions: Option<usize>,
    opts: HttpOptions,
    client: HttpClient,
    dim: AtomicUsize,
    id: String,
    local: bool,
}

#[derive(Serialize)]
struct Req<'a> {
    model: &'a str,
    input: &'a [String],
    truncate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Deserialize)]
struct Resp {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedder {
    /// `prompts: None` infers the conventional prompts from the model name.
    /// `dimensions` asks the server for Matryoshka-truncated output (needs a
    /// model that supports it, e.g. qwen3-embedding).
    pub fn new(
        base_url: Option<&str>,
        model: &str,
        prompts: Option<Prompts>,
        dimensions: Option<usize>,
        opts: HttpOptions,
    ) -> Self {
        let base_url = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/').to_owned();
        let client = HttpClient::new(&base_url, &opts);
        let id = match dimensions {
            Some(d) => format!("ollama:{model}@{d}"),
            None => format!("ollama:{model}"),
        };
        Self {
            local: is_loopback_url(&base_url),
            prompts: prompts.unwrap_or_else(|| Prompts::for_model(model)),
            model: model.to_owned(),
            dim: AtomicUsize::new(dimensions.unwrap_or(0)),
            base_url,
            dimensions,
            opts,
            client,
            id,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn run(&self, texts: &[String], prefix: &str) -> Result<Vec<Vec<f32>>> {
        let url = join(&self.base_url, "/api/embed");
        let prepared = prepare(texts, prefix, self.opts.max_input_tokens);
        let mut out = Vec::with_capacity(texts.len());
        for chunk in prepared.chunks(self.opts.batch_size.max(1)) {
            let req = Req { model: &self.model, input: chunk, truncate: true, dimensions: self.dimensions };
            let resp: Resp = self
                .client
                .post_json(&url, &req, None)
                .with_context(|| format!("ollama embed with model {}", self.model))?;
            let known = self.dim.load(Ordering::Relaxed);
            let vecs = finish(resp.embeddings, chunk.len(), (known > 0).then_some(known))?;
            if known == 0 {
                if let Some(v) = vecs.first() {
                    self.dim.store(v.len(), Ordering::Relaxed);
                }
            }
            out.extend(vecs);
        }
        Ok(out)
    }
}

impl Embedder for OllamaEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        let d = self.dim.load(Ordering::Relaxed);
        if d > 0 {
            return d;
        }
        match self.run(&["dimension probe".to_owned()], "") {
            Ok(v) => v.first().map_or(0, Vec::len),
            Err(e) => {
                tracing::warn!(error = %e, "could not determine embedding dimension");
                0
            }
        }
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.run(texts, &self.prompts.document)
    }
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.run(texts, &self.prompts.query)
    }
    fn is_local(&self) -> bool {
        self.local
    }
}
