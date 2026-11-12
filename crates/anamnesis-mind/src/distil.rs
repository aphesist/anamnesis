//! Distil: a few atomic notes (idea / decision / open question / plan) proposed from
//! the user's own messages in a conversation.
//!
//! Every proposal quotes the user's exact words: a model answer is accepted only when
//! its quote occurs verbatim in the turn it names (or, failing that, in another of the
//! user's turns), like [`crate::threads::validate_extraction`]. The assistant's replies
//! are never shown to the distiller, so nothing it said can come back as the user's.
//!
//! Two distillers share the output type ([`DistilProposal`]):
//!
//! - [`distil_rules`]: no model. The rule-based thread extractor
//!   ([`crate::threads::extract_rules`]) finds decisions, open questions and plans
//!   (a rejection is a decision); explicit idea markers ("idea:", "what if we…",
//!   "ideia:", "e se a gente…") find ideas. Titles are the user's own words,
//!   shortened.
//! - model-based: [`distil_prompt`] builds the prompt and json schema,
//!   [`validate_distil`] checks the answer. Titles are written by the model and
//!   must be marked as such wherever they are stored.

use anamnesis_core::{DistilKind, DistilProposal, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::threads::{
    ExtractionPrompt, RULE_EXTRACTOR, extract_rules, find_verbatim, spans, strip_fences,
    strip_markers,
};
use anamnesis_core::ThreadKind;

/// At most this many proposals per conversation and run: a few notes, not a transcript.
pub const MAX_PROPOSALS: usize = 5;
/// Shortest quote worth a note, in characters.
const MIN_QUOTE_CHARS: usize = 12;
/// Longest title kept, in characters.
const MAX_TITLE_CHARS: usize = 80;

/// One of the user's messages in a conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// The chat turn's item (`None` for text handed in from outside the brain).
    pub item_id: Option<Id>,
    pub text: String,
}

/// Explicit idea markers, matched at the start of a sentence (lowercase).
const IDEA_LEADS: &[&str] = &[
    "idea:",
    "an idea:",
    "new idea:",
    "idea -",
    "what if i ",
    "what if we ",
    "i have an idea",
    "i've got an idea",
    "here's an idea",
    "ideia:",
    "uma ideia:",
    "nova ideia:",
    "tenho uma ideia",
    "e se eu ",
    "e se a gente ",
    "e se nós ",
];

fn kind_of(t: ThreadKind) -> DistilKind {
    match t {
        ThreadKind::Decision | ThreadKind::Rejection => DistilKind::Decision,
        ThreadKind::OpenQuestion => DistilKind::OpenQuestion,
        ThreadKind::Plan => DistilKind::Plan,
    }
}

/// A title from the user's own words: the sentence without list markers or an idea
/// cue, cut at a word boundary.
pub fn title_from_quote(quote: &str) -> String {
    let (body, _) = strip_markers(quote);
    let mut t = body.trim();
    let lower = t.to_lowercase();
    for cue in [
        "idea:",
        "an idea:",
        "new idea:",
        "idea -",
        "ideia:",
        "uma ideia:",
        "nova ideia:",
    ] {
        if lower.starts_with(cue) {
            t = t[cue.len()..].trim_start();
            break;
        }
    }
    let t = t.trim_end_matches(['.', '!', ';', ':', ',']).trim();
    shorten(t, MAX_TITLE_CHARS)
}

fn shorten(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let cut: String = s.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end_matches([',', ';', ':', ' ']))
}

fn is_idea(sentence: &str) -> bool {
    let (body, _) = strip_markers(sentence);
    let lower = body.trim().to_lowercase();
    IDEA_LEADS.iter().any(|c| lower.starts_with(c))
}

