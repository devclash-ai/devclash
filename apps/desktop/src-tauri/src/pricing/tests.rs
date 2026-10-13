use super::*;
use rates::Rates;
use serde_json::json;

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}
fn estimate(provider: &str, model: &str, tokens: TokenBreakdown) -> f64 {
    ModelPricing::bundled()
        .estimate(provider, model, tokens, "2026-09-30T12:00:00Z")
        .unwrap()
}
#[test]
fn uses_all_upstream_resources_and_aliases_without_rewriting_them() {
    let pricing = ModelPricing::bundled();
    assert!(pricing.primary.models.len() > 2000);
    assert!(pricing.secondary.models.len() > 2000);
    assert!(pricing.supplement.aliases.len() > 100);
    assert_eq!(
        pricing.resolved("Opus 5.5 (Auto Balanced)").0,
        "claude-opus-5-5"
    );
    assert_eq!(
        pricing.resolved("gemini-3-pro-high").0,
        "gemini-3-pro-preview"
    );
    assert_eq!(pricing.resolved("gemini-3-pro").0, "gemini-3-pro");
    close(pricing.resolved("claude-opus-5-5").1.unwrap().i, 4.);
}
#[test]
fn claude_separate_cache_buckets_and_speed_match_openusage() {
    let tokens = TokenBreakdown {
        input: 10_000,
        output: 2000,
        cache_read: 20_000,
        cache_write_5m: 1000,
        cache_write_1h: 2000,
        ..TokenBreakdown::default()
    };
    close(estimate("claude", "claude-opus-5-5", tokens), 0.105);
    close(
        estimate(
            "claude",
            "claude-opus-5-5",
            TokenBreakdown {
                is_fast: true,
                ..tokens
            },
        ),
        0.21,
    );
    close(
        estimate(
            "claude",
            "claude-opus-5-5-fast",
            TokenBreakdown {
                is_fast: true,
                ..tokens
            },
        ),
        0.21,
    );
}
#[test]
fn codex_cached_input_and_request_wide_context_tier_match_openusage() {
    close(
        estimate(
            "codex",
            "gpt-6.1-sol",
            TokenBreakdown {
                input: 1000,
                cache_read: 9000,
                output: 100,
                ..TokenBreakdown::default()
            },
        ),
        0.0039,
    );
    let tokens = TokenBreakdown {
        input: 22_000,
        cache_read: 250_000,
        output: 1000,
        ..TokenBreakdown::default()
    };
    close(estimate("codex", "gpt-6-astra", tokens), 0.52);
    close(
        estimate(
            "codex",
            "gpt-6-astra",
            TokenBreakdown {
                input: 22_001,
                ..tokens
            },
        ),
        1.01502,
    );
    close(
        estimate(
            "codex",
            "gpt-5.4-2026-03-05",
            TokenBreakdown {
                input: 22_001,
                ..tokens
            },
        ),
        estimate(
            "codex",
            "gpt-5.4",
            TokenBreakdown {
                input: 22_001,
                ..tokens
            },
        ),
    );
}
#[test]
fn codex_fast_alias_is_scaled_once_and_ultrafast_uses_astra_multiplier() {
    let tokens = TokenBreakdown {
        input: 1000,
        output: 100,
        ..TokenBreakdown::default()
    };
    let base = estimate("codex", "gpt-6-astra", tokens);
    close(
        estimate(
            "codex",
            "gpt-6-astra-fast",
            TokenBreakdown {
                is_fast: true,
                ..tokens
            },
        ),
        base * 2.,
    );
    close(
        estimate(
            "codex",
            "gpt-6-astra",
            TokenBreakdown {
                is_ultrafast: true,
                ..tokens
            },
        ),
        base * 6.,
    );
    let sol = estimate("codex", "gpt-6-sol", tokens);
    close(
        estimate(
            "codex",
            "gpt-6-sol",
            TokenBreakdown {
                is_ultrafast: true,
                ..tokens
            },
        ),
        sol * 2.,
    );
}
#[test]
fn codex_reserve_and_auto_review_use_upstreams_known_pricing_models() {
    let tokens = TokenBreakdown {
        input: 1000,
        output: 100,
        ..TokenBreakdown::default()
    };
    let luna = estimate("codex", "gpt-5.6-luna", tokens);
    close(estimate("codex", "gpt-reserve", tokens), luna);
    close(estimate("codex", "codex-auto-review", tokens), luna);
    assert_eq!(
        codex_pricing_model("codex-auto-review", "2026-05-01"),
        "gpt-5.5"
    );
}
#[test]
fn cursor_aggregate_rows_never_guess_a_request_context_surcharge() {
    let pricing = ModelPricing::bundled();
    let tokens = TokenBreakdown {
        input: 300_000,
        output: 10_000,
        ..TokenBreakdown::default()
    };
    let rates = pricing.resolved("claude-sonnet-4-20250514").1.unwrap();
    close(
        pricing
            .estimate("cursor", "claude-sonnet-4-20250514", tokens, "")
            .unwrap(),
        rates.cost(tokens, 200_000, false),
    );
    assert!(rates.cost(tokens, 200_000, true) > rates.cost(tokens, 200_000, false));
}
#[test]
fn unknown_prices_and_numeric_version_collisions_stay_unpriced() {
    let pricing = ModelPricing::bundled();
    assert!(pricing.resolved("unreleased-private-model").1.is_none());
    assert!(pricing
        .resolved("unreleased-private-model-fast")
        .1
        .is_none());
    assert!(!catalog::matches_key(
        "claude-sonnet-4",
        "claude-sonnet-4-5"
    ));
    assert!(catalog::matches_key(
        "claude-sonnet-4",
        "anthropic/claude-sonnet-4-20250514"
    ));
    assert!(catalog::matches_key("grok-4.3", "xai/grok-4-3"));
}
#[test]
fn feeds_convert_per_token_units_and_track_explicit_cache_discounts() {
    let bytes = serde_json::to_vec(&json!({"a": {"input_cost_per_token":0.000002,
        "output_cost_per_token":0.00001},"broken":{"input_cost_per_token":-1}}))
    .unwrap();
    let rates = catalog::Catalog::litellm(&bytes).unwrap().models["a"];
    close(rates.i, 2.);
    close(rates.o, 10.);
    assert!(!rates.cre);
    close(rates.codex("a", false).0.cr, 2.);
    let bytes = serde_json::to_vec(&json!({"provider":{"models":{"a":{"cost":{
        "input":2,"output":10,"cache_read":0.1}}}}}))
    .unwrap();
    let rates = catalog::Catalog::models_dev(&bytes).unwrap().models["a"];
    close(rates.cr, 0.1);
    assert!(rates.cre);
}
#[test]
fn newer_bundled_supplement_beats_an_old_offline_cache() {
    let mut old: serde_json::Value = serde_json::from_slice(SUPPLEMENT).unwrap();
    old["updated_at"] = json!("2020-01-01");
    old["pricing"]["gpt-6-sol"]["input_per_million"] = json!(999);
    let bytes = serde_json::to_vec(&old).unwrap();
    let pricing = ModelPricing::from_cached(Some(&bytes), None, None).unwrap();
    close(pricing.resolved("gpt-6-sol").1.unwrap().i, 2.);
    old["updated_at"] = json!("2026-09-30T00:00:00.100Z");
    let newer = serde_json::to_vec(&old).unwrap();
    let pricing = ModelPricing::from_cached(Some(&newer), None, None).unwrap();
    close(pricing.resolved("gpt-6-sol").1.unwrap().i, 999.);
    let invalid = Rates {
        i: -1.,
        o: 1.,
        cw: 1.,
        cr: 1.,
        ia: None,
        oa: None,
        cwa: None,
        cra: None,
        cre: true,
        fast: 1.,
    };
    assert!(!invalid.valid());
}
