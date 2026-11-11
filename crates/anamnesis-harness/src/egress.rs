//! The egress gate: before any non-local model call, check every item's privacy flag.
//!
//! `LocalOnly` items are refused outright. `Ask` items need the user's consent for this
//! call (pass their ids as approved). The resulting record converts into
//! `EventKind::Egress` — counts and ids only, never text.

use std::collections::HashSet;

use anamnesis_core::{EventKind, Id, Privacy};
use serde::{Deserialize, Serialize};

use crate::error::{HarnessError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EgressRecord {
    pub provider: String,
    pub model: String,
    pub purpose: String,
    pub items: Vec<Id>,
    /// Characters sent (system prompt + messages).
    pub chars: u64,
}

impl EgressRecord {
    /// Fill in the call details (the gate itself only knows the items).
    pub fn for_call(mut self, provider: &str, model: &str, purpose: &str, chars: u64) -> Self {
        self.provider = provider.to_string();
        self.model = model.to_string();
        self.purpose = purpose.to_string();
        self.chars = chars;
        self
    }

    pub fn into_event_kind(self) -> EventKind {
        EventKind::Egress {
            provider: self.provider,
            model: self.model,
            purpose: self.purpose,
            items: self.items,
            chars: self.chars,
        }
    }
}

impl From<EgressRecord> for EventKind {
    fn from(r: EgressRecord) -> Self {
        r.into_event_kind()
    }
}

/// Check items for a non-local call. `Ask` items count as refused (needs consent).
pub fn egress_check(items: &[(Id, Privacy)]) -> Result<EgressRecord> {
    egress_check_with_consent(items, &[])
}

/// Like `egress_check`, with ids the user approved for this call.
pub fn egress_check_with_consent(items: &[(Id, Privacy)], approved: &[Id]) -> Result<EgressRecord> {
    let local_only: Vec<Id> = items
        .iter()
        .filter(|(_, p)| *p == Privacy::LocalOnly)
        .map(|(i, _)| i.clone())
        .collect();
    if !local_only.is_empty() {
        return Err(HarnessError::EgressLocalOnly(local_only));
    }
    let ok: HashSet<&Id> = approved.iter().collect();
    let needs: Vec<Id> = items
        .iter()
        .filter(|(i, p)| *p == Privacy::Ask && !ok.contains(i))
        .map(|(i, _)| i.clone())
        .collect();
    if !needs.is_empty() {
        return Err(HarnessError::EgressNeedsConsent(needs));
    }
    let mut seen = HashSet::new();
    let ids = items
        .iter()
        .filter(|(i, _)| seen.insert(i.clone()))
        .map(|(i, _)| i.clone())
        .collect();
    Ok(EgressRecord {
        provider: String::new(),
        model: String::new(),
        purpose: String::new(),
        items: ids,
        chars: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Id {
        Id(s.to_string())
    }

    #[test]
    fn local_only_refused() {
        let r = egress_check(&[(id("a"), Privacy::CloudOk), (id("b"), Privacy::LocalOnly)]);
        match r {
            Err(HarnessError::EgressLocalOnly(v)) => assert_eq!(v, vec![id("b")]),
            other => panic!("{other:?}"),
        }
        // consent cannot override local-only
        assert!(egress_check_with_consent(&[(id("b"), Privacy::LocalOnly)], &[id("b")]).is_err());
    }

    #[test]
    fn ask_needs_consent() {
        let items = [(id("a"), Privacy::Ask), (id("c"), Privacy::CloudOk)];
        assert!(matches!(
            egress_check(&items),
            Err(HarnessError::EgressNeedsConsent(_))
        ));
        let rec = egress_check_with_consent(&items, &[id("a")]).unwrap();
        assert_eq!(rec.items, vec![id("a"), id("c")]);
    }

    #[test]
    fn converts_to_event_without_text() {
        let rec = egress_check(&[(id("a"), Privacy::CloudOk), (id("a"), Privacy::CloudOk)])
            .unwrap()
            .for_call("anthropic", "claude-haiku-5-5", "label", 1234);
        match rec.into_event_kind() {
            EventKind::Egress {
                provider,
                model,
                purpose,
                items,
                chars,
            } => {
                assert_eq!(
                    (provider.as_str(), model.as_str(), purpose.as_str()),
                    ("anthropic", "claude-haiku-5-5", "label")
                );
                assert_eq!(items, vec![id("a")]);
                assert_eq!(chars, 1234);
            }
            _ => unreachable!(),
        }
    }
}
