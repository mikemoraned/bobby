use tracing::instrument;

use crate::{SkeetStore, StoreError};

const DEFAULT_COMPACT_EVERY_N_WRITES: u64 = 100;

#[derive(Clone, Debug, clap::Args)]
pub struct StoreArgs {
    /// Store location: local path or S3 URI (e.g. s3://bucket/path)
    #[arg(long)]
    pub store_path: String,

    /// S3-compatible endpoint URL (e.g. https://<account>.r2.cloudflarestorage.com)
    #[arg(long, env = "BOBBY_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    /// S3 access key ID
    #[arg(long, env = "BOBBY_S3_ACCESS_KEY_ID")]
    pub s3_access_key_id: Option<String>,

    /// S3 secret access key
    #[arg(long, env = "BOBBY_S3_SECRET_ACCESS_KEY")]
    pub s3_secret_access_key: Option<String>,

    /// S3 region (default: auto, suitable for Cloudflare R2)
    #[arg(long, default_value = "auto")]
    pub s3_region: String,

    /// SSE-C encryption key (base64-encoded 256-bit AES key); enables server-side encryption
    #[arg(long, env = "BOBBY_SSE_C_KEY")]
    pub sse_c_key: Option<String>,

    /// Trigger automatic compaction after this many writes (omit to disable)
    #[arg(long, default_value_t = DEFAULT_COMPACT_EVERY_N_WRITES)]
    pub compact_every_n_writes: u64,
}

impl StoreArgs {
    const fn compact_option(&self) -> Option<u64> {
        Some(self.compact_every_n_writes)
    }

    pub fn storage_options(&self) -> Vec<(String, String)> {
        let mut opts = Vec::new();
        if let Some(endpoint) = &self.s3_endpoint {
            opts.push(("aws_endpoint".into(), endpoint.clone()));
        }
        if let Some(key_id) = &self.s3_access_key_id {
            opts.push(("aws_access_key_id".into(), key_id.clone()));
        }
        if let Some(secret) = &self.s3_secret_access_key {
            opts.push(("aws_secret_access_key".into(), secret.clone()));
        }
        opts.push(("aws_region".into(), self.s3_region.clone()));
        opts.push(("timeout".into(), "120s".into()));
        opts.push(("connect_timeout".into(), "10s".into()));
        opts.push(("client_max_retries".into(), "3".into()));
        opts.push(("client_retry_timeout".into(), "300".into()));
        if let Some(key) = &self.sse_c_key {
            opts.push(("aws_server_side_encryption".into(), "sse-c".into()));
            opts.push(("aws_sse_customer_key_base64".into(), key.clone()));
        }
        opts
    }

    #[instrument(skip(self), fields(store_path = %self.store_path))]
    pub async fn open_store(&self) -> Result<SkeetStore, StoreError> {
        SkeetStore::open(
            &self.store_path,
            self.storage_options(),
            self.compact_option(),
        )
        .await
    }
}
