//! Threads: decisions, open questions, plans and rejections found in text.
//!
//! Two extractors share one output type ([`Thread`], always with a verbatim
//! quote):
//!
//! - [`extract_rules`]: cue phrases in English and Portuguese, question
//!   marks and task checkboxes. Cheap, offline, predictable; misses implicit
//!   phrasing.
//! - model-based: [`extraction_prompt`] builds a prompt + JSON schema for the
//!   harness to run, and [`validate_extraction`] turns the answer into
//!   threads, **rejecting any item whose quote is not a verbatim substring of
//!   the source** (whitespace runs may differ; the stored quote is always the
//!   exact source span).

use anamnesis_core::{Id, Thread, ThreadKind, ThreadStatus, Timestamp};
use serde::{Deserialize, Serialize};

const REJECTION_CUES: &[&str] = &[
    // english
    "decided not to",
    "decided against",
    "i won't",
    "i will not",
    "we won't",
    "we will not",
    "i'm not going to",
    "i am not going to",
    "we're not going to",
    "not going to do",
    "ruled out",
    "i rejected",
    "we rejected",
    "rejected:",
    "giving up on",
    "gave up on",
    "i'm dropping",
    "we're dropping",
    "i dropped",
    "i scrapped",
    "we scrapped",
    "i abandoned",
    "no longer going to",
    "not worth it",
    "won't do",
    // portuguese
    "decidi não",
    "decidimos não",
    "resolvi não",
    "não vou",
    "não vamos",
    "descartei",
    "descartamos",
    "descartado",
    "desisti",
    "desistimos",
    "abandonei",
    "não vale a pena",
    "rejeitei",
    "deixei de lado",
    "abri mão",
    "nunca mais vou",
];

const DECISION_CUES: &[&str] = &[
    // english
    "i decided",
    "i've decided",
    "i have decided",
    "we decided",
    "we've decided",
    "decided to",
    "decision:",
    "i chose",
    "we chose",
    "i'm going with",
    "we're going with",
    "i'll go with",
    "settled on",
    "the decision is",
    "final decision",
    "i concluded",
    // portuguese
    "decidi",
    "decidimos",
    "decisão:",
    "resolvi",
    "optei",
    "optamos",
    "escolhi",
    "escolhemos",
    "vou ficar com",
    "fechei com",
    "bati o martelo",
    "ficou decidido",
    "está decidido",
];

const QUESTION_CUES: &[&str] = &[
    "open question",
    "question:",
    "i wonder",
    "i'm wondering",
    "unclear whether",
    "not sure whether",
    "not sure if",
    "pergunta:",
    "questão:",
    "dúvida:",
    "será que",
    "não sei se",
    "me pergunto",
    "não está claro",
];

const PLAN_CUES: &[&str] = &[
    // english
    "todo:",
    "to do:",
    "to-do:",
    "i will",
    "i'll",
    "we will",
    "we'll",
    "i'm going to",
    "i am going to",
    "we're going to",
    "we are going to",
    "plan:",
    "plan to",
    "planning to",
    "next step",
    "i need to",
    "we need to",
    "i should",
    "we should",
    "let's",
    "i want to",
    // portuguese
    "vou",
    "vamos",
    "plano:",
    "planejo",
    "pretendo",
    "próximo passo",
    "próximos passos",
    "preciso",
    "precisamos",
    "tenho que",
    "temos que",
    "a fazer:",
    "quero",
];

/// Does `hay` (lowercase) contain `cue` at word boundaries?
fn has_cue(hay: &str, cue: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = hay[start..].find(cue) {
        let a = start + pos;
        let b = a + cue.len();
        let before_ok = hay[..a]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = cue.ends_with(|c: char| !c.is_alphanumeric())
            || hay[b..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = a + cue.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    false
}

/// A candidate span of the source: byte offsets of a trimmed sentence/line.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

/// Split text into sentence-ish spans (per line, then on `. ! ?` followed by
/// whitespace), skipping fenced code blocks.
fn spans(text: &str) -> Vec<Span> {
    let mut out = vec![];
    let mut in_fence = false;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let ls = line_start;
        line_start += line.len();
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || trimmed.is_empty() {
            continue;
        }
        // sentence boundaries inside the line
        let bytes = line.as_bytes();
        let mut s = 0;
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if (c == b'.' || c == b'!' || c == b'?')
                && (i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace())
            {
                // don't split on "e.g." style abbreviations of one letter
                let prev_is_short = i >= 2 && bytes[i - 2] == b'.';
                if !prev_is_short {
                    push_span(line, ls, s, i + 1, &mut out);
                    s = i + 1;
                }
            }
            i += 1;
        }
        push_span(line, ls, s, line.len(), &mut out);
    }
    out
}

