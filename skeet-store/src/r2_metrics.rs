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
}

impl R2MetricsWrapper {
    pub fn new(cli_name: &str) -> Self {
        let meter = opentelemetry::global::meter("r2");
        let counter = meter
            .u64_counter("r2.operations")
            .with_description("Count of R2 object store operations by type and CLI")
            .with_unit("operations")
            .build();
        Self {
            cli_name: cli_name.to_string(),
            counter,
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
            cli_name: self.cli_name.clone(),
            store_prefix: store_prefix.to_string(),
        })
    }
}

#[derive(Debug)]
struct CountingObjectStore {
    inner: Arc<dyn object_store::ObjectStore>,
    counter: Counter<u64>,
    cli_name: String,
    store_prefix: String,
}

impl std::fmt::Display for CountingObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountingObjectStore({})", self.inner)
    }
}

impl CountingObjectStore {
    fn record(&self, operation: &'static str, r2_class: &'static str) {
        self.counter.add(
            1,
            &[
                KeyValue::new("operation", operation),
                KeyValue::new("r2_class", r2_class),
                KeyValue::new("cli", self.cli_name.clone()),
                KeyValue::new("store_prefix", self.store_prefix.clone()),
            ],
        );
    }
}

#[async_trait]
impl object_store::ObjectStore for CountingObjectStore {
    async fn put(&self, location: &Path, payload: PutPayload) -> OSResult<PutResult> {
        self.record("put", "A");
        self.inner.put(location, payload).await
    }

    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> OSResult<PutResult> {
        self.record("put", "A");
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart(&self, location: &Path) -> OSResult<Box<dyn MultipartUpload>> {
        self.record("put_multipart", "A");
        self.inner.put_multipart(location).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> OSResult<Box<dyn MultipartUpload>> {
        self.record("put_multipart", "A");
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get(&self, location: &Path) -> OSResult<GetResult> {
        self.record("get", "B");
        self.inner.get(location).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> OSResult<GetResult> {
        self.record("get", "B");
        self.inner.get_opts(location, options).await
    }

    async fn get_range(&self, location: &Path, range: Range<u64>) -> OSResult<Bytes> {
        self.record("get_range", "B");
        self.inner.get_range(location, range).await
    }

    async fn get_ranges(
        &self,
        location: &Path,
        ranges: &[Range<u64>],
    ) -> OSResult<Vec<Bytes>> {
        self.record("get_ranges", "B");
        self.inner.get_ranges(location, ranges).await
    }

    async fn head(&self, location: &Path) -> OSResult<ObjectMeta> {
        self.record("head", "B");
        self.inner.head(location).await
    }

    async fn delete(&self, location: &Path) -> OSResult<()> {
        self.record("delete", "A");
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
        self.record("list", "A");
        self.inner.list(prefix)
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, OSResult<ObjectMeta>> {
        self.record("list", "A");
        self.inner.list_with_offset(prefix, offset)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> OSResult<ListResult> {
        self.record("list", "A");
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record("copy", "A");
        self.inner.copy(from, to).await
    }

    async fn rename(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record("rename", "A");
        self.inner.rename(from, to).await
    }

    async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record("copy_if_not_exists", "A");
        self.inner.copy_if_not_exists(from, to).await
    }

    async fn rename_if_not_exists(&self, from: &Path, to: &Path) -> OSResult<()> {
        self.record("rename_if_not_exists", "A");
        self.inner.rename_if_not_exists(from, to).await
    }
}
