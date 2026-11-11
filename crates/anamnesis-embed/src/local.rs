//! On-device embeddings through `fastembed` (ONNX Runtime). No Python, no
//! server. Weights download from Hugging Face on first use into the cache
//! directory passed to [`LocalEmbedder::new`], then load from disk.
//!
//! ## default model: `multilingual-e5-small`
//!
//! Portuguese and English both matter, so the default must be multilingual.
//! Of the models fastembed runs, `intfloat/multilingual-e5-small` is the best
//! balance for a CPU-only, first-run default:
//!
//! - trained for retrieval on ~100 languages (strong on pt + en), with
//!   `query:` / `passage:` prefixes for asymmetric search;
//! - 118M parameters, 384 dims, 512-token context: fast on a laptop CPU and
//!   small vectors to store;
//! - MIT licence, so an Apache-2.0 app can fetch it without extra terms.
//!
//! Alternatives in [`CATALOGUE`]:
//!
//! - `embeddinggemma-300m-q` — clearly stronger multilingual retrieval
//!   (MMTEB), 768 dims with Matryoshka 512/256/128, 2048-token context;
//!   ~3x slower on CPU and distributed under the Gemma terms of use, which
//!   the user should accept knowingly. The best "quality" pick.
//! - `multilingual-e5-base` / `-large` — same family, better, 2-6x slower.
//! - `bge-m3` — excellent multilingual, 8192-token context, but 568M params
//!   and 1024 dims: heavy for CPU.
//! - `paraphrase-multilingual-minilm-l12-v2-q` — tiny and very fast, but a
//!   paraphrase (not retrieval) model; weaker for search.
//! - Qwen3-Embedding-0.6B is not available through fastembed's ONNX path
//!   (only its candle backend); use it via the Ollama backend.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde::Serialize;

use crate::text::{Prompts, prepare};
use crate::{Embedder, l2_normalize};

/// Default local model name.
pub const DEFAULT_MODEL: &str = "multilingual-e5-small";

/// A curated local model.
#[derive(Debug, Clone, Serialize)]
pub struct LocalModel {
    /// Short name used in specs and ids.
    pub name: &'static str,
    /// Hugging Face repository the weights come from.
    pub source: &'static str,
    pub dim: usize,
    /// Context length in tokens.
    pub max_tokens: usize,
    /// Approximate download size in MB.
    pub download_mb: usize,
    pub multilingual: bool,
    /// Matryoshka sizes the model was trained for (empty if none).
    pub matryoshka: &'static [usize],
    pub license: &'static str,
    pub note: &'static str,
    #[serde(skip)]
    model: fn() -> EmbeddingModel,
}

/// Curated models, default first.
pub const CATALOGUE: &[LocalModel] = &[
    LocalModel {
        name: "multilingual-e5-small",
        source: "intfloat/multilingual-e5-small",
        dim: 384,
        max_tokens: 512,
        download_mb: 470,
        multilingual: true,
        matryoshka: &[],
        license: "MIT",
        note: "default: multilingual retrieval, fast on cpu",
        model: || EmbeddingModel::MultilingualE5Small,
    },
    LocalModel {
        name: "embeddinggemma-300m-q",
        source: "onnx-community/embeddinggemma-300m-ONNX (int8)",
        dim: 768,
        max_tokens: 2048,
        download_mb: 310,
        multilingual: true,
        matryoshka: &[768, 512, 256, 128],
        license: "Gemma terms of use",
        note: "best multilingual quality here; slower on cpu",
        model: || EmbeddingModel::EmbeddingGemma300MQ,
    },
    LocalModel {
        name: "embeddinggemma-300m",
        source: "onnx-community/embeddinggemma-300m-ONNX",
        dim: 768,
        max_tokens: 2048,
        download_mb: 1200,
        multilingual: true,
        matryoshka: &[768, 512, 256, 128],
        license: "Gemma terms of use",
        note: "full precision embeddinggemma",
        model: || EmbeddingModel::EmbeddingGemma300M,
    },
    LocalModel {
        name: "multilingual-e5-base",
        source: "intfloat/multilingual-e5-base",
        dim: 768,
        max_tokens: 512,
        download_mb: 1110,
        multilingual: true,
        matryoshka: &[],
        license: "MIT",
        note: "better than small, ~3x slower",
        model: || EmbeddingModel::MultilingualE5Base,
    },
    LocalModel {
        name: "multilingual-e5-large",
        source: "intfloat/multilingual-e5-large",
        dim: 1024,
        max_tokens: 512,
        download_mb: 2240,
        multilingual: true,
        matryoshka: &[],
        license: "MIT",
        note: "strong, heavy on cpu",
        model: || EmbeddingModel::MultilingualE5Large,
    },
    LocalModel {
        name: "bge-m3",
        source: "BAAI/bge-m3",
        dim: 1024,
        max_tokens: 8192,
        download_mb: 2270,
        multilingual: true,
        matryoshka: &[],
        license: "MIT",
        note: "long context multilingual, heavy on cpu",
        model: || EmbeddingModel::BGEM3,
    },
    LocalModel {
        name: "paraphrase-multilingual-minilm-l12-v2-q",
        source: "Qdrant/paraphrase-multilingual-MiniLM-L12-v2-onnx-Q",
        dim: 384,
        max_tokens: 128,
        download_mb: 120,
        multilingual: true,
        matryoshka: &[],
        license: "Apache-2.0",
        note: "tiny and fast; paraphrase model, weaker for search",
        model: || EmbeddingModel::ParaphraseMLMiniLML12V2Q,
    },
    LocalModel {
        name: "paraphrase-multilingual-mpnet-base-v2",
        source: "sentence-transformers/paraphrase-multilingual-mpnet-base-v2",
        dim: 768,
        max_tokens: 128,
        download_mb: 1110,
        multilingual: true,
        matryoshka: &[],
        license: "Apache-2.0",
        note: "paraphrase model, short context",
        model: || EmbeddingModel::ParaphraseMLMpnetBaseV2,
    },
    LocalModel {
        name: "nomic-embed-text-v1.5",
        source: "nomic-ai/nomic-embed-text-v1.5",
        dim: 768,
        max_tokens: 8192,
        download_mb: 550,
        multilingual: false,
        matryoshka: &[768, 512, 256, 128, 64],
        license: "Apache-2.0",
        note: "english only, long context",
        model: || EmbeddingModel::NomicEmbedTextV15,
    },
    LocalModel {
        name: "bge-small-en-v1.5",
        source: "BAAI/bge-small-en-v1.5",
        dim: 384,
        max_tokens: 512,
        download_mb: 130,
        multilingual: false,
        matryoshka: &[],
        license: "MIT",
        note: "english only, small and fast",
        model: || EmbeddingModel::BGESmallENV15,
    },
];

