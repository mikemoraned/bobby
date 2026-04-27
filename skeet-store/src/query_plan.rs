use std::sync::LazyLock;

use regex::Regex;

/// Parsed shape of a lancedb `explain_plan` string.
///
/// Lancedb's `explain_plan` returns datafusion's free-form `Display` output (no
/// `Serialize` impl), so parsing has to live on our side. Doing it once at log
/// time and emitting flat fields lets Tempo/TraceQL filter on individual
/// attributes (e.g. `event.plan.full_scan=true`) instead of forcing each
/// consumer to re-parse.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QueryPlan {
    pub table: Option<String>,
    pub columns: Option<String>,
    pub num_fragments: Option<u64>,
    pub full_filter: Option<String>,
    pub index: Option<String>,
}

// Matches one `key=value` field, where `value` is either `[bracketed]`
// (allowing internal commas, as in `projection=[a, b]`) or anything up to the
// next `, ` separator.
static FIELD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\w+)=(\[[^\]]*\]|[^,]*)").expect("static regex"));

impl QueryPlan {
    pub fn parse(raw: &str) -> Self {
        let mut plan = Self::default();
        for line in raw.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("LanceRead: ") {
                for cap in FIELD_RE.captures_iter(rest) {
                    let (_, [key, value]) = cap.extract();
                    match key {
                        "uri" => plan.table = lance_segment(value).map(str::to_owned),
                        "projection" => {
                            plan.columns = Some(value.trim_matches(['[', ']']).to_owned());
                        }
                        "num_fragments" => plan.num_fragments = value.parse().ok(),
                        "full_filter" if value != "--" => {
                            plan.full_filter = Some(value.to_owned());
                        }
                        _ => {}
                    }
                }
            } else if line.starts_with("ScalarIndexQuery:") {
                plan.index = Some(line.to_owned());
            }
        }
        plan
    }

    pub const fn full_scan(&self) -> bool {
        self.num_fragments.is_some() && self.full_filter.is_none()
    }
}

fn lance_segment(uri: &str) -> Option<&str> {
    uri.split('/').find(|s| s.ends_with(".lance"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real plan strings captured from the 25th Apr spike
    const FULL_SCAN_PLAN: &str = "LanceRead: uri=encrypted-store/images_v6.lance/data, projection=[image_id, discovered_at], num_fragments=66, range_before=None, range_after=None, row_id=false, row_addr=false, full_filter=--, refine_filter=--\n";
    const INDEXED_PLAN: &str = "LanceRead: uri=encrypted-store/images_score_v2.lance/data, projection=[image_id], num_fragments=4, range_before=None, range_after=None, row_id=false, row_addr=false, full_filter=model_version = Utf8(\"ea219ee0\"), refine_filter=--\n  ScalarIndexQuery: query=[model_version = ea219ee0]@model_version_idx\n";

    #[test]
    fn parses_full_scan_plan() {
        let plan = QueryPlan::parse(FULL_SCAN_PLAN);
        assert_eq!(plan.table.as_deref(), Some("images_v6.lance"));
        assert_eq!(plan.columns.as_deref(), Some("image_id, discovered_at"));
        assert_eq!(plan.num_fragments, Some(66));
        assert_eq!(plan.full_filter, None);
        assert_eq!(plan.index, None);
        assert!(plan.full_scan());
    }

    #[test]
    fn parses_indexed_query_plan() {
        let plan = QueryPlan::parse(INDEXED_PLAN);
        assert_eq!(plan.table.as_deref(), Some("images_score_v2.lance"));
        assert_eq!(plan.columns.as_deref(), Some("image_id"));
        assert_eq!(plan.num_fragments, Some(4));
        assert_eq!(
            plan.full_filter.as_deref(),
            Some("model_version = Utf8(\"ea219ee0\")")
        );
        assert_eq!(
            plan.index.as_deref(),
            Some("ScalarIndexQuery: query=[model_version = ea219ee0]@model_version_idx")
        );
        assert!(!plan.full_scan());
    }

    #[test]
    fn empty_plan_string_yields_default() {
        let plan = QueryPlan::parse("");
        assert_eq!(plan, QueryPlan::default());
        assert!(!plan.full_scan());
    }
}
