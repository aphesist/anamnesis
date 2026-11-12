//! Threads: decisions, open questions, plans and rejections found in text.
//!
//! Two extractors share one output type ([`Thread`], always with a verbatim
//! quote):
//!
//! - [`extract_rules`]: explicit cues in English and Portuguese. Cheap,
//!   offline, predictable, and deliberately **precise rather than complete**:
//!   a sentence becomes a thread only when the writer commits to something in
//!   the first person ("i decided to…", "decidi…", "i won't…", "i'll… tomorrow"),
//!   deliberates in the first person ("should i…?", "i wonder whether…",
//!   "será que…", "não sei se…"), or marks it explicitly ("question:",
//!   "todo:", "- [ ] …", a question under an "open questions" heading).
//!   A bare question mark is not enough: rhetorical questions, questions inside
//!   quotation marks or block quotes, questions addressed to "you" / "thou",
//!   negative questions ("am i not…?") and paragraphs dense with questions are
//!   skipped. The extractor version is recorded on every thread
//!   ([`RULE_EXTRACTOR`]) so a server can re-run an improved extractor.
//! - model-based: [`extraction_prompt`] builds a prompt + JSON schema for the
//!   harness to run, and [`validate_extraction`] turns the answer into
//!   threads, **rejecting any item whose quote is not a verbatim substring of
//!   the source** (whitespace runs may differ; the stored quote is always the
//!   exact source span).
//!
//! Only run either extractor on text the owner wrote: an article's questions
//! are not the user's.

use anamnesis_core::{Id, Thread, ThreadKind, ThreadStatus, Timestamp};
use serde::{Deserialize, Serialize};

/// The `extractor` recorded on rule-based threads. Bump the version whenever
/// the rules change, so stored threads can be re-extracted (see
/// [`is_rule_extractor`]).
pub const RULE_EXTRACTOR: &str = "rule:v2";

/// Was a thread produced by some version of the rule extractor (`"rule"` was
/// the unversioned first one)?
pub fn is_rule_extractor(extractor: &str) -> bool {
    extractor == "rule" || extractor.starts_with("rule:")
}

/// Explicit first-person rejections. Checked before decisions ("decided not
/// to" contains "decided").
const REJECTION_CUES: &[&str] = &[
    // english
    "i decided not to",
    "i've decided not to",
    "i have decided not to",
    "we decided not to",
    "we've decided not to",
    "i decided against",
    "we decided against",
    "i won't",
    "we won't",
    "i'm not going to",
    "i am not going to",
    "we're not going to",
    "we are not going to",
    "i ruled out",
    "we ruled out",
    "i rejected",
    "we rejected",
    "rejected:",
    "i'm giving up on",
    "i am giving up on",
    "i gave up on",
    "i'm dropping",
    "we're dropping",
    "i dropped the idea",
    "i scrapped",
    "we scrapped",
    "i abandoned the idea",
    "i'm no longer going to",
    "i am no longer going to",
    // portuguese
    "decidi não",
    "decidimos não",
    "resolvi não",
    "não vou mais",
    "não vamos mais",
    "descartei",
    "descartamos",
    "desisti de",
    "desistimos de",
    "abandonei a ideia",
    "rejeitei",
    "deixei de lado",
    "abri mão",
    "nunca mais vou",
];

/// Rejection cues that only count at the start of the sentence ("não vou
/// fazer login social" is a rejection; "eu não vou dizer que…" mid-sentence is
/// usually rhetoric).
const REJECTION_LEAD_CUES: &[&str] = &["não vou", "não vamos"];

