//! Per-item privacy flags.
//!
//! An item's flag comes from, most specific first:
//! 1. the user, by hand (`PUT /api/items/:id {privacy}`), recorded as
//!    `meta.privacy_explicit = true` on the item;
//! 2. the document itself (e.g. frontmatter `privacy:` in a markdown note), carried by the
//!    connector as `meta.privacy`;
//! 3. the source's `default_privacy`.
//!
//! Every change is an `ItemUpdated` event, so the flag replays with the log. Changing a
//! source's default re-flags its items that got their flag from (3).

use anamnesis_connectors::META_PRIVACY;
use anamnesis_core::{Actor, EventKind, Id, Item, ItemFilter, Privacy, Source, Store};
use anyhow::Result;
use serde_json::{Value, json};

/// Item meta key: the user set this item's privacy by hand; nothing re-flags it.
pub const META_PRIVACY_EXPLICIT: &str = "privacy_explicit";

/// True when the user set this item's privacy by hand.
pub fn is_explicit(item: &Item) -> bool {
    item.meta.get(META_PRIVACY_EXPLICIT) == Some(&Value::Bool(true))
}

/// The privacy the document itself states (`meta.privacy`), if any.
pub fn document_privacy(meta: &Value) -> Option<Privacy> {
    meta.get(META_PRIVACY)
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
}

/// The flag an item should carry under the rules above.
pub fn desired_privacy(item: &Item, source: &Source) -> Privacy {
    if is_explicit(item) {
        return item.privacy;
    }
    document_privacy(&item.meta).unwrap_or(source.default_privacy)
}

/// Bring one item's flag in line with [`desired_privacy`]. Returns the updated item when
/// it changed (an `ItemUpdated` event was appended).
pub fn reconcile_item(
    store: &Store,
    item: &Item,
    source: &Source,
    actor: Actor,
) -> Result<Option<Item>> {
    if item.deleted {
        return Ok(None);
    }
    let want = desired_privacy(item, source);
    if want == item.privacy {
        return Ok(None);
    }
    let mut next = item.clone();
    next.privacy = want;
    store.append(actor, EventKind::ItemUpdated { item: next.clone() })?;
    Ok(Some(next))
}

/// Re-flag every live item of a source whose flag was not set by hand (after the
/// source's `default_privacy` changed). Returns how many items changed.
pub fn reflag_source(store: &Store, source: &Source, actor: Actor) -> Result<usize> {
    let items = store.list_items(&ItemFilter {
        source_id: Some(source.id.clone()),
        ..Default::default()
    })?;
    let mut n = 0;
    for it in &items {
        if reconcile_item(store, it, source, actor.clone())?.is_some() {
            n += 1;
        }
    }
    Ok(n)
}

/// Set (or with `None`, clear) an item's privacy by hand. Clearing returns the item to
/// its document's or source's flag.
pub fn set_item_privacy(
    store: &Store,
    id: &Id,
    privacy: Option<Privacy>,
    actor: Actor,
) -> Result<Item> {
    let item = store
        .get_item(id)?
        .filter(|i| !i.deleted)
        .ok_or_else(|| anamnesis_core::Error::NotFound(format!("item {id}")))?;
    let mut next = item.clone();
    if !next.meta.is_object() {
        next.meta = json!({});
    }
    match privacy {
        Some(p) => {
            next.privacy = p;
            next.meta[META_PRIVACY_EXPLICIT] = json!(true);
        }
        None => {
            if let Some(o) = next.meta.as_object_mut() {
                o.remove(META_PRIVACY_EXPLICIT);
            }
            if let Some(src) = store.get_source(&item.source_id)? {
                next.privacy = desired_privacy(&next, &src);
            }
        }
    }
    if next == item {
        return Ok(item);
    }
    store.append(actor, EventKind::ItemUpdated { item: next.clone() })?;
    Ok(next)
}

/// The `privacy.default` setting: the flag new sources get when none is given.
pub fn default_setting(store: &Store) -> Option<Privacy> {
    store
        .get_setting("privacy")
        .ok()
        .flatten()
        .and_then(|v| v.get("default").cloned())
        .and_then(|d| serde_json::from_value(d).ok())
}
