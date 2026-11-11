//! Canonical JSON and the event hash.
//!
//! The hash of an event is `sha256(prev_hash ‖ canonical_json({seq, at, actor, kind}))`, hex
//! encoded. Canonical JSON is compact JSON with object keys sorted by byte order at every
//! level, independent of whether `serde_json` keeps insertion order.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::{Actor, EventKind, Timestamp};

/// The `prev_hash` of the first event.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Serialise a JSON value canonically (sorted keys, no whitespace).
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).expect("string serialises"));
                out.push(':');
                write_canonical(&map[k.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).expect("scalar serialises")),
    }
}

/// sha256 hex of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Compute the hash of an event from its parts.
pub fn event_hash(
    prev_hash: &str,
    seq: u64,
    at: &Timestamp,
    actor: &Actor,
    kind: &EventKind,
) -> Result<String, serde_json::Error> {
    let body = serde_json::json!({
        "seq": seq,
        "at": at,
        "actor": serde_json::to_value(actor)?,
        "kind": serde_json::to_value(kind)?,
    });
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(canonical_json(&body).as_bytes());
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_sorted_recursively() {
        let v: Value =
            serde_json::from_str(r#"{"b":1,"a":{"z":[{"y":1,"x":2}],"c":"é"}}"#).unwrap();
        assert_eq!(
            canonical_json(&v),
            r#"{"a":{"c":"é","z":[{"x":2,"y":1}]},"b":1}"#
        );
    }
}