/// Explicit first-person decisions.
const DECISION_CUES: &[&str] = &[
    // english
    "i decided to",
    "i decided that",
    "i've decided",
    "i have decided",
    "i had decided",
    "we decided to",
    "we decided that",
    "we've decided",
    "we have decided",
    "i've resolved to",
    "i have resolved to",
    "i resolved to",
    "i determined to",
    "i made up my mind",
    "my mind is made up",
    "decision:",
    "i chose to",
    "we chose to",
    "i'm going with",
    "we're going with",
    "i'll go with",
    "i settled on",
    "we settled on",
    "final decision",
    // portuguese
    "decidi",
    "decidimos",
    "decisão:",
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

/// Explicit question markers: the line says it is a question.
const QUESTION_MARKERS: &[&str] = &[
    "question:",
    "q:",
    "open question",
    "pergunta:",
    "questão:",
    "questao:",
    "dúvida:",
    "duvida:",
];

/// First-person deliberation phrased as a direct question: counts only when
/// the sentence ends with "?".
const QUESTION_DIRECT_CUES: &[&str] = &[
    // english ("shall i" / "should i" are handled by `deliberative_shall`)
    "how do i",
    "how should i",
    "what if i",
    "where should i go",
    "which should i",
    "which one should",
    "do i want",
    "do i need",
    "do i really",
    "is it worth",
    "would it be better",
    "am i supposed",
    // portuguese
    "devo",
    "deveria",
    "devemos",
    "o que eu faço",
    "o que faço",
    "como eu faço",
    "e se eu",
    "vale a pena",
    "faz sentido",
];

/// First-person deliberation phrased indirectly: no "?" needed.
const QUESTION_INDIRECT_CUES: &[&str] = &[
    // english
    "i wonder if",
    "i wonder whether",
    "i wonder what",
    "i wonder when",
    "i wonder where",
    "i wonder who",
    "i'm wondering",
    "i am wondering",
    "i'm not sure if",
    "i'm not sure whether",
    "i am not sure if",
    "i am not sure whether",
    "not sure whether",
    "i don't know if",
    "i don't know whether",
    "i do not know whether",
    "i know not whether",
    "i need to decide",
    "i have to decide",
    "i must decide",
    "i haven't decided",
    "i have not decided",
    "i can't decide",
    "i cannot decide",
    "still undecided",
    "unclear whether",
    // portuguese
    "será que",
    "sera que",
    "não sei se",
    "nao sei se",
    "me pergunto se",
    "me pergunto como",
    "me pergunto o que",
    "fico pensando se",
    "preciso decidir",
    "tenho que decidir",
    "ainda não decidi",
    "não decidi",
    "não consigo decidir",
];

/// Verbs after "shall i" / "should i" that make it a practical choice ("shall i
/// write memoirs?", "what should i read?") rather than a lament or an argument
/// ("shall i trust the deceiver again?", "when shall i come to the end…?").
const DELIBERATE_VERBS: &[&str] = &[
    "write",
    "read",
    "go",
    "call",
    "name",
    "publish",
    "print",
    "buy",
    "sell",
    "take",
    "send",
    "begin",
    "start",
    "try",
    "choose",
    "accept",
    "decline",
    "stay",
    "leave",
    "move",
    "keep",
    "lecture",
    "travel",
    "visit",
    "walk",
    "build",
    "plant",
    "answer",
    "reply",
    "use",
    "give",
    "do",
    "cultivate",
    "subdue",
    "raise",
    "study",
    "learn",
    "spend",
    "sign",
    "join",
    "quit",
    "marry",
    "settle",
    "invite",
    "finish",
    "drop",
    "add",
    "cut",
    "rewrite",
    "translate",
    "post",
    "pay",
    "rent",
    "hire",
    "apply",
    "teach",
    "return",
    "wait",
];

/// "shall i write memoirs?", "what should i read?": a first-person "shall/should
/// i" followed by a practical verb, not introduced by "when", "how", "why" or
/// "ever" (laments and arguments).
fn deliberative_shall(lower: &str) -> bool {
    let w: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    w.windows(3).enumerate().any(|(k, x)| {
        matches!(x[0], "shall" | "should")
            && x[1] == "i"
            && DELIBERATE_VERBS.contains(&x[2])
            && (k == 0 || !matches!(w[k - 1], "when" | "how" | "why" | "ever" | "never"))
    })
}

/// Questions with these are rhetorical or addressed to someone else.
const ADDRESSED_WORDS: &[&str] = &[
    "you", "your", "yours", "thou", "thee", "thy", "thine", "ye", "você", "vocês", "voce", "tu",
    "teu", "tua",
];

/// Negative or rhetorical question forms ("am i not partly leaves…?").
const RHETORICAL_CUES: &[&str] = &[
    "is not",
    "are not",
    "isn't it",
    "shall i say",
    "should i say",
    "shall we say",
    "must i",
    "why should i",
    "why should we",
    "why shall i",
    "who shall",
    "who would",
    "who can",
    "não é",
    "não seria",
    "pois não",
];

/// Explicit plan markers at the start of the line.
const PLAN_MARKERS: &[&str] = &[
    "todo:",
    "to do:",
    "to-do:",
    "plan:",
    "next step:",
    "next steps:",
    "plano:",
    "a fazer:",
    "próximo passo:",
    "próximos passos:",
];

/// First-person intentions that are plans by themselves.
const PLAN_INTENT_CUES: &[&str] = &[
    "i plan to",
    "i'm planning to",
    "i am planning to",
    "we plan to",
    "we're planning to",
    "i intend to",
    "we intend to",
    "my plan is to",
    "next step is to",
    "pretendo",
    "planejo",
    "estou planejando",
    "meu plano é",
    "o próximo passo é",
];

/// First-person commitments that are plans only with a concrete time
/// ("i'll sketch the timeline tomorrow", "vou reler o mênon amanhã").
const PLAN_COMMIT_CUES: &[&str] = &[
    "i will",
    "i'll",
    "i'm going to",
    "i am going to",
    "i need to",
    "i have to",
    "we will",
    "we'll",
    "we're going to",
    "we need to",
    "let's",
    "vou",
    "vamos",
    "preciso",
    "precisamos",
    "tenho que",
    "temos que",
];

const PLAN_TIME_CUES: &[&str] = &[
    // english
    "tomorrow",
    "to-morrow",
    "tonight",
    "to-night",
    "next week",
    "this week",
    "next month",
    "this month",
    "this weekend",
    "next year",
    "later today",
    "by monday",
    "by tuesday",
    "by wednesday",
    "by thursday",
    "by friday",
    "on monday",
    "on tuesday",
    "on wednesday",
    "on thursday",
    "on friday",
    "on saturday",
    "on sunday",
    "before the end of",
    // portuguese
    "amanhã",
    "amanha",
    "hoje à noite",
    "hoje a noite",
    "semana que vem",
    "próxima semana",
    "proxima semana",
    "esta semana",
    "essa semana",
    "mês que vem",
    "próximo mês",
    "fim de semana",
    "segunda-feira",
    "terça-feira",
    "quarta-feira",
    "quinta-feira",
    "sexta-feira",
    "até sexta",
    "até segunda",
];

/// Headings under which a plain question is an open question.
const QUESTION_HEADINGS: &[&str] = &[
    "open questions",
    "questions",
    "question",
    "perguntas",
    "perguntas abertas",
    "questões",
    "questões abertas",
    "dúvidas",
    "duvidas",
    "unknowns",
    "to decide",
    "a decidir",
];

/// Does `hay` (lowercase) contain `cue` at word boundaries?
fn has_cue(hay: &str, cue: &str) -> bool {
    find_cue(hay, cue).is_some()
}

/// Byte offset of the first occurrence of `cue` in `hay` at word boundaries.
fn find_cue(hay: &str, cue: &str) -> Option<usize> {
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
            return Some(a);
        }
        start = a + cue.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    None
}

