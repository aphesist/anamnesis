//! Inputs for naming regions: c-TF-IDF keywords, representative items, a
//! deterministic fallback label, and a prompt + schema so a model can do
//! better. The model call itself happens in the harness.

use std::collections::{HashMap, HashSet};

use anamnesis_core::Id;
use serde::{Deserialize, Serialize};

use crate::linalg::{Rows, dot};

/// English + Portuguese stopwords (lowercase). Not exhaustive; enough to keep
/// function words, filler and speech verbs out of region keywords.
const STOPWORDS: &[&str] = &[
    // english
    "the",
    "and",
    "for",
    "are",
    "but",
    "not",
    "you",
    "all",
    "any",
    "can",
    "had",
    "her",
    "was",
    "one",
    "our",
    "out",
    "day",
    "get",
    "has",
    "him",
    "his",
    "how",
    "man",
    "new",
    "now",
    "old",
    "see",
    "two",
    "way",
    "who",
    "boy",
    "did",
    "its",
    "let",
    "put",
    "say",
    "she",
    "too",
    "use",
    "that",
    "with",
    "have",
    "this",
    "will",
    "your",
    "from",
    "they",
    "know",
    "want",
    "been",
    "good",
    "much",
    "some",
    "time",
    "very",
    "when",
    "come",
    "here",
    "just",
    "like",
    "long",
    "make",
    "many",
    "more",
    "only",
    "over",
    "such",
    "take",
    "than",
    "them",
    "well",
    "were",
    "what",
    "into",
    "also",
    "about",
    "after",
    "again",
    "being",
    "could",
    "would",
    "should",
    "there",
    "their",
    "these",
    "those",
    "which",
    "while",
    "where",
    "other",
    "because",
    "then",
    "thing",
    "things",
    "really",
    "maybe",
    "even",
    "still",
    "each",
    "most",
    "does",
    "doing",
    "done",
    "dont",
    "don",
    "isn",
    "it's",
    "i'm",
    "ive",
    "i've",
    "yes",
    "yet",
    "own",
    "same",
    "both",
    "few",
    "off",
    "why",
    "under",
    "until",
    "upon",
    "via",
    "per",
    "within",
    "without",
    "through",
    "between",
    "into",
    "onto",
    "might",
    "must",
    "shall",
    "may",
    "had",
    "having",
    "itself",
    "myself",
    "yourself",
    "ourselves",
    "themselves",
    "every",
    "whom",
    "whose",
    "http",
    "https",
    "www",
    "com",
    "org",
    "html",
    "png",
    "jpg",
    "pdf",
    "md",
    // filler, archaic and speech words that name no topic ("said", "apparently", "o'er")
    "said",
    "says",
    "saying",
    "tell",
    "tells",
    "told",
    "asked",
    "answered",
    "replied",
    "apparently",
    "perhaps",
    "indeed",
    "quite",
    "rather",
    "almost",
    "always",
    "never",
    "ever",
    "often",
    "once",
    "twice",
    "already",
    "though",
    "although",
    "thus",
    "hence",
    "therefore",
    "whether",
    "whereas",
    "wherein",
    "whereof",
    "thereby",
    "however",
    "somewhat",
    "thee",
    "thou",
    "thy",
    "thine",
    "hath",
    "doth",
    "ye",
    "o'er",
    "e'er",
    "ne'er",
    "'tis",
    "twas",
    "'twas",
    "whew",
    "alas",
    "part",
    "prime",
    "made",
    "seen",
    "saw",
    "went",
    "gone",
    "going",
    "came",
    "got",
    "give",
    "gave",
    "given",
    "found",
    "find",
    "think",
    "thought",
    "seem",
    "seems",
    "seemed",
    "look",
    "looked",
    "looks",
    "set",
    "sir",
    "mrs",
    "among",
    "little",
    "great",
    "nothing",
    "something",
    "anything",
    "everything",
    "first",
    "last",
    "next",
    "three",
    "four",
    "five",
    "half",
    "whole",
    "enough",
    "far",
    "men",
    "away",
    "back",
    "down",
    "ago",
    "goes",
    "keep",
    "kept",
    "left",
    "right",
    "till",
    "unto",
    "cannot",
    // portuguese
    "que",
    "não",
    "nao",
    "uma",
    "para",
    "com",
    "por",
    "mais",
    "como",
    "mas",
    "foi",
    "ele",
    "ela",
    "das",
    "dos",
    "tem",
    "seu",
    "sua",
    "ser",
    "quando",
    "muito",
    "nos",
    "já",
    "eu",
    "também",
    "tambem",
    "só",
    "pelo",
    "pela",
    "até",
    "isso",
    "entre",
    "era",
    "depois",
    "sem",
    "mesmo",
    "aos",
    "ter",
    "seus",
    "quem",
    "nas",
    "esse",
    "eles",
    "estão",
    "você",
    "voce",
    "tinha",
    "foram",
    "essa",
    "num",
    "nem",
    "suas",
    "meu",
    "minha",
    "têm",
    "numa",
    "pelos",
    "elas",
    "havia",
    "seja",
    "qual",
    "será",
    "nós",
    "tenho",
    "lhe",
    "deles",
    "essas",
    "esses",
    "pelas",
    "este",
    "fosse",
    "dele",
    "vocês",
    "vos",
    "lhes",
    "meus",
    "minhas",
    "teu",
    "tua",
    "teus",
    "tuas",
    "nosso",
    "nossa",
    "nossos",
    "nossas",
    "dela",
    "delas",
    "esta",
    "estes",
    "estas",
    "aquele",
    "aquela",
    "aqueles",
    "aquelas",
    "isto",
    "aquilo",
    "estou",
    "está",
    "esta",
    "estamos",
    "estava",
    "sobre",
    "onde",
    "porque",
    "então",
    "entao",
    "ainda",
    "sim",
    "aqui",
    "ali",
    "cada",
    "outro",
    "outra",
    "outros",
    "outras",
    "todo",
    "toda",
    "todos",
    "todas",
    "coisa",
    "coisas",
    "pode",
    "fazer",
    "vai",
    "vou",
    "acho",
    "talvez",
    "bem",
    "assim",
    "agora",
    "sempre",
    "nunca",
    "algo",
    "alguma",
    "algum",
    "quase",
    "dia",
    // portuguese filler
    "disse",
    "dizer",
    "diz",
    "dizia",
    "parte",
    "apenas",
    "feito",
    "ver",
    "vez",
    "vezes",
    "tão",
    "tao",
    "lá",
    "cá",
    "aí",
    "pois",
    "ora",
    "enfim",
    "logo",
    "tudo",
    "nada",
    "fez",
    "faz",
    "ficou",
    "fica",
    "primeiro",
    "primeira",
    "grande",
    "pouco",
    "pouca",
    "muitos",
    "muitas",
];

