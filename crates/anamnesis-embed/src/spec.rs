//! Serialisable backend configuration and the factory that builds it.
//!
//! A spec is safe to store in user settings: it never contains an API key.
//! Keys are passed separately to [`build_with_secret`] by the caller (which
//! keeps them in the OS keychain or the harness's secret store).

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::http::{HttpOptions, is_loopback_url};
use crate::text::Prompts;
use crate::{Embedder, HashEmbedder, Matryoshka, OllamaEmbedder, OpenAiCompatibleEmbedder};

/// Which embedding backend to use, and how.
///
/// JSON form, e.g. `{"backend":"ollama","model":"qwen3-embedding:0.6b"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum EmbedderSpec {
    /// Deterministic feature hashing (no model). Offline fallback, tests.
    Hash {
        #[serde(default = "default_hash_dim")]
        dim: usize,
    },
    /// On-device ONNX model via fastembed (needs the `local` feature).
    Local {
        /// Short name from `local::CATALOGUE` or its Hugging Face id.
        #[serde(default = "default_local_model")]
        model: String,
        #[serde(default = "default_local_batch")]
        batch_size: usize,
        /// Token budget per input; 0 = model maximum.
        #[serde(default)]
        max_tokens: usize,
        /// ONNX Runtime threads; omitted = all cores.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        threads: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompts: Option<Prompts>,
        /// Client-side Matryoshka truncation (only for models trained for it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncate_dim: Option<usize>,
    },
    /// A local Ollama server.
    Ollama {
        /// Defaults to `http://127.0.0.1:11434`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompts: Option<Prompts>,
        /// Server-side output size (`dimensions` request field).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dimensions: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncate_dim: Option<usize>,
        #[serde(flatten)]
        http: HttpOptions,
    },
    /// Any OpenAI-compatible `/v1/embeddings` server.
    OpenaiCompatible {
        /// API root including the version, e.g. `http://127.0.0.1:8080/v1`.
        base_url: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompts: Option<Prompts>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dimensions: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncate_dim: Option<usize>,
        #[serde(flatten)]
        http: HttpOptions,
    },
}

fn default_hash_dim() -> usize {
    crate::hash::DEFAULT_DIM
}
fn default_local_model() -> String {
    "multilingual-e5-small".into()
}
fn default_local_batch() -> usize {
    16
}

impl Default for EmbedderSpec {
    /// The local default model when built with the `local` feature,
    /// otherwise the hashing embedder.
    fn default() -> Self {
        if cfg!(feature = "local") {
            Self::local(default_local_model())
        } else {
            Self::hash()
        }
    }
}

impl EmbedderSpec {
    pub fn hash() -> Self {
        Self::Hash {
            dim: default_hash_dim(),
        }
    }

    pub fn local(model: impl Into<String>) -> Self {
        Self::Local {
            model: model.into(),
            batch_size: default_local_batch(),
            max_tokens: 0,
            threads: None,
            prompts: None,
            truncate_dim: None,
        }
    }

    pub fn ollama(model: impl Into<String>) -> Self {
        Self::Ollama {
            base_url: None,
            model: model.into(),
            prompts: None,
            dimensions: None,
            truncate_dim: None,
            http: HttpOptions::default(),
        }
    }

    pub fn openai_compatible(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self::OpenaiCompatible {
            base_url: base_url.into(),
            model: model.into(),
            prompts: None,
            dimensions: None,
            truncate_dim: None,
            http: HttpOptions::default(),
        }
    }

    /// Whether text stays on this machine with this spec (known before
    /// building, so the egress gate can decide up front).
    pub fn is_local(&self) -> bool {
        match self {
            Self::Hash { .. } | Self::Local { .. } => true,
            Self::Ollama { base_url, .. } => is_loopback_url(
                base_url
                    .as_deref()
                    .unwrap_or(crate::ollama::DEFAULT_BASE_URL),
            ),
            Self::OpenaiCompatible { base_url, .. } => is_loopback_url(base_url),
        }
    }

