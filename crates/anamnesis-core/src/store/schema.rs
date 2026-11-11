//! Versioned schema migrations (tracked in `PRAGMA user_version`).

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Tables that are pure projections of the event log. `rebuild_projections` clears these and
/// replays the log. Everything else is either the log itself (`events`) or a cache that is not
/// derivable from it (`embeddings`, `placements`).
pub(crate) const PROJECTION_TABLES: &[&str] = &[
    "sources",
    "items",
    "item_keys",
    "item_refs",
    "chunks",
    "chunks_fts",
    "links",
    "suggestions",
    "threads",
    "regions",
    "region_runs",
    "layout_runs",
    "resurfacings",
    "retrieval_labels",
    "chat_turns",
    "settings",
];

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
CREATE TABLE events (
    seq        INTEGER PRIMARY KEY,
    at         TEXT NOT NULL,
    actor      TEXT NOT NULL,
    kind_type  TEXT NOT NULL,
    kind       TEXT NOT NULL,
    prev_hash  TEXT NOT NULL,
    hash       TEXT NOT NULL
);
CREATE INDEX events_kind_type ON events(kind_type, seq);

CREATE TABLE sources (
    id         TEXT PRIMARY KEY,
    connector  TEXT NOT NULL,
    json       TEXT NOT NULL
);

CREATE TABLE items (
    id           TEXT PRIMARY KEY,
    source_id    TEXT NOT NULL,
    external_id  TEXT NOT NULL,
    kind         TEXT NOT NULL,
    origin       TEXT NOT NULL,
    title        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    modified_at  TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    deleted      INTEGER NOT NULL,
    json         TEXT NOT NULL,
    UNIQUE (source_id, external_id)
);
CREATE INDEX items_hash ON items(content_hash);
CREATE INDEX items_created ON items(created_at);

CREATE TABLE item_keys (
    key      TEXT NOT NULL,
    item_id  TEXT NOT NULL,
    PRIMARY KEY (key, item_id)
);
CREATE INDEX item_keys_item ON item_keys(item_id);

CREATE TABLE item_refs (
    item_id  TEXT NOT NULL,
    ord      INTEGER NOT NULL,
    ref      TEXT NOT NULL,
    cand     INTEGER NOT NULL,
    key      TEXT NOT NULL,
    PRIMARY KEY (item_id, ord, cand)
);
CREATE INDEX item_refs_key ON item_refs(key);

CREATE TABLE chunks (
    rowid      INTEGER PRIMARY KEY,
    id         TEXT NOT NULL UNIQUE,
    item_id    TEXT NOT NULL,
    ord        INTEGER NOT NULL,
    span_start INTEGER NOT NULL,
    span_end   INTEGER NOT NULL,
    text       TEXT NOT NULL,
    text_hash  TEXT NOT NULL
);
CREATE INDEX chunks_item ON chunks(item_id, ord);
CREATE INDEX chunks_hash ON chunks(text_hash);

CREATE VIRTUAL TABLE chunks_fts USING fts5(
    title, text,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE embeddings (
    text_hash  TEXT NOT NULL,
    model_id   TEXT NOT NULL,
    dim        INTEGER NOT NULL,
    vec        BLOB NOT NULL,
    PRIMARY KEY (text_hash, model_id)
);

CREATE TABLE links (
    from_id    TEXT NOT NULL,
    to_id      TEXT NOT NULL,
    kind       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (from_id, to_id, kind)
);
CREATE INDEX links_to ON links(to_id);

CREATE TABLE suggestions (
    id          TEXT PRIMARY KEY,
    from_id     TEXT NOT NULL,
    to_id       TEXT NOT NULL,
    score       REAL NOT NULL,
    rank        INTEGER NOT NULL,
    shown       INTEGER NOT NULL,
    model       TEXT NOT NULL,
    made_at     TEXT NOT NULL,
    status      TEXT NOT NULL,
    decided_at  TEXT
);
CREATE INDEX suggestions_from ON suggestions(from_id);

CREATE TABLE threads (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    status      TEXT NOT NULL,
    item_id     TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    json        TEXT NOT NULL
);
CREATE INDEX threads_item ON threads(item_id);

CREATE TABLE regions (
    id      TEXT PRIMARY KEY,
    run_id  TEXT NOT NULL,
    json    TEXT NOT NULL
);

CREATE TABLE region_runs (
    run_id       TEXT PRIMARY KEY,
    seq          INTEGER NOT NULL,
    at           TEXT NOT NULL,
    regions      TEXT NOT NULL,
    transitions  TEXT NOT NULL
);

CREATE TABLE layout_runs (
    run_id           TEXT PRIMARY KEY,
    seq              INTEGER NOT NULL,
    at               TEXT NOT NULL,
    placements_hash  TEXT NOT NULL,
    count            INTEGER NOT NULL
);

CREATE TABLE placements (
    item_id    TEXT PRIMARY KEY,
    run_id     TEXT NOT NULL,
    x          REAL NOT NULL,
    y          REAL NOT NULL,
    z          REAL NOT NULL,
    region_id  TEXT
);

CREATE TABLE resurfacings (
    id        TEXT PRIMARY KEY,
    item_id   TEXT NOT NULL,
    arm       TEXT NOT NULL,
    shown_at  TEXT NOT NULL,
    useful    INTEGER,
    rated_at  TEXT
);

CREATE TABLE retrieval_labels (
    query    TEXT NOT NULL,
    item_id  TEXT NOT NULL,
    grade    INTEGER NOT NULL,
    at       TEXT NOT NULL,
    PRIMARY KEY (query, item_id)
);

CREATE TABLE chat_turns (
    seq      INTEGER PRIMARY KEY,
    chat_id  TEXT NOT NULL,
    role     TEXT NOT NULL,
    item_id  TEXT NOT NULL,
    cited    TEXT NOT NULL,
    at       TEXT NOT NULL
);
CREATE INDEX chat_turns_chat ON chat_turns(chat_id, seq);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
"#,
    // v2: when a suggestion was first put in front of the user (`SuggestionDisplayed`)
    r#"
ALTER TABLE suggestions ADD COLUMN displayed_at TEXT;
"#,
];

/// Latest schema version this build knows.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Bring the schema up to [`SCHEMA_VERSION`].
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