/// Lowercase word tokens of length ≥ 3, minus stopwords and pure numbers.
pub fn tokenize(text: &str) -> Vec<String> {
    let stop: &HashSet<&'static str> = stopset();
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\'').to_lowercase())
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !stop.contains(w.as_str()))
        .collect()
}

fn stopset() -> &'static HashSet<&'static str> {
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| STOPWORDS.iter().copied().collect())
}

/// c-TF-IDF (Grootendorst 2022, as in BERTopic): treat each class as one
/// document; weight `W(t,c) = tf(t,c)/|c| · ln(1 + A / f(t))` where `f(t)` is
/// the frequency of `t` over all classes and `A` the mean words per class,
/// times `ln(1 + C / c(t))` where `c(t)` is the number of the `C` classes `t`
/// occurs in, so words spread over every region don't name one.
/// `classes[c]` lists the texts of class `c`. Returns the top `top` terms per
/// class with their weights.
pub fn c_tf_idf(classes: &[Vec<&str>], top: usize) -> Vec<Vec<(String, f32)>> {
    let mut counts: Vec<HashMap<String, u32>> = Vec::with_capacity(classes.len());
    let mut global: HashMap<String, u32> = HashMap::new();
    // number of classes a term occurs in
    let mut spread: HashMap<String, u32> = HashMap::new();
    let mut total_words = 0usize;
    for texts in classes {
        let mut m: HashMap<String, u32> = HashMap::new();
        for t in texts {
            for w in tokenize(t) {
                *m.entry(w).or_default() += 1;
                total_words += 1;
            }
        }
        for (w, c) in &m {
            *global.entry(w.clone()).or_default() += c;
            *spread.entry(w.clone()).or_default() += 1;
        }
        counts.push(m);
    }
    let nclasses = classes.iter().filter(|c| !c.is_empty()).count().max(1) as f32;
    let a = total_words as f32 / classes.len().max(1) as f32;
    let mut ranked: Vec<Vec<(String, f32)>> = counts
        .iter()
        .map(|m| {
            let len: u32 = m.values().sum();
            let mut v: Vec<(String, f32)> = m
                .iter()
                .filter(|(w, _)| !common_adverb(w, spread[*w] as f32 / nclasses))
                .map(|(w, &c)| {
                    let f = global[w] as f32;
                    // words that occur in most classes ("said", "men") say little
                    // about any one of them
                    let class_idf = (1.0 + nclasses / spread[w] as f32).ln();
                    let weight = c as f32 / len.max(1) as f32
                        * (1.0 + a / f).ln()
                        * class_idf
                        * part_of_speech_weight(w);
                    (w.clone(), weight)
                })
                .collect();
            v.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
            v
        })
        .collect();
    let sizes: Vec<u32> = counts.iter().map(|m| m.values().sum()).collect();
    diversify(&mut ranked, &sizes);
    for v in &mut ranked {
        v.truncate(top);
    }
    ranked
}

