//! Markdown analysis shared by the folder and obsidian connectors.
//!
//! Splits YAML frontmatter, then walks the CommonMark event stream (pulldown-cmark, with its
//! wikilink extension) to collect the title, `[[wikilinks]]` (incl. `[[note|alias]]`,
//! `[[note#heading]]` and embeds `![[...]]`), markdown links, bare urls and `#tags`.
//! Nothing inside code blocks or code spans counts.

use std::sync::LazyLock;

use anamnesis_core::Timestamp;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yaml_rust2::{Yaml, YamlLoader};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiLink {
    /// Note name or path, without heading/alias (`folder/note` in `[[folder/note#h|a]]`).
    pub target: String,
    /// Heading or block reference after `#` (`^block` for block refs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// Display text after `|`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// `![[...]]`.
    pub embed: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// Frontmatter as json (object), or `Null` when there is none or it did not parse.
    pub frontmatter: Value,
    pub frontmatter_error: Option<String>,
    /// The text after the frontmatter, verbatim.
    pub body: String,
    /// Frontmatter `title`, else the first level-1 heading.
    pub title: Option<String>,
    pub aliases: Vec<String>,
    /// Frontmatter tags then inline `#tags`, without `#`, deduplicated, original case.
    pub tags: Vec<String>,
    /// Frontmatter `created` / `date` / `created_at`.
    pub created: Option<Timestamp>,
    /// Frontmatter `modified` / `updated` / `updated_at`.
    pub modified: Option<Timestamp>,
    pub wikilinks: Vec<WikiLink>,
    /// Markdown link destinations and bare urls, in order, deduplicated.
    pub links: Vec<String>,
}

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^\p{L}\p{N}_&/#\]\)])#([\p{L}\p{N}_][\p{L}\p{N}_/\-]*)").unwrap()
});
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>()\[\]"'`]+"#).unwrap());

/// Split `---\n yaml \n---` off the start. Returns (yaml, rest).
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(after) = t.strip_prefix("---\n").or_else(|| t.strip_prefix("---\r\n")) else {
        return (None, text);
    };
    // Find a closing line that is exactly `---` (or `...`).
    let mut offset = 0;
    for line in after.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = &after[..offset];
            let rest = &after[offset + line.len()..];
            return (Some(yaml), rest);
        }
        offset += line.len();
    }
    (None, text)
}

/// YAML → json. Multiple documents: the first one. Scalars stay strings unless they are
/// YAML ints/floats/bools.
pub fn yaml_to_json(src: &str) -> Result<Value, String> {
    let docs = YamlLoader::load_from_str(src).map_err(|e| e.to_string())?;
    Ok(docs.into_iter().next().map(|y| conv(&y)).unwrap_or(Value::Null))
}

fn conv(y: &Yaml) -> Value {
    match y {
        Yaml::Real(s) => s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(Value::String(s.clone())),
        Yaml::Integer(i) => Value::from(*i),
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Array(a) => Value::Array(a.iter().map(conv).collect()),
        Yaml::Hash(h) => {
            let mut m = Map::new();
            for (k, v) in h {
                let key = match k {
                    Yaml::String(s) | Yaml::Real(s) => s.clone(),
                    Yaml::Integer(i) => i.to_string(),
                    Yaml::Boolean(b) => b.to_string(),
                    other => format!("{other:?}"),
                };
                m.insert(key, conv(v));
            }
            Value::Object(m)
        }
        Yaml::Alias(_) | Yaml::Null | Yaml::BadValue => Value::Null,
    }
}

/// Parse dates as people write them in frontmatter: rfc3339, `YYYY-MM-DD`,
/// `YYYY-MM-DD HH:MM[:SS]`, `YYYY-MM-DDTHH:MM[:SS]` (naive times are taken as utc).
pub fn parse_date(s: &str) -> Option<Timestamp> {
    let s = s.trim();
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc2822(s) {
        return Some(t.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, f) {
            return Some(Utc.from_utc_datetime(&t));
        }
    }
    for f in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, f) {
            return Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?));
        }
    }
    None
}

fn value_date(v: Option<&Value>) -> Option<Timestamp> {
    match v? {
        Value::String(s) => parse_date(s),
        // Unix seconds.
        Value::Number(n) => n.as_i64().and_then(|s| Utc.timestamp_opt(s, 0).single()),
        _ => None,
    }
}