/// Rule-based proposals from the user's turns (no model). `known` holds quotes already
/// proposed for this conversation; they are not proposed again.
pub fn distil_rules(
    turns: &[Turn],
    chat_id: Option<&Id>,
    known: &[String],
    at: Timestamp,
) -> Vec<DistilProposal> {
    let mut out: Vec<DistilProposal> = Vec::new();
    for turn in turns {
        let scratch = Id::new();
        let mut found: Vec<(usize, usize, DistilKind)> = Vec::new();
        for t in extract_rules(&scratch, &turn.text, at) {
            if let Some((s, e)) = find_verbatim(&turn.text, &t.quote) {
                found.push((s, e, kind_of(t.kind)));
            }
        }
        for sp in spans(&turn.text) {
            if sp.ctx.quoted || sp.ctx.block_quote {
                continue;
            }
            let sentence = &turn.text[sp.start..sp.end];
            if is_idea(sentence) && !found.iter().any(|f| f.0 == sp.start) {
                found.push((sp.start, sp.end, DistilKind::Idea));
            }
        }
        found.sort_by_key(|f| f.0);
        for (s, e, kind) in found {
            let quote = turn.text[s..e].trim().to_string();
            if quote.chars().count() < MIN_QUOTE_CHARS || overlaps(&out, known, &quote) {
                continue;
            }
            out.push(DistilProposal {
                id: Id::new(),
                chat_id: chat_id.cloned(),
                kind,
                title: title_from_quote(&quote),
                quote: quote.clone(),
                source_item: turn.item_id.clone(),
                start: s + (turn.text[s..e].len() - turn.text[s..e].trim_start().len()),
                end: s + turn.text[s..e].trim_end().len(),
                extractor: RULE_EXTRACTOR.to_string(),
                created_at: at,
            });
            if out.len() >= MAX_PROPOSALS {
                return out;
            }
        }
    }
    out
}

/// A quote that repeats or contains (or sits inside) one already proposed.
fn overlaps(out: &[DistilProposal], known: &[String], quote: &str) -> bool {
    let q = quote.trim();
    out.iter()
        .map(|p| p.quote.as_str())
        .chain(known.iter().map(String::as_str))
        .any(|k| {
            let k = k.trim();
            k == q || (!k.is_empty() && (k.contains(q) || q.contains(k)))
        })
}

/// JSON schema the model must answer with.
pub fn distil_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["items"],
        "properties": {
            "items": {
                "type": "array",
                "maxItems": MAX_PROPOSALS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["type", "title", "quote", "turn"],
                    "properties": {
                        "type": {"type": "string", "enum": ["idea", "decision", "open_question", "plan"]},
                        "title": {"type": "string", "description": "a short title (at most 8 words) in the user's language, lowercase"},
                        "quote": {"type": "string", "description": "the user's exact words, copied character for character from one message"},
                        "turn": {"type": "integer", "description": "the number of the message the quote comes from"}
                    }
                }
            }
        }
    })
}

const DISTIL_SYSTEM: &str = "you help a person keep their own thinking from a conversation. \
you see only the person's own messages, numbered. propose at most five atomic notes worth keeping: \
- idea: a new thought, proposal or possibility the person put forward. \
- decision: something the person settled on, or decided against. \
- open_question: a question the person is still asking themselves (not a request for information addressed to an assistant). \
- plan: a concrete intention or task. \
each note needs a quote: the person's exact words, copied character for character from one message, \
long enough to stand on its own (usually one or two sentences), and the number of that message. \
do not paraphrase in the quote, do not merge messages, do not invent. \
the title is yours: at most eight words, lowercase, in the person's language. \
small talk, greetings, thanks and plain requests (\"summarise this\", \"what does x mean?\") are not notes. \
most conversations hold few notes or none: return {\"items\": []} then. answer only with json matching the schema.";

/// Build the distil prompt for the user's turns (assistant replies are left out).
pub fn distil_prompt(turns: &[Turn]) -> ExtractionPrompt {
    let mut user = String::from("the person's messages:\n");
    for (i, t) in turns.iter().enumerate() {
        user.push_str(&format!(
            "\n[{}]\n\"\"\"\n{}\n\"\"\"\n",
            i + 1,
            t.text.trim()
        ));
    }
    ExtractionPrompt {
        system: DISTIL_SYSTEM.to_string(),
        user,
        schema: distil_schema(),
    }
}

/// A model answer item that did not become a proposal, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedDistil {
    pub item: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistilReport {
    pub accepted: Vec<DistilProposal>,
    pub rejected: Vec<RejectedDistil>,
}

/// Parse a raw model answer (tolerates ```json fences) and validate it.
pub fn validate_distil_str(
    turns: &[Turn],
    answer: &str,
    chat_id: Option<&Id>,
    known: &[String],
    extractor: &str,
    at: Timestamp,
) -> Result<DistilReport, String> {
    let v: serde_json::Value =
        serde_json::from_str(strip_fences(answer)).map_err(|e| format!("not json: {e}"))?;
    if !v.get("items").is_some_and(|i| i.is_array()) {
        return Err("no items array".into());
    }
    Ok(validate_distil(turns, &v, chat_id, known, extractor, at))
}

