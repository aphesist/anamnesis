//! Reference extraction and resolution keys.
//!
//! A written reference (`[[Title]]`, `[[Title|shown]]`, `[[Title#heading]]`, `![[embed]]`,
//! `[text](../dir/note.md)`, a url) is turned into a list of normalised *candidate keys*. Every
//! item publishes *item keys* (its title, aliases, uri and path forms of its external id). A
//! reference resolves to the item whose key matches, preferring the referring item's own
//! source, then the earliest candidate, then the oldest item id.

use crate::model::Item;

const STRIP_EXTS: [&str; 3] = [".md", ".markdown", ".txt"];

/// Extract outgoing references from Markdown text: wikilinks (including embeds) and the
/// targets of inline Markdown links. Anchors (`#...`) and `mailto:` links are skipped.
/// Returned in order of first appearance, deduplicated.
pub fn extract_refs(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    };
    // [[wikilinks]]
    let mut rest = body;
    while let Some(i) = rest.find("[[") {
        let after = &rest[i + 2..];
        match after.find("]]") {
            Some(j) if !after[..j].contains('\n') => {
                push(&after[..j]);
                rest = &after[j + 2..];
            }
            _ => rest = after,
        }
    }
    // [text](target)
    let mut rest = body;
    while let Some(i) = rest.find("](") {
        let after = &rest[i + 2..];
        match after.find(')') {
            Some(j) if !after[..j].contains('\n') => {
                let target = after[..j].split_whitespace().next().unwrap_or("");
                let target = target.trim_matches(['<', '>']);
                if !target.starts_with('#') && !target.starts_with("mailto:") {
                    push(target);
                }
                rest = &after[j + 1..];
            }
            _ => rest = after,
        }
    }
    out
}

/// Normalise a key: lowercase, `\` → `/`, collapsed whitespace, no leading `./` or `/`.
pub(crate) fn norm_key(s: &str) -> String {
    let lowered = s.trim().to_lowercase().replace('\\', "/");
    let mut collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        if let Some(r) = collapsed.strip_prefix("./") {
            collapsed = r.to_string();
        } else if let Some(r) = collapsed.strip_prefix('/') {
            collapsed = r.to_string();
        } else {
            break;
        }
    }
    collapsed
}

fn strip_ext(s: &str) -> &str {
    let lower = s.to_lowercase();
    for ext in STRIP_EXTS {
        if lower.ends_with(ext) {
            return &s[..s.len() - ext.len()];
        }
    }
    s
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn dirname(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(i) => &path[..i],
        None => "",
    }
}

/// Join `rel` onto directory `dir`, resolving `.` and `..`.
fn join_path(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if rel.starts_with('/') {
        Vec::new()
    } else {
        dir.split(['/', '\\']).filter(|p| !p.is_empty()).collect()
    };
    for p in rel.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                16,
            )
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_url(s: &str) -> bool {
    s.contains("://")
}

/// Strip wikilink decoration: `![[a#h|b]]` → `a`. Returns `None` for pure anchors.
fn clean_ref(raw: &str) -> Option<String> {
    let mut r = raw.trim();
    r = r.strip_prefix('!').unwrap_or(r);
    r = r.strip_prefix("[[").unwrap_or(r);
    r = r.strip_suffix("]]").unwrap_or(r);
    if is_url(r) {
        return Some(r.trim().to_string());
    }
    let r = r.split('|').next().unwrap_or("");
    let r = r.split('#').next().unwrap_or("");
    let r = r.split('^').next().unwrap_or("");
    let r = percent_decode(r.trim());
    let r = r.trim();
    (!r.is_empty()).then(|| r.to_string())
}

/// Candidate keys for a reference written inside the item with external id `from_external_id`,
/// most specific first.
pub(crate) fn ref_candidates(from_external_id: &str, raw: &str) -> Vec<String> {
    let Some(r) = clean_ref(raw) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    if is_url(&r) {
        push(norm_key(&r));
        return out;
    }
    if r.contains('/') || r.contains('\\') || r.starts_with('.') {
        let joined = join_path(dirname(from_external_id), &r);
        push(norm_key(&joined));
        push(norm_key(strip_ext(&joined)));
    }
    push(norm_key(&r));
    push(norm_key(strip_ext(&r)));
    out
}

fn aliases(meta: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for v in [
        meta.get("aliases"),
        meta.get("frontmatter").and_then(|f| f.get("aliases")),
    ]
    .into_iter()
    .flatten()
    {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => {
                out.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string)))
            }
            _ => {}
        }
    }
    out
}

/// Keys under which an item can be referenced.
pub(crate) fn item_keys(item: &Item) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push(norm_key(&item.title));
    for a in aliases(&item.meta) {
        push(norm_key(&a));
    }
    if let Some(uri) = &item.uri {
        push(norm_key(uri));
    }
    let eid = item.external_id.as_str();
    if is_url(eid) {
        push(norm_key(eid));
    } else {
        push(norm_key(eid));
        push(norm_key(strip_ext(eid)));
        push(norm_key(basename(eid)));
        push(norm_key(strip_ext(basename(eid))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_wikilinks_and_md_links() {
        let body = "see [[Alpha]] and [[Beta#h|b]], ![[img.png]], [x](../notes/c.md \"t\"), [y](#anchor), [z](https://e.org/a)";
        assert_eq!(
            extract_refs(body),
            vec![
                "Alpha",
                "Beta#h|b",
                "img.png",
                "../notes/c.md",
                "https://e.org/a"
            ]
        );
    }

    #[test]
    fn candidates() {
        assert_eq!(
            ref_candidates("a/b/n.md", "[[Foo Bar#x|y]]"),
            vec!["foo bar"]
        );
        assert_eq!(
            ref_candidates("a/b/n.md", "../c/My%20Note.md"),
            vec![
                "a/c/my note.md",
                "a/c/my note",
                "../c/my note.md",
                "../c/my note"
            ]
        );
        assert!(ref_candidates("n.md", "#only").is_empty());
    }
}
