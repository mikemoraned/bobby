use std::fmt;

use serde::Deserialize;

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub struct Did(pub String);

impl fmt::Display for Did {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub struct Collection(pub String);

impl fmt::Display for Collection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub struct Rkey(pub String);

#[derive(Debug, Deserialize)]
pub struct JetstreamEvent {
    pub did: Did,
    #[allow(dead_code)]
    pub time_us: u64,
    pub kind: EventKind,
    pub commit: Option<Commit>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Commit,
    Identity,
    Account,
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventKind::Commit => write!(f, "commit"),
            EventKind::Identity => write!(f, "identity"),
            EventKind::Account => write!(f, "account"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Commit {
    pub operation: Operation,
    pub collection: Collection,
    #[allow(dead_code)]
    pub rkey: Rkey,
    pub record: Option<serde_json::Value>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Create,
    Update,
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_real_jetstream_message() {
        let json = r#"{
            "did": "did:plc:abc123",
            "time_us": 1700000000000000,
            "kind": "commit",
            "commit": {
                "rev": "3abc",
                "operation": "create",
                "collection": "app.bsky.feed.post",
                "rkey": "3abc123",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "hello world",
                    "createdAt": "2024-01-01T00:00:00Z"
                },
                "cid": "bafyrei5678"
            }
        }"#;

        let event: JetstreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.did, Did("did:plc:abc123".to_string()));
        assert_eq!(event.kind, EventKind::Commit);
        assert!(event.commit.is_some());
        let commit = event.commit.as_ref().unwrap();
        assert_eq!(commit.operation, Operation::Create);
        assert_eq!(
            commit.collection,
            Collection("app.bsky.feed.post".to_string())
        );
    }

    #[test]
    fn deserializes_identity_event() {
        let json = r#"{
            "did": "did:web:example.com",
            "time_us": 1700000000000000,
            "kind": "identity"
        }"#;

        let event: JetstreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.kind, EventKind::Identity);
        assert!(event.commit.is_none());
    }
}