/// Validate a model answer against the user's turns. An item needs a known type, a
/// title, and a quote that occurs verbatim (whitespace runs may differ) in the turn it
/// names or else in another of the user's turns; the stored quote is the exact span.
pub fn validate_distil(
    turns: &[Turn],
    answer: &serde_json::Value,
    chat_id: Option<&Id>,
    known: &[String],
    extractor: &str,
    at: Timestamp,
) -> DistilReport {
    let mut accepted: Vec<DistilProposal> = Vec::new();
    let mut rejected = Vec::new();
    let items = answer
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for it in items {
        let reject = |reason: &str, rejected: &mut Vec<RejectedDistil>| {
            rejected.push(RejectedDistil {
                item: it.clone(),
                reason: reason.to_string(),
            })
        };
        if accepted.len() >= MAX_PROPOSALS {
            reject("too many", &mut rejected);
            continue;
        }
        let kind = match it.get("type").and_then(|v| v.as_str()) {
            Some("idea") => DistilKind::Idea,
            Some("decision") | Some("rejection") => DistilKind::Decision,
            Some("open_question") => DistilKind::OpenQuestion,
            Some("plan") => DistilKind::Plan,
            _ => {
                reject("unknown type", &mut rejected);
                continue;
            }
        };
        let title = it
            .get("title")
            .and_then(|v| v.as_str())
            .map(|t| shorten(t.trim().trim_matches(['"', '“', '”']), MAX_TITLE_CHARS))
            .unwrap_or_default();
        if title.is_empty() {
            reject("missing title", &mut rejected);
            continue;
        }
        let raw = it.get("quote").and_then(|v| v.as_str()).unwrap_or("");
        let trimmed = raw
            .trim()
            .trim_matches(|c| c == '"' || c == '“' || c == '”');
        if trimmed.chars().count() < MIN_QUOTE_CHARS {
            reject("missing or too short quote", &mut rejected);
            continue;
        }
        // the named turn first (1-based), then the others
        let named = it
            .get("turn")
            .and_then(|v| v.as_u64())
            .and_then(|n| (n as usize).checked_sub(1))
            .filter(|&i| i < turns.len());
        let order: Vec<usize> = named
            .into_iter()
            .chain((0..turns.len()).filter(|i| Some(*i) != named))
            .collect();
        let found = order.into_iter().find_map(|i| {
            let text = &turns[i].text;
            find_verbatim(text, trimmed)
                .or_else(|| find_verbatim(text, raw))
                .map(|(s, e)| (i, s, e))
        });
        let Some((ti, s, e)) = found else {
            reject(
                "quote is not verbatim in the user's messages",
                &mut rejected,
            );
            continue;
        };
        let quote = turns[ti].text[s..e].to_string();
        if overlaps(&accepted, known, &quote) {
            reject("duplicate", &mut rejected);
            continue;
        }
        accepted.push(DistilProposal {
            id: Id::new(),
            chat_id: chat_id.cloned(),
            kind,
            title,
            quote,
            source_item: turns[ti].item_id.clone(),
            start: s,
            end: e,
            extractor: extractor.to_string(),
            created_at: at,
        });
    }
    DistilReport { accepted, rejected }
}

