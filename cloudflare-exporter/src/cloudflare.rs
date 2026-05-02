use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const GRAPHQL_ENDPOINT: &str = "https://api.cloudflare.com/client/v4/graphql";

#[derive(Debug, Error)]
pub enum CloudflareError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GraphQL errors: {0}")]
    GraphQL(String),
    #[error("No account data returned for the given account tag")]
    NoAccount,
}

#[derive(Debug, Clone)]
pub struct R2OperationGroup {
    pub action_type: String,
    pub bucket_name: String,
    pub requests: u64,
}

#[derive(Debug, Clone)]
pub struct R2StorageGroup {
    pub bucket_name: String,
    pub payload_size: u64,
    pub object_count: u64,
}

#[derive(Debug, Clone)]
pub struct R2Metrics {
    pub operations: Vec<R2OperationGroup>,
    pub storage: Vec<R2StorageGroup>,
}

#[derive(Deserialize)]
struct GraphQLResponse {
    data: Option<GraphQLData>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize)]
struct GraphQLError {
    message: String,
    #[serde(default)]
    path: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct GraphQLData {
    viewer: Viewer,
}

#[derive(Deserialize)]
struct Viewer {
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
    #[serde(rename = "r2OperationsAdaptiveGroups")]
    r2_operations: Vec<R2OpGroup>,
    #[serde(rename = "r2StorageAdaptiveGroups")]
    r2_storage: Vec<R2StorGroup>,
}

#[derive(Deserialize)]
struct R2OpGroup {
    dimensions: R2OpDimensions,
    sum: R2OpSum,
}

#[derive(Deserialize)]
struct R2OpDimensions {
    #[serde(rename = "actionType")]
    action_type: String,
    #[serde(rename = "bucketName")]
    bucket_name: String,
}

#[derive(Deserialize)]
struct R2OpSum {
    requests: u64,
}

#[derive(Deserialize)]
struct R2StorGroup {
    dimensions: R2StorDimensions,
    max: R2StorMax,
}

#[derive(Deserialize)]
struct R2StorDimensions {
    #[serde(rename = "bucketName")]
    bucket_name: String,
}

#[derive(Deserialize)]
struct R2StorMax {
    #[serde(rename = "payloadSize")]
    payload_size: u64,
    #[serde(rename = "objectCount")]
    object_count: u64,
}

#[derive(Serialize)]
struct GraphQLRequest<'a> {
    query: &'a str,
    variables: serde_json::Value,
}

static QUERY: &str = r#"
    query R2Metrics($accountTag: String!, $from: String!, $to: String!) {
      viewer {
        accounts(filter: {accountTag: $accountTag}) {
          r2OperationsAdaptiveGroups(
            filter: {datetime_geq: $from, datetime_lt: $to}
            limit: 10000
          ) {
            dimensions {
              actionType
              bucketName
            }
            sum {
              requests
            }
          }
          r2StorageAdaptiveGroups(
            filter: {datetime_geq: $from, datetime_lt: $to}
            limit: 10000
          ) {
            dimensions {
              bucketName
            }
            max {
              payloadSize
              objectCount
            }
          }
        }
      }
    }
"#;

pub async fn fetch_r2_metrics(
    client: &Client,
    api_token: &str,
    account_tag: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<R2Metrics, CloudflareError> {
    let variables = serde_json::json!({
        "accountTag": account_tag,
        "from": from.to_rfc3339(),
        "to": to.to_rfc3339(),
    });

    let body = GraphQLRequest {
        query: QUERY,
        variables,
    };

    let response: GraphQLResponse = client
        .post(GRAPHQL_ENDPOINT)
        .header("Authorization", format!("Bearer {api_token}"))
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    if let Some(errors) = response.errors {
        let msg = errors
            .into_iter()
            .map(|e| {
                if e.path.is_empty() {
                    e.message
                } else {
                    let path = e
                        .path
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(".");
                    format!("{} (path: {path})", e.message)
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(CloudflareError::GraphQL(msg));
    }

    let mut accounts = response
        .data
        .map(|d| d.viewer.accounts)
        .unwrap_or_default();

    if accounts.is_empty() {
        return Err(CloudflareError::NoAccount);
    }

    let account = accounts.swap_remove(0);

    let operations = account
        .r2_operations
        .into_iter()
        .map(|g| R2OperationGroup {
            action_type: g.dimensions.action_type,
            bucket_name: g.dimensions.bucket_name,
            requests: g.sum.requests,
        })
        .collect();

    let storage = account
        .r2_storage
        .into_iter()
        .map(|g| R2StorageGroup {
            bucket_name: g.dimensions.bucket_name,
            payload_size: g.max.payload_size,
            object_count: g.max.object_count,
        })
        .collect();

    Ok(R2Metrics {
        operations,
        storage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live Cloudflare API credentials via BOBBY_CLOUDFLARE_API_TOKEN and BOBBY_CLOUDFLARE_ACCOUNT_TAG"]
    async fn fetch_r2_metrics_real_api() {
        let api_token = std::env::var("BOBBY_CLOUDFLARE_API_TOKEN")
            .expect("BOBBY_CLOUDFLARE_API_TOKEN must be set");
        let account_tag = std::env::var("BOBBY_CLOUDFLARE_ACCOUNT_TAG")
            .expect("BOBBY_CLOUDFLARE_ACCOUNT_TAG must be set");

        let now = Utc::now();
        let from = now - chrono::Duration::minutes(10);
        let to = now - chrono::Duration::minutes(5);

        let client = Client::new();
        let result = fetch_r2_metrics(&client, &api_token, &account_tag, from, to).await;

        let metrics = result.expect("fetch_r2_metrics should succeed");
        println!(
            "operations: {}, storage groups: {}",
            metrics.operations.len(),
            metrics.storage.len()
        );
        // All operation groups must have non-empty fields
        for op in &metrics.operations {
            assert!(!op.action_type.is_empty());
            assert!(!op.bucket_name.is_empty());
        }
    }
}
