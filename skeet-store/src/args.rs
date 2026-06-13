use tracing::instrument;

use crate::{SkeetStore, StoreError};

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
}

impl StoreArgs {
    /// Whether `store_path` points at a remote, shared object store (`s3://`,
    /// which includes Cloudflare R2) rather than a local directory. Pure
    /// inspection of the configured path; callers decide what to do with it
    /// (e.g. the pruner refuses to write a remote shared store by default).
    pub fn is_remote(&self) -> bool {
        self.store_path.starts_with("s3://")
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
    pub async fn open_store(&self, cli_name: &str) -> Result<SkeetStore, StoreError> {
        SkeetStore::open(
            &self.store_path,
            self.storage_options(),
            cli_name,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with(store_path: &str) -> StoreArgs {
        StoreArgs {
            store_path: store_path.to_string(),
            s3_endpoint: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            s3_region: "auto".into(),
            sse_c_key: None,
        }
    }

    #[test]
    fn s3_paths_are_remote() {
        assert!(args_with("s3://hom-bobby/encrypted-store").is_remote());
    }

    #[test]
    fn local_paths_are_not_remote() {
        assert!(!args_with("/tmp/store").is_remote());
        assert!(!args_with("file:///tmp/store").is_remote());
        assert!(!args_with("./store").is_remote());
    }
}