    /// Short backend name: `hash`, `local`, `ollama`, `openai_compatible`.
    pub fn backend(&self) -> &'static str {
        match self {
            Self::Hash { .. } => "hash",
            Self::Local { .. } => "local",
            Self::Ollama { .. } => "ollama",
            Self::OpenaiCompatible { .. } => "openai_compatible",
        }
    }
}

/// Default model cache: `<data dir>/anamnesis/models` (e.g.
/// `~/.local/share/anamnesis/models` on Linux,
/// `~/Library/Application Support/anamnesis/models` on macOS).
pub fn default_cache_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".anamnesis"))
        .join("anamnesis")
        .join("models")
}

/// Build an embedder. `cache_dir` is where local model weights live.
pub fn build(spec: &EmbedderSpec, cache_dir: &Path) -> Result<Box<dyn Embedder>> {
    build_with_secret(spec, cache_dir, None)
}

/// Build an embedder, passing an API key for backends that need one
/// (`openai_compatible`). The key is ignored by the other backends.
pub fn build_with_secret(
    spec: &EmbedderSpec,
    cache_dir: &Path,
    api_key: Option<&str>,
) -> Result<Box<dyn Embedder>> {
    let (inner, truncate_dim): (Box<dyn Embedder>, Option<usize>) = match spec {
        EmbedderSpec::Hash { dim } => (Box::new(HashEmbedder::new(*dim)), None),
        EmbedderSpec::Local {
            model,
            batch_size,
            max_tokens,
            threads,
            prompts,
            truncate_dim,
        } => (
            build_local(
                model,
                *batch_size,
                *max_tokens,
                *threads,
                prompts.clone(),
                cache_dir,
            )?,
            *truncate_dim,
        ),
        EmbedderSpec::Ollama {
            base_url,
            model,
            prompts,
            dimensions,
            truncate_dim,
            http,
        } => (
            Box::new(OllamaEmbedder::new(
                base_url.as_deref(),
                model,
                prompts.clone(),
                *dimensions,
                http.clone(),
            )),
            *truncate_dim,
        ),
        EmbedderSpec::OpenaiCompatible {
            base_url,
            model,
            prompts,
            dimensions,
            truncate_dim,
            http,
        } => (
            Box::new(OpenAiCompatibleEmbedder::new(
                base_url,
                model,
                api_key.map(str::to_owned),
                prompts.clone(),
                *dimensions,
                http.clone(),
            )),
            *truncate_dim,
        ),
    };
    Ok(match truncate_dim {
        Some(d) if d > 0 => Box::new(Matryoshka::new(inner, d)),
        _ => inner,
    })
}

#[cfg(feature = "local")]
fn build_local(
    model: &str,
    batch_size: usize,
    max_tokens: usize,
    threads: Option<usize>,
    prompts: Option<Prompts>,
    cache_dir: &Path,
) -> Result<Box<dyn Embedder>> {
    let opts = crate::local::LocalOptions {
        batch_size,
        max_tokens,
        threads,
        prompts,
        show_download_progress: false,
    };
    Ok(Box::new(crate::LocalEmbedder::new(model, cache_dir, opts)?))
}