fn any_cue(hay: &str, cues: &[&str]) -> bool {
    cues.iter().any(|c| has_cue(hay, c))
}

/// Does the sentence start with `cue` (after an optional "eu"/"and"/"so")?
fn leads_with(lower: &str, cue: &str) -> bool {
    let t = lower.trim_start_matches(|c: char| !c.is_alphanumeric());
    for lead in ["", "and ", "so ", "but ", "e ", "mas ", "então "] {
        if let Some(rest) = t.strip_prefix(lead)
            && rest.starts_with(cue)
            && rest[cue.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
        {
            return true;
        }
    }
    false
}

/// A candidate span of the source: byte offsets of a trimmed sentence/line,
/// plus what the surrounding text says about it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) ctx: Ctx,
}

/// What the surroundings of a sentence say about it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ctx {
    /// Under a heading such as "open questions" or "perguntas".
    pub question_heading: bool,
    /// The paragraph holds three or more question marks: rhetoric, a dialogue
    /// or a catechism, not the writer's own open questions.
    pub dense_questions: bool,
    /// The sentence starts inside quotation marks.
    pub quoted: bool,
    /// The line is a block quote ("> …").
    pub block_quote: bool,
}

fn is_open_quote(c: char) -> bool {
    matches!(c, '“' | '«' | '„')
}
fn is_close_quote(c: char) -> bool {
    matches!(c, '”' | '»')
}

