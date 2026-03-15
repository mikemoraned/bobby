use serde_json::Value;

const BLOCKED_LABEL_VALUES: &[&str] = &["porn", "sexual", "nudity"];

/// Check whether a `getPostThread` JSON response contains labels that should
/// cause the post to be blocked. Returns the list of blocked label values found.
pub fn blocked_labels(post_thread_json: &Value) -> Vec<String> {
    let mut found = Vec::new();

    let Some(labels) = post_thread_json
        .pointer("/thread/post/labels")
        .and_then(Value::as_array)
    else {
        return found;
    };

    for label in labels {
        if let Some(val) = label.get("val").and_then(Value::as_str) {
            if BLOCKED_LABEL_VALUES.contains(&val) {
                found.push(val.to_string());
            }
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_porn_label() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "labels": [
                        { "val": "porn", "src": "did:plc:someone" }
                    ]
                }
            }
        });
        assert_eq!(blocked_labels(&json), vec!["porn"]);
    }

    #[test]
    fn no_labels_returns_empty() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "labels": []
                }
            }
        });
        assert!(blocked_labels(&json).is_empty());
    }

    #[test]
    fn ignores_non_blocked_labels() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "labels": [
                        { "val": "spam", "src": "did:plc:someone" }
                    ]
                }
            }
        });
        assert!(blocked_labels(&json).is_empty());
    }
}