/// How many region labels may share a word before it is passed over in the
/// smaller regions' labels (when they have other keywords to offer).
const MAX_LABELS_PER_WORD: usize = 2;
/// Words a label is made of (see [`fallback_label`]).
const LABEL_WORDS: usize = 3;

/// Reorders each class's keywords so that the first [`LABEL_WORDS`] (the
/// label) avoid words already used by [`MAX_LABELS_PER_WORD`] larger classes:
/// "men · nature · poetry", "men · house · life", "england · men · said" become
/// distinct names. Classes are visited from the largest; a class keeps a
/// repeated word only when it has nothing else among its top candidates.
fn diversify(ranked: &mut [Vec<(String, f32)>], sizes: &[u32]) {
    const POOL: usize = 12;
    let mut order: Vec<usize> = (0..ranked.len()).collect();
    order.sort_by(|&x, &y| sizes[y].cmp(&sizes[x]).then(x.cmp(&y)));
    let mut used: HashMap<String, usize> = HashMap::new();
    for c in order {
        let v = &mut ranked[c];
        let pool = v.len().min(POOL);
        let mut first: Vec<usize> = Vec::with_capacity(LABEL_WORDS);
        for (k, (w, _)) in v.iter().enumerate().take(pool) {
            if first.len() == LABEL_WORDS {
                break;
            }
            // "berries · berry", "flowers · flower": one form is enough
            let twin = first.iter().any(|&j| stem(&v[j].0) == stem(w));
            if !twin && used.get(w).copied().unwrap_or(0) < MAX_LABELS_PER_WORD {
                first.push(k);
            }
        }
        // too few fresh words: fill up with the best of the rest, in order
        for k in 0..pool {
            if first.len() == LABEL_WORDS {
                break;
            }
            if !first.contains(&k) {
                first.push(k);
            }
        }
        first.sort_unstable();
        for &k in &first {
            *used.entry(v[k].0.clone()).or_default() += 1;
        }
        let mut reordered: Vec<(String, f32)> = first.iter().map(|&k| v[k].clone()).collect();
        reordered.extend(
            v.iter()
                .enumerate()
                .filter(|(k, _)| !first.contains(k))
                .map(|(_, e)| e.clone()),
        );
        *v = reordered;
    }
}

/// A crude singular for comparing label words: "berries" → "berry",
/// "flowers" → "flower", "summer's" → "summer" ("glass" stays).
fn stem(w: &str) -> String {
    let w = w.strip_suffix("'s").unwrap_or(w);
    if let Some(b) = w.strip_suffix("ies").filter(|b| b.chars().count() >= 2) {
        return format!("{b}y");
    }
    match w.strip_suffix('s') {
        Some(b) if !b.ends_with('s') && b.chars().count() >= 3 => b.to_string(),
        _ => w.to_string(),
    }
}

/// An "-ly" adverb ("apparently", "probably", "certainly") spread over many
/// classes: it describes how the notes are written, not what they are about.
/// Short "-ly" words ("lily", "holy", "july", "italy") are kept.
fn common_adverb(w: &str, spread_share: f32) -> bool {
    w.chars().count() >= 7 && w.ends_with("ly") && spread_share >= 0.2
}

/// Names are mostly nouns: without a tagger, lean away from words that look
/// like verb forms ("walked", "looking"); long words only, so "seed", "bed"
/// and "king" are untouched, and a few common nouns are exempt.
fn part_of_speech_weight(w: &str) -> f32 {
    const NOUNS: &[&str] = &[
        "morning", "evening", "spring", "string", "building", "painting", "writing", "wedding",
        "ceiling", "clothing", "hundred", "sacred",
    ];
    let n = w.chars().count();
    if n >= 6 && (w.ends_with("ed") || w.ends_with("ing")) && !NOUNS.contains(&w) {
        0.7
    } else {
        1.0
    }
}

