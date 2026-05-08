use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

use crate::types::{AccountTag, ApiToken, BucketName, BucketNameError};

const API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, Error)]
pub enum R2RestError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Cloudflare API errors: {0}")]
    Api(String),
    #[error("Cloudflare returned an invalid bucket name: {0}")]
    InvalidBucketName(#[from] BucketNameError),
}

#[derive(Debug, Clone)]
pub struct Bucket {
    pub name: BucketName,
}

#[derive(Debug, Clone, Copy)]
pub struct R2BucketUsage {
    pub payload_size: u64,
    pub object_count: u64,
}

#[derive(Deserialize)]
struct Envelope<T> {
    result: Option<T>,
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct ListBucketsResult {
    buckets: Vec<RawBucket>,
}

#[derive(Deserialize)]
struct RawBucket {
    name: String,
}

#[derive(Deserialize)]
struct UsageResult {
    #[serde(rename = "payloadSize")]
    payload_size: NumOrString,
    #[serde(rename = "objectCount")]
    object_count: NumOrString,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NumOrString {
    Num(u64),
    Str(String),
}

impl NumOrString {
    fn as_u64(&self) -> Result<u64, R2RestError> {
        match self {
            Self::Num(n) => Ok(*n),
            Self::Str(s) => s
                .parse::<u64>()
                .map_err(|e| R2RestError::Api(format!("invalid u64 '{s}': {e}"))),
        }
    }
}

async fn get<T: serde::de::DeserializeOwned>(
    client: &Client,
    api_token: &ApiToken,
    url: &str,
) -> Result<T, R2RestError> {
    let envelope: Envelope<T> = client
        .get(url)
        .header("Authorization", format!("Bearer {}", api_token.as_str()))
        .send()
        .await?
        .json()
        .await?;

    if !envelope.success || envelope.result.is_none() {
        let msg = envelope
            .errors
            .into_iter()
            .map(|e| format!("[{}] {}", e.code, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(R2RestError::Api(if msg.is_empty() {
            "request unsuccessful".into()
        } else {
            msg
        }));
    }

    Ok(envelope.result.expect("checked above"))
}

pub async fn list_buckets(
    client: &Client,
    api_token: &ApiToken,
    account_tag: &AccountTag,
) -> Result<Vec<Bucket>, R2RestError> {
    let result: ListBucketsResult = get(
        client,
        api_token,
        &format!("{API_BASE}/accounts/{account_tag}/r2/buckets"),
    )
    .await?;
    result
        .buckets
        .into_iter()
        .map(|b| Ok(Bucket { name: BucketName::new(b.name)? }))
        .collect()
}

pub async fn fetch_bucket_usage(
    client: &Client,
    api_token: &ApiToken,
    account_tag: &AccountTag,
    bucket_name: &BucketName,
) -> Result<R2BucketUsage, R2RestError> {
    let result: UsageResult = get(
        client,
        api_token,
        &format!("{API_BASE}/accounts/{account_tag}/r2/buckets/{bucket_name}/usage"),
    )
    .await?;
    Ok(R2BucketUsage {
        payload_size: result.payload_size.as_u64()?,
        object_count: result.object_count.as_u64()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_bucket_usage_end_to_end() {
        let api_token: ApiToken = std::env::var("BOBBY_CLOUDFLARE_API_TOKEN")
            .expect("BOBBY_CLOUDFLARE_API_TOKEN must be set")
            .parse()
            .expect("api token must be valid");
        let account_tag: AccountTag = std::env::var("BOBBY_CLOUDFLARE_ACCOUNT_TAG")
            .expect("BOBBY_CLOUDFLARE_ACCOUNT_TAG must be set")
            .parse()
            .expect("account tag must be valid");

        let client = Client::new();
        let buckets = list_buckets(&client, &api_token, &account_tag)
            .await
            .expect("list_buckets should succeed");
        assert!(!buckets.is_empty(), "expected at least one bucket");

        for bucket in &buckets {
            let usage = fetch_bucket_usage(&client, &api_token, &account_tag, &bucket.name)
                .await
                .expect("fetch_bucket_usage should succeed");
            println!(
                "bucket={}, payload_size={}, object_count={}",
                bucket.name, usage.payload_size, usage.object_count
            );
        }
    }
}
