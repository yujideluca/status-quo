use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cumulative {
    pub stream: String,
    #[serde(default)]
    pub subsecond: u32,
    pub input: u64,
    pub output: u64,
    pub cache_read: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenRecord {
    pub id: String,
    pub provider: String,
    pub at: i64,
    pub input: u64,
    pub output: u64,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cumulative: Option<Cumulative>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bucket {
    pub at: i64,
    pub input: u64,
    pub output: u64,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenLedger {
    pub records: BTreeMap<String, TokenRecord>,
}
impl TokenLedger {
    pub fn insert(&mut self, record: TokenRecord) -> Result<bool, String> {
        if record.id.is_empty() || record.provider.is_empty() {
            return Err("Missing token identity/provider".into());
        }
        if let Some(old) = self.records.get(&record.id) {
            if old == &record
                || (old.provider == record.provider
                    && old.cumulative.is_some()
                    && old.cumulative == record.cumulative)
                || (old.provider == record.provider
                    && old.input == record.input
                    && old.output == record.output
                    && old.cache_read == record.cache_read
                    && old.cache_write == record.cache_write)
            {
                return Ok(false);
            }
            let regressed = |a: Option<u64>, b: Option<u64>| match (a, b) {
                (Some(a), Some(b)) => b < a,
                (Some(_), None) => true,
                _ => false,
            };
            if old.provider != record.provider
                || record.at < old.at
                || record.input < old.input
                || record.output < old.output
                || regressed(old.cache_read, record.cache_read)
                || regressed(old.cache_write, record.cache_write)
            {
                return Err(format!(
                    "Regressive/colliding token record {} preserved",
                    record.id
                ));
            }
        }
        self.records.insert(record.id.clone(), record);
        Ok(true)
    }
    pub fn prune(&mut self, cutoff: i64) -> usize {
        let before = self.records.len();
        // A compact predecessor is needed to subtract counters crossing retention boundaries.
        type Order = (i64, u32, u64, Option<u64>, u64);
        let mut baselines: BTreeMap<String, (Order, String)> = BTreeMap::new();
        for (id, r) in &self.records {
            if r.at < cutoff {
                if let Some(c) = &r.cumulative {
                    let order = (r.at, c.subsecond, c.input, c.cache_read, c.output);
                    if baselines.get(&c.stream).is_none_or(|(old, _)| *old < order) {
                        baselines.insert(c.stream.clone(), (order, id.clone()));
                    }
                }
            }
        }
        self.records.retain(|id, r| {
            r.at >= cutoff
                || r.cumulative
                    .as_ref()
                    .and_then(|c| baselines.get(&c.stream))
                    .is_some_and(|(_, keep)| keep == id)
        });
        before - self.records.len()
    }
    pub fn buckets(
        &self,
        provider: &str,
        start: i64,
        end: i64,
        step: i64,
    ) -> Result<Vec<Bucket>, String> {
        if step <= 0 || end <= start {
            return Err("Invalid bucket interval".into());
        }
        let mut output: BTreeMap<i64, Bucket> = BTreeMap::new();
        let mut observations: Vec<_> = self
            .records
            .values()
            .filter(|r| r.provider == provider && r.at < end)
            .collect();
        observations.sort_by(|a, b| {
            let order = |r: &TokenRecord| {
                r.cumulative
                    .as_ref()
                    .map(|c| (c.subsecond, c.input, c.cache_read, c.output))
                    .unwrap_or_default()
            };
            (a.at, order(a), &a.id).cmp(&(b.at, order(b), &b.id))
        });
        let mut previous: BTreeMap<&str, &Cumulative> = BTreeMap::new();
        for record in observations {
            let mut normalized = record.clone();
            if let Some(current) = &record.cumulative {
                let mut input = current.input;
                let mut output = current.output;
                let mut cache = current.cache_read;
                if let Some(old) = previous.get(current.stream.as_str()) {
                    let regressed = current.input < old.input
                        || current.output < old.output
                        || matches!((current.cache_read,old.cache_read),(Some(n),Some(p)) if n<p);
                    if regressed
                        && (current.input > old.input
                            || current.output > old.output
                            || matches!((current.cache_read,old.cache_read),(Some(n),Some(p)) if n>p))
                    {
                        return Err("Ambiguous partial cumulative counter regression; graph unavailable rather than double-counted".into());
                    }
                    if !regressed {
                        input -= old.input;
                        output -= old.output;
                        cache = match (current.cache_read, old.cache_read) {
                            (Some(n), Some(p)) => Some(n - p),
                            _ => None,
                        };
                    }
                }
                if cache.is_some_and(|n| n > input) {
                    return Err(
                        "Codex cumulative cache delta exceeds input; graph retained as unavailable"
                            .into(),
                    );
                }
                normalized.input = input - cache.unwrap_or(0);
                normalized.output = output;
                normalized.cache_read = cache;
                previous.insert(current.stream.as_str(), current);
            }
            let r = &normalized;
            if r.at < start {
                continue;
            }
            let at = r.at.div_euclid(step) * step;
            match output.get_mut(&at) {
                None => {
                    output.insert(
                        at,
                        Bucket {
                            at,
                            input: r.input,
                            output: r.output,
                            cache_read: r.cache_read,
                            cache_write: r.cache_write,
                        },
                    );
                }
                Some(b) => {
                    b.input = add(b.input, r.input)?;
                    b.output = add(b.output, r.output)?;
                    b.cache_read = add_optional(b.cache_read, r.cache_read)?;
                    b.cache_write = add_optional(b.cache_write, r.cache_write)?;
                }
            }
        }
        Ok(output.into_values().collect())
    }
}
pub fn add(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b)
        .ok_or_else(|| "Token overflow; counters not silently wrapped".into())
}
pub fn add_optional(a: Option<u64>, b: Option<u64>) -> Result<Option<u64>, String> {
    match (a, b) {
        (Some(a), Some(b)) => Ok(Some(add(a, b)?)),
        _ => Ok(None),
    }
}
pub fn regroup(
    buckets: &[Bucket],
    start: i64,
    step: i64,
    cumulative: bool,
) -> Result<Vec<Bucket>, String> {
    if step <= 0 {
        return Err("Invalid graph interval".into());
    }
    let mut ledger = TokenLedger::default();
    for b in buckets.iter().filter(|b| b.at >= start) {
        ledger.insert(TokenRecord {
            id: b.at.to_string(),
            provider: "graph".into(),
            at: b.at,
            input: b.input,
            output: b.output,
            cache_read: b.cache_read,
            cache_write: b.cache_write,
            cumulative: None,
        })?;
    }
    let mut result = ledger.buckets("graph", start, i64::MAX, step)?;
    if cumulative {
        let mut previous: Option<Bucket> = None;
        for b in &mut result {
            if let Some(p) = previous {
                b.input = add(p.input, b.input)?;
                b.output = add(p.output, b.output)?;
                b.cache_read = add_optional(p.cache_read, b.cache_read)?;
                b.cache_write = add_optional(p.cache_write, b.cache_write)?;
            }
            previous = Some(b.clone());
        }
    }
    Ok(result)
}
