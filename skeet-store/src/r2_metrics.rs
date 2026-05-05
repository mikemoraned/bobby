use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use lance_io::object_store::WrappingObjectStore;
use object_store::path::Path;
use object_store::{
    GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMultipartOptions,
    PutOptions, PutPayload, PutResult, Result as OSResult,
};
use opentelemetry::{KeyValue, metrics::{Counter, Histogram}};

/// Implements [`WrappingObjectStore`] to count R2 API operations via OTel metrics.
///
/// R2 billing class "A" covers writes and lists; class "B" covers reads.
#[derive(Debug)]
pub struct R2MetricsWrapper {
    cli_name: String,
    counter: Counter<u64>,
    bytes_counter: Counter<u64>,
    duration_histogram: Histogram<f64>,
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
        let duration_histogram = meter
            .f64_histogram("r2.duration")
            .with_description("Wall-clock duration of R2 object store operations in seconds")
            .with_unit("s")
            .with_boundaries(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ])
            .build();
        Self {
            cli_name: cli_name.to_string(),
            counter,
            bytes_counter,
            duration_histogram,
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
            duration_histogram: self.duration_histogram.clone(),
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
    duration_histogram: Histogram<f64>,
    cli_name: String,
    store_prefix: String,
}

impl std::fmt::Display for CountingObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountingObjectStore({})", self.inner)
    }
}

impl CountingObjectStore {
    fn recorder(
        &self,
        location: &Path,
        operation: &'static str,
        r2_class: &'static str,
    ) -> MetricsRecorder<'_> {
        MetricsRecorder::new(
            &self.counter,
            &self.bytes_counter,
            &self.duration_histogram,
            [
                KeyValue::new("operation", operation),
                KeyValue::new("r2_class", r2_class),
                KeyValue::new("cli", self.cli_name.clone()),
                KeyValue::new("store_prefix", self.store_prefix.clone()),
                KeyValue::new("table", table_from_path(location)),
                KeyValue::new("kind", kind_from_path(location)),
            ],
        )
    }
}

struct MetricsRecorder<'a> {
    counter: &'a Counter<u64>,
    bytes_counter: &'a Counter<u64>,
    duration_histogram: &'a Histogram<f64>,
    labels: [KeyValue; 6],
    bytes: Option<u64>,
    start: Instant,
}

impl<'a> MetricsRecorder<'a> {
    fn new(
        counter: &'a Counter<u64>,
        bytes_counter: &'a Counter<u64>,
        duration_histogram: &'a Histogram<f64>,
        labels: [KeyValue; 6],
    ) -> Self {
        Self {
            counter,
            bytes_counter,
            duration_histogram,
            labels,
            bytes: None,
            start: Instant::now(),
        }
    }

    const fn add_bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    fn completed(self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        self.counter.add(1, &self.labels);
        if let Some(bytes) = self.bytes {
            self.bytes_counter.add(bytes, &self.labels);
        }
        self.duration_histogram.record(elapsed, &self.labels);
    }
}

#[async_trait]
impl object_store::ObjectStore for CountingObjectStore {
    async fn put(&self, location: &Path, payload: PutPayload) -> OSResult<PutResult> {
        let recorder = self.recorder(location, "put", "A").add_bytes(payload.content_length() as u64);
        let result = self.inner.put(location, payload).await;
        recorder.completed();
        result
    }

    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> OSResult<PutResult> {
        let recorder = self.recorder(location, "put", "A").add_bytes(payload.content_length() as u64);
        let result = self.inner.put_opts(location, payload, opts).await;
        recorder.completed();
        result
    }

