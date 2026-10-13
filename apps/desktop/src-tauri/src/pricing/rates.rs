use serde::{Deserialize, Serialize};

/// Disjoint billable buckets, matching OpenUsage's TokenBreakdown. Codex cache
/// reads are subtracted from input before constructing this value.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBreakdown {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub is_fast: bool,
    pub is_ultrafast: bool,
}
impl TokenBreakdown {
    pub fn total(self) -> Option<u64> {
        self.input
            .checked_add(self.output)?
            .checked_add(self.cache_read)?
            .checked_add(self.cache_write_5m)?
            .checked_add(self.cache_write_1h)
    }
}

/// The same compact fields as OpenUsage's bundled catalogs. Rates are USD/M tokens.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Rates {
    pub i: f64,
    pub o: f64,
    pub cw: f64,
    pub cr: f64,
    pub ia: Option<f64>,
    pub oa: Option<f64>,
    pub cwa: Option<f64>,
    pub cra: Option<f64>,
    #[serde(default = "explicit")]
    pub cre: bool,
    #[serde(default = "one")]
    pub fast: f64,
}
fn explicit() -> bool {
    true
}
fn one() -> f64 {
    1.
}
impl Rates {
    pub fn valid(self) -> bool {
        [
            Some(self.i),
            Some(self.o),
            Some(self.cw),
            Some(self.cr),
            self.ia,
            self.oa,
            self.cwa,
            self.cra,
            Some(self.fast),
        ]
        .into_iter()
        .flatten()
        .all(|value| value.is_finite() && value >= 0.)
            && self.fast >= 1.
    }
    pub fn scaled(mut self, factor: f64) -> Self {
        self.i *= factor;
        self.o *= factor;
        self.cw *= factor;
        self.cr *= factor;
        self.ia = self.ia.map(|rate| rate * factor);
        self.oa = self.oa.map(|rate| rate * factor);
        self.cwa = self.cwa.map(|rate| rate * factor);
        self.cra = self.cra.map(|rate| rate * factor);
        self.fast = 1.;
        self
    }
    pub fn cost(self, tokens: TokenBreakdown, threshold: u64, request_boundaries: bool) -> f64 {
        let prompt = tokens
            .input
            .saturating_add(tokens.cache_read)
            .saturating_add(tokens.cache_write_5m)
            .saturating_add(tokens.cache_write_1h);
        let long = request_boundaries && prompt > threshold;
        let selected =
            |base: f64, higher: Option<f64>| if long { higher.unwrap_or(base) } else { base };
        let input = selected(self.i, self.ia);
        let dollars = tokens.input as f64 * input
            + tokens.output as f64 * selected(self.o, self.oa)
            + tokens.cache_read as f64 * selected(self.cr, self.cra)
            + tokens.cache_write_5m as f64 * selected(self.cw, self.cwa)
            + tokens.cache_write_1h as f64 * input * 2.;
        dollars / 1_000_000.
            * if tokens.is_fast || tokens.is_ultrafast {
                self.fast
            } else {
                1.
            }
    }

    // Ported from OpenUsage's CodexUsagePricing: request-wide tiers start >272k.
    pub fn codex(mut self, model: &str, ultrafast: bool) -> (Self, u64) {
        let base = dated_base_model(model);
        let long_context = match base {
            "gpt-5.4" => Some((5., 22.5, 0.5)),
            "gpt-5.4-pro" | "gpt-5.5-pro" => Some((60., 270., 60.)),
            "gpt-5.5" => Some((10., 45., 1.)),
            "gpt-5.6-sol" => Some((8., 30., 0.8)),
            "gpt-5.6-terra" => Some((4., 18., 0.4)),
            "gpt-5.6-luna" => Some((0.4, 1.8, 0.04)),
            "gpt-6-astra" => Some((20., 75., 2.)),
            "gpt-6.1-sol" => Some((4., 15., 0.2)),
            "gpt-6-sol" => Some((4., 15., 0.4)),
            "gpt-6-luna" => Some((0.2, 0.75, 0.02)),
            _ => None,
        };
        if let Some((input, output, cached)) = long_context {
            self.ia = Some(input);
            self.oa = Some(output);
            self.cra = Some(cached);
        }
        if !self.cre || matches!(base, "gpt-5.4-pro" | "gpt-5.5-pro") {
            self.cr = self.i;
            self.cra = self.ia;
        }
        self.fast = match base {
            "gpt-6-astra" if ultrafast => 6.,
            "gpt-5.5" | "gpt-5.5-pro" => 2.5,
            "gpt-5.4" | "gpt-5.4-pro" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna"
            | "gpt-6-astra" | "gpt-6.1-sol" | "gpt-6-sol" | "gpt-6-luna" => 2.,
            _ if self.fast == 1. => 2.,
            _ => self.fast,
        };
        (
            self,
            if long_context.is_some() {
                272_000
            } else {
                200_000
            },
        )
    }
}

pub fn dated_base_model(model: &str) -> &str {
    for pattern in ["-dddd-dd-dd", "-dddddddd"] {
        if model.len() > pattern.len() {
            let boundary = model.len() - pattern.len();
            if let Some(suffix) = model.get(boundary..) {
                if suffix.bytes().zip(pattern.bytes()).all(|(a, b)| {
                    if b == b'd' {
                        a.is_ascii_digit()
                    } else {
                        a == b
                    }
                }) {
                    return &model[..boundary];
                }
            }
        }
    }
    model
}