/// A list or a comma/space-separated string.
fn string_list(v: Option<&Value>, split_spaces: bool) -> Vec<String> {
    let mut out = Vec::new();
    match v {
        Some(Value::Array(a)) => {
            for x in a {
                match x {
                    Value::String(s) => out.push(s.trim().to_string()),
                    Value::Number(n) => out.push(n.to_string()),
                    _ => {}
                }
            }
        }
        Some(Value::String(s)) => {
            for p in s.split(|c: char| c == ',' || (split_spaces && c.is_whitespace())) {
                out.push(p.trim().to_string());
            }
        }
        _ => {}
    }
    out.retain(|s| !s.is_empty());
    out
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.iter().any(|x| x == &s) {
        v.push(s);
    }
}

/// Split `note#heading` into (note, heading).
fn split_heading(dest: &str) -> (String, Option<String>) {
    match dest.split_once('#') {
        Some((t, h)) => (t.trim().to_string(), Some(h.trim().to_string()).filter(|h| !h.is_empty())),
        None => (dest.trim().to_string(), None),
    }
}

pub fn inline_tags(text: &str) -> impl Iterator<Item = String> + '_ {
    TAG_RE.captures_iter(text).filter_map(|c| {
        let t = c.get(1)?.as_str().trim_end_matches(['/', '-']);
        // Obsidian: a tag needs at least one non-digit.
        if t.is_empty() || t.chars().all(|c| c.is_ascii_digit()) { None } else { Some(t.to_string()) }
    })
}

