use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

const COSTS_ENDPOINT: &str = "https://api.openai.com/v1/organization/costs";

#[derive(Debug, Error)]
pub enum OpenAIError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
    #[error("Invalid amount value '{0}': {1}")]
    ParseAmount(String, String),
}

#[derive(Debug, Clone)]
pub struct CostEntry {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub line_item: String,
    pub project_id: String,
    pub amount_usd: f64,
}

#[derive(Deserialize)]
struct CostsPage {
    has_more: bool,
    next_page: Option<String>,
    data: Vec<Bucket>,
}

#[derive(Deserialize)]
struct Bucket {
    start_time: i64,
    end_time: i64,
    results: Vec<Result>,
}

#[derive(Deserialize)]
struct Result {
    line_item: Option<String>,
    project_id: Option<String>,
    amount: Amount,
}

#[derive(Deserialize)]
struct Amount {
    // OpenAI returns value as a JSON string, not a number.
    value: String,
}

pub async fn fetch_costs(
    client: &Client,
    api_key: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> std::result::Result<Vec<CostEntry>, OpenAIError> {
    let mut entries = Vec::new();
    let mut next_page: Option<String> = None;

    loop {
        let mut req = client
            .get(COSTS_ENDPOINT)
            .bearer_auth(api_key)
            .query(&[
                ("start_time", from.timestamp().to_string()),
                ("end_time", to.timestamp().to_string()),
                ("bucket_width", "1d".to_string()),
                ("group_by[]", "line_item".to_string()),
                ("group_by[]", "project_id".to_string()),
            ]);

        if let Some(ref page) = next_page {
            req = req.query(&[("page", page.as_str())]);
        }

        let response = req.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(OpenAIError::Api(format!("{status}: {body}")));
        }

        let page: CostsPage = response.json().await?;

        for bucket in &page.data {
            let start_time = DateTime::from_timestamp(bucket.start_time, 0)
                .unwrap_or_default();
            let end_time = DateTime::from_timestamp(bucket.end_time, 0)
                .unwrap_or_default();
            for result in &bucket.results {
                let amount_usd = result.amount.value.parse::<f64>().map_err(|e| {
                    OpenAIError::ParseAmount(result.amount.value.clone(), e.to_string())
                })?;
                entries.push(CostEntry {
                    start_time,
                    end_time,
                    line_item: result.line_item.clone().unwrap_or_default(),
                    project_id: result.project_id.clone().unwrap_or_default(),
                    amount_usd,
                });
            }
        }

        if page.has_more {
            next_page = page.next_page;
        } else {
            break;
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live OpenAI admin credentials via BOBBY_OPENAI_ADMIN_KEY"]
    async fn fetch_costs_real_api() {
        let api_key = std::env::var("BOBBY_OPENAI_ADMIN_KEY")
            .expect("BOBBY_OPENAI_ADMIN_KEY must be set");

        let to = Utc::now();
        let from = to - chrono::Duration::days(7);

        let client = Client::new();
        let entries = fetch_costs(&client, &api_key, from, to)
            .await
            .expect("fetch_costs should succeed");

        assert!(!entries.is_empty(), "expected at least one cost entry");
        for e in &entries {
            assert!(!e.line_item.is_empty());
            assert!(!e.project_id.is_empty());
            assert!(e.amount_usd >= 0.0);
        }
        println!("fetched {} cost entries", entries.len());
    }
}