/// Look up a curated model by short name (case-insensitive) or by its
/// Hugging Face source id.
pub fn find_model(name: &str) -> Option<&'static LocalModel> {
    let n = name.trim().to_ascii_lowercase();
    CATALOGUE
        .iter()
        .find(|m| m.name == n)
        .or_else(|| CATALOGUE.iter().find(|m| m.source.to_ascii_lowercase() == n))
}

/// Options for [`LocalEmbedder`].
#[derive(Debug, Clone)]
pub struct LocalOptions {
    /// Texts per ONNX run.
    pub batch_size: usize,
    /// Token budget per input (capped at the model's context). 0 = model max.
    pub max_tokens: usize,
    /// ONNX Runtime intra-op threads. `None` uses all cores.
    pub threads: Option<usize>,
    /// Override the model's conventional prompts.
    pub prompts: Option<Prompts>,
    pub show_download_progress: bool,
}

impl Default for LocalOptions {
    fn default() -> Self {
        Self { batch_size: 16, max_tokens: 0, threads: None, prompts: None, show_download_progress: false }
    }
}

pub struct LocalEmbedder {
    model: Mutex<TextEmbedding>,
    info: &'static LocalModel,
    prompts: Prompts,
    batch_size: usize,
    max_tokens: usize,
    id: String,
}

impl LocalEmbedder {
    /// Load (downloading on first use) a curated model into `cache_dir`.
    /// This can take minutes on first run; call it off the async runtime.
    pub fn new(name: &str, cache_dir: &Path, opts: LocalOptions) -> Result<Self> {
        let info = find_model(name).ok_or_else(|| {
            let names: Vec<_> = CATALOGUE.iter().map(|m| m.name).collect();
            anyhow!("unknown local embedding model {name:?}; known: {}", names.join(", "))
        })?;
        std::fs::create_dir_all(cache_dir)
            .with_context(|| format!("creating model cache dir {}", cache_dir.display()))?;
        let max_tokens = match opts.max_tokens {
            0 => info.max_tokens,
            n => n.min(info.max_tokens),
        };
        let mut init = TextInitOptions::new((info.model)())
            .with_cache_dir(cache_dir.to_path_buf())
            .with_max_length(max_tokens)
            .with_show_download_progress(opts.show_download_progress);
        if let Some(t) = opts.threads {
            init = init.with_intra_threads(t.max(1));
        }
        let model = TextEmbedding::try_new(init)
            .with_context(|| format!("loading local embedding model {}", info.name))?;
        Ok(Self {
            model: Mutex::new(model),
            prompts: opts.prompts.unwrap_or_else(|| Prompts::for_model(info.source)),
            batch_size: opts.batch_size.max(1),
            max_tokens,
            id: format!("local:{}@{}", info.name, info.dim),
            info,
        })
    }

