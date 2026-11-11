//! OpenAI-compatible backend: `POST {base_url}/embeddings`.
//!
//! Works with OpenAI, OpenRouter, llama.cpp `llama-server --embeddings`,
//! LM Studio, vLLM, Text Embeddings Inference and others. `base_url` is the
//! API root including the version segment, e.g. `http://127.0.0.1:8080/v1`
//! or `https://api.openai.com/v1`.
//!
//! The API key is supplied by the caller at construction time, is sent only
//! as an `Authorization: Bearer` header, and is never logged, serialised or
//! included in errors.

use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::Embedder;
use crate::http::{HttpClient, HttpOptions, finish, is_loopback_url, join};
use crate::text::{Prompts, prepare};

pub struct OpenAiCompatibleEmbedder {
    base_url: String,
    model: String,
    api_key: Option<String>,
    prompts: Prompts,
    dimensions: Option<usize>,
    opts: HttpOptions,
    client: HttpClient,
    dim: AtomicUsize,
    id: String,
    local: bool,
}

impl std::fmt::Debug for OpenAiCompatibleEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleEmbedder")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct Req<'a> {
    model: &'a str,
    input: &'a [String],
    encoding_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Deserialize)]
struct Resp {
    data: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    embedding: Vec<f32>,
    #[serde(default)]
    index: Option<usize>,
}

impl OpenAiCompatibleEmbedder {
    /// `prompts: None` infers the conventional prompts from the model name.
    /// `dimensions` is sent as the `dimensions` request field (OpenAI
    /// text-embedding-3 and other Matryoshka models).
    pub fn new(
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        prompts: Option<Prompts>,
        dimensions: Option<usize>,
        opts: HttpOptions,
    ) -> Self {
        let base_url = base_url.trim_end_matches('/').to_owned();
        let client = HttpClient::new(&base_url, &opts);
        let host = base_url
            .split_once("://")
            .map_or(base_url.as_str(), |(_, r)| r);
        let host = host.split('/').next().unwrap_or(host);
        let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
        let id = match dimensions {
            Some(d) => format!("openai-compatible:{host}:{model}@{d}"),
            None => format!("openai-compatible:{host}:{model}"),
        };
        Self {
            local: is_loopback_url(&base_url),
            prompts: prompts.unwrap_or_else(|| Prompts::for_model(model)),
            model: model.to_owned(),
            api_key: api_key.filter(|k| !k.is_empty()),
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
        let url = join(&self.base_url, "/embeddings");
        let prepared = prepare(texts, prefix, self.opts.max_input_tokens);
        let mut out = Vec::with_capacity(texts.len());
        for chunk in prepared.chunks(self.opts.batch_size.max(1)) {
            let req = Req {
                model: &self.model,
                input: chunk,
                encoding_format: "float",
                dimensions: self.dimensions,
            };
            let resp: Resp = self
                .client
                .post_json(&url, &req, self.api_key.as_deref())
                .with_context(|| format!("embeddings with model {}", self.model))?;
            let mut items = resp.data;
            // The spec says `data` is ordered by `index`, but not every
            // server guarantees it.
            if items.iter().all(|i| i.index.is_some()) {
                items.sort_by_key(|i| i.index);
            }
            let known = self.dim.load(Ordering::Relaxed);
            let vecs = finish(
                items.into_iter().map(|i| i.embedding).collect(),
                chunk.len(),
                (known > 0).then_some(known),
            )?;
            if known == 0
                && let Some(v) = vecs.first()
            {
                self.dim.store(v.len(), Ordering::Relaxed);
            }
            out.extend(vecs);
        }
        Ok(out)
    }
}

impl Embedder for OpenAiCompatibleEmbedder {
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
