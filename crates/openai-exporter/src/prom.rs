use prometheus_reqwest_remote_write::{
    Label, Sample, TimeSeries, WriteRequest, CONTENT_TYPE, HEADER_NAME_REMOTE_WRITE_VERSION,
    REMOTE_WRITE_VERSION_01,
};
use reqwest::Client;
use thiserror::Error;

use crate::openai::CostEntry;

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

pub async fn push(
    client: &Client,
    endpoint: &str,
    basic_auth: &str,
    entries: &[CostEntry],
    timestamp_ms: i64,
) -> Result<(), PromError> {
    let (username, password) = basic_auth
        .split_once(':')
        .ok_or(PromError::InvalidAuth)?;

    let compressed = build_write_request(entries, timestamp_ms)
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

fn build_write_request(entries: &[CostEntry], timestamp_ms: i64) -> WriteRequest {
    let timeseries = entries
        .iter()
        .map(|e| {
            TimeSeries {
                labels: vec![
                    Label {
                        name: "__name__".into(),
                        value: "openai_cost_usd_total".into(),
                    },
                    Label {
                        name: "line_item".into(),
                        value: e.line_item.clone(),
                    },
                    Label {
                        name: "project_id".into(),
                        value: e.project_id.clone(),
                    },
                ],
                samples: vec![Sample {
                    value: e.amount_usd,
                    timestamp: timestamp_ms,
                }],
            }
        })
        .collect();

    WriteRequest { timeseries }.sorted()
}