fn push_span(line: &str, offset: usize, s: usize, e: usize, out: &mut Vec<Span>) {
    let seg = &line[s..e];
    let lead = seg.len() - seg.trim_start().len();
    let trail = seg.len() - seg.trim_end().len();
    if seg.trim().is_empty() {
        return;
    }
    out.push(Span {
        start: offset + s + lead,
        end: offset + e - trail,
    });
}

/// Strip list/quote/heading markers and checkboxes from the start.
/// Returns the remainder and the checkbox state, if any.
fn strip_markers(s: &str) -> (&str, Option<bool>) {
    let mut t = s.trim_start();
    loop {
        let before = t;
        for m in ["- ", "* ", "+ ", "> "] {
            if let Some(r) = t.strip_prefix(m) {
                t = r.trim_start();
            }
        }
        t = t.trim_start_matches('#').trim_start();
        // ordered list "12. "
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && t[digits..].starts_with(". ") {
            t = t[digits + 2..].trim_start();
        }
        if t == before {
            break;
        }
    }
    for (m, done) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(r) = t.strip_prefix(m) {
            return (r.trim_start(), Some(done));
        }
    }
    (t, None)
}

fn statement_of(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 280 {
        let mut t: String = s.chars().take(279).collect();
        t.push('…');
        t
    } else {
        s.to_string()
    }
}

/// Classify one sentence. Returns the kind and whether it is already done.
pub fn classify_sentence(sentence: &str) -> Option<(ThreadKind, ThreadStatus)> {
    let (body, checkbox) = strip_markers(sentence);
    if let Some(done) = checkbox {
        return Some((
            ThreadKind::Plan,
            if done {
                ThreadStatus::Resolved
            } else {
                ThreadStatus::Open
            },
        ));
    }
    let lower = body.to_lowercase().replace(['’', '‘'], "'");
    if REJECTION_CUES.iter().any(|c| has_cue(&lower, c)) {
        return Some((ThreadKind::Rejection, ThreadStatus::Open));
    }
    if DECISION_CUES.iter().any(|c| has_cue(&lower, c)) {
        return Some((ThreadKind::Decision, ThreadStatus::Open));
    }
    if lower.trim_end().ends_with('?') || QUESTION_CUES.iter().any(|c| has_cue(&lower, c)) {
        // very short "?" lines ("ok?") are noise
        if lower.split_whitespace().count() >= 3 {
            return Some((ThreadKind::OpenQuestion, ThreadStatus::Open));
        }
        return None;
    }
    if body.contains("TODO") || PLAN_CUES.iter().any(|c| has_cue(&lower, c)) {
        return Some((ThreadKind::Plan, ThreadStatus::Open));
    }
    None
}

/// Rule-based extraction over an item's text.
pub fn extract_rules(item_id: &Id, text: &str, at: Timestamp) -> Vec<Thread> {
    let mut out: Vec<Thread> = vec![];
    for sp in spans(text) {
        let quote = &text[sp.start..sp.end];
        if let Some((kind, status)) = classify_sentence(quote) {
            if out.iter().any(|t| t.quote == quote) {
                continue;
            }
            let (body, _) = strip_markers(quote);
            out.push(Thread {
                id: Id::new(),
                kind,
                status,
                statement: statement_of(body),
                item_id: item_id.clone(),
                quote: quote.to_string(),
                resolved_by: None,
                created_at: at,
                updated_at: at,
                extractor: "rule".to_string(),
            });
        }
    }
    out
}

// --- model-based extraction ---------------------------------------------------