/// Split text into sentence-ish spans (per line, then on `. ! ?` followed by
/// whitespace), skipping fenced code blocks, and note each span's context.
pub(crate) fn spans(text: &str) -> Vec<Span> {
    let mut out = vec![];
    let mut in_fence = false;
    let mut line_start = 0;
    let mut heading_is_questions = false;
    // quotation state, reset at blank lines (paragraph boundaries)
    let mut in_quote = false;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    // question marks per paragraph (prose lines) or per list item
    let mut dense: Vec<bool> = vec![false; lines.len()];
    {
        let mut i = 0;
        while i < lines.len() {
            if lines[i].trim().is_empty() {
                i += 1;
                continue;
            }
            let mut j = i;
            let mut q = 0;
            while j < lines.len() && !lines[j].trim().is_empty() && !is_item_line(lines[j]) {
                q += lines[j].matches('?').count();
                j += 1;
            }
            if j == i {
                // a list item or heading: its own unit
                dense[i] = lines[i].matches('?').count() >= 3;
                i += 1;
                continue;
            }
            for d in dense.iter_mut().take(j).skip(i) {
                *d = q >= 3;
            }
            i = j;
        }
    }
    for (li, line) in lines.iter().enumerate() {
        let ls = line_start;
        line_start += line.len();
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if trimmed.is_empty() {
            in_quote = false;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(h) = heading_text(trimmed) {
            heading_is_questions = QUESTION_HEADINGS.contains(&h.as_str());
            if heading_is_questions || trimmed.starts_with('#') {
                continue;
            }
        }
        let block_quote = trimmed.starts_with('>');
        // sentence boundaries inside the line
        let mut s = 0;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut quote_at_start = in_quote;
        for (k, &(i, c)) in chars.iter().enumerate() {
            if c == '"' {
                in_quote = !in_quote;
            } else if is_open_quote(c) {
                in_quote = true;
            } else if is_close_quote(c) {
                in_quote = false;
            }
            let next = chars.get(k + 1).map(|x| x.1);
            if matches!(c, '.' | '!' | '?') && next.is_none_or(char::is_whitespace) {
                // don't split on "e.g." style abbreviations of one letter, nor after
                // "Mr.", "Dr.", "St."
                let prev_is_short = k >= 2 && chars[k - 2].1 == '.';
                if !prev_is_short && !(c == '.' && is_abbreviation(&line[s..i])) {
                    let ctx = Ctx {
                        question_heading: heading_is_questions,
                        dense_questions: dense[li],
                        quoted: quote_at_start,
                        block_quote,
                    };
                    push_span(line, ls, s, i + c.len_utf8(), ctx, &mut out);
                    s = i + c.len_utf8();
                    quote_at_start = in_quote;
                }
            }
        }
        let ctx = Ctx {
            question_heading: heading_is_questions,
            dense_questions: dense[li],
            quoted: quote_at_start,
            block_quote,
        };
        push_span(line, ls, s, line.len(), ctx, &mut out);
    }
    out
}

/// Does `before` (the text up to a period) end with a title abbreviation?
fn is_abbreviation(before: &str) -> bool {
    const ABBR: &[&str] = &[
        "Mr", "Mrs", "Ms", "Dr", "St", "Mt", "Prof", "Rev", "Capt", "Col", "Gen", "Sr", "Jr",
        "Sra", "Dra", "vs", "cf", "Vol", "vol", "p", "pp",
    ];
    let last = before
        .rsplit(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("");
    ABBR.contains(&last)
}

/// A list item line ("- ", "* ", "1. ", "- [ ] ").
fn is_item_line(line: &str) -> bool {
    let t = line.trim_start();
    if ["- ", "* ", "+ "].iter().any(|m| t.starts_with(m)) {
        return true;
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && t[digits..].starts_with(". ")
}

/// The normalised text of a heading: a markdown heading, or a short line
/// ending with ":" ("Open questions:"). `None` for other lines.
fn heading_text(trimmed: &str) -> Option<String> {
    let t = if trimmed.starts_with('#') {
        trimmed.trim_start_matches('#').trim()
    } else {
        let t = trimmed.strip_suffix(':')?;
        if t.split_whitespace().count() > 4 || is_item_line(t) {
            return None;
        }
        t.trim()
    };
    Some(
        t.trim_end_matches(':')
            .trim()
            .trim_matches(|c: char| c == '*' || c == '_')
            .to_lowercase(),
    )
}

fn push_span(line: &str, offset: usize, s: usize, e: usize, ctx: Ctx, out: &mut Vec<Span>) {
    let seg = &line[s..e];
    let lead = seg.len() - seg.trim_start().len();
    let trail = seg.len() - seg.trim_end().len();
    if seg.trim().is_empty() {
        return;
    }
    // a sentence that opens with a quotation mark is someone else's words
    let quoted = ctx.quoted || seg.trim_start().starts_with(['"', '“', '«', '„']);
    out.push(Span {
        start: offset + s + lead,
        end: offset + e - trail,
        ctx: Ctx { quoted, ..ctx },
    });
}

/// Strip list/quote/heading markers and checkboxes from the start.
/// Returns the remainder and the checkbox state, if any.
pub(crate) fn strip_markers(s: &str) -> (&str, Option<bool>) {
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

/// Does the (lowercase) sentence end with a question mark, ignoring closing
/// brackets? `Some(true)` if it ends with `?` followed by a closing quotation
/// mark (a quoted question).
fn question_end(lower: &str) -> Option<bool> {
    let t = lower.trim_end().trim_end_matches([')', ']', '*', '_']);
    if t.ends_with('?') {
        return Some(false);
    }
    let unq = t.trim_end_matches(['"', '”', '»', '\'', '’']);
    (unq.len() < t.len() && unq.ends_with('?')).then_some(true)
}

/// "should we not pity him?", "am i not partly leaves?": a modal or auxiliary,
/// a subject pronoun, then "not" — the classic rhetorical question.
fn negative_question(lower: &str) -> bool {
    const AUX: &[&str] = &[
        "shall", "should", "can", "could", "would", "will", "do", "does", "did", "am", "is", "are",
        "was", "were", "have", "has", "had", "may", "might", "must",
    ];
    const SUBJ: &[&str] = &["i", "we", "it", "he", "she", "they", "one", "this", "that"];
    let w: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    w.windows(3)
        .any(|x| AUX.contains(&x[0]) && SUBJ.contains(&x[1]) && x[2] == "not")
}

/// The sentence with quoted passages removed: "the farmer says, “next year i will
/// raise rye”" keeps only "the farmer says, ". An opening quotation mark that is
/// never closed drops the rest; a closing one never opened drops what came before
/// (the sentence began inside someone else's words).
fn unquoted(s: &str) -> String {
    let s = s.replace("’’", "”").replace("''", "”");
    // a closing mark with no opening one before it
    if let Some(close) = s.find('”')
        && !s[..close].contains('“')
    {
        return unquoted(&s[close + '”'.len_utf8()..]);
    }
    let mut out = String::with_capacity(s.len());
    let mut in_q = false;
    for c in s.chars() {
        match c {
            '“' | '«' | '„' => in_q = true,
            '”' | '»' => {
                in_q = false;
                continue;
            }
            '"' => {
                in_q = !in_q;
                continue;
            }
            _ => {}
        }
        if !in_q {
            out.push(c);
        }
    }
    out
}

fn addressed(lower: &str) -> bool {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| ADDRESSED_WORDS.contains(&w))
}

/// Is this an open question of the writer's?
fn is_open_question(lower: &str, ctx: &Ctx) -> bool {
    let end = question_end(lower);
    if end == Some(true) || ctx.quoted || ctx.block_quote {
        return false;
    }
    let ends_q = end == Some(false);
    let start = lower.trim_start();
    if QUESTION_MARKERS.iter().any(|m| start.starts_with(m))
        || lower.contains("? todo")
        || lower.contains("?todo")
    {
        return true;
    }
    if addressed(lower) || any_cue(lower, RHETORICAL_CUES) || negative_question(lower) {
        return false;
    }
    // "if we knew all things…, should we know anything?" argues, it does not ask;
    // "why should i…?" complains
    if ends_q && (start.starts_with("if ") || start.starts_with("why")) {
        return false;
    }
    if ends_q && ctx.question_heading {
        return true;
    }
    if any_cue(lower, QUESTION_INDIRECT_CUES) {
        // "será que…?" in a paragraph full of questions is rhetoric too
        return !(ends_q && ctx.dense_questions);
    }
    ends_q
        && !ctx.dense_questions
        && (any_cue(lower, QUESTION_DIRECT_CUES) || deliberative_shall(lower))
}

/// "vou" + "resolvi" etc. need care in Portuguese: "resolvi o problema" is
/// "i solved", not "i resolved to". Count "resolvi" only before an infinitive
/// or "que".
fn resolvi_decision(lower: &str) -> bool {
    let Some(p) = find_cue(lower, "resolvi") else {
        return false;
    };
    let next = lower[p + "resolvi".len()..]
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| !w.is_empty())
        .unwrap_or("");
    next == "que" || (next.len() > 3 && ["ar", "er", "ir"].iter().any(|s| next.ends_with(s)))
}

/// Classify one sentence on its own (no surrounding context). Returns the
/// kind and whether it is already done.
pub fn classify_sentence(sentence: &str) -> Option<(ThreadKind, ThreadStatus)> {
    classify_in_context(sentence, &Ctx::default())
}

/// Classify one sentence given what its surroundings say about it.
pub fn classify_in_context(sentence: &str, ctx: &Ctx) -> Option<(ThreadKind, ThreadStatus)> {
    let (body, checkbox) = strip_markers(sentence);
    let full = body.to_lowercase();
    let lower = unquoted(&full).replace(['’', '‘'], "'");
    if lower.split_whitespace().count() < 3 {
        // "ok?", "- [ ] x": too short to say anything
        return None;
    }
    if let Some(done) = checkbox {
        // an explicit task; "- [ ] …?" is an open question
        if !done && question_end(&lower) == Some(false) {
            return Some((ThreadKind::OpenQuestion, ThreadStatus::Open));
        }
        let status = if done {
            ThreadStatus::Resolved
        } else {
            ThreadStatus::Open
        };
        return Some((ThreadKind::Plan, status));
    }
    if is_open_question(&lower, ctx) {
        return Some((ThreadKind::OpenQuestion, ThreadStatus::Open));
    }
    // everything below is a statement of the writer's own commitment
    if ctx.quoted || ctx.block_quote || question_end(&lower).is_some() {
        return None;
    }
    let start = lower.trim_start();
    // conditionals and hypotheticals commit to nothing
    if ["if ", "se ", "had i ", "unless ", "when i ", "quando "]
        .iter()
        .any(|p| start.starts_with(p))
    {
        return None;
    }
    if any_cue(&lower, REJECTION_CUES) || REJECTION_LEAD_CUES.iter().any(|c| leads_with(&lower, c))
    {
        return Some((ThreadKind::Rejection, ThreadStatus::Open));
    }
    if any_cue(&lower, DECISION_CUES) || resolvi_decision(&lower) {
        return Some((ThreadKind::Decision, ThreadStatus::Open));
    }
    // hedged or conditional commitments are not plans
    let hedged = ["perhaps", "lest", "if", "would", "unless", "talvez"]
        .iter()
        .any(|w| has_cue(&lower, w));
    let todo_word = body
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w == "TODO");
    if todo_word
        || PLAN_MARKERS.iter().any(|m| start.starts_with(m))
        || any_cue(&lower, PLAN_INTENT_CUES)
        || (!hedged && any_cue(&lower, PLAN_COMMIT_CUES) && any_cue(&lower, PLAN_TIME_CUES))
    {
        return Some((ThreadKind::Plan, ThreadStatus::Open));
    }
    None
}

/// Rule-based extraction over an item's text. Every thread carries
/// [`RULE_EXTRACTOR`] as its extractor.
pub fn extract_rules(item_id: &Id, text: &str, at: Timestamp) -> Vec<Thread> {
    let mut out: Vec<Thread> = vec![];
    for sp in spans(text) {
        let quote = &text[sp.start..sp.end];
        if let Some((kind, status)) = classify_in_context(quote, &sp.ctx) {
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
                extractor: RULE_EXTRACTOR.to_string(),
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

pub(crate) fn strip_fences(s: &str) -> &str {
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
