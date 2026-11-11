//! HTML helpers: html → markdown-ish text, and readability-style main-content extraction.
//!
//! The extractor is a small heuristic in the spirit of Arc90 readability: paragraphs score
//! their parent (fully) and grandparent (half) by text length and comma count; candidates
//! whose class/id looks like chrome (nav, footer, comments, share...) are penalised, link-heavy
//! ones are discounted, `<article>` / `<main>` get a bonus. The winning subtree is serialised
//! without scripts, styles, forms, navigation and similar, then rendered to text.

use std::collections::HashMap;
use std::sync::LazyLock;

use ego_tree::NodeId;
use html2text::render::{TaggedLine, TextDecorator};
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

/// Markdown-flavoured decorator: headings `#`, emphasis `*`/`**`, code backticks, quotes
/// `> `, list items `- `; links reduced to their text, images to their alt text.
#[derive(Clone, Debug, Default)]
struct MdDecorator;

impl TextDecorator for MdDecorator {
    type Annotation = ();
    fn decorate_link_start(&mut self, _url: &str) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_link_end(&mut self) -> String {
        String::new()
    }
    fn decorate_em_start(&self) -> (String, ()) {
        ("*".into(), ())
    }
    fn decorate_em_end(&self) -> String {
        "*".into()
    }
    fn decorate_strong_start(&self) -> (String, ()) {
        ("**".into(), ())
    }
    fn decorate_strong_end(&self) -> String {
        "**".into()
    }
    fn decorate_strikeout_start(&self) -> (String, ()) {
        ("~~".into(), ())
    }
    fn decorate_strikeout_end(&self) -> String {
        "~~".into()
    }
    fn decorate_code_start(&self) -> (String, ()) {
        ("`".into(), ())
    }
    fn decorate_code_end(&self) -> String {
        "`".into()
    }
    fn decorate_preformat_first(&self) {}
    fn decorate_preformat_cont(&self) {}
    fn decorate_image(&mut self, _src: &str, title: &str) -> (String, ()) {
        (title.to_string(), ())
    }
    fn header_prefix(&self, level: usize) -> String {
        "#".repeat(level) + " "
    }
    fn quote_prefix(&self) -> String {
        "> ".into()
    }
    fn unordered_item_prefix(&self) -> String {
        "- ".into()
    }
    fn ordered_item_prefix(&self, i: i64) -> String {
        format!("{i}. ")
    }
    fn make_subblock_decorator(&self) -> Self {
        MdDecorator
    }
    fn finalise(&mut self, _urls: Vec<String>) -> Vec<TaggedLine<()>> {
        Vec::new()
    }
}

/// Render HTML as markdown-flavoured plain text, no wrapping, links reduced to their text.
pub fn html_to_text(html: &str) -> String {
    let out = html2text::config::with_decorator(MdDecorator)
        .link_footnotes(false)
        .no_link_wrapping()
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_else(|_| strip_tags(html));
    tidy(&out)
}

/// Collapse 3+ newlines, trim trailing spaces.
pub fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let l = line.trim_end();
        if l.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(l);
    }
    out
}

fn strip_tags(html: &str) -> String {
    let doc = Html::parse_fragment(html);
    doc.root_element().text().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extracted {
    pub title: Option<String>,
    /// `<link rel=canonical>` / `og:url`, absolute.
    pub canonical: Option<String>,
    pub author: Option<String>,
    pub site_name: Option<String>,
    pub published: Option<String>,
    pub modified: Option<String>,
    pub description: Option<String>,
    pub lang: Option<String>,
    pub og_type: Option<String>,
    /// Cleaned html of the main content.
    pub content_html: String,
    /// Main content as markdown-ish text.
    pub text: String,
    /// Absolute http(s) links inside the main content.
    pub links: Vec<String>,
    /// The page has an `<article>` element or `og:type=article`.
    pub is_article: bool,
}

static UNLIKELY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(nav|navbar|menu|footer|header|sidebar|side-bar|comment|comments|share|sharing|social|related|promo|advert|ads?|banner|cookie|consent|newsletter|subscribe|popup|modal|breadcrumbs?|pagination|skip|masthead|widget|sponsor)\b").unwrap()
});
static LIKELY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(article|content|entry|post|main|body|text|story|prose|markdown)\b").unwrap()
});

