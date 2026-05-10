use serde::Deserialize;
use std::collections::HashMap;

const PRICES_TOML: &str = include_str!("../prices.toml");

#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    #[error("unknown model: {0}")]
    UnknownModel(String),
    #[error("failed to parse prices.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Deserialize)]
struct ModelPrice {
    input_per_million_usd: f64,
    output_per_million_usd: f64,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct ModelPrices(HashMap<String, ModelPrice>);

impl ModelPrices {
    /// Load prices from the embedded `eval/prices.toml` baked into the binary.
    pub fn embedded() -> Result<Self, PricingError> {
        Self::from_toml_str(PRICES_TOML)
    }

    pub fn from_toml_str(s: &str) -> Result<Self, PricingError> {
        toml::from_str(s).map_err(PricingError::Parse)
    }

    pub fn cost_for(
        &self,
        model_name: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<f64, PricingError> {
        let price = self
            .0
            .get(model_name)
            .ok_or_else(|| PricingError::UnknownModel(model_name.to_string()))?;
        Ok(input_tokens as f64 * price.input_per_million_usd / 1_000_000.0
            + output_tokens as f64 * price.output_per_million_usd / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMMY_PRICES: &str = r#"
[fast-cheap]
input_per_million_usd = 1.00
output_per_million_usd = 2.00

[slow-expensive]
input_per_million_usd = 10.00
output_per_million_usd = 30.00
"#;

    fn dummy() -> ModelPrices {
        ModelPrices::from_toml_str(DUMMY_PRICES).expect("dummy prices parse")
    }

    #[test]
    fn cost_combines_input_and_output_per_million() {
        // 1M input @ $1.00 + 100k output @ $2.00 = $1.00 + $0.20 = $1.20
        let cost = dummy()
            .cost_for("fast-cheap", 1_000_000, 100_000)
            .expect("known model");
        assert!((cost - 1.20).abs() < 1e-9, "expected $1.20 got {cost}");
    }

    #[test]
    fn cost_scales_with_token_counts() {
        let prices = dummy();
        let small = prices
            .cost_for("slow-expensive", 1_000, 1_000)
            .expect("known model");
        let large = prices
            .cost_for("slow-expensive", 10_000, 10_000)
            .expect("known model");
        assert!((large - small * 10.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_errors() {
        assert!(
            matches!(
                dummy().cost_for("does-not-exist", 100, 100),
                Err(PricingError::UnknownModel(_))
            )
        );
    }

    #[test]
    fn embedded_prices_parse() {
        // Smoke test: the bundled prices.toml is well-formed.
        ModelPrices::embedded().expect("embedded prices parse");
    }
}
