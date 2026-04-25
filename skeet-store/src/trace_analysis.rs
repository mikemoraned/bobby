use std::collections::HashMap;

use crate::tempo::{Span, SpanEvent, Trace, TraceInfo};

// ── Domain types ──────────────────────────────────────────────────────────────

struct SlowQuery {
    label: String,
    elapsed: String,
    raw_plan: String,
}

struct AnnotatedSpan<'a> {
    span: &'a Span,
    slow_queries: Vec<SlowQuery>,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn summarise(info: &TraceInfo, trace: &Trace) -> String {
    let annotated = annotate(&trace.spans);

    let span_ids: std::collections::HashSet<&str> =
        annotated.iter().map(|a| a.span.span_id.as_str()).collect();

    let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, a) in annotated.iter().enumerate() {
        if let Some(pid) = &a.span.parent_span_id {
            children.entry(pid.as_str()).or_default().push(i);
        }
    }

    let roots: Vec<usize> = annotated
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            a.span
                .parent_span_id
                .as_deref()
                .is_none_or(|pid| !span_ids.contains(pid))
        })
        .map(|(i, _)| i)
        .collect();

    let trace_duration_ns = roots.iter().map(|&i| annotated[i].span.duration_ns).max().unwrap_or(0);

    let mut out = format!(
        "=== Trace {} ({}) — {}/{}\n",
        &info.trace_id[..8.min(info.trace_id.len())],
        fmt_ns(trace_duration_ns),
        info.root_service_name,
        info.root_trace_name,
    );
    render_tree(&annotated, &children, &roots, &mut out, 1);
    out.push('\n');
    out
}

// ── Annotation: extract domain-specific fields from generic spans ─────────────

fn annotate(spans: &[Span]) -> Vec<AnnotatedSpan<'_>> {
    spans
        .iter()
        .map(|span| {
            let slow_queries = span
                .events
                .iter()
                .filter(|e| e.name == "slow query")
                .filter_map(extract_slow_query)
                .collect();
            AnnotatedSpan { span, slow_queries }
        })
        .collect()
}