const DROP_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "iframe", "form", "button", "input", "select",
    "textarea", "nav", "footer", "aside", "svg", "canvas", "object", "embed", "dialog", "head",
    "link", "meta",
];
const VOID: &[&str] = &["img", "br", "hr", "wbr", "source", "col"];
const KEEP_ATTRS: &[&str] = &["href", "src", "alt", "title"];

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn class_id(e: &ElementRef) -> String {
    let v = e.value();
    format!(
        "{} {}",
        v.attr("class").unwrap_or(""),
        v.attr("id").unwrap_or("")
    )
}

fn is_unlikely(e: &ElementRef) -> bool {
    let ci = class_id(e);
    let role = e.value().attr("role").unwrap_or("");
    (UNLIKELY.is_match(&ci) && !LIKELY.is_match(&ci))
        || matches!(
            role,
            "navigation" | "banner" | "contentinfo" | "complementary" | "dialog"
        )
        || e.value().attr("aria-hidden") == Some("true")
        || e.value().attr("hidden").is_some()
}

fn meta_content(doc: &Html, selectors: &[&str]) -> Option<String> {
    for s in selectors {
        if let Some(e) = doc.select(&sel(s)).next() {
            let v = e
                .value()
                .attr("content")
                .or_else(|| e.value().attr("href"))
                .map(str::trim)
                .unwrap_or("");
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn text_of(e: &ElementRef) -> String {
    let mut s = String::new();
    collect_text(e, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text(e: &ElementRef, out: &mut String) {
    for c in e.children() {
        match c.value() {
            Node::Text(t) => {
                out.push_str(t);
                out.push(' ');
            }
            Node::Element(el) if !DROP_TAGS.contains(&el.name()) => {
                if let Some(ce) = ElementRef::wrap(c) {
                    collect_text(&ce, out);
                }
            }
            _ => {}
        }
    }
}

fn link_density(e: &ElementRef) -> f64 {
    let total = text_of(e).len().max(1);
    let a = sel("a");
    let linked: usize = e.select(&a).map(|l| text_of(&l).len()).sum();
    (linked as f64 / total as f64).min(1.0)
}

fn escape(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

fn serialise_clean(
    e: &ElementRef,
    base: &Url,
    out: &mut String,
    links: &mut Vec<String>,
    is_root: bool,
) {
    let name = e.value().name();
    if !is_root && (DROP_TAGS.contains(&name) || is_unlikely(e)) {
        return;
    }
    out.push('<');
    out.push_str(name);
    for (k, v) in e.value().attrs() {
        if !KEEP_ATTRS.contains(&k) {
            continue;
        }
        let v = if k == "href" || k == "src" {
            match base.join(v) {
                Ok(u) => {
                    if k == "href" && matches!(u.scheme(), "http" | "https") {
                        let s = crate::url_norm::normalize_url(u.as_str())
                            .unwrap_or_else(|_| u.to_string());
                        if !links.contains(&s) {
                            links.push(s);
                        }
                    }
                    u.to_string()
                }
                Err(_) => v.to_string(),
            }
        } else {
            v.to_string()
        };
        out.push_str(&format!(" {k}=\"{}\"", escape(&v, true)));
    }
    out.push('>');
    if VOID.contains(&name) {
        return;
    }
    for c in e.children() {
        match c.value() {
            Node::Text(t) => out.push_str(&escape(t, false)),
            Node::Element(_) => {
                if let Some(ce) = ElementRef::wrap(c) {
                    serialise_clean(&ce, base, out, links, false);
                }
            }
            _ => {}
        }
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

/// Pick the element most likely to hold the main content.
fn best_candidate<'a>(doc: &'a Html) -> Option<ElementRef<'a>> {
    let mut scores: HashMap<NodeId, f64> = HashMap::new();
    let para = sel("p, pre, blockquote, li, td, h2, h3");
    for p in doc.select(&para) {
        if p.ancestors()
            .filter_map(ElementRef::wrap)
            .any(|a| DROP_TAGS.contains(&a.value().name()) || is_unlikely(&a))
        {
            continue;
        }
        let t = text_of(&p);
        if t.len() < 25 {
            continue;
        }
        let mut s = 1.0 + t.matches(',').count() as f64 + (t.len() as f64 / 100.0).min(3.0);
        if p.value().name() == "li" || p.value().name() == "td" {
            s *= 0.5;
        }
        let mut anc = p.ancestors().filter_map(ElementRef::wrap);
        if let Some(parent) = anc.next() {
            *scores.entry(parent.id()).or_default() += s;
        }
        if let Some(gp) = anc.next() {
            *scores.entry(gp.id()).or_default() += s / 2.0;
        }
    }
    let mut best: Option<(ElementRef<'a>, f64)> = None;
    for (id, s) in scores {
        let Some(e) = doc.tree.get(id).and_then(ElementRef::wrap) else {
            continue;
        };
        if matches!(e.value().name(), "html" | "body") {
            continue;
        }
        let ci = class_id(&e);
        let mut score = s;
        if LIKELY.is_match(&ci) {
            score *= 1.25;
        }
        if matches!(e.value().name(), "article" | "main") || e.value().attr("role") == Some("main")
        {
            score *= 1.5;
        }
        score *= 1.0 - link_density(&e);
        if best.as_ref().is_none_or(|(_, b)| score > *b) {
            best = Some((e, score));
        }
    }
    let best = best.map(|(e, _)| e);
    // A paragraph-heavy element inside an <article> is usually the article minus its header:
    // widen to the article when it is not much larger (keeps headings and lede).
    if let Some(b) = best {
        if let Some(art) = b
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|a| a.value().name() == "article")
        {
            return Some(art);
        }
        return Some(b);
    }
    doc.select(&sel("article, main, [role=main]"))
        .next()
        .or_else(|| doc.select(&sel("body")).next())
}

/// Extract metadata and main content from a page fetched from `base`.
pub fn extract(html: &str, base: &Url) -> Extracted {
    let doc = Html::parse_document(html);
    let mut x = Extracted {
        title: meta_content(
            &doc,
            &["meta[property='og:title']", "meta[name='twitter:title']"],
        )
        .or_else(|| {
            doc.select(&sel("title"))
                .next()
                .map(|t| text_of(&t))
                .filter(|s| !s.is_empty())
        }),
        author: meta_content(
            &doc,
            &["meta[name='author']", "meta[property='article:author']"],
        )
        .or_else(|| {
            doc.select(&sel("[rel=author], .byline, .author"))
                .next()
                .map(|e| text_of(&e))
                .filter(|s| !s.is_empty())
        }),
        site_name: meta_content(&doc, &["meta[property='og:site_name']"]),
        published: meta_content(
            &doc,
            &[
                "meta[property='article:published_time']",
                "meta[name='date']",
                "meta[itemprop='datePublished']",
            ],
        )
        .or_else(|| {
            doc.select(&sel("time[datetime]"))
                .next()
                .and_then(|t| t.value().attr("datetime").map(str::to_string))
        }),
        modified: meta_content(
            &doc,
            &[
                "meta[property='article:modified_time']",
                "meta[itemprop='dateModified']",
            ],
        ),
        description: meta_content(
            &doc,
            &[
                "meta[property='og:description']",
                "meta[name='description']",
            ],
        ),
        lang: doc
            .select(&sel("html"))
            .next()
            .and_then(|h| h.value().attr("lang").map(str::to_string)),
        og_type: meta_content(&doc, &["meta[property='og:type']"]),
        canonical: meta_content(&doc, &["link[rel='canonical']", "meta[property='og:url']"])
            .and_then(|c| base.join(&c).ok())
            .map(|u| u.to_string()),
        ..Default::default()
    };
    if let Some(t) = &x.title
        && t.is_empty()
    {
        x.title = None;
    }
    x.is_article =
        x.og_type.as_deref() == Some("article") || doc.select(&sel("article")).next().is_some();
    if let Some(root) = best_candidate(&doc) {
        let mut out = String::new();
        let mut links = Vec::new();
        serialise_clean(&root, base, &mut out, &mut links, true);
        x.text = html_to_text(&out);
        x.content_html = out;
        x.links = links;
    }
    if x.title.is_none() {
        x.title = doc
            .select(&sel("h1"))
            .next()
            .map(|h| text_of(&h))
            .filter(|s| !s.is_empty());
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text_keeps_structure() {
        let t = html_to_text(
            "<h1>Title</h1><p>Hello <b>world</b> and <a href='x'>a link</a>.</p><ul><li>one</li><li>two</li></ul>",
        );
        assert!(t.contains("# Title"), "{t}");
        assert!(t.contains("Hello **world** and a link."), "{t}");
        assert!(t.contains("one") && t.contains("two"), "{t}");
        assert!(!t.contains("[1]"), "{t}");
    }
}
