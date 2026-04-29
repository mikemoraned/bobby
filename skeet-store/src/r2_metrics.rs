use std::ops::Range;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use lance_io::object_store::WrappingObjectStore;
use object_store::path::Path;
use object_store::{
    GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMultipartOptions,
    PutOptions, PutPayload, PutResult, Result as OSResult,
};
use opentelemetry::{KeyValue, metrics::Counter};

/// Implements [`WrappingObjectStore`] to count R2 API operations via OTel metrics.
///
/// R2 billing class "A" covers writes and lists; class "B" covers reads.
#[derive(Debug)]
pub struct R2MetricsWrapper {
    cli_name: String,
    counter: Counter<u64>,
    bytes_counter: Counter<u64>,
}

impl R2MetricsWrapper {
    pub fn new(cli_name: &str, meter: opentelemetry::metrics::Meter) -> Self {
        let counter = meter
            .u64_counter("r2.operations")
            .with_description("Count of R2 object store operations by type and CLI")
            .with_unit("operations")
            .build();
        let bytes_counter = meter
            .u64_counter("r2.bytes")
            .with_description("Bytes transferred in R2 object store operations by type and CLI")
            .with_unit("bytes")
            .build();
        Self {
            cli_name: cli_name.to_string(),
            counter,
            bytes_counter,
        }
    }
}

impl WrappingObjectStore for R2MetricsWrapper {
    fn wrap(
        &self,
        store_prefix: &str,
        original: Arc<dyn object_store::ObjectStore>,
    ) -> Arc<dyn object_store::ObjectStore> {
        Arc::new(CountingObjectStore {
            inner: original,
            counter: self.counter.clone(),
            bytes_counter: self.bytes_counter.clone(),
            cli_name: self.cli_name.clone(),
            store_prefix: store_prefix.to_string(),
        })
    }
}

#[derive(Debug)]
struct CountingObjectStore {
    inner: Arc<dyn object_store::ObjectStore>,
    counter: Counter<u64>,
    bytes_counter: Counter<u64>,
    cli_name: String,
    store_prefix: String,
}

impl std::fmt::Display for CountingObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountingObjectStore({})", self.inner)
    }
}

impl CountingObjectStore {
    fn labels(
        &self,
        location: &Path,
        operation: &'static str,
        r2_class: &'static str,
    ) -> [KeyValue; 6] {
        [
            KeyValue::new("operation", operation),
            KeyValue::new("r2_class", r2_class),
            KeyValue::new("cli", self.cli_name.clone()),
            KeyValue::new("store_prefix", self.store_prefix.clone()),
            KeyValue::new("table", table_from_path(location)),
            KeyValue::new("kind", kind_from_path(location)),
        ]
    }

    fn record(&self, location: &Path, operation: &'static str, r2_class: &'static str) {
        self.counter.add(1, &self.labels(location, operation, r2_class));
    }

    fn record_bytes(
        &self,
        location: &Path,
        operation: &'static str,
        r2_class: &'static str,
        bytes: u64,
    ) {
        self.counter.add(1, &self.labels(location, operation, r2_class));
        self.bytes_counter
            .add(bytes, &self.labels(location, operation, r2_class));
    }
}