    async fn put_multipart(&self, location: &Path) -> OSResult<Box<dyn MultipartUpload>> {
        let recorder = self.recorder(location, "put_multipart", "A");
        let result = self.inner.put_multipart(location).await;
        recorder.completed();
        result
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> OSResult<Box<dyn MultipartUpload>> {
        let recorder = self.recorder(location, "put_multipart", "A");
        let result = self.inner.put_multipart_opts(location, opts).await;
        recorder.completed();
        result
    }

    async fn get(&self, location: &Path) -> OSResult<GetResult> {
        let recorder = self.recorder(location, "get", "B");
        let result = self.inner.get(location).await;
        recorder.completed();
        result
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> OSResult<GetResult> {
        let recorder = self.recorder(location, "get", "B");
        let result = self.inner.get_opts(location, options).await;
        recorder.completed();
        result
    }

    async fn get_range(&self, location: &Path, range: Range<u64>) -> OSResult<Bytes> {
        let recorder = self.recorder(location, "get_range", "B").add_bytes(bytes_for_range(&range));
        let result = self.inner.get_range(location, range).await;
        recorder.completed();
        result
    }

    async fn get_ranges(
        &self,
        location: &Path,
        ranges: &[Range<u64>],
    ) -> OSResult<Vec<Bytes>> {
        let recorder = self.recorder(location, "get_ranges", "B").add_bytes(bytes_for_ranges(ranges));
        let result = self.inner.get_ranges(location, ranges).await;
        recorder.completed();
        result
    }

    async fn head(&self, location: &Path) -> OSResult<ObjectMeta> {
        let recorder = self.recorder(location, "head", "B");
        let result = self.inner.head(location).await;
        recorder.completed();
        result
    }

    async fn delete(&self, location: &Path) -> OSResult<()> {
        let recorder = self.recorder(location, "delete", "A");
        let result = self.inner.delete(location).await;
        recorder.completed();
        result
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
        let recorder = self.recorder(prefix.unwrap_or(&empty), "list", "A");
        let result = self.inner.list(prefix);
        recorder.completed();
        result
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, OSResult<ObjectMeta>> {
        let empty = Path::from("");
        let recorder = self.recorder(prefix.unwrap_or(&empty), "list", "A");
        let result = self.inner.list_with_offset(prefix, offset);
        recorder.completed();
        result
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> OSResult<ListResult> {
        let empty = Path::from("");
        let recorder = self.recorder(prefix.unwrap_or(&empty), "list", "A");
        let result = self.inner.list_with_delimiter(prefix).await;
        recorder.completed();
        result
    }

    async fn copy(&self, from: &Path, to: &Path) -> OSResult<()> {
        let recorder = self.recorder(from, "copy", "A");
        let result = self.inner.copy(from, to).await;
        recorder.completed();
        result
    }

    async fn rename(&self, from: &Path, to: &Path) -> OSResult<()> {
        let recorder = self.recorder(from, "rename", "A");
        let result = self.inner.rename(from, to).await;
        recorder.completed();
        result
    }

    async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> OSResult<()> {
        let recorder = self.recorder(from, "copy_if_not_exists", "A");
        let result = self.inner.copy_if_not_exists(from, to).await;
        recorder.completed();
        result
    }

    async fn rename_if_not_exists(&self, from: &Path, to: &Path) -> OSResult<()> {
        let recorder = self.recorder(from, "rename_if_not_exists", "A");
        let result = self.inner.rename_if_not_exists(from, to).await;
        recorder.completed();
        result
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
    use test_support::{histogram_observation_count, sum_counter};

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

        assert_eq!(sum_counter(&provider, &exporter, "r2.bytes", None), 20);
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

        assert_eq!(sum_counter(&provider, &exporter, "r2.bytes", None), 40); // 10 + 30
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

        assert_eq!(sum_counter(&provider, &exporter, "r2.bytes", None), 42);
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

        assert_eq!(sum_counter(&provider, &exporter, "r2.bytes", None), 17);
    }

    #[tokio::test]
    async fn get_range_records_duration() {
        let (wrapper, provider, exporter) = make_test_wrapper();
        let inner = Arc::new(InMemory::new());
        let store = wrapper.wrap("test", inner.clone());
        let path = Path::from("test-object");
        inner
            .put(&path, Bytes::from(vec![0u8; 100]).into())
            .await
            .unwrap();

        store.get_range(&path, 0u64..50u64).await.unwrap();
        store.get_range(&path, 50u64..100u64).await.unwrap();

        assert_eq!(histogram_observation_count(&provider, &exporter, "r2.duration", None), 2);
    }
}
