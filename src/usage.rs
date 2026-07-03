//! Per-call and accumulated token usage, plus cost-weighted accounting.

/// Token usage reported by a backend for a single call.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    /// Fresh input that writes the cache, at full price. Large prompts land
    /// here rather than in `input_tokens`.
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    /// Tokens served from cache: cheap (~0.1x) and NOT real burn.
    pub cache_read_input_tokens: u64,
    /// API-equivalent cost of this call, in USD. claude reports it directly;
    /// codex is derived from a price snapshot.
    pub cost_usd: f64,
}

impl Usage {
    /// Fresh tokens actually processed. This is what counts toward a burn target.
    pub fn processed(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.output_tokens
    }
}

/// Accumulated result of a burn run.
#[derive(Debug, Default, Clone, Copy)]
pub struct Report {
    pub calls: u64,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// Accumulated API-equivalent cost in USD.
    pub cost_usd: f64,
}

/// Weight applied to cache-read tokens in cost-weighted accounting: cache
/// reads cost roughly a tenth of fresh input.
// ponytail: cache-writes are weighted 1.0 here; real APIs charge about 1.25x.
// Bump it if that precision ever matters.
pub const CACHE_READ_WEIGHT: f64 = 0.1;

impl Report {
    pub fn processed(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.output_tokens
    }

    /// Every token at face value. This is the inflated number leaderboards quote.
    pub fn raw_tokens(&self) -> u64 {
        self.processed() + self.cache_read_input_tokens
    }

    /// Honest burn: fresh input/output/cache-writes at full weight, cache
    /// reads discounted to what they actually cost.
    pub fn cost_weighted_tokens(&self) -> f64 {
        self.processed() as f64 + self.cache_read_input_tokens as f64 * CACHE_READ_WEIGHT
    }

    /// Fraction of input served from cache. High means the padding is being
    /// cached and the burn is not real. Entropy should keep this near zero.
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens;
        if total == 0 {
            0.0
        } else {
            self.cache_read_input_tokens as f64 / total as f64
        }
    }
}

pub(crate) fn accumulate(report: &mut Report, usage: &Usage) {
    report.calls += 1;
    report.input_tokens += usage.input_tokens;
    report.cache_creation_input_tokens += usage.cache_creation_input_tokens;
    report.output_tokens += usage.output_tokens;
    report.cache_read_input_tokens += usage.cache_read_input_tokens;
    report.cost_usd += usage.cost_usd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_ratio_flags_absorbed_padding() {
        let r = Report {
            calls: 1,
            input_tokens: 10,
            cache_read_input_tokens: 90,
            ..Default::default()
        };
        assert!((r.cache_hit_ratio() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn accounting_separates_raw_from_cost_weighted() {
        let r = Report {
            calls: 1,
            input_tokens: 1000,
            cache_creation_input_tokens: 500,
            output_tokens: 100,
            cache_read_input_tokens: 900,
            ..Default::default()
        };
        assert_eq!(r.processed(), 1600); // excludes cache reads
        assert_eq!(r.raw_tokens(), 2500); // everything at face value
        assert!((r.cost_weighted_tokens() - 1690.0).abs() < 1e-9); // 1600 + 0.1*900
    }
}
