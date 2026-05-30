use chrono::{DateTime, Utc};
use reqwest::Client;

use crate::types::{AccountTag, ApiToken, BucketName, BucketNameError};

pub fn one_minute_windows(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut windows = Vec::new();
    let mut cursor = from;
    while cursor < to {
        let next = (cursor + chrono::Duration::minutes(1)).min(to);
        windows.push((cursor, next));
        cursor = next;
    }
    windows
}
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
    #[error("Cloudflare returned an invalid bucket name: {0}")]
    InvalidBucketName(#[from] BucketNameError),
}

#[derive(Debug, Clone)]
pub struct R2OperationGroup {
    pub action_type: String,
    pub bucket_name: Option<BucketName>,
    pub requests: u64,
}

#[derive(Debug, Clone)]
pub struct R2Operations {
    pub operations: Vec<R2OperationGroup>,
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

#[derive(Serialize)]
struct GraphQLRequest<'a> {
    query: &'a str,
    variables: serde_json::Value,
}

static QUERY: &str = r#"
    query R2Operations($accountTag: String!, $from: String!, $to: String!) {
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
        }
      }
    }
"#;

pub async fn fetch_r2_operations(
    client: &Client,
    api_token: &ApiToken,
    account_tag: &AccountTag,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<R2Operations, CloudflareError> {
    let variables = serde_json::json!({
        "accountTag": account_tag.as_str(),
        "from": from.to_rfc3339(),
        "to": to.to_rfc3339(),
    });

    let body = GraphQLRequest {
        query: QUERY,
        variables,
    };

    let response: GraphQLResponse = client
        .post(GRAPHQL_ENDPOINT)
        .header("Authorization", format!("Bearer {}", api_token.as_str()))
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

    let mut accounts = response.data.map(|d| d.viewer.accounts).unwrap_or_default();

    if accounts.is_empty() {
        return Err(CloudflareError::NoAccount);
    }

    let account = accounts.swap_remove(0);

    let operations = account
        .r2_operations
        .into_iter()
        .map(|g| {
            let bucket_name = if g.dimensions.bucket_name.is_empty() {
                None
            } else {
                Some(BucketName::new(g.dimensions.bucket_name)?)
            };
            Ok(R2OperationGroup {
                action_type: g.dimensions.action_type,
                bucket_name,
                requests: g.sum.requests,
            })
        })
        .collect::<Result<Vec<_>, CloudflareError>>()?;

    Ok(R2Operations { operations })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().into()
    }

    #[test]
    fn single_minute_gives_one_window() {
        let windows = one_minute_windows(ts("2026-01-01T00:00:00Z"), ts("2026-01-01T00:01:00Z"));
        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0],
            (ts("2026-01-01T00:00:00Z"), ts("2026-01-01T00:01:00Z"))
        );
    }

    #[test]
    fn two_minutes_gives_two_windows() {
        let windows = one_minute_windows(ts("2026-01-01T00:00:00Z"), ts("2026-01-01T00:02:00Z"));
        assert_eq!(windows.len(), 2);
        assert_eq!(
            windows[0],
            (ts("2026-01-01T00:00:00Z"), ts("2026-01-01T00:01:00Z"))
        );
        assert_eq!(
            windows[1],
            (ts("2026-01-01T00:01:00Z"), ts("2026-01-01T00:02:00Z"))
        );
    }

    #[test]
    fn each_window_has_distinct_midpoint() {
        let windows = one_minute_windows(ts("2026-01-01T00:00:00Z"), ts("2026-01-01T00:03:00Z"));
        let midpoints: Vec<i64> = windows
            .iter()
            .map(|(f, t)| (f.timestamp_millis() + t.timestamp_millis()) / 2)
            .collect();
        assert_eq!(
            midpoints,
            vec![
                ts("2026-01-01T00:00:30Z").timestamp_millis(),
                ts("2026-01-01T00:01:30Z").timestamp_millis(),
                ts("2026-01-01T00:02:30Z").timestamp_millis(),
            ]
        );
    }

    #[tokio::test]
    async fn fetch_r2_operations_end_to_end() {
        let api_token: ApiToken = std::env::var("BOBBY_CLOUDFLARE_API_TOKEN")
            .expect("BOBBY_CLOUDFLARE_API_TOKEN must be set")
            .parse()
            .expect("api token must be valid");
        let account_tag: AccountTag = std::env::var("BOBBY_CLOUDFLARE_ACCOUNT_TAG")
            .expect("BOBBY_CLOUDFLARE_ACCOUNT_TAG must be set")
            .parse()
            .expect("account tag must be valid");

        let now = Utc::now();
        let from = now - chrono::Duration::minutes(10);
        let to = now - chrono::Duration::minutes(5);

        let client = Client::new();
        let result = fetch_r2_operations(&client, &api_token, &account_tag, from, to).await;

        let metrics = result.expect("fetch_r2_operations should succeed");
        println!("operations: {}", metrics.operations.len());
        for op in &metrics.operations {
            assert!(!op.action_type.is_empty());
        }
    }
}