/// A ready-to-run extraction request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionPrompt {
    pub system: String,
    pub user: String,
    pub schema: serde_json::Value,
}

/// JSON schema the model must answer with.
pub fn extraction_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["items"],
        "properties": {
            "language": {"type": "string", "description": "main language of the text, e.g. \"pt\" or \"en\""},
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["type", "statement", "quote", "status"],
                    "properties": {
                        "type": {"type": "string", "enum": ["decision", "open_question", "plan", "rejection"]},
                        "statement": {"type": "string", "description": "short restatement in the author's words and language"},
                        "quote": {"type": "string", "description": "exact, verbatim span copied from the text that supports it"},
                        "status": {"type": "string", "enum": ["open", "done", "abandoned", "superseded"]},
                        "confidence": {"type": "number", "minimum": 0, "maximum": 1}
                    }
                }
            }
        }
    })
}

const EXTRACTION_SYSTEM: &str = "you extract decisions, open questions, plans and rejections from one personal note. \
the note may be in portuguese or english (or both); keep the author's language. \
- decision: something the author settled on (\"i decided to…\", \"decidi…\", \"optei por…\"). \
- rejection: something the author decided against or dropped (\"decided not to…\", \"não vou…\", \"descartei…\", \"desisti de…\"). hedges like \"acho que não vale\" or \"talvez não\" are not rejections unless the author commits. \
- open question: a question the author is still asking themselves. rhetorical questions and questions quoted from others are not. \
- plan: a concrete intention or task (\"vou…\", \"i'll…\", \"- [ ] …\"). \
every item needs a quote copied character for character from the note; if you cannot quote it, leave it out. \
most notes contain nothing: return {\"items\": []} then. answer only with json matching the schema.";

const EXTRACTION_EXAMPLES: &str = r#"example note:
"Fiquei a semana pensando no app. Decidi usar sqlite em vez de postgres. Não vou fazer login social por enquanto. Será que o modo offline basta? - [ ] escrever o README"
example answer:
{"language": "pt", "items": [
 {"type": "decision", "statement": "usar sqlite em vez de postgres", "quote": "Decidi usar sqlite em vez de postgres.", "status": "open", "confidence": 0.95},
 {"type": "rejection", "statement": "não fazer login social por enquanto", "quote": "Não vou fazer login social por enquanto.", "status": "open", "confidence": 0.9},
 {"type": "open_question", "statement": "o modo offline basta?", "quote": "Será que o modo offline basta?", "status": "open", "confidence": 0.85},
 {"type": "plan", "statement": "escrever o README", "quote": "- [ ] escrever o README", "status": "open", "confidence": 0.9}]}

example note:
"Long walk today. Maybe I should read more Weil, who knows."
example answer:
{"language": "en", "items": []}"#;

/// Build the extraction prompt for one item.
pub fn extraction_prompt(title: &str, text: &str) -> ExtractionPrompt {
    let user =
        format!("{EXTRACTION_EXAMPLES}\n\nnote title: {title}\nnote:\n\"\"\"\n{text}\n\"\"\"");
    ExtractionPrompt {
        system: EXTRACTION_SYSTEM.to_string(),
        user,
        schema: extraction_schema(),
    }
}

/// An extracted item that did not become a thread, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedExtraction {
    pub item: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionReport {
    pub accepted: Vec<Thread>,
    pub rejected: Vec<RejectedExtraction>,
}

