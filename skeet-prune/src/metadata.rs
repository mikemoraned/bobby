use shared::skeet_id::SkeetId;

const BSKY_PUBLIC_API: &str = "https://public.api.bsky.app/xrpc";

/// Fetch the `getPostThread` JSON for the given skeet.
pub async fn fetch_post_thread(
    http: &reqwest::Client,
    skeet_id: &SkeetId,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let uri = skeet_id.to_string();
    let resp = http
        .get(format!("{BSKY_PUBLIC_API}/app.bsky.feed.getPostThread"))
        .query(&[("uri", uri.as_str()), ("depth", "0"), ("parentHeight", "0")])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("API error ({status}): {body}").into());
    }

    Ok(resp.json().await?)
}
