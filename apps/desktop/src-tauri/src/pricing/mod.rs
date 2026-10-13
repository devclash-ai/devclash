mod catalog;
mod rates;
mod store;
#[cfg(test)]
mod tests;

use catalog::{number, Catalog};
use fancy_regex::Regex;
use rates::Rates;
pub use rates::TokenBreakdown;
use serde_json::Value;
use std::{collections::HashMap, sync::Mutex};
pub use store::PricingStore;

// Vendored OpenUsage pricing snapshots retain their upstream aliases and rates.
// See resources/README.md for provenance and the refresh procedure.
const SUPPLEMENT: &[u8] = include_bytes!("../../../resources/pricing_supplement.json");
const LITELLM: &[u8] = include_bytes!("../../../resources/pricing_litellm_snapshot.json");
const MODELS_DEV: &[u8] = include_bytes!("../../../resources/pricing_models_dev_snapshot.json");

pub struct Supplement {
    pricing: HashMap<String, Rates>,
    fast: HashMap<String, f64>,
    aliases: Vec<(Regex, String)>,
    updated_at: String,
}
impl Supplement {
    fn decode(data: &[u8]) -> Result<Self, String> {
        let root: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        let mut supplement = Self {
            pricing: HashMap::new(),
            fast: HashMap::new(),
            aliases: Vec::new(),
            updated_at: root["updated_at"].as_str().unwrap_or("").into(),
        };
        if let Some(fast) = root["fast_multipliers"].as_object() {
            for (model, value) in fast {
                if let Some(factor) = value.as_f64().filter(|n| n.is_finite() && *n >= 1.) {
                    supplement.fast.insert(model.clone(), factor);
                }
            }
        }
        for (model, entry) in root["pricing"]
            .as_object()
            .ok_or("Invalid pricing supplement")?
        {
            let (Some(input), Some(output)) = (
                number(entry, "input_per_million"),
                number(entry, "output_per_million"),
            ) else {
                return Err(format!("Invalid rates for {model}"));
            };
            let cache_read = number(entry, "cache_read_per_million");
            supplement.pricing.insert(
                model.clone(),
                Rates {
                    i: input,
                    o: output,
                    cw: number(entry, "cache_write_per_million").unwrap_or(input),
                    cr: cache_read.unwrap_or(input * 0.1),
                    cre: cache_read.is_some(),
                    ia: None,
                    oa: None,
                    cwa: None,
                    cra: None,
                    fast: supplement.fast.get(model).copied().unwrap_or(1.),
                },
            );
        }
        for rule in root["alias_rules"]
            .as_array()
            .ok_or("Missing pricing aliases")?
        {
            let regex = Regex::new(rule["pattern"].as_str().ok_or("Invalid pricing alias")?)
                .map_err(|e| e.to_string())?;
            let canonical = rule["canonical"]
                .as_str()
                .ok_or("Invalid canonical model")?;
            supplement.aliases.push((regex, canonical.into()));
        }
        if supplement.pricing.is_empty() {
            return Err("No usable supplement prices".into());
        }
        Ok(supplement)
    }
    fn canonical(&self, model: &str) -> String {
        for (pattern, canonical) in &self.aliases {
            match pattern.is_match(model) {
                Ok(true) => return canonical.clone(),
                Err(error) => eprintln!("Pricing alias evaluation failed: {error}"),
                _ => {}
            }
        }
        model.into()
    }
    fn fast_multiplier(&self, model: &str) -> Option<f64> {
        self.fast.get(model).copied().or_else(|| {
            self.fast
                .iter()
                .filter(|(key, _)| catalog::matches_key(key, model))
                .max_by_key(|(key, _)| key.len())
                .map(|(_, factor)| *factor)
        })
    }
}