    pub fn info(&self) -> &'static LocalModel {
        self.info
    }

    fn run(&self, texts: &[String], prefix: &str) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // The tokenizer truncates at max_length too; truncating here first
        // keeps tokenisation cheap for very long documents.
        let prepared = prepare(texts, prefix, self.max_tokens * 2);
        let mut model = self.model.lock().map_err(|_| anyhow!("local embedder poisoned"))?;
        let mut out = model
            .embed(&prepared, Some(self.batch_size))
            .with_context(|| format!("local embedding with {}", self.info.name))?;
        drop(model);
        anyhow::ensure!(out.len() == texts.len(), "model returned {} vectors for {} texts", out.len(), texts.len());
        for v in &mut out {
            l2_normalize(v);
        }
        Ok(out)
    }
}

impl Embedder for LocalEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.info.dim
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.run(texts, &self.prompts.document)
    }
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.run(texts, &self.prompts.query)
    }
    fn is_local(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_lookup() {
        assert_eq!(CATALOGUE[0].name, DEFAULT_MODEL);
        assert_eq!(find_model("Multilingual-E5-Small").unwrap().dim, 384);
        assert_eq!(find_model("intfloat/multilingual-e5-small").unwrap().name, DEFAULT_MODEL);
        assert!(find_model("nope").is_none());
        // Every curated entry agrees with fastembed on the output dimension.
        for m in CATALOGUE {
            let model = (m.model)();
            let fe = TextEmbedding::get_model_info(&model).expect("model known to fastembed");
            assert_eq!(fe.dim, m.dim, "{}", m.name);
        }
    }

    #[test]
    fn e5_prompts_applied() {
        let p = Prompts::for_model(find_model(DEFAULT_MODEL).unwrap().source);
        assert_eq!((p.query.as_str(), p.document.as_str()), ("query: ", "passage: "));
        let g = Prompts::for_model(find_model("embeddinggemma-300m-q").unwrap().source);
        assert!(g.query.starts_with("task: search result"));
    }

    /// Downloads the default model (~470 MB) and embeds a few texts.
    /// Run with `cargo test -p anamnesis-embed -- --ignored --nocapture`.
    #[test]
    #[ignore = "downloads model weights"]
    fn local_model_end_to_end() {
        let cache = std::env::var_os("ANAMNESIS_EMBED_TEST_CACHE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("anamnesis-embed-models"));
        let name = std::env::var("ANAMNESIS_EMBED_TEST_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let t0 = std::time::Instant::now();
        let e = LocalEmbedder::new(&name, &cache, LocalOptions { show_download_progress: true, ..Default::default() })
            .expect("load model");
        eprintln!("loaded {} in {:.1}s", e.id(), t0.elapsed().as_secs_f32());

        let docs: Vec<String> = [
            "a memória não é um arquivo, é uma reconstrução feita a cada lembrança",
            "memory is not an archive; it is rebuilt every time we remember",
            "receita de pão de queijo com polvilho azedo",
            "the train to porto leaves at seven in the morning",
        ]
        .map(String::from)
        .to_vec();
        let v = e.embed(&docs).unwrap();
        assert_eq!(v.len(), 4);
        for x in &v {
            assert_eq!(x.len(), e.dim());
            assert!((crate::dot(x, x) - 1.0).abs() < 1e-4);
        }
        let s = |a: usize, b: usize| crate::dot(&v[a], &v[b]);
        eprintln!("pt~en {:.3}  pt~bread {:.3}  pt~train {:.3}", s(0, 1), s(0, 2), s(0, 3));
        assert!(s(0, 1) > s(0, 2) && s(0, 1) > s(0, 3), "cross-lingual paraphrase should be closest");

        let q = e.embed_query(&["o que é lembrar?".into()]).unwrap();
        let qs: Vec<f32> = v.iter().map(|d| crate::dot(&q[0], d)).collect();
        eprintln!("query scores {qs:?}");
        assert!(qs[0] > qs[2] && qs[1] > qs[3]);

        // Throughput on paragraph-sized inputs.
        let para = "anamnese: a ideia de que aprender é recordar o que a alma já sabia. \
                    in the meno, socrates leads a boy to rediscover a geometric truth he was never taught. "
            .repeat(3);
        let batch: Vec<String> = (0..64).map(|i| format!("{i} {para}")).collect();
        let t = std::time::Instant::now();
        e.embed(&batch).unwrap();
        let secs = t.elapsed().as_secs_f32();
        eprintln!(
            "throughput: {} texts (~{} tokens each) in {:.2}s = {:.1} texts/s",
            batch.len(),
            crate::approx_tokens(&batch[0]),
            secs,
            batch.len() as f32 / secs
        );
    }
}
