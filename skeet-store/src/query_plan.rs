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

impl QueryPlan {
    pub fn parse(raw: &str) -> Self {
        let mut plan = Self::default();
        for line in raw.lines() {
            let line = line.trim();
            if line.starts_with("LanceRead:") {
                if let Some(uri) = extract_field(line, "uri") {
                    plan.table = uri
                        .split('/')
                        .find(|s| s.ends_with(".lance"))
                        .map(str::to_owned);
                }
                if let Some(proj) = extract_field(line, "projection") {
                    plan.columns =
                        Some(proj.trim_matches(|c| c == '[' || c == ']').to_owned());
                }
                plan.num_fragments =
                    extract_field(line, "num_fragments").and_then(|s| s.parse().ok());
                plan.full_filter = match extract_field(line, "full_filter") {
                    Some("--") | None => None,
                    Some(s) => Some(s.to_owned()),
                };
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

// Extract `key=value` from a Lance plan line; handles `projection=[a, b]` with
// inner commas by treating `[…]` as a single value.
fn extract_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let pos = line.find(needle.as_str())?;
    let rest = &line[pos + needle.len()..];
    if rest.starts_with('[') {
        let end = rest.find(']')?;
        Some(&rest[..=end])
    } else {
        let end = rest.find(", ").unwrap_or(rest.len());
        Some(&rest[..end])
    }
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