/// Deterministic lowercase label from keywords: the top three joined by " · ".
pub fn fallback_label(keywords: &[String]) -> String {
    if keywords.is_empty() {
        return "unnamed region".to_string();
    }
    keywords
        .iter()
        .take(3)
        .map(|k| k.to_lowercase())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Representatives of a region: the `n_central` members closest to the
/// centroid, then `n_diverse` picked by maximal marginal relevance so the
/// model also sees the region's edges. Returns member positions.
pub fn representatives(
    x: &Rows,
    members: &[usize],
    centroid: &[f32],
    n_central: usize,
    n_diverse: usize,
) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = members
        .iter()
        .map(|&i| (i, dot(x.row(i), centroid)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut chosen: Vec<usize> = scored.iter().take(n_central).map(|&(i, _)| i).collect();
    for _ in 0..n_diverse {
        let mut best: Option<(usize, f32)> = None;
        for &(i, rel) in &scored {
            if chosen.contains(&i) {
                continue;
            }
            let red = chosen
                .iter()
                .map(|&j| dot(x.row(i), x.row(j)))
                .fold(f32::MIN, f32::max);
            let mmr = 0.3 * rel - 0.7 * red;
            if best.is_none_or(|(_, b)| mmr > b) {
                best = Some((i, mmr));
            }
        }
        match best {
            Some((i, _)) => chosen.push(i),
            None => break,
        }
    }
    chosen
}

/// Everything a model needs to name one region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionLabelInput {
    pub region_id: Id,
    pub keywords: Vec<String>,
    /// Representative items (central first, then diverse).
    pub representatives: Vec<Id>,
    pub size: u32,
    /// The current label, so the model can keep it unless content changed.
    pub previous_label: Option<String>,
    /// Deterministic label used until a model names the region.
    pub fallback_label: String,
}

/// A ready-to-send labelling request (the harness picks the backend).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelPrompt {
    pub system: String,
    pub user: String,
    pub schema: serde_json::Value,
}

/// JSON schema of the labelling answer.
pub fn label_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["labels"],
        "properties": {
            "labels": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["region_id", "label"],
                    "properties": {
                        "region_id": {"type": "string"},
                        "label": {"type": "string", "description": "2–5 lowercase words naming the region"},
                        "summary": {"type": "string", "description": "one lowercase line"}
                    }
                }
            }
        }
    })
}

/// Build the labelling prompt. `snippet` maps a representative id to a short
/// text (title + first lines); ids without a snippet are skipped.
pub fn label_prompt(
    inputs: &[RegionLabelInput],
    snippet: &dyn Fn(&Id) -> Option<String>,
) -> LabelPrompt {
    let system = "you name regions of a personal knowledge base. each region is a cluster of notes. \
        answer with a short, concrete, lowercase name (2–5 words) in the language the notes are mostly written in. \
        if a previous label is given, keep it unless the content clearly changed. never invent topics that are not in the snippets. \
        answer only with json matching the schema."
        .to_string();
    let mut user = String::new();
    for r in inputs {
        user.push_str(&format!("## region {}\n", r.region_id));
        user.push_str(&format!("size: {}\n", r.size));
        if let Some(p) = &r.previous_label {
            user.push_str(&format!("previous label: {p}\n"));
        }
        user.push_str(&format!("keywords: {}\n", r.keywords.join(", ")));
        user.push_str("snippets:\n");
        for id in &r.representatives {
            if let Some(s) = snippet(id) {
                let s: String = s.chars().take(400).collect();
                user.push_str(&format!("- {}\n", s.replace('\n', " ")));
            }
        }
        user.push('\n');
    }
    LabelPrompt {
        system,
        user,
        schema: label_schema(),
    }
}

