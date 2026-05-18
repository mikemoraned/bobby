use std::collections::HashMap;

use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TempoError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}

pub struct TempoClient {
    client: Client,
    base_url: String,
    user: String,
    token: String,
}

// ── OTLP JSON wire types ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    traces: Vec<TraceInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceInfo {
    #[serde(rename = "traceID")]
    pub trace_id: String,
    pub root_service_name: String,
    pub root_trace_name: String,
    pub start_time_unix_nano: String,
}

#[derive(Deserialize)]
struct TraceResponse {
    #[serde(default)]
    batches: Vec<ResourceSpans>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceSpans {
    #[serde(default)]
    scope_spans: Vec<ScopeSpans>,
}

#[derive(Deserialize)]
struct ScopeSpans {
    #[serde(default)]
    spans: Vec<OtlpSpan>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OtlpSpan {
    span_id: String,
    #[serde(default)]
    parent_span_id: String,
    name: String,
    start_time_unix_nano: String,
    end_time_unix_nano: String,
    #[serde(default)]
    attributes: Vec<OtlpKeyValue>,
    #[serde(default)]
    events: Vec<OtlpEvent>,
}

#[derive(Deserialize)]
struct OtlpEvent {
    name: String,
    #[serde(default)]
    attributes: Vec<OtlpKeyValue>,
}

#[derive(Deserialize)]
struct OtlpKeyValue {
    key: String,
    value: OtlpValue,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct OtlpValue {
    string_value: Option<String>,
    int_value: Option<String>,
    bool_value: Option<bool>,
}

impl OtlpValue {
    fn into_attr(self) -> Option<AttrValue> {
        if let Some(s) = self.string_value {
            return Some(AttrValue::Str(s));
        }
        if let Some(i) = self.int_value.as_deref().and_then(|s| s.parse().ok()) {
            return Some(AttrValue::Int(i));
        }
        if let Some(b) = self.bool_value {
            return Some(AttrValue::Bool(b));
        }
        None
    }
}

fn to_attr_map(kvs: Vec<OtlpKeyValue>) -> HashMap<String, AttrValue> {
    kvs.into_iter()
        .filter_map(|kv| kv.value.into_attr().map(|v| (kv.key, v)))
        .collect()
}

// ── Public domain types ───────────────────────────────────────────────────────

pub enum AttrValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

impl AttrValue {
    pub const fn as_str(&self) -> Option<&str> {
        if let Self::Str(s) = self { Some(s.as_str()) } else { None }
    }

    pub const fn as_i64(&self) -> Option<i64> {
        if let Self::Int(n) = self { Some(*n) } else { None }
    }

    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self { Some(*b) } else { None }
    }
}

pub struct SpanEvent {
    pub name: String,
    pub attributes: HashMap<String, AttrValue>,
}

pub struct Span {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub duration_ns: u64,
    pub attributes: HashMap<String, AttrValue>,
    pub events: Vec<SpanEvent>,
}

pub struct Trace {
    pub spans: Vec<Span>,
}

// ── Client ────────────────────────────────────────────────────────────────────

impl TempoClient {
    pub fn new(
        base_url: impl Into<String>,
        user: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            user: user.into(),
            token: token.into(),
        }
    }

