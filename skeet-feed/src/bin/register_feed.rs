#![warn(clippy::all, clippy::nursery)]

use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Bluesky handle (e.g. yourname.bsky.social)
    #[arg(long, env = "BOBBY_BSKY_APP_REGISTER_HANDLE")]
    handle: String,

    /// Bluesky app password
    #[arg(long, env = "BOBBY_BSKY_APP_REGISTER_PASSWORD")]
    app_password: String,

    /// Hostname of the feed generator (e.g. bobby-staging.houseofmoran.io)
    #[arg(long)]
    hostname: String,

    /// Feed name identifier
    #[arg(long, default_value = "bobby-dev")]
    feed_name: String,

    /// Display name shown in Bluesky
    #[arg(long, default_value = "Bobby Dev")]
    display_name: String,

    /// Description shown in Bluesky
    #[arg(long, default_value = "Selfies with landmarks, found by Bobby")]
    description: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let did = format!("did:web:{}", args.hostname);
    let feed_uri = format!("at://{did}/app.bsky.feed.generator/{}", args.feed_name);

    println!("Registering feed:");
    println!("  DID: {did}");
    println!("  Feed URI: {feed_uri}");
    println!("  Display name: {}", args.display_name);
    println!("  Description: {}", args.description);

    let client = reqwest::Client::new();

    // Step 1: Create session (login)
    let session_resp = client
        .post("https://bsky.social/xrpc/com.atproto.server.createSession")
        .json(&serde_json::json!({
            "identifier": args.handle,
            "password": args.app_password,
        }))
        .send()
        .await?;

    if !session_resp.status().is_success() {
        let status = session_resp.status();
        let body = session_resp.text().await?;
        return Err(format!("login failed ({status}): {body}").into());
    }

    let session: serde_json::Value = session_resp.json().await?;
    let access_jwt = session["accessJwt"]
        .as_str()
        .ok_or("missing accessJwt in session response")?;
    let user_did = session["did"]
        .as_str()
        .ok_or("missing did in session response")?;

    println!("  Logged in as: {user_did}");

    // Step 2: Create the feed generator record
    let record = serde_json::json!({
        "$type": "app.bsky.feed.generator",
        "did": did,
        "displayName": args.display_name,
        "description": args.description,
        "createdAt": chrono::Utc::now().to_rfc3339(),
    });

    let create_resp = client
        .post("https://bsky.social/xrpc/com.atproto.repo.putRecord")
        .header("Authorization", format!("Bearer {access_jwt}"))
        .json(&serde_json::json!({
            "repo": user_did,
            "collection": "app.bsky.feed.generator",
            "rkey": args.feed_name,
            "record": record,
        }))
        .send()
        .await?;

    if !create_resp.status().is_success() {
        let status = create_resp.status();
        let body = create_resp.text().await?;
        return Err(format!("failed to create feed record ({status}): {body}").into());
    }

    let result: serde_json::Value = create_resp.json().await?;
    println!("Feed registered successfully!");
    println!("  URI: {}", result["uri"]);

    Ok(())
}