pub struct ModelPricing {
    supplement: Supplement,
    primary: Catalog,
    secondary: Catalog,
    memo: Mutex<HashMap<String, (String, Option<Rates>)>>,
}
impl ModelPricing {
    pub(crate) fn bundled() -> Self {
        Self::from_cached(None, None, None).expect("Bundled OpenUsage prices must be valid")
    }
    fn from_cached(
        supplement: Option<&[u8]>,
        primary: Option<&[u8]>,
        secondary: Option<&[u8]>,
    ) -> Result<Self, String> {
        let bundled = Supplement::decode(SUPPLEMENT)?;
        let supplement = supplement
            .and_then(|bytes| Supplement::decode(bytes).ok())
            .filter(|cached| {
                source_timestamp(&cached.updated_at) >= source_timestamp(&bundled.updated_at)
            })
            .unwrap_or(bundled);
        let primary = primary
            .and_then(|bytes| Catalog::compact(bytes).ok())
            .map_or(Catalog::compact(LITELLM)?, |cached| {
                Catalog::compact(LITELLM).unwrap().merged(cached)
            });
        let secondary = secondary
            .and_then(|bytes| Catalog::compact(bytes).ok())
            .map_or(Catalog::compact(MODELS_DEV)?, |cached| {
                Catalog::compact(MODELS_DEV).unwrap().merged(cached)
            });
        Ok(Self {
            supplement,
            primary,
            secondary,
            memo: Mutex::new(HashMap::new()),
        })
    }
    fn resolved(&self, model: &str) -> (String, Option<Rates>) {
        if let Some(cached) = self.memo.lock().unwrap().get(model).cloned() {
            return cached;
        }
        // Names are an external boundary. Don't evaluate feed regexes on oversized labels.
        if model.len() > 512 {
            return (model.into(), None);
        }
        let canonical = self.supplement.canonical(model);
        let rates = self.lookup(&canonical).or_else(|| {
            if canonical != model {
                self.lookup(model)
            } else {
                None
            }
        });
        self.memo
            .lock()
            .unwrap()
            .insert(model.into(), (canonical.clone(), rates));
        (canonical, rates)
    }
    fn lookup(&self, model: &str) -> Option<Rates> {
        if let Some(rates) = self
            .supplement
            .pricing
            .get(model)
            .or_else(|| self.primary.models.get(model))
        {
            return Some(*rates);
        }
        if let Some(base) = model.strip_suffix("-fast") {
            let entry = self
                .supplement
                .pricing
                .get(base)
                .map(|rates| (base, *rates))
                .or_else(|| self.primary.models.get(base).map(|rates| (base, *rates)))
                .or_else(|| self.primary.fuzzy(base))
                .or_else(|| self.secondary.models.get(base).map(|rates| (base, *rates)));
            if let Some((key, rates)) = entry {
                let factor = if rates.fast != 1. {
                    Some(rates.fast)
                } else {
                    self.supplement
                        .fast_multiplier(key)
                        .or_else(|| self.supplement.fast_multiplier(base))
                };
                if let Some(factor) = factor {
                    return Some(rates.scaled(factor));
                }
            }
            return self.secondary.models.get(model).copied();
        }
        self.primary
            .fuzzy(model)
            .map(|(_, rates)| rates)
            .or_else(|| self.secondary.models.get(model).copied())
    }
    pub fn estimate(
        &self,
        provider: &str,
        model: &str,
        mut tokens: TokenBreakdown,
        timestamp: &str,
    ) -> Option<f64> {
        let pricing_model = if provider == "codex" {
            codex_pricing_model(model, timestamp)
        } else {
            model
        };
        let (canonical, rates) = self.resolved(pricing_model);
        let cost = if provider == "codex" {
            let base = canonical.strip_suffix("-fast");
            let base_rates = base.and_then(|base| self.resolved(base).1);
            if base.is_some() {
                tokens.is_fast = base_rates.is_some();
                tokens.is_ultrafast &= base_rates.is_some();
            }
            let (rates, threshold) = base_rates
                .or(rates)?
                .codex(base.unwrap_or(&canonical), tokens.is_ultrafast);
            rates.cost(tokens, threshold, true)
        } else {
            let mut rates = rates?;
            // Fast slugs already carry scaled rates. The logged speed mustn't scale them twice.
            if canonical.ends_with("-fast") {
                tokens.is_fast = false;
            }
            if rates.fast == 1. {
                rates.fast = self.supplement.fast_multiplier(&canonical).unwrap_or(1.);
            }
            rates.cost(tokens, 200_000, provider != "cursor")
        };
        (cost.is_finite() && cost >= 0.).then_some(cost)
    }
}
fn source_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date| date.and_utc())
        })
}
fn codex_pricing_model<'a>(model: &'a str, timestamp: &str) -> &'a str {
    if model == "gpt-reserve" {
        return "gpt-5.6-luna";
    }
    if model != "codex-auto-review" {
        return model;
    }
    let day = timestamp.get(..10).unwrap_or("");
    [
        ("2026-07-09", "gpt-5.6-luna"),
        ("2026-04-23", "gpt-5.5"),
        ("2026-03-05", "gpt-5.4"),
        ("2026-02-05", "gpt-5.3-codex"),
        ("2025-12-11", "gpt-5.2-codex"),
        ("2025-11-13", "gpt-5.1-codex"),
        ("2025-09-15", "gpt-5-codex"),
        ("2025-08-07", "gpt-5"),
    ]
    .into_iter()
    .find(|(released, _)| day >= *released)
    .map_or("gpt-5", |(_, model)| model)
}