pub fn parse(text: &str) -> Parsed {
    let (yaml, body) = split_frontmatter(text);
    let mut p = Parsed { body: body.to_string(), ..Default::default() };
    if let Some(y) = yaml {
        match yaml_to_json(y) {
            Ok(v @ Value::Object(_)) => p.frontmatter = v,
            Ok(Value::Null) => {}
            Ok(_) => p.frontmatter_error = Some("frontmatter is not a mapping".into()),
            Err(e) => p.frontmatter_error = Some(e),
        }
    }
    let fm = p.frontmatter.as_object().cloned().unwrap_or_default();
    p.title = fm.get("title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    p.aliases = string_list(fm.get("aliases").or_else(|| fm.get("alias")), false);
    for t in string_list(fm.get("tags").or_else(|| fm.get("tag")), true) {
        push_unique(&mut p.tags, t.trim_start_matches('#').to_string());
    }
    p.created = value_date(fm.get("created").or_else(|| fm.get("date")).or_else(|| fm.get("created_at")));
    p.modified = value_date(fm.get("modified").or_else(|| fm.get("updated")).or_else(|| fm.get("updated_at")));

    let opts = Options::ENABLE_WIKILINKS
        | Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut in_code = 0usize;
    let mut h1: Option<String> = None;
    let mut in_h1 = false;
    // (wikilink under construction, collected display text)
    let mut wl: Option<(WikiLink, bool, String)> = None;
    for ev in Parser::new_ext(&p.body, opts) {
        match ev {
            Event::Start(Tag::CodeBlock(_)) => in_code += 1,
            Event::End(TagEnd::CodeBlock) => in_code = in_code.saturating_sub(1),
            Event::Start(Tag::Heading { level: HeadingLevel::H1, .. }) if h1.is_none() => {
                in_h1 = true;
                h1 = Some(String::new());
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => in_h1 = false,
            Event::Start(Tag::Link { link_type: LinkType::WikiLink { has_pothole }, dest_url, .. }) => {
                let (target, heading) = split_heading(&dest_url);
                wl = Some((WikiLink { target, heading, alias: None, embed: false }, has_pothole, String::new()));
            }
            Event::Start(Tag::Image { link_type: LinkType::WikiLink { has_pothole }, dest_url, .. }) => {
                let (target, heading) = split_heading(&dest_url);
                wl = Some((WikiLink { target, heading, alias: None, embed: true }, has_pothole, String::new()));
            }
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) if wl.is_some() => {
                let (mut link, piped, text) = wl.take().unwrap();
                if piped && !text.trim().is_empty() {
                    link.alias = Some(text.trim().to_string());
                }
                if !link.target.is_empty() || link.heading.is_some() {
                    if !p.wikilinks.contains(&link) {
                        p.wikilinks.push(link);
                    }
                }
            }
            Event::Start(Tag::Link { dest_url, link_type, .. }) | Event::Start(Tag::Image { dest_url, link_type, .. }) => {
                let d = dest_url.trim();
                let skip = d.is_empty() || d.starts_with('#') || matches!(link_type, LinkType::Email) || d.starts_with("mailto:");
                if !skip {
                    push_unique(&mut p.links, d.to_string());
                }
            }
            Event::Text(t) => {
                if in_h1 && let Some(h) = h1.as_mut() {
                    h.push_str(&t);
                }
                if let Some((_, _, buf)) = wl.as_mut() {
                    buf.push_str(&t);
                    continue;
                }
                if in_code > 0 {
                    continue;
                }
                for tag in inline_tags(&t) {
                    push_unique(&mut p.tags, tag);
                }
                for m in URL_RE.find_iter(&t) {
                    let u = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
                    push_unique(&mut p.links, u.to_string());
                }
            }
            Event::Code(t) => {
                if in_h1 && let Some(h) = h1.as_mut() {
                    h.push_str(&t);
                }
            }
            _ => {}
        }
    }
    if p.title.is_none() {
        p.title = h1.map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: On Recollection\naliases: [anamnesis, \"meno note\"]\ntags:\n  - philosophy\n  - '#plato'\ncreated: 2024-03-05\n---\n# Heading ignored for title\n\nLearning is [[Recollection|remembering]] per [[Meno#The slave boy]].\nSee ![[diagram.png]] and [[folder/Other Note]]. #idea #2024 #nested/tag\n\n```\n#notatag [[notalink]]\n```\n\nAlso `#nottag` and [site](https://example.com/a) and https://plato.stanford.edu/entries/plato.\n";

    #[test]
    fn parses_obsidian_note() {
        let p = parse(NOTE);
        assert_eq!(p.title.as_deref(), Some("On Recollection"));
        assert_eq!(p.aliases, vec!["anamnesis", "meno note"]);
        assert_eq!(p.tags, vec!["philosophy", "plato", "idea", "nested/tag"]);
        assert_eq!(p.created.unwrap().to_rfc3339(), "2024-03-05T00:00:00+00:00");
        assert!(p.body.starts_with("# Heading"));
        let targets: Vec<_> = p.wikilinks.iter().map(|w| (w.target.as_str(), w.heading.as_deref(), w.alias.as_deref(), w.embed)).collect();
        assert_eq!(
            targets,
            vec![
                ("Recollection", None, Some("remembering"), false),
                ("Meno", Some("The slave boy"), None, false),
                ("diagram.png", None, None, true),
                ("folder/Other Note", None, None, false),
            ]
        );
        assert_eq!(p.links, vec!["https://example.com/a", "https://plato.stanford.edu/entries/plato"]);
    }

    #[test]
    fn title_from_h1_without_frontmatter() {
        let p = parse("# The `Meno`\n\ntext #tag");
        assert_eq!(p.title.as_deref(), Some("The Meno"));
        assert_eq!(p.frontmatter, Value::Null);
        assert_eq!(p.tags, vec!["tag"]);
    }

    #[test]
    fn frontmatter_edge_cases() {
        assert_eq!(split_frontmatter("no fm").0, None);
        assert_eq!(split_frontmatter("---\nunterminated").0, None);
        let p = parse("---\ntags: a, b c\n---\nbody");
        assert_eq!(p.tags, vec!["a", "b", "c"]);
        let p = parse("---\nkey: [unclosed\n---\nbody");
        assert!(p.frontmatter_error.is_some());
        assert_eq!(p.body, "body");
        assert!(parse_date("2024-01-02 10:30").is_some());
        assert!(parse_date("2024-01-02T10:30:00+02:00").is_some());
        assert!(parse_date("someday").is_none());
    }

    #[test]
    fn tags_are_not_anchors_or_numbers() {
        let tags: Vec<_> = inline_tags("issue #12 a#b #ok-tag/ (#paren) &#39;").collect();
        assert_eq!(tags, vec!["ok-tag", "paren"]);
    }
}
