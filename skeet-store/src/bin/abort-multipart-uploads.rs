#![warn(clippy::all, clippy::nursery)]

use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use clap::Parser;
use skeet_store::StoreArgs;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(about = "List and abort incomplete multipart uploads in the store's S3 bucket")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Actually abort the uploads (default is dry-run)
    #[arg(long)]
    abort: bool,
}

fn parse_s3_path(store_path: &str) -> Option<(String, String)> {
    let stripped = store_path.strip_prefix("s3://")?;
    let (bucket, prefix) = stripped.split_once('/')?;
    Some((bucket.to_string(), prefix.to_string()))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let (bucket, prefix) = parse_s3_path(&args.store.store_path).ok_or(
        "store-path must be an S3 URI like s3://bucket/prefix",
    )?;

    let endpoint = args
        .store
        .s3_endpoint
        .as_ref()
        .ok_or("--s3-endpoint (or BOBBY_S3_ENDPOINT) is required")?;

    let access_key_id = args
        .store
        .s3_access_key_id
        .as_ref()
        .ok_or("--s3-access-key-id (or BOBBY_S3_ACCESS_KEY_ID) is required")?;

    let secret_access_key = args
        .store
        .s3_secret_access_key
        .as_ref()
        .ok_or("--s3-secret-access-key (or BOBBY_S3_SECRET_ACCESS_KEY) is required")?;

    let credentials =
        Credentials::new(access_key_id, secret_access_key, None, None, "bobby-env");

    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(Region::new(args.store.s3_region.clone()))
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .load()
        .await;

    let client = Client::new(&config);

    info!(bucket = %bucket, prefix = %prefix, "listing incomplete multipart uploads");

    let mut total = 0u64;
    let mut aborted = 0u64;
    let mut key_marker: Option<String> = None;
    let mut upload_id_marker: Option<String> = None;

    loop {
        let mut request = client
            .list_multipart_uploads()
            .bucket(&bucket)
            .prefix(&prefix);

        if let Some(km) = &key_marker {
            request = request.key_marker(km);
        }
        if let Some(uim) = &upload_id_marker {
            request = request.upload_id_marker(uim);
        }

        let response = request.send().await?;

        let uploads = response.uploads();
        if uploads.is_empty() {
            break;
        }

        for upload in uploads {
            let key = upload.key().unwrap_or("<unknown>");
            let upload_id = upload.upload_id().unwrap_or("<unknown>");
            let initiated = upload
                .initiated()
                .map(|t| t.to_string())
                .unwrap_or_default();

            total += 1;

            if args.abort {
                match client
                    .abort_multipart_upload()
                    .bucket(&bucket)
                    .key(key)
                    .upload_id(upload_id)
                    .send()
                    .await
                {
                    Ok(_) => {
                        aborted += 1;
                        info!(key = %key, upload_id = %upload_id, initiated = %initiated, "aborted");
                    }
                    Err(e) => {
                        error!(key = %key, upload_id = %upload_id, error = %e, "failed to abort");
                    }
                }
            } else {
                info!(key = %key, upload_id = %upload_id, initiated = %initiated, "found incomplete upload");
            }
        }

        if response.is_truncated().unwrap_or(false) {
            key_marker = response.next_key_marker().map(Into::into);
            upload_id_marker = response.next_upload_id_marker().map(Into::into);
        } else {
            break;
        }
    }

    if args.abort {
        info!(total = total, aborted = aborted, "done");
    } else {
        info!(total = total, "dry run complete — re-run with --abort to remove them");
        if total > 0 {
            warn!("use --abort to actually remove the incomplete uploads");
        }
    }

    Ok(())
}