/// Was this proposal's title written by a model (not cut from the user's words)?
pub fn title_by_model(p: &DistilProposal) -> bool {
    !crate::threads::is_rule_extractor(&p.extractor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turns(texts: &[&str]) -> Vec<Turn> {
        texts
            .iter()
            .map(|t| Turn {
                item_id: Some(Id::new()),
                text: t.to_string(),
            })
            .collect()
    }

    fn at() -> Timestamp {
        anamnesis_core::now()
    }

    #[test]
    fn rules_find_decisions_questions_plans_and_ideas() {
        let ts = turns(&[
            "hi! can you summarise the weil essay for me?",
            "I decided to write every morning before email. Should I keep the notes private? \
             I'll draft the outline tomorrow.",
            "Idea: a walking journal that only records questions.",
            "thanks, that's helpful.",
        ]);
        let chat = Id::new();
        let ps = distil_rules(&ts, Some(&chat), &[], at());
        let kinds: Vec<DistilKind> = ps.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DistilKind::Decision,
                DistilKind::OpenQuestion,
                DistilKind::Plan,
                DistilKind::Idea
            ],
            "{ps:#?}"
        );
        for p in &ps {
            let src = ts
                .iter()
                .find(|t| t.item_id == p.source_item)
                .expect("source turn");
            assert_eq!(&src.text[p.start..p.end], p.quote, "exact span");
            assert_eq!(p.chat_id.as_ref(), Some(&chat));
            assert!(!title_by_model(p));
        }
        assert_eq!(ps[3].title, "a walking journal that only records questions");
        // a second run proposes nothing new
        let known: Vec<String> = ps.iter().map(|p| p.quote.clone()).collect();
        assert!(distil_rules(&ts, Some(&chat), &known, at()).is_empty());
    }

    #[test]
    fn rules_skip_requests_and_small_talk() {
        let ts = turns(&[
            "what does apophatic mean?",
            "ok thanks",
            "summarise this please",
        ]);
        assert!(distil_rules(&ts, None, &[], at()).is_empty());
    }

    #[test]
    fn model_quotes_must_be_verbatim_user_words() {
        let ts = turns(&[
            "I keep circling the same thought: attention is a form of generosity.",
            "Maybe the essay should open with the  bees,   not the library.",
        ]);
        let answer = json!({ "items": [
            { "type": "idea", "title": "attention as generosity", "quote": "attention is a form of generosity.", "turn": 1 },
            // whitespace differs, and the turn number is wrong: still found, exact span kept
            { "type": "idea", "title": "open with the bees", "quote": "the essay should open with the bees, not the library", "turn": 1 },
            // paraphrase: rejected
            { "type": "decision", "title": "x", "quote": "I will open the essay with bees.", "turn": 2 },
            { "type": "plan", "title": "", "quote": "attention is a form of generosity.", "turn": 1 },
            { "type": "summary", "title": "y", "quote": "attention is a form of generosity.", "turn": 1 },
            { "type": "idea", "title": "dupe", "quote": "attention is a form of generosity.", "turn": 1 }
        ]});
        let r = validate_distil(&ts, &answer, None, &[], "local:qwen", at());
        assert_eq!(r.accepted.len(), 2, "{:#?}", r.rejected);
        assert_eq!(
            r.accepted[1].quote,
            "the essay should open with the  bees,   not the library"
        );
        assert_eq!(r.accepted[1].source_item, ts[1].item_id);
        assert!(title_by_model(&r.accepted[0]));
        let reasons: Vec<&str> = r.rejected.iter().map(|x| x.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "quote is not verbatim in the user's messages",
                "missing title",
                "unknown type",
                "duplicate"
            ]
        );
    }

    #[test]
    fn answers_are_parsed_with_fences_and_capped() {
        let ts = turns(&["one two three four five six seven eight nine ten eleven twelve"]);
        let items: Vec<_> = (0..8)
            .map(|i| json!({"type": "idea", "title": format!("t{i}"), "quote": format!("{} {}", ["one two", "two three", "three four", "four five", "five six", "six seven", "seven eight", "eight nine"][i], "x"), "turn": 1}))
            .collect();
        let ans = format!("```json\n{}\n```", json!({ "items": items }));
        let r = validate_distil_str(&ts, &ans, None, &[], "m", at()).unwrap();
        assert!(r.accepted.is_empty()); // none of these quotes is verbatim ("… x")
        assert!(validate_distil_str(&ts, "nope", None, &[], "m", at()).is_err());
        assert!(validate_distil_str(&ts, "\"a string\"", None, &[], "m", at()).is_err());
        assert!(validate_distil_str(&ts, "{\"items\": []}", None, &[], "m", at()).is_ok());
        let prompt = distil_prompt(&ts);
        assert!(prompt.user.contains("[1]"));
        assert!(prompt.system.contains("exact words"));
    }

    #[test]
    fn titles_are_short_and_clean() {
        assert_eq!(
            title_from_quote("- [ ] Idea: call the printer."),
            "call the printer"
        );
        let long = "I decided that the whole book should be organised around walks rather than chapters, because walking is how the thinking actually happens";
        let t = title_from_quote(long);
        assert!(t.chars().count() <= MAX_TITLE_CHARS + 1, "{t}");
        assert!(t.ends_with('…'));
    }
}
