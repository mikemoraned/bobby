use std::collections::BTreeMap;

use serde::Deserialize;

use crate::pricing::ModelPrice;
use crate::usd::Usd;

pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

#[derive(Debug, thiserror::Error)]
pub enum UpdatePricesError {
    #[error("failed to parse models.dev response: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("model not found in response: {0}")]
    ModelNotFound(String),
    #[error("model {0} has no cost field in response")]
    MissingCost(String),
}

#[derive(Deserialize)]
struct ModelsDevResponse {
    openai: Provider,
}

#[derive(Deserialize)]
struct Provider {
    models: BTreeMap<String, ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    /// Some models in the real API lack a `cost` field; keep optional so the response
    /// parses even when irrelevant entries are incomplete.
    #[serde(default)]
    cost: Option<Cost>,
}

#[derive(Deserialize)]
struct Cost {
    input: Usd,
    output: Usd,
}

/// Parse a models.dev API response and extract per-million pricing for each requested
/// OpenAI model. Errors if any requested model is missing from the response or lacks
/// a `cost` field.
pub fn extract_prices(
    json: &str,
    requested_models: &[String],
) -> Result<BTreeMap<String, ModelPrice>, UpdatePricesError> {
    let response: ModelsDevResponse = serde_json::from_str(json)?;
    let mut prices = BTreeMap::new();
    for name in requested_models {
        let entry = response
            .openai
            .models
            .get(name)
            .ok_or_else(|| UpdatePricesError::ModelNotFound(name.clone()))?;
        let cost = entry
            .cost
            .as_ref()
            .ok_or_else(|| UpdatePricesError::MissingCost(name.clone()))?;
        prices.insert(
            name.clone(),
            ModelPrice {
                input_per_million: cost.input,
                output_per_million: cost.output,
            },
        );
    }
    Ok(prices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Captured 2026-05-10 from `https://models.dev/api.json`, trimmed to the
    /// fields this parser uses. Includes a non-OpenAI provider and an extra
    /// OpenAI model to verify the parser handles both correctly.
    const FIXTURE: &str = r#"{
        "openai": {
            "models": {
                "gpt-4o": {
                    "id": "gpt-4o",
                    "cost": { "input": 2.5, "output": 10, "cache_read": 1.25 }
                },
                "gpt-4o-mini": {
                    "id": "gpt-4o-mini",
                    "cost": { "input": 0.15, "output": 0.6, "cache_read": 0.08 }
                },
                "gpt-3.5-turbo": {
                    "id": "gpt-3.5-turbo",
                    "cost": { "input": 0.5, "output": 1.5 }
                }
            }
        },
        "anthropic": {
            "models": {
                "claude-3-opus": {
                    "id": "claude-3-opus",
                    "cost": { "input": 15.0, "output": 75.0 }
                }
            }
        }
    }"#;

    #[test]
    fn extracts_requested_openai_models() {
        let prices =
            extract_prices(FIXTURE, &["gpt-4o".into(), "gpt-4o-mini".into()]).expect("parse");
        assert_eq!(prices.len(), 2);
        assert_eq!(
            prices["gpt-4o"].input_per_million,
            Usd::from_str("2.5").expect("valid")
        );
        assert_eq!(
            prices["gpt-4o"].output_per_million,
            Usd::from_str("10").expect("valid")
        );
        assert_eq!(
            prices["gpt-4o-mini"].input_per_million,
            Usd::from_str("0.15").expect("valid")
        );
        assert_eq!(
            prices["gpt-4o-mini"].output_per_million,
            Usd::from_str("0.6").expect("valid")
        );
    }

    #[test]
    fn ignores_unrequested_models() {
        let prices = extract_prices(FIXTURE, &["gpt-4o".into()]).expect("parse");
        assert_eq!(prices.len(), 1);
        assert!(!prices.contains_key("gpt-3.5-turbo"));
    }

    #[test]
    fn errors_on_missing_model() {
        let result = extract_prices(FIXTURE, &["gpt-nonexistent".into()]);
        assert!(matches!(
            result,
            Err(UpdatePricesError::ModelNotFound(name)) if name == "gpt-nonexistent"
        ));
    }

    #[test]
    fn tolerates_unrequested_models_without_cost() {
        // Real models.dev includes some entries without a `cost` field; the parser must
        // not fail on the response as a whole, only on requested models that are missing it.
        const FIXTURE_WITH_INCOMPLETE_ENTRY: &str = r#"{
            "openai": {
                "models": {
                    "gpt-4o": { "id": "gpt-4o", "cost": { "input": 2.5, "output": 10 } },
                    "experimental": { "id": "experimental" }
                }
            }
        }"#;
        let prices =
            extract_prices(FIXTURE_WITH_INCOMPLETE_ENTRY, &["gpt-4o".into()]).expect("parse");
        assert_eq!(prices.len(), 1);

        let result = extract_prices(FIXTURE_WITH_INCOMPLETE_ENTRY, &["experimental".into()]);
        assert!(matches!(
            result,
            Err(UpdatePricesError::MissingCost(name)) if name == "experimental"
        ));
    }
}