/// Find `quote` in `source`: exactly, or modulo whitespace runs. Returns the
/// exact source byte span.
pub fn find_verbatim(source: &str, quote: &str) -> Option<(usize, usize)> {
    let q = quote.trim();
    if q.is_empty() {
        return None;
    }
    if let Some(p) = source.find(q) {
        return Some((p, p + q.len()));
    }
    // collapse whitespace in both, keeping a map back to source offsets
    let mut collapsed = String::with_capacity(source.len());
    let mut map: Vec<usize> = Vec::with_capacity(source.len()); // collapsed byte -> source byte
    let mut prev_ws = false;
    for (i, c) in source.char_indices() {
        if c.is_whitespace() {
            if !prev_ws {
                collapsed.push(' ');
                map.push(i);
            }
            prev_ws = true;
        } else {
            let before = collapsed.len();
            collapsed.push(c);
            for _ in before..collapsed.len() {
                map.push(i);
            }
            prev_ws = false;
        }
    }
    map.push(source.len());
    let qc = q.split_whitespace().collect::<Vec<_>>().join(" ");
    let p = collapsed.find(&qc)?;
    let start = map[p];
    let last = p + qc.len() - 1; // last byte of the match in collapsed
    let last_src = map[last];
    let end = last_src
        + source[last_src..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
    Some((start, end))
}

fn strip_fences(s: &str) -> &str {
    let t = s.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    t.strip_suffix("```").unwrap_or(t).trim()
}

/// Parse a raw model answer (tolerates ```json fences) and validate it.
pub fn validate_extraction_str(
    source: &str,
    answer: &str,
    item_id: &Id,
    at: Timestamp,
    extractor: &str,
    min_confidence: f32,
) -> Result<ExtractionReport, String> {
    let v: serde_json::Value =
        serde_json::from_str(strip_fences(answer)).map_err(|e| format!("not json: {e}"))?;
    Ok(validate_extraction(
        source,
        &v,
        item_id,
        at,
        extractor,
        min_confidence,
    ))
}

/// Validate a model answer against the source text. Items must have a known
/// type and a quote that occurs verbatim in `source`; items below
/// `min_confidence` (when given) are rejected too.
pub fn validate_extraction(
    source: &str,
    answer: &serde_json::Value,
    item_id: &Id,
    at: Timestamp,
    extractor: &str,
    min_confidence: f32,
) -> ExtractionReport {
    let mut accepted: Vec<Thread> = vec![];
    let mut rejected = vec![];
    let items = answer
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for it in items {
        let reject = |reason: &str, rejected: &mut Vec<RejectedExtraction>| {
            rejected.push(RejectedExtraction {
                item: it.clone(),
                reason: reason.to_string(),
            })
        };
        let kind = match it.get("type").and_then(|v| v.as_str()) {
            Some("decision") => ThreadKind::Decision,
            Some("open_question") => ThreadKind::OpenQuestion,
            Some("plan") => ThreadKind::Plan,
            Some("rejection") => ThreadKind::Rejection,
            _ => {
                reject("unknown type", &mut rejected);
                continue;
            }
        };
        let status = match it.get("status").and_then(|v| v.as_str()).unwrap_or("open") {
            "open" => ThreadStatus::Open,
            "done" => ThreadStatus::Resolved,
            "abandoned" => ThreadStatus::Abandoned,
            "superseded" => ThreadStatus::Superseded,
            _ => {
                reject("unknown status", &mut rejected);
                continue;
            }
        };
        if let Some(c) = it.get("confidence").and_then(|v| v.as_f64())
            && (c as f32) < min_confidence
        {
            reject("low confidence", &mut rejected);
            continue;
        }
        let raw_quote = it.get("quote").and_then(|v| v.as_str()).unwrap_or("");
        let trimmed = raw_quote
            .trim()
            .trim_matches(|c| c == '"' || c == '“' || c == '”');
        if trimmed.chars().count() < 3 {
            reject("missing quote", &mut rejected);
            continue;
        }
        let Some((s, e)) =
            find_verbatim(source, trimmed).or_else(|| find_verbatim(source, raw_quote))
        else {
            reject("quote is not verbatim in the source", &mut rejected);
            continue;
        };
        let quote = source[s..e].to_string();
        if accepted.iter().any(|t| t.quote == quote && t.kind == kind) {
            reject("duplicate", &mut rejected);
            continue;
        }
        let statement = it
            .get("statement")
            .and_then(|v| v.as_str())
            .map(statement_of)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| statement_of(strip_markers(&quote).0));
        accepted.push(Thread {
            id: Id::new(),
            kind,
            status,
            statement,
            item_id: item_id.clone(),
            quote,
            resolved_by: None,
            created_at: at,
            updated_at: at,
            extractor: extractor.to_string(),
        });
    }
    ExtractionReport { accepted, rejected }
}
