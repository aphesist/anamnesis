//! Usage and cost: per-backend counters and an editable price table.
//!
//! Metered backends show tokens × price. Plan backends (chatgpt, claude cli) show tokens
//! and an api-equivalent estimate, always flagged `estimated`. Local backends show tokens only.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::types::{CostModel, Usage};

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input_per_m: f64,
    pub output_per_m: f64,
}

impl Price {
    pub fn cost(&self, input: u64, output: u64) -> f64 {
        (input as f64 * self.input_per_m + output as f64 * self.output_per_m) / 1_000_000.0
    }
    /// A rough single number for ranking "cheapest" (3:1 input:output blend).
    pub fn blended(&self) -> f64 {
        (3.0 * self.input_per_m + self.output_per_m) / 4.0
    }
}

/// Model id → price. Keys may end in `*` for a prefix match. Users edit this in settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PriceTable(pub BTreeMap<String, Price>);

impl Default for PriceTable {
    fn default() -> Self {
        let mut m = BTreeMap::new();
        // anthropic first-party api rates for prompts up to 100k tokens
        m.insert("claude-haiku-5-5".into(), Price { input_per_m: 0.10, output_per_m: 0.50 });
        Self(m)
    }
}

impl PriceTable {
    pub fn lookup(&self, model: &str) -> Option<Price> {
        if let Some(p) = self.0.get(model) {
            return Some(*p);
        }
        // provider-prefixed ids, e.g. "anthropic/claude-haiku-5-5" on openrouter
        if let Some((_, tail)) = model.rsplit_once('/') {
            if let Some(p) = self.0.get(tail) {
                return Some(*p);
            }
        }
        self.0
            .iter()
            .filter_map(|(k, p)| k.strip_suffix('*').filter(|pre| model.starts_with(pre)).map(|pre| (pre.len(), *p)))
            .max_by_key(|(len, _)| *len)
            .map(|(_, p)| p)
    }

    pub fn set(&mut self, model: impl Into<String>, price: Price) {
        self.0.insert(model.into(), price);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackendUsage {
    pub backend: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Any of the token counts above is an estimate.
    pub tokens_estimated: bool,
    /// Money actually charged per the price table (metered backends).
    pub cost_usd: f64,
    /// Calls whose model had no price in the table.
    pub unpriced_requests: u64,
    /// Api-equivalent value of plan usage (not charged; an estimate).
    pub plan_equivalent_usd: f64,
    pub cost_model: Option<CostModel>,
}

#[derive(Debug, Default)]
pub struct UsageLedger {
    by_backend: Mutex<HashMap<String, BackendUsage>>,
}

impl UsageLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one call's usage; returns the cost attributed to it (charged or plan-equivalent).
    pub fn record(&self, backend: &str, model: &str, cost_model: CostModel, u: &Usage, prices: &PriceTable) -> Option<f64> {
        let mut m = self.by_backend.lock().unwrap();
        let e = m.entry(backend.to_string()).or_insert_with(|| BackendUsage { backend: backend.to_string(), ..Default::default() });
        e.cost_model = Some(cost_model);
        e.requests += 1;
        e.input_tokens += u.input;
        e.output_tokens += u.output;
        e.tokens_estimated |= u.estimated;
        let priced = prices.lookup(model).map(|p| p.cost(u.input, u.output));
        match cost_model {
            CostModel::FreeLocal => Some(0.0),
            CostModel::Metered => {
                if priced.is_none() {
                    e.unpriced_requests += 1;
                }
                if let Some(c) = priced {
                    e.cost_usd += c;
                }
                priced
            }
            CostModel::Subscription => {
                let c = u.reported_cost_usd.or(priced);
                if let Some(c) = c {
                    e.plan_equivalent_usd += c;
                }
                c
            }
        }
    }

    pub fn snapshot(&self) -> Vec<BackendUsage> {
        let mut v: Vec<_> = self.by_backend.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.backend.cmp(&b.backend));
        v
    }

    pub fn reset(&self) {
        self.by_backend.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn haiku_price_math() {
        let t = PriceTable::default();
        let p = t.lookup("claude-haiku-5-5").unwrap();
        // 1M in + 1M out = $0.10 + $0.50
        assert!(close(p.cost(1_000_000, 1_000_000), 0.60));
        // 12_345 in, 678 out
        assert!(close(p.cost(12_345, 678), 12_345.0 * 0.10 / 1e6 + 678.0 * 0.50 / 1e6));
        assert!(t.lookup("anthropic/claude-haiku-5-5").is_some());
        assert!(t.lookup("gpt-unknown").is_none());
    }

    #[test]
    fn prefix_and_edit() {
        let mut t = PriceTable::default();
        t.set("gemini-*", Price { input_per_m: 0.3, output_per_m: 2.5 });
        t.set("gemini-pro*", Price { input_per_m: 1.0, output_per_m: 10.0 });
        assert_eq!(t.lookup("gemini-flash-latest").unwrap().input_per_m, 0.3);
        assert_eq!(t.lookup("gemini-pro-latest").unwrap().input_per_m, 1.0);
        t.set("claude-haiku-5-5", Price { input_per_m: 0.2, output_per_m: 1.0 });
        assert_eq!(t.lookup("claude-haiku-5-5").unwrap().output_per_m, 1.0);
    }

    #[test]
    fn ledger_by_cost_model() {
        let l = UsageLedger::new();
        let t = PriceTable::default();
        let u = Usage { input: 2_000_000, output: 1_000_000, estimated: false, reported_cost_usd: None };
        assert!(close(l.record("anthropic", "claude-haiku-5-5", CostModel::Metered, &u, &t).unwrap(), 0.70));
        assert_eq!(l.record("ollama", "llama", CostModel::FreeLocal, &u, &t), Some(0.0));
        let plan = Usage { input: 10, output: 10, estimated: true, reported_cost_usd: Some(0.02) };
        assert_eq!(l.record("claude-cli", "default", CostModel::Subscription, &plan, &t), Some(0.02));
        assert_eq!(l.record("openrouter", "x/unknown", CostModel::Metered, &u, &t), None);
        let s = l.snapshot();
        let a = s.iter().find(|b| b.backend == "anthropic").unwrap();
        assert!(close(a.cost_usd, 0.70));
        assert!(!a.tokens_estimated);
        let c = s.iter().find(|b| b.backend == "claude-cli").unwrap();
        assert!(c.tokens_estimated);
        assert_eq!(c.cost_usd, 0.0);
        assert!(close(c.plan_equivalent_usd, 0.02));
        assert_eq!(s.iter().find(|b| b.backend == "openrouter").unwrap().unpriced_requests, 1);
    }
}
