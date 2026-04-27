use std::collections::HashMap;

use crate::query_plan::QueryPlan;
use crate::tempo::{Span, SpanEvent, Trace, TraceInfo};

// ── Domain types ──────────────────────────────────────────────────────────────

struct SlowQuery {
    label: String,
    elapsed: String,
    plan: QueryPlan,
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
    let plan = QueryPlan {
        table: non_empty_str(event, "plan.table"),
        columns: non_empty_str(event, "plan.columns"),
        num_fragments: event
            .attributes
            .get("plan.num_fragments")
            .and_then(|v| v.as_i64())
            .filter(|&n| n > 0)
            .map(|n| n as u64),
        full_filter: non_empty_str(event, "plan.full_filter"),
        index: non_empty_str(event, "plan.index"),
        unknown_keys: std::collections::BTreeSet::default(),
    };
    Some(SlowQuery { label, elapsed, plan })
}

fn non_empty_str(event: &SpanEvent, key: &str) -> Option<String> {
    event
        .attributes
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
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
            let ps = render_plan(&sq.plan, &indent);
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

fn render_plan(plan: &QueryPlan, indent: &str) -> String {
    let mut out = Vec::new();
    if let Some(table) = &plan.table {
        out.push(format!("{indent}    table:     {table}"));
    }
    if let Some(cols) = &plan.columns {
        out.push(format!("{indent}    columns:   {cols}"));
    }
    if let Some(n) = plan.num_fragments {
        if plan.full_scan() {
            out.push(format!(
                "{indent}    fragments: {n}  *** FULL SCAN - no filter ***"
            ));
        } else {
            out.push(format!("{indent}    fragments: {n}"));
            if let Some(f) = &plan.full_filter {
                out.push(format!("{indent}    filter:    {f}"));
            }
        }
    }
    if let Some(idx) = &plan.index {
        out.push(format!("{indent}    index:     {idx}"));
    }
    out.join("\n")
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

    fn make_full_scan_event() -> SpanEvent {
        let mut attrs = HashMap::new();
        attrs.insert("label".to_owned(), AttrValue::Str("list_unscored:scored_ids".to_owned()));
        attrs.insert("elapsed".to_owned(), AttrValue::Str("1.510759087s".to_owned()));
        attrs.insert("plan.table".to_owned(), AttrValue::Str("images_v6.lance".to_owned()));
        attrs.insert(
            "plan.columns".to_owned(),
            AttrValue::Str("image_id, discovered_at".to_owned()),
        );
        attrs.insert("plan.num_fragments".to_owned(), AttrValue::Int(66));
        attrs.insert("plan.full_scan".to_owned(), AttrValue::Bool(true));
        attrs.insert("plan.full_filter".to_owned(), AttrValue::Str(String::new()));
        attrs.insert("plan.index".to_owned(), AttrValue::Str(String::new()));
        SpanEvent { name: "slow query".to_owned(), attributes: attrs }
    }

    #[test]
    fn slow_query_extracted_from_full_scan_attrs() {
        let event = make_full_scan_event();
        let sq = extract_slow_query(&event).expect("should extract");
        assert_eq!(sq.label, "list_unscored:scored_ids");
        assert_eq!(sq.elapsed, "1.510759087s");
        assert_eq!(sq.plan.table.as_deref(), Some("images_v6.lance"));
        assert_eq!(sq.plan.num_fragments, Some(66));
        assert!(sq.plan.full_scan());
        assert!(sq.plan.full_filter.is_none());
        assert!(sq.plan.index.is_none());
    }

    // Disabled until the flat `plan.*` event attributes from `lancedb_utils.rs`
    // are deployed and a fresh fixture is captured (`just capture-trace-fixtures`).
    // The committed fixture still carries the legacy single `plan` string, so
    // this assertion would fail today against real-but-stale data. Re-enable
    // once new attributes are visible in production traces.
    #[test]
    #[ignore = "awaits redeploy + fresh fixture capture"]
    fn slow_query_extracted_from_real_fixture() {
        let trace = crate::tempo::trace_from_fixture_for_tests();
        let event = trace
            .spans
            .iter()
            .flat_map(|s| &s.events)
            .find(|e| e.name == "slow query")
            .expect("fixture has a slow query event");
        let sq = extract_slow_query(event).expect("flat plan.* attrs present");
        assert!(sq.plan.table.is_some(), "plan.table populated");
        assert!(sq.plan.num_fragments.is_some(), "plan.num_fragments populated");
    }

    #[test]
    fn slow_query_returns_none_when_label_missing() {
        let mut event = make_full_scan_event();
        event.attributes.remove("label");
        assert!(extract_slow_query(&event).is_none());
    }

    #[test]
    fn render_plan_flags_full_scan() {
        let plan = QueryPlan {
            table: Some("images_v6.lance".to_owned()),
            columns: Some("image_id, discovered_at".to_owned()),
            num_fragments: Some(66),
            full_filter: None,
            index: None,
            unknown_keys: std::collections::BTreeSet::default(),
        };
        let out = render_plan(&plan, "");
        assert!(out.contains("FULL SCAN"));
        assert!(out.contains("images_v6.lance"));
        assert!(out.contains("66"));
        assert!(!out.contains("filter:"));
    }

    #[test]
    fn render_plan_shows_filter_and_index_for_indexed_query() {
        let plan = QueryPlan {
            table: Some("images_score_v2.lance".to_owned()),
            columns: Some("image_id".to_owned()),
            num_fragments: Some(4),
            full_filter: Some("model_version = Utf8(\"ea219ee0\")".to_owned()),
            index: Some(
                "ScalarIndexQuery: query=[model_version = ea219ee0]@model_version_idx"
                    .to_owned(),
            ),
            unknown_keys: std::collections::BTreeSet::default(),
        };
        let out = render_plan(&plan, "");
        assert!(!out.contains("FULL SCAN"));
        assert!(out.contains("ScalarIndexQuery"));
        assert!(out.contains("model_version"));
        assert!(out.contains("images_score_v2.lance"));
        assert!(out.contains("4"));
    }
}
