#![warn(clippy::all, clippy::nursery)]

use prometheus_reqwest_remote_write::{
    Label, Sample, TimeSeries, WriteRequest, CONTENT_TYPE, HEADER_NAME_REMOTE_WRITE_VERSION,
    REMOTE_WRITE_VERSION_01,
};
use reqwest::Client;
use thiserror::Error;

use crate::cloudflare::R2Operations;
use crate::r2_rest::{Bucket, R2BucketUsage};
use crate::types::BucketName;

#[derive(Debug, Error)]
pub enum PromError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Snappy compression failed: {0}")]
    Compression(String),
    #[error("Invalid auth format (expected instance_id:api_key)")]
    InvalidAuth,
    #[error("Remote write failed with status {0}: {1}")]
    RemoteWrite(u16, String),
}

pub async fn push_timeseries(
    client: &Client,
    endpoint: &str,
    basic_auth: &str,
    timeseries: Vec<TimeSeries>,
) -> Result<(), PromError> {
    let (username, password) = basic_auth
        .split_once(':')
        .ok_or(PromError::InvalidAuth)?;

    let compressed = WriteRequest { timeseries }
        .sorted()
        .encode_compressed()
        .map_err(|e| PromError::Compression(e.to_string()))?;

    let response = client
        .post(endpoint)
        .basic_auth(username, Some(password))
        .header(reqwest::header::CONTENT_TYPE, CONTENT_TYPE)
        .header(reqwest::header::CONTENT_ENCODING, "snappy")
        .header(HEADER_NAME_REMOTE_WRITE_VERSION, REMOTE_WRITE_VERSION_01)
        .body(compressed)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(PromError::RemoteWrite(status.as_u16(), body));
    }
    if !body.is_empty() {
        tracing::warn!(status = status.as_u16(), body, "Mimir remote_write warning");
    }

    Ok(())
}

pub async fn push_operations(
    client: &Client,
    endpoint: &str,
    basic_auth: &str,
    operations: &R2Operations,
    timestamp_ms: i64,
) -> Result<(), PromError> {
    push_timeseries(
        client,
        endpoint,
        basic_auth,
        build_operations_timeseries(operations, timestamp_ms),
    )
    .await
}

fn build_operations_timeseries(operations: &R2Operations, timestamp_ms: i64) -> Vec<TimeSeries> {
    operations
        .operations
        .iter()
        .map(|op| TimeSeries {
            labels: vec![
                Label {
                    name: "__name__".into(),
                    value: "cloudflare_r2_operations_total".into(),
                },
                Label {
                    name: "action_type".into(),
                    value: op.action_type.clone(),
                },
                Label {
                    name: "bucket".into(),
                    value: op
                        .bucket_name
                        .as_ref()
                        .map(BucketName::as_str)
                        .unwrap_or("")
                        .into(),
                },
            ],
            samples: vec![Sample {
                value: op.requests as f64,
                timestamp: timestamp_ms,
            }],
        })
        .collect()
}

pub async fn push_storage(
    client: &Client,
    endpoint: &str,
    basic_auth: &str,
    usages: &[(Bucket, R2BucketUsage)],
    timestamp_ms: i64,
) -> Result<(), PromError> {
    push_timeseries(
        client,
        endpoint,
        basic_auth,
        build_storage_timeseries(usages, timestamp_ms),
    )
    .await
}

fn build_storage_timeseries(
    usages: &[(Bucket, R2BucketUsage)],
    timestamp_ms: i64,
) -> Vec<TimeSeries> {
    let mut series = Vec::with_capacity(usages.len() * 2);
    for (bucket, usage) in usages {
        series.push(TimeSeries {
            labels: vec![
                Label {
                    name: "__name__".into(),
                    value: "cloudflare_r2_storage_bytes".into(),
                },
                Label {
                    name: "bucket".into(),
                    value: bucket.name.as_str().into(),
                },
            ],
            samples: vec![Sample {
                value: usage.payload_size as f64,
                timestamp: timestamp_ms,
            }],
        });
        series.push(TimeSeries {
            labels: vec![
                Label {
                    name: "__name__".into(),
                    value: "cloudflare_r2_storage_objects".into(),
                },
                Label {
                    name: "bucket".into(),
                    value: bucket.name.as_str().into(),
                },
            ],
            samples: vec![Sample {
                value: usage.object_count as f64,
                timestamp: timestamp_ms,
            }],
        });
    }
    series
}