    pub async fn search(
        &self,
        service: &str,
        span_name: Option<&str>,
        limit: u32,
        lookback_secs: u64,
    ) -> Result<Vec<TraceInfo>, TempoError> {
        let q = span_name.map_or_else(
            || format!(r#"{{resource.service.name="{service}"}}"#),
            |name| format!(r#"{{resource.service.name="{service}" && name="{name}"}}"#),
        );
        let now = chrono::Utc::now();
        let start = now - chrono::Duration::seconds(lookback_secs as i64);
        let limit_str = limit.to_string();
        let start_str = start.timestamp().to_string();
        let end_str = now.timestamp().to_string();

        let resp: SearchResponse = self
            .client
            .get(format!("{}/api/search", self.base_url))
            .basic_auth(&self.user, Some(&self.token))
            .query(&[
                ("q", q.as_str()),
                ("limit", limit_str.as_str()),
                ("start", start_str.as_str()),
                ("end", end_str.as_str()),
            ])
            .send()
            .await?
            .json()
            .await?;

        Ok(resp.traces)
    }

    pub async fn fetch_trace(&self, info: &TraceInfo) -> Result<Trace, TempoError> {
        let resp: TraceResponse = self
            .client
            .get(format!("{}/api/traces/{}", self.base_url, info.trace_id))
            .basic_auth(&self.user, Some(&self.token))
            .send()
            .await?
            .json()
            .await?;

        Ok(flatten_trace(resp))
    }
}

#[cfg(test)]
const TRACE_FIXTURE_FOR_TESTS: &str =
    include_str!("../tests/fixtures/tempo_trace_response.json");

#[cfg(test)]
pub(crate) fn trace_from_fixture_for_tests() -> Trace {
    let resp: TraceResponse = serde_json::from_str(TRACE_FIXTURE_FOR_TESTS)
        .expect("trace fixture should parse");
    flatten_trace(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str =
        include_str!("../tests/fixtures/tempo_search_response.json");
    const TRACE_FIXTURE: &str = TRACE_FIXTURE_FOR_TESTS;

    #[test]
    fn search_response_deserialises() {
        let resp: SearchResponse =
            serde_json::from_str(SEARCH_FIXTURE).expect("search fixture should parse");
        assert!(!resp.traces.is_empty(), "fixture has at least one trace");
        let first = &resp.traces[0];
        // Catches traceID (capital D) vs traceId mismatches
        assert!(!first.trace_id.is_empty(), "trace_id is populated");
        assert!(!first.root_service_name.is_empty(), "root_service_name populated");
        assert!(!first.start_time_unix_nano.is_empty(), "start_time_unix_nano populated");
    }

    #[test]
    fn trace_response_flattens_spans() {
        let resp: TraceResponse =
            serde_json::from_str(TRACE_FIXTURE).expect("trace fixture should parse");
        let trace = flatten_trace(resp);
        assert!(!trace.spans.is_empty(), "trace has spans");
    }

    #[test]
    fn slow_query_events_survive_flattening() {
        let resp: TraceResponse =
            serde_json::from_str(TRACE_FIXTURE).expect("trace fixture should parse");
        let trace = flatten_trace(resp);
        let slow_query_count = trace
            .spans
            .iter()
            .flat_map(|s| &s.events)
            .filter(|e| e.name.starts_with("slow query"))
            .count();
        assert_eq!(slow_query_count, 2, "both slow query events preserved");
    }

    #[test]
    fn integer_attributes_parsed() {
        // Real fixture has a span with busy_ns as an intValue
        let resp: TraceResponse =
            serde_json::from_str(TRACE_FIXTURE).expect("trace fixture should parse");
        let trace = flatten_trace(resp);
        let span = trace
            .spans
            .iter()
            .find(|s| s.attributes.contains_key("busy_ns"))
            .expect("fixture has a span with busy_ns");
        assert!(
            span.attributes["busy_ns"].as_i64().is_some(),
            "busy_ns parsed as integer"
        );
    }

    #[test]
    fn parent_child_relationship_preserved() {
        let resp: TraceResponse =
            serde_json::from_str(TRACE_FIXTURE).expect("trace fixture should parse");
        let trace = flatten_trace(resp);
        let child = trace
            .spans
            .iter()
            .find(|s| s.name == "list_all_image_ids_by_most_recent")
            .expect("child span present");
        let parent_id = child.parent_span_id.as_deref().expect("child has a parent");
        let parent = trace
            .spans
            .iter()
            .find(|s| s.span_id == parent_id)
            .expect("parent span present in trace");
        assert_eq!(parent.name, "list_unscored_image_ids_for_version");
    }
}

fn flatten_trace(resp: TraceResponse) -> Trace {
    let spans = resp
        .batches
        .into_iter()
        .flat_map(|batch| batch.scope_spans)
        .flat_map(|scope| scope.spans)
        .map(|s| {
            let start = s.start_time_unix_nano.parse::<u64>().unwrap_or(0);
            let end = s.end_time_unix_nano.parse::<u64>().unwrap_or(0);
            let events = s
                .events
                .into_iter()
                .map(|e| SpanEvent {
                    name: e.name,
                    attributes: to_attr_map(e.attributes),
                })
                .collect();

            Span {
                span_id: s.span_id,
                parent_span_id: if s.parent_span_id.is_empty() {
                    None
                } else {
                    Some(s.parent_span_id)
                },
                name: s.name,
                duration_ns: end.saturating_sub(start),
                attributes: to_attr_map(s.attributes),
                events,
            }
        })
        .collect();

    Trace { spans }
}