#[async_trait]
impl object_store::ObjectStore for CountingObjectStore {
    async fn put(&self, location: &Path, payload: PutPayload) -> OSResult<PutResult> {
        self.record_bytes(location, "put", "A", payload.content_length() as u64);
        self.inner.put(location, payload).await
    }

    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> OSResult<PutResult> {
        self.record_bytes(location, "put", "A", payload.content_length() as u64);
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart(&self, location: &Path) -> OSResult<Box<dyn MultipartUpload>> {
        self.record(location, "put_multipart", "A");
        self.inner.put_multipart(location).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> OSResult<Box<dyn MultipartUpload>> {
        self.record(location, "put_multipart", "A");
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get(&self, location: &Path) -> OSResult<GetResult> {
        self.record(location, "get", "B");
        self.inner.get(location).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> OSResult<GetResult> {
        self.record(location, "get", "B");
        self.inner.get_opts(location, options).await
    }

    async fn get_range(&self, location: &Path, range: Range<u64>) -> OSResult<Bytes> {
        self.record_bytes(location, "get_range", "B", bytes_for_range(&range));
        self.inner.get_range(location, range).await
    }

    async fn get_ranges(
        &self,
        location: &Path,
        ranges: &[Range<u64>],
    ) -> OSResult<Vec<Bytes>> {
        self.record_bytes(location, "get_ranges", "B", bytes_for_ranges(ranges));
        self.inner.get_ranges(location, ranges).await
    }

    async fn head(&self, location: &Path) -> OSResult<ObjectMeta> {
        self.record(location, "head", "B");
        self.inner.head(location).await
    }

    async fn delete(&self, location: &Path) -> OSResult<()> {
        self.record(location, "delete", "A");
        self.inner.delete(location).await
    }

    fn delete_stream<'a>(
        &'a self,
        locations: BoxStream<'a, OSResult<Path>>,
    ) -> BoxStream<'a, OSResult<Path>> {
        // individual deletes are already counted via delete()
        self.inner.delete_stream(locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, OSResult<ObjectMeta>> {
        let empty = Path::from("");
        self.record(prefix.unwrap_or(&empty), "list", "A");
        self.inner.list(prefix)
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, OSResult<ObjectMeta>> {
        let empty = Path::from("");
        self.record(prefix.unwrap_or(&empty), "list", "A");
        self.inner.list_with_offset(prefix, offset)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> OSResult<ListResult> {
        let empty = Path::from("");
        self.record(prefix.unwrap_or(&empty), "list", "A");
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record(from, "copy", "A");
        self.inner.copy(from, to).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record(from, "rename", "A");
        self.inner.rename(from, to).await
    }

    async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record(from, "copy_if_not_exists", "A");
        self.inner.copy_if_not_exists(from, to).await
    }

    async fn rename_if_not_exists(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record(from, "rename_if_not_exists", "A");
        self.inner.rename_if_not_exists(from, to).await
    }
}

const fn bytes_for_range(range: &Range<u64>) -> u64 {
    range.end - range.start
}

fn bytes_for_ranges(ranges: &[Range<u64>]) -> u64 {
    ranges.iter().map(bytes_for_range).sum()
}

fn table_from_path(location: &Path) -> String {
    location
        .parts()
        .find(|part| part.as_ref().ends_with(".lance"))
        .map_or_else(|| "unknown".to_string(), |part| part.as_ref().to_string())
}

/// Classify the path segment immediately after the `<table>.lance/` directory.
///
/// Returns one of: `data`, `_indices`, `_versions`, `_transactions`, `manifest`
/// (for top-level `.manifest` files inside the `.lance/` dir), or `other`.
fn kind_from_path(location: &Path) -> &'static str {
    let mut parts = location.parts();
    while let Some(part) = parts.next() {
        if part.as_ref().ends_with(".lance") {
            return parts.next().map_or("other", |next| match next.as_ref() {
                "data" => "data",
                "_indices" => "_indices",
                "_versions" => "_versions",
                "_transactions" => "_transactions",
                s if s.ends_with(".manifest") => "manifest",
                _ => "other",
            });
        }
    }
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use object_store::memory::InMemory;
    use object_store::ObjectStore;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

    fn make_test_wrapper() -> (
        R2MetricsWrapper,
        SdkMeterProvider,
        InMemoryMetricExporter,
    ) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let meter = provider.meter("r2");
        let wrapper = R2MetricsWrapper::new("test-cli", meter);
        (wrapper, provider, exporter)
    }

    fn total_bytes(exporter: &InMemoryMetricExporter, provider: &SdkMeterProvider) -> u64 {
        provider.force_flush().unwrap();
        let metrics = exporter.get_finished_metrics().unwrap();
        metrics
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .filter(|m| m.name() == "r2.bytes")
            .flat_map(|m| {
                use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
                if let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() {
                    sum.data_points().map(|dp| dp.value()).collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .sum()
    }

    #[test]
    fn table_from_path_extracts_lance_segment() {
        assert_eq!(
            table_from_path(&Path::from("encrypted-store/images_v6.lance/data/abc.lance")),
            "images_v6.lance"
        );
    }

    #[test]
    fn table_from_path_falls_back_to_unknown_when_no_lance_segment() {
        assert_eq!(
            table_from_path(&Path::from("encrypted-store/data/abc.arrow")),
            "unknown"
        );
    }

    #[test]
    fn table_from_path_falls_back_to_unknown_for_empty_path() {
        assert_eq!(table_from_path(&Path::from("")), "unknown");
    }

    #[test]
    fn kind_from_path_data_segment() {
        assert_eq!(
            kind_from_path(&Path::from("encrypted-store/images_v6.lance/data/abc.lance")),
            "data"
        );
    }

    #[test]
    fn kind_from_path_indices_segment() {
        assert_eq!(
            kind_from_path(&Path::from(
                "encrypted-store/images_score_v2.lance/_indices/uuid-abc/index.idx"
            )),
            "_indices"
        );
    }

    #[test]
    fn kind_from_path_versions_segment() {
        assert_eq!(
            kind_from_path(&Path::from(
                "encrypted-store/images_score_v2.lance/_versions/123.manifest"
            )),
            "_versions"
        );
    }

    #[test]
    fn kind_from_path_transactions_segment() {
        assert_eq!(
            kind_from_path(&Path::from(
                "encrypted-store/images_v6.lance/_transactions/0-abc.txn"
            )),
            "_transactions"
        );
    }

    #[test]
    fn kind_from_path_top_level_manifest_file() {
        assert_eq!(
            kind_from_path(&Path::from(
                "encrypted-store/images_v6.lance/_latest.manifest"
            )),
            "manifest"
        );
    }

    #[test]
    fn kind_from_path_unknown_segment_is_other() {
        assert_eq!(
            kind_from_path(&Path::from(
                "encrypted-store/images_v6.lance/something_unexpected/x"
            )),
            "other"
        );
    }

    #[test]
    fn kind_from_path_no_lance_segment_is_other() {
        assert_eq!(
            kind_from_path(&Path::from("encrypted-store/data/abc.arrow")),
            "other"
        );
    }

    #[test]
    fn kind_from_path_empty_path_is_other() {
        assert_eq!(kind_from_path(&Path::from("")), "other");
    }

    #[test]
    fn kind_from_path_lance_segment_with_no_child_is_other() {
        assert_eq!(
            kind_from_path(&Path::from("encrypted-store/images_v6.lance")),
            "other"
        );
    }

    #[test]
    fn bytes_for_range_returns_length() {
        assert_eq!(bytes_for_range(&(10u64..20u64)), 10);
        assert_eq!(bytes_for_range(&(0u64..100u64)), 100);
        assert_eq!(bytes_for_range(&(5u64..5u64)), 0);
    }

    #[test]
    fn bytes_for_ranges_sums_lengths() {
        assert_eq!(bytes_for_ranges(&[10u64..20u64, 30u64..50u64]), 30);
        assert_eq!(bytes_for_ranges(&[]), 0);
    }

    #[tokio::test]
    async fn get_range_records_bytes() {
        let (wrapper, provider, exporter) = make_test_wrapper();
        let inner = Arc::new(InMemory::new());
        let store = wrapper.wrap("test", inner.clone());
        let path = Path::from("test-object");
        inner
            .put(&path, Bytes::from(vec![0u8; 100]).into())
            .await
            .unwrap();

        store.get_range(&path, 10u64..30u64).await.unwrap();

        assert_eq!(total_bytes(&exporter, &provider), 20);
    }

    #[tokio::test]
    async fn get_ranges_records_summed_bytes() {
        let (wrapper, provider, exporter) = make_test_wrapper();
        let inner = Arc::new(InMemory::new());
        let store = wrapper.wrap("test", inner.clone());
        let path = Path::from("test-object");
        inner
            .put(&path, Bytes::from(vec![0u8; 100]).into())
            .await
            .unwrap();

        store
            .get_ranges(&path, &[0u64..10u64, 20u64..50u64])
            .await
            .unwrap();

        assert_eq!(total_bytes(&exporter, &provider), 40); // 10 + 30
    }

    #[tokio::test]
    async fn put_records_payload_bytes() {
        let (wrapper, provider, exporter) = make_test_wrapper();
        let inner = Arc::new(InMemory::new());
        let store = wrapper.wrap("test", inner.clone());
        let path = Path::from("test-object");

        store
            .put(&path, Bytes::from(vec![0u8; 42]).into())
            .await
            .unwrap();

        assert_eq!(total_bytes(&exporter, &provider), 42);
    }

    #[tokio::test]
    async fn put_opts_records_payload_bytes() {
        let (wrapper, provider, exporter) = make_test_wrapper();
        let inner = Arc::new(InMemory::new());
        let store = wrapper.wrap("test", inner.clone());
        let path = Path::from("test-object");

        store
            .put_opts(&path, Bytes::from(vec![0u8; 17]).into(), PutOptions::default())
            .await
            .unwrap();

        assert_eq!(total_bytes(&exporter, &provider), 17);
    }
}
