use super::rates::Rates;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Catalog {
    pub retrieved_at: Option<String>,
    pub models: BTreeMap<String, Rates>,
}
impl Catalog {
    pub fn compact(data: &[u8]) -> Result<Self, String> {
        let mut catalog: Self = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        catalog.models.retain(|_, rates| rates.valid());
        if catalog.models.is_empty() {
            return Err("No usable model prices".into());
        }
        Ok(catalog)
    }
    pub fn merged(mut self, other: Self) -> Self {
        self.models.extend(other.models);
        self.retrieved_at = other.retrieved_at.or(self.retrieved_at);
        self
    }
    pub fn fuzzy(&self, model: &str) -> Option<(&str, Rates)> {
        self.models
            .iter()
            .filter(|(key, _)| matches_key(key, model))
            .max_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| b.cmp(a)))
            .map(|(key, rates)| (key.as_str(), *rates))
    }
    pub fn litellm(data: &[u8]) -> Result<Self, String> {
        let root: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        let mut catalog = Self::default();
        for (key, entry) in root.as_object().ok_or("Invalid LiteLLM feed")? {
            let (Some(input), Some(output)) = (
                number(entry, "input_cost_per_token"),
                number(entry, "output_cost_per_token"),
            ) else {
                continue;
            };
            let cache_read = number(entry, "cache_read_input_token_cost");
            let rates = Rates {
                i: input * 1e6,
                o: output * 1e6,
                cw: number(entry, "cache_creation_input_token_cost").unwrap_or(input) * 1e6,
                cr: cache_read.unwrap_or(input * 0.1) * 1e6,
                cre: cache_read.is_some(),
                ia: number(entry, "input_cost_per_token_above_200k_tokens").map(|n| n * 1e6),
                oa: number(entry, "output_cost_per_token_above_200k_tokens").map(|n| n * 1e6),
                cwa: number(entry, "cache_creation_input_token_cost_above_200k_tokens")
                    .map(|n| n * 1e6),
                cra: number(entry, "cache_read_input_token_cost_above_200k_tokens")
                    .map(|n| n * 1e6),
                fast: number(&entry["provider_specific_entry"], "fast").unwrap_or(1.),
            };
            if rates.valid() {
                catalog.models.insert(key.clone(), rates);
            }
        }
        catalog.checked()
    }
    pub fn models_dev(data: &[u8]) -> Result<Self, String> {
        let root: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        let mut catalog = Self::default();
        let sorted = root
            .as_object()
            .ok_or("Invalid models.dev feed")?
            .iter()
            .collect::<BTreeMap<_, _>>();
        for provider in sorted.values() {
            let Some(models) = provider["models"].as_object() else {
                continue;
            };
            for (id, model) in models {
                if catalog.models.contains_key(id) {
                    continue;
                }
                let cost = &model["cost"];
                let (Some(input), Some(output)) = (number(cost, "input"), number(cost, "output"))
                else {
                    continue;
                };
                let cache_read = number(cost, "cache_read");
                let rates = Rates {
                    i: input,
                    o: output,
                    cw: number(cost, "cache_write").unwrap_or(input),
                    cr: cache_read.unwrap_or(input * 0.1),
                    cre: cache_read.is_some(),
                    ia: None,
                    oa: None,
                    cwa: None,
                    cra: None,
                    fast: 1.,
                };
                if rates.valid() {
                    catalog.models.insert(id.clone(), rates);
                }
            }
        }
        catalog.checked()
    }
    fn checked(self) -> Result<Self, String> {
        if self.models.is_empty() {
            Err("No usable model prices".into())
        } else {
            Ok(self)
        }
    }
}
pub fn number(value: &Value, key: &str) -> Option<f64> {
    value[key].as_f64().filter(|n| n.is_finite() && *n >= 0.)
}
fn normalized(value: &str) -> String {
    value.replace(['.', '@'], "-")
}
pub fn matches_key(key: &str, model: &str) -> bool {
    contains_key(model, key)
        || contains_key(key, model)
        || contains_key(&normalized(model), &normalized(key))
        || contains_key(&normalized(key), &normalized(model))
}
// Boundary-aware lookup: don't confuse Sonnet 4 with Sonnet 4.5. Date suffixes match.
fn contains_key(value: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    value.match_indices(key).any(|(start, _)| {
        let bytes = value.as_bytes();
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            return false;
        }
        let suffix = &bytes[start + key.len()..];
        let Some(first) = suffix.first() else {
            return true;
        };
        if first.is_ascii_alphanumeric() {
            return false;
        }
        if key.as_bytes().last().is_some_and(u8::is_ascii_digit) && matches!(first, b'-' | b'.') {
            let digits = suffix[1..]
                .iter()
                .take_while(|n| n.is_ascii_digit())
                .count();
            if digits > 0
                && !(digits == 8 && suffix.get(9).is_none_or(|n| !n.is_ascii_alphanumeric()))
            {
                return false;
            }
        }
        true
    })
}
