#![warn(clippy::all, clippy::nursery)]

use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use clap::Parser;
use skeet_store::{
    IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME, StoreArgs,
    TABLE_NAME, VALIDATE_TABLE_NAME,
};
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    about = "Count manifest objects in each table's _versions/ prefix on R2; also report LIST page count and oldest object age"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

fn parse_s3_path(store_path: &str) -> Option<(String, String)> {
    let stripped = store_path.strip_prefix("s3://")?;
    let (bucket, prefix) = stripped.split_once('/')?;
    Some((bucket.to_string(), prefix.to_string()))
}

struct VersionsReport {
    table: &'static str,
    object_count: u64,
    list_pages: u64,
    oldest_age_hours: Option<f64>,
    newest_age_hours: Option<f64>,
}

async fn count_versions(
    client: &Client,
    bucket: &str,
    store_prefix: &str,
    table: &'static str,
) -> Result<VersionsReport, Box<dyn std::error::Error>> {
    let prefix = format!("{store_prefix}/{table}.lance/_versions/");
    let mut continuation_token: Option<String> = None;
    let mut object_count: u64 = 0;
    let mut list_pages: u64 = 0;
    let mut oldest_ts: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut newest_ts: Option<chrono::DateTime<chrono::Utc>> = None;

    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(&prefix);
        if let Some(token) = &continuation_token {
            req = req.continuation_token(token);
        }
        let resp = req.send().await?;
        list_pages += 1;

        for obj in resp.contents() {
            object_count += 1;
            if let Some(modified) = obj.last_modified() {
                let secs = modified.secs();
                let nanos = modified.subsec_nanos();
                if let Some(ts) =
                    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
                {
                    if oldest_ts.map(|t| ts < t).unwrap_or(true) {
                        oldest_ts = Some(ts);
                    }
                    if newest_ts.map(|t| ts > t).unwrap_or(true) {
                        newest_ts = Some(ts);
                    }
                }
            }
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation_token = resp.next_continuation_token().map(Into::into);
        } else {
            break;
        }
    }

    let now = chrono::Utc::now();
    let to_hours = |t: chrono::DateTime<chrono::Utc>| {
        (now - t).num_seconds() as f64 / 3600.0
    };

    Ok(VersionsReport {
        table,
        object_count,
        list_pages,
        oldest_age_hours: oldest_ts.map(to_hours),
        newest_age_hours: newest_ts.map(to_hours),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");
    info!(git_hash = env!("BUILD_GIT_HASH"), "count-versions starting");

    let args = Args::parse();

    let (bucket, store_prefix) = parse_s3_path(&args.store.store_path)
        .ok_or("store-path must be an S3 URI like s3://bucket/prefix")?;

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

    let tables = [
        TABLE_NAME,
        SCORE_TABLE_NAME,
        SKEET_APPRAISAL_TABLE_NAME,
        IMAGE_APPRAISAL_TABLE_NAME,
        VALIDATE_TABLE_NAME,
    ];

    println!(
        "{:<32} {:>10} {:>10} {:>14} {:>14}",
        "table", "manifests", "list_pages", "oldest_h", "newest_h"
    );
    println!("{}", "-".repeat(86));
    for table in tables {
        let report = count_versions(&client, &bucket, &store_prefix, table).await?;
        let oldest = report
            .oldest_age_hours
            .map(|h| format!("{h:.1}"))
            .unwrap_or_else(|| "—".into());
        let newest = report
            .newest_age_hours
            .map(|h| format!("{h:.1}"))
            .unwrap_or_else(|| "—".into());
        println!(
            "{:<32} {:>10} {:>10} {:>14} {:>14}",
            report.table, report.object_count, report.list_pages, oldest, newest
        );
        if report.list_pages > 1 {
            warn!(
                table = report.table,
                manifests = report.object_count,
                pages = report.list_pages,
                "_versions/ LIST requires multiple R2 pages — pruning would reduce per-LIST cost"
            );
        }
    }

    Ok(())
}
