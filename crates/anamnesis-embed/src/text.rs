//! Text helpers shared by all backends: approximate token counting,
//! truncation to a token budget, and query/document instruction prompts.

use serde::{Deserialize, Serialize};

/// Approximate the number of tokens a subword tokenizer would produce.
///
/// Embedding models use different tokenizers (WordPiece, SentencePiece, BPE),
/// so an exact count would need the model's own tokenizer. This heuristic is
/// deliberately pessimistic for Latin-script text (Portuguese and English):
/// roughly one token per 3.5 characters of words, at least one per word, and
/// one per non-ASCII-letter symbol outside words (punctuation, emoji, CJK).
pub fn approx_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut word_chars = 0usize;
    let flush = |word_chars: &mut usize, tokens: &mut usize| {
        if *word_chars > 0 {
            *tokens += (*word_chars * 2).div_ceil(7).max(1);
            *word_chars = 0;
        }
    };
    for c in text.chars() {
        if c.is_alphanumeric() && !is_cjk(c) {
            word_chars += 1;
        } else {
            flush(&mut word_chars, &mut tokens);
            if !c.is_whitespace() {
                tokens += 1;
            }
        }
    }
    flush(&mut word_chars, &mut tokens);
    tokens
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Truncate `text` so that [`approx_tokens`] of the result is at most
/// `max_tokens`. Cuts at a char boundary, preferring the last whitespace
/// before the limit so words are not split. Returns the input unchanged when
/// it already fits or when `max_tokens == 0` (no limit).
pub fn truncate_to_tokens(text: &str, max_tokens: usize) -> &str {
    if max_tokens == 0 || approx_tokens(text) <= max_tokens {
        return text;
    }
    // Binary search the largest char-boundary prefix that fits.
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let (mut lo, mut hi) = (0usize, bounds.len() - 1);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if approx_tokens(&text[..bounds[mid]]) <= max_tokens {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let cut = bounds[lo];
    let prefix = &text[..cut];
    // Prefer to end on whitespace if that keeps most of the budget.
    match prefix.rfind(char::is_whitespace) {
        Some(ws) if ws * 5 >= cut * 4 => prefix[..ws].trim_end(),
        _ => prefix,
    }
}

/// Prefixes that instruction-tuned embedding models expect in front of
/// queries and documents. Asymmetric retrieval models (E5, Qwen3-Embedding,
/// EmbeddingGemma, Nomic) score noticeably worse without them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompts {
    /// Prepended to every query passed to `embed_query`.
    #[serde(default)]
    pub query: String,
    /// Prepended to every document passed to `embed`.
    #[serde(default)]
    pub document: String,
}

/// The default retrieval instruction for Qwen3-Embedding style models.
pub const QWEN3_QUERY_INSTRUCTION: &str =
    "Given a search query, retrieve relevant notes and passages that answer the query";

impl Prompts {
    pub fn none() -> Self {
        Self::default()
    }

    /// Qwen3-Embedding format: `Instruct: <task>\nQuery:<query>`; documents
    /// are embedded without a prefix.
    pub fn qwen3(instruction: &str) -> Self {
        Self {
            query: format!("Instruct: {instruction}\nQuery:"),
            document: String::new(),
        }
    }

    /// Infer the conventional prompts from a model name (Ollama tag, Hugging
    /// Face id or fastembed short name). Unknown models get no prefixes.
    pub fn for_model(model: &str) -> Self {
        let m = model.to_ascii_lowercase();
        if m.contains("qwen3-embedding") || m.contains("qwen3_embedding") {
            Self::qwen3(QWEN3_QUERY_INSTRUCTION)
        } else if (m.contains("multilingual-e5") || m.contains("/e5-") || m.starts_with("e5-"))
            && !m.contains("e5-mistral")
        {
            Self {
                query: "query: ".into(),
                document: "passage: ".into(),
            }
        } else if m.contains("embeddinggemma") {
            Self {
                query: "task: search result | query: ".into(),
                document: "title: none | text: ".into(),
            }
        } else if m.contains("nomic-embed-text") {
            Self {
                query: "search_query: ".into(),
                document: "search_document: ".into(),
            }
        } else if m.contains("snowflake-arctic-embed") || (m.contains("bge-") && m.contains("-en"))
        {
            Self {
                query: "Represent this sentence for searching relevant passages: ".into(),
                document: String::new(),
            }
        } else {
            Self::none()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty() && self.document.is_empty()
    }
}

/// Apply a prefix and a token budget to a batch of texts. The budget applies
/// to the text itself; the prefix is accounted for by subtracting its size.
pub(crate) fn prepare(texts: &[String], prefix: &str, max_tokens: usize) -> Vec<String> {
    let budget = if max_tokens == 0 {
        0
    } else {
        max_tokens.saturating_sub(approx_tokens(prefix)).max(1)
    };
    texts
        .iter()
        .map(|t| {
            let t = truncate_to_tokens(t, budget);
            if prefix.is_empty() {
                t.to_owned()
            } else {
                format!("{prefix}{t}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_estimate_is_reasonable() {
        assert_eq!(approx_tokens(""), 0);
        assert_eq!(approx_tokens("a"), 1);
        // ~ 9 words, typical tokenizers give 10-14 tokens.
        let n = approx_tokens("the quick brown fox jumps over the lazy dog.");
        assert!((9..=20).contains(&n), "{n}");
        let pt = approx_tokens("a memória é uma reconstrução, não uma gravação");
        assert!((8..=24).contains(&pt), "{pt}");
    }

    #[test]
    fn truncation_respects_budget_and_boundaries() {
        let text = "palavra ".repeat(1000) + "ção";
        let cut = truncate_to_tokens(&text, 50);
        assert!(approx_tokens(cut) <= 50);
        assert!(approx_tokens(cut) >= 40);
        assert!(text.starts_with(cut));
        assert!(!cut.ends_with(' '));
        // Multibyte text must cut on a char boundary.
        let s = "ãéíõú".repeat(500);
        let c = truncate_to_tokens(&s, 10);
        assert!(approx_tokens(c) <= 10 && !c.is_empty());
        assert_eq!(truncate_to_tokens("short", 10), "short");
        assert_eq!(truncate_to_tokens(&s, 0), s);
    }

    #[test]
    fn prompts_by_model() {
        assert_eq!(
            Prompts::for_model("intfloat/multilingual-e5-small").query,
            "query: "
        );
        assert!(
            Prompts::for_model("qwen3-embedding:0.6b")
                .query
                .starts_with("Instruct: ")
        );
        assert!(
            Prompts::for_model("qwen3-embedding:0.6b")
                .query
                .ends_with("\nQuery:")
        );
        assert!(
            Prompts::for_model("qwen3-embedding:0.6b")
                .document
                .is_empty()
        );
        assert_eq!(
            Prompts::for_model("nomic-embed-text").document,
            "search_document: "
        );
        assert!(Prompts::for_model("text-embedding-3-small").is_empty());
        assert!(Prompts::for_model("bge-m3").is_empty());
    }

    #[test]
    fn prepare_prefixes_and_truncates() {
        let out = prepare(&["x ".repeat(100)], "query: ", 20);
        assert!(out[0].starts_with("query: "));
        assert!(approx_tokens(&out[0]) <= 21);
    }
}