#[cfg(not(feature = "local"))]
fn build_local(
    model: &str,
    _batch_size: usize,
    _max_tokens: usize,
    _threads: Option<usize>,
    _prompts: Option<Prompts>,
    _cache_dir: &Path,
) -> Result<Box<dyn Embedder>> {
    anyhow::bail!(
        "local embedding model {model:?} needs anamnesis-embed built with the `local` feature"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RetryPolicy;

    fn roundtrip(spec: &EmbedderSpec) {
        let json = serde_json::to_string(spec).unwrap();
        let back: EmbedderSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, spec, "{json}");
    }

    #[test]
    fn serde_roundtrip_all_variants() {
        roundtrip(&EmbedderSpec::hash());
        roundtrip(&EmbedderSpec::local("multilingual-e5-small"));
        roundtrip(&EmbedderSpec::Local {
            model: "embeddinggemma-300m-q".into(),
            batch_size: 8,
            max_tokens: 1024,
            threads: Some(2),
            prompts: Some(Prompts {
                query: "q: ".into(),
                document: "d: ".into(),
            }),
            truncate_dim: Some(256),
        });
        roundtrip(&EmbedderSpec::ollama("qwen3-embedding:0.6b"));
        roundtrip(&EmbedderSpec::Ollama {
            base_url: Some("http://10.0.0.2:11434".into()),
            model: "qwen3-embedding:0.6b".into(),
            prompts: Some(Prompts::qwen3("find notes")),
            dimensions: Some(512),
            truncate_dim: None,
            http: HttpOptions {
                batch_size: 4,
                max_input_tokens: 2000,
                timeout_secs: 5,
                retry: RetryPolicy {
                    max_retries: 1,
                    base_delay_ms: 10,
                    max_delay_ms: 20,
                },
            },
        });
        roundtrip(&EmbedderSpec::openai_compatible(
            "https://api.openai.com/v1",
            "text-embedding-3-small",
        ));
    }

    #[test]
    fn json_shape_and_defaults() {
        let s: EmbedderSpec =
            serde_json::from_str(r#"{"backend":"ollama","model":"bge-m3"}"#).unwrap();
        assert_eq!(s, EmbedderSpec::ollama("bge-m3"));
        let s: EmbedderSpec = serde_json::from_str(r#"{"backend":"local"}"#).unwrap();
        assert_eq!(s, EmbedderSpec::local("multilingual-e5-small"));
        let s: EmbedderSpec = serde_json::from_str(r#"{"backend":"hash"}"#).unwrap();
        assert_eq!(s, EmbedderSpec::Hash { dim: 256 });
        let v = serde_json::to_value(EmbedderSpec::openai_compatible("http://x/v1", "m")).unwrap();
        assert_eq!(v["backend"], "openai_compatible");
        assert_eq!(v["batch_size"], 32);
        assert!(v.get("api_key").is_none());
    }

    #[test]
    fn locality() {
        assert!(EmbedderSpec::hash().is_local());
        assert!(EmbedderSpec::local("x").is_local());
        assert!(EmbedderSpec::ollama("m").is_local());
        assert!(EmbedderSpec::openai_compatible("http://127.0.0.1:8080/v1", "m").is_local());
        assert!(!EmbedderSpec::openai_compatible("https://openrouter.ai/api/v1", "m").is_local());
    }

    #[test]
    fn build_hash_and_http_without_network() {
        let dir = std::env::temp_dir();
        let h = build(&EmbedderSpec::hash(), &dir).unwrap();
        assert_eq!((h.id(), h.dim(), h.is_local()), ("hash@256", 256, true));
        // Building HTTP backends does not touch the network.
        let o = build(&EmbedderSpec::ollama("qwen3-embedding:0.6b"), &dir).unwrap();
        assert!(o.is_local());
        assert_eq!(o.id(), "ollama:qwen3-embedding:0.6b");
        let r = build_with_secret(
            &EmbedderSpec::openai_compatible("https://api.openai.com/v1", "text-embedding-3-small"),
            &dir,
            Some("sk-test"),
        )
        .unwrap();
        assert!(!r.is_local());
        assert_eq!(
            r.id(),
            "openai-compatible:api.openai.com:text-embedding-3-small"
        );
        let m = build(&EmbedderSpec::Hash { dim: 128 }, &dir).unwrap();
        assert_eq!(m.dim(), 128);
    }

    #[test]
    fn truncate_dim_wraps() {
        let spec = EmbedderSpec::Ollama {
            base_url: None,
            model: "qwen3-embedding:0.6b".into(),
            prompts: None,
            dimensions: None,
            truncate_dim: Some(256),
            http: HttpOptions::default(),
        };
        let e = build(&spec, &std::env::temp_dir()).unwrap();
        assert_eq!(e.dim(), 256);
        assert_eq!(e.id(), "ollama:qwen3-embedding:0.6b@256");
    }

    #[cfg(not(feature = "local"))]
    #[test]
    fn local_without_feature_errors() {
        assert!(
            build(
                &EmbedderSpec::local("multilingual-e5-small"),
                &std::env::temp_dir()
            )
            .is_err()
        );
    }
}