fn extract_slow_query(event: &SpanEvent) -> Option<SlowQuery> {
    let label = event.attributes.get("label")?.as_str()?.to_owned();
    let elapsed = event.attributes.get("elapsed")?.as_str()?.to_owned();
    let raw_plan = event.attributes.get("plan")?.as_str()?.to_owned();
    Some(SlowQuery { label, elapsed, raw_plan })
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render_tree(
    annotated: &[AnnotatedSpan<'_>],
    children: &HashMap<&str, Vec<usize>>,
    indices: &[usize],
    out: &mut String,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let mut i = 0;
    while i < indices.len() {
        let idx = indices[i];
        let name = &annotated[idx].span.name;

        // Collapse a run of same-named siblings (e.g. many read_fragment spans)
        let run = indices[i..]
            .iter()
            .take_while(|&&j| &annotated[j].span.name == name)
            .count();

        if run > 1 {
            let total: u64 = indices[i..i + run]
                .iter()
                .map(|&j| annotated[j].span.duration_ns)
                .sum();
            out.push_str(&format!(
                "{indent}{run} × {name}  (total {})\n",
                fmt_ns(total)
            ));
            i += run;
            continue;
        }

        let a = &annotated[idx];
        let busy = a
            .span
            .attributes
            .get("busy_ns")
            .and_then(|v| v.as_i64())
            .map(|b| format!("  busy={}", fmt_ns(b as u64)))
            .unwrap_or_default();
        let target_tag = a
            .span
            .attributes
            .get("target")
            .and_then(|v| v.as_str())
            .filter(|t| t.starts_with("skeet_store"))
            .map(|t| format!("  <{t}>"))
            .unwrap_or_default();

        out.push_str(&format!(
            "{indent}{}  wall={}{}{}\n",
            name,
            fmt_ns(a.span.duration_ns),
            busy,
            target_tag,
        ));

        for sq in &a.slow_queries {
            out.push_str(&format!(
                "{indent}  [slow query] label={}  elapsed={}\n",
                sq.label, sq.elapsed
            ));
            let ps = plan_summary(&sq.raw_plan);
            if !ps.is_empty() {
                out.push_str(&ps);
                out.push('\n');
            }
        }

        if let Some(child_indices) = children.get(a.span.span_id.as_str()) {
            render_tree(annotated, children, child_indices, out, depth + 1);
        }

        i += 1;
    }
}

// ── Lance plan parsing ────────────────────────────────────────────────────────

fn plan_summary(raw: &str) -> String {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with("LanceRead:") {
            if let Some(uri) = extract_field(line, "uri") {
                let table = uri
                    .split('/')
                    .find(|s| s.ends_with(".lance"))
                    .unwrap_or(uri);
                out.push(format!("    table:     {table}"));
            }
            if let Some(proj) = extract_field(line, "projection") {
                let cols = proj.trim_matches(|c| c == '[' || c == ']');
                out.push(format!("    columns:   {cols}"));
            }
            let frags = extract_field(line, "num_fragments");
            let filter = extract_field(line, "full_filter");
            if let Some(n) = frags {
                if matches!(filter, Some("--") | None) {
                    out.push(format!("    fragments: {n}  *** FULL SCAN - no filter ***"));
                } else if let Some(f) = filter {
                    out.push(format!("    fragments: {n}"));
                    out.push(format!("    filter:    {f}"));
                }
            }
        } else if line.starts_with("ScalarIndexQuery:") {
            out.push(format!("    index:     {line}"));
        }
    }
    out.join("\n")
}

// Extract "key=value" from a Lance plan line (handles projection=[a, b] with inner commas).
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

// ── Formatting ────────────────────────────────────────────────────────────────

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2}s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.0}ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{ns}ns")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::tempo::{AttrValue, SpanEvent};

    // Real plan strings captured from the 25th Apr spike
    const FULL_SCAN_PLAN: &str = "LanceRead: uri=encrypted-store/images_v6.lance/data, projection=[image_id, discovered_at], num_fragments=66, range_before=None, range_after=None, row_id=false, row_addr=false, full_filter=--, refine_filter=--\n";
    const INDEXED_PLAN: &str = "LanceRead: uri=encrypted-store/images_score_v2.lance/data, projection=[image_id], num_fragments=4, range_before=None, range_after=None, row_id=false, row_addr=false, full_filter=model_version = Utf8(\"ea219ee0\"), refine_filter=--\n  ScalarIndexQuery: query=[model_version = ea219ee0]@model_version_idx\n";

    #[test]
    fn plan_summary_flags_full_scan() {
        let out = plan_summary(FULL_SCAN_PLAN);
        assert!(out.contains("FULL SCAN"), "full scan flagged");
        assert!(out.contains("images_v6.lance"), "table name extracted");
        assert!(out.contains("image_id, discovered_at"), "columns extracted");
        assert!(out.contains("66"), "fragment count extracted");
        assert!(!out.contains("filter:"), "no filter line for full scan");
    }

    #[test]
    fn plan_summary_shows_filter_and_index_for_indexed_query() {
        let out = plan_summary(INDEXED_PLAN);
        assert!(!out.contains("FULL SCAN"), "not flagged as full scan");
        assert!(out.contains("ScalarIndexQuery"), "index shown");
        assert!(out.contains("model_version"), "filter value shown");
        assert!(out.contains("images_score_v2.lance"), "table name extracted");
        assert!(out.contains("4"), "fragment count shown");
    }

    fn make_slow_query_event() -> SpanEvent {
        let mut attrs = HashMap::new();
        attrs.insert("label".to_owned(), AttrValue::Str("list_unscored:scored_ids".to_owned()));
        attrs.insert("elapsed".to_owned(), AttrValue::Str("1.510759087s".to_owned()));
        attrs.insert("plan".to_owned(), AttrValue::Str(FULL_SCAN_PLAN.to_owned()));
        SpanEvent { name: "slow query".to_owned(), attributes: attrs }
    }

    #[test]
    fn slow_query_extracted_from_well_formed_event() {
        let event = make_slow_query_event();
        let sq = extract_slow_query(&event).expect("should extract slow query");
        assert_eq!(sq.label, "list_unscored:scored_ids");
        assert_eq!(sq.elapsed, "1.510759087s");
        assert!(sq.raw_plan.contains("LanceRead"));
    }

    #[test]
    fn slow_query_returns_none_when_label_missing() {
        let mut event = make_slow_query_event();
        event.attributes.remove("label");
        assert!(extract_slow_query(&event).is_none());
    }

    #[test]
    fn slow_query_returns_none_when_plan_missing() {
        let mut event = make_slow_query_event();
        event.attributes.remove("plan");
        assert!(extract_slow_query(&event).is_none());
    }
}