/// Parse a labelling answer into `region id → label`, lowercasing and
/// dropping empty or overlong labels (> 8 words or > 60 chars).
pub fn parse_labels(answer: &serde_json::Value) -> HashMap<Id, String> {
    let mut out = HashMap::new();
    if let Some(arr) = answer.get("labels").and_then(|v| v.as_array()) {
        for e in arr {
            let (Some(id), Some(label)) = (
                e.get("region_id").and_then(|v| v.as_str()),
                e.get("label").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let l = label.trim().to_lowercase();
            if l.is_empty() || l.split_whitespace().count() > 8 || l.chars().count() > 60 {
                continue;
            }
            out.insert(Id(id.to_string()), l);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_separate_classes() {
        let a = vec![
            "the ethics of attention",
            "attention and care, ethics",
            "attention economy ethics",
        ];
        let b = vec![
            "sourdough bread recipe",
            "bread hydration and sourdough starter",
            "pão de fermentação natural, sourdough",
        ];
        let kw = c_tf_idf(&[a, b], 3);
        assert!(kw[0].iter().any(|(w, _)| w == "attention"));
        assert!(kw[1].iter().any(|(w, _)| w == "sourdough"));
        assert!(!kw[0].iter().any(|(w, _)| w == "the" || w == "and"));
        let label = fallback_label(&kw[1].iter().map(|(w, _)| w.clone()).collect::<Vec<_>>());
        assert!(label.contains("sourdough"));
        assert_eq!(label, label.to_lowercase());
    }

    #[test]
    fn filler_and_speech_words_are_not_keywords() {
        let t = tokenize("Apparently he said o'er the pond, whew, the prime part was ice");
        for w in ["said", "o'er", "whew", "prime", "part"] {
            assert!(!t.contains(&w.to_string()), "{w}");
        }
        assert!(t.contains(&"pond".to_string()));
        assert!(t.contains(&"ice".to_string()));
    }

    #[test]
    fn common_adverbs_are_dropped_and_labels_do_not_repeat_words() {
        // "suddenly" is in every class; "river" is in the three biggest, which
        // each have other words to offer
        let classes: Vec<Vec<&str>> = vec![
            vec![
                "river suddenly fish",
                "river fish trout",
                "river trout suddenly pickerel",
                "river",
            ],
            vec![
                "river suddenly snow",
                "river snow ice",
                "river ice suddenly thaw",
                "river",
            ],
            vec![
                "river suddenly bird",
                "river bird song",
                "river song suddenly nest",
                "river",
            ],
            vec!["suddenly pine wood", "pine wood oak", "oak pine suddenly"],
        ];
        let kw = c_tf_idf(&classes, 5);
        for k in &kw {
            assert!(!k.iter().any(|(w, _)| w == "suddenly"));
        }
        let labels: Vec<String> = kw
            .iter()
            .map(|k| fallback_label(&k.iter().map(|(w, _)| w.clone()).collect::<Vec<_>>()))
            .collect();
        let with_river = labels
            .iter()
            .filter(|l| l.split(" · ").any(|w| w == "river"))
            .count();
        assert!(with_river <= MAX_LABELS_PER_WORD, "{labels:?}");
        assert!(labels[3].contains("pine"));
    }

    #[test]
    fn labels_keep_one_form_of_a_word() {
        assert_eq!(stem("berries"), stem("berry"));
        assert_eq!(stem("flowers"), "flower");
        assert_eq!(stem("summer's"), "summer");
        assert_eq!(stem("glass"), "glass");
        let classes: Vec<Vec<&str>> = vec![
            vec!["flowers flower flowers meadow", "flower flowers bloom"],
            vec!["snow ice", "ice snow cold"],
        ];
        let kw = c_tf_idf(&classes, 5);
        let label = fallback_label(&kw[0].iter().map(|(w, _)| w.clone()).collect::<Vec<_>>());
        assert_eq!(label.matches("flower").count(), 1, "{label}");
    }

    #[test]
    fn verb_forms_rank_below_nouns() {
        assert!(part_of_speech_weight("walked") < 1.0);
        assert!(part_of_speech_weight("looking") < 1.0);
        assert_eq!(part_of_speech_weight("morning"), 1.0);
        assert_eq!(part_of_speech_weight("seed"), 1.0);
    }

    #[test]
    fn tokenizer_handles_portuguese() {
        let t = tokenize("A atenção não é uma mercadoria; é cuidado.");
        assert!(t.contains(&"atenção".to_string()));
        assert!(t.contains(&"mercadoria".to_string()));
        assert!(!t.contains(&"não".to_string()));
    }

    #[test]
    fn parses_labels() {
        let v = serde_json::json!({"labels": [{"region_id": "r1", "label": "Ethics of Attention"}, {"region_id": "r2", "label": ""}]});
        let m = parse_labels(&v);
        assert_eq!(m.get(&Id("r1".into())).unwrap(), "ethics of attention");
        assert!(!m.contains_key(&Id("r2".into())));
        let p = label_prompt(
            &[RegionLabelInput {
                region_id: Id("r1".into()),
                keywords: vec!["attention".into()],
                representatives: vec![Id("i1".into())],
                size: 3,
                previous_label: None,
                fallback_label: "attention".into(),
            }],
            &|_| Some("a note".into()),
        );
        assert!(p.user.contains("a note"));
    }
}
