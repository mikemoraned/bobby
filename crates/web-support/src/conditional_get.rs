use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use cot::http::HeaderValue;
use cot::http::header::{CACHE_CONTROL, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use cot::http::request::Parts as RequestHead;
use cot::response::Response;
use cot::{Body, StatusCode};

/// The `Last-Modified` / `If-Modified-Since` wire format (RFC 9110 IMF-fixdate).
const HTTP_DATE_FORMAT: &str = "%a, %d %b %Y %H:%M:%S GMT";

/// An HTTP-date rendering of an instant, for `Last-Modified`.
pub fn http_date(at: DateTime<Utc>) -> String {
    at.format(HTTP_DATE_FORMAT).to_string()
}

/// Parse an HTTP-date header value (e.g. `Sun, 06 Nov 1994 08:49:37 GMT`) to a
/// UTC instant. `None` if it isn't IMF-fixdate — the format every client echoes
/// from the `Last-Modified` we sent.
fn parse_http_date(value: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value, HTTP_DATE_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

/// Set a response's `Last-Modified` to `at`.
///
/// The companion of [`not_modified_since`] for the full-render (`200`) path, so
/// both the `200` and the `304` carry the same header from one place. A no-op in
/// the impossible case that the HTTP-date can't be made into a header value.
pub fn set_last_modified(response: &mut Response, at: DateTime<Utc>) {
    if let Ok(value) = HeaderValue::from_str(&http_date(at)) {
        response.headers_mut().insert(LAST_MODIFIED, value);
    }
}

/// A `304 Not Modified` response when the resource has not changed since the
/// client's `If-Modified-Since`, otherwise `None` (the caller renders in full).
///
/// Per HTTP semantics this is "not modified" whenever `last_modified <=
/// if_modified_since` — a client whose copy is at least as new as ours gets a
/// `304`, not only on an exact match. The comparison is at the one-second
/// granularity of HTTP-dates, so a sub-second `last_modified` is not mistaken for
/// being newer than the whole-second value the client echoed.
///
/// Returning this lets a handler skip its expensive work — a store read, an image
/// encode, an HTML render — on a revalidation hit, which a response-rewriting
/// middleware could not (it only sees the response after that work is done). The
/// `304` echoes `Last-Modified` (and `Cache-Control`, when given) so a cache can
/// refresh its freshness without a body.
pub fn not_modified_since(
    head: &RequestHead,
    last_modified: DateTime<Utc>,
    cache_control: Option<&str>,
) -> Option<Response> {
    let if_modified_since = head
        .headers
        .get(IF_MODIFIED_SINCE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_http_date)?;

    // HTTP-dates carry whole-second precision, so compare against `last_modified`
    // with its sub-second part dropped — otherwise an instant 0.5s into a second
    // would look newer than the whole-second value the client echoed, and never 304.
    if let Some(last_modified) = last_modified.with_nanosecond(0)
        && last_modified > if_modified_since
    {
        return None; // modified since the client's copy — render it
    }

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_MODIFIED;
    if let Ok(value) = HeaderValue::from_str(&http_date(last_modified)) {
        response.headers_mut().insert(LAST_MODIFIED, value);
    }
    if let Some(cc) = cache_control
        && let Ok(value) = HeaderValue::from_str(cc)
    {
        response.headers_mut().insert(CACHE_CONTROL, value);
    }
    Some(response)
}

/// Set a response's strong `ETag` to `etag` (quoted as the wire format requires).
///
/// The companion of [`not_modified_by_etag`] for the full-render (`200`) path, so
/// both the `200` and the `304` carry the same validator from one place. A no-op
/// in the impossible case that the value can't be made into a header.
pub fn set_etag(response: &mut Response, etag: &str) {
    if let Ok(value) = HeaderValue::from_str(&format!("\"{etag}\"")) {
        response.headers_mut().insert(ETAG, value);
    }
}

/// The inner value of one `If-None-Match` entry: its optional weak `W/` prefix
/// and surrounding quotes stripped, so `W/"abc"` and `"abc"` both yield `abc`.
fn etag_value(entry: &str) -> &str {
    entry
        .trim()
        .strip_prefix("W/")
        .unwrap_or_else(|| entry.trim())
        .trim_matches('"')
}

/// A `304 Not Modified` response when the client's `If-None-Match` already holds
/// this `etag`, otherwise `None` (the caller renders in full).
///
/// `If-None-Match` may be `*` (matches any existing representation) or a
/// comma-separated list of entries; a match uses weak comparison (the `W/` prefix
/// is ignored), per HTTP semantics for conditional GET.
///
/// Because the `etag` is the content's own signature, it can be computed cheaply
/// and checked *before* the expensive work, so a
/// revalidation hit skips that work entirely — which a response-rewriting
/// middleware could not. The `304` echoes `ETag` (and `Cache-Control`, when
/// given) so a cache can refresh its freshness without a body.
pub fn not_modified_by_etag(
    head: &RequestHead,
    etag: &str,
    cache_control: Option<&str>,
) -> Option<Response> {
    let if_none_match = head
        .headers
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())?;

    let matches = if_none_match
        .split(',')
        .any(|entry| entry.trim() == "*" || etag_value(entry) == etag);
    if !matches {
        return None; // the client's copy differs — render it
    }

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_MODIFIED;
    set_etag(&mut response, etag);
    if let Some(cc) = cache_control
        && let Ok(value) = HeaderValue::from_str(cc)
    {
        response.headers_mut().insert(CACHE_CONTROL, value);
    }
    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn head_with(if_modified_since: Option<&str>) -> RequestHead {
        let mut builder = cot::http::Request::builder().uri("/");
        if let Some(ims) = if_modified_since {
            builder = builder.header(IF_MODIFIED_SINCE, ims);
        }
        builder
            .body(Body::empty())
            .expect("build request")
            .into_parts()
            .0
    }

    fn at(hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 15, hour, min, sec).unwrap()
    }

    #[test]
    fn formats_http_date() {
        assert_eq!(http_date(at(9, 30, 0)), "Sat, 15 Jun 2024 09:30:00 GMT");
    }

    #[test]
    fn exact_match_is_not_modified() {
        let head = head_with(Some("Sat, 15 Jun 2024 09:30:00 GMT"));
        let response =
            not_modified_since(&head, at(9, 30, 0), Some("public, max-age=60")).expect("304");
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            response
                .headers()
                .get(LAST_MODIFIED)
                .expect("last-modified"),
            "Sat, 15 Jun 2024 09:30:00 GMT"
        );
        assert_eq!(
            response
                .headers()
                .get(CACHE_CONTROL)
                .expect("cache-control"),
            "public, max-age=60"
        );
    }

    #[test]
    fn client_copy_newer_than_resource_is_not_modified() {
        // Client last saw it at 09:31; the resource is from 09:30 → still current.
        let head = head_with(Some("Sat, 15 Jun 2024 09:31:00 GMT"));
        assert!(not_modified_since(&head, at(9, 30, 0), None).is_some());
    }

    #[test]
    fn resource_newer_than_client_copy_renders() {
        // Client last saw it at 09:29; the resource changed at 09:30 → render.
        let head = head_with(Some("Sat, 15 Jun 2024 09:29:00 GMT"));
        assert!(not_modified_since(&head, at(9, 30, 0), None).is_none());
    }

    #[test]
    fn sub_second_last_modified_matches_whole_second_header() {
        // last_modified at 09:30:00.500 must not be treated as newer than the
        // whole-second 09:30:00 the client echoed.
        let last_modified = at(9, 30, 0) + chrono::Duration::milliseconds(500);
        let head = head_with(Some("Sat, 15 Jun 2024 09:30:00 GMT"));
        assert!(not_modified_since(&head, last_modified, None).is_some());
    }

    #[test]
    fn absent_or_unparseable_if_modified_since_renders() {
        assert!(not_modified_since(&head_with(None), at(9, 30, 0), None).is_none());
        assert!(not_modified_since(&head_with(Some("whenever")), at(9, 30, 0), None).is_none());
    }

    fn head_with_if_none_match(if_none_match: Option<&str>) -> RequestHead {
        let mut builder = cot::http::Request::builder().uri("/");
        if let Some(inm) = if_none_match {
            builder = builder.header(IF_NONE_MATCH, inm);
        }
        builder
            .body(Body::empty())
            .expect("build request")
            .into_parts()
            .0
    }

    #[test]
    fn set_etag_quotes_the_value() {
        let mut response = Response::new(Body::empty());
        set_etag(&mut response, "abc123");
        assert_eq!(response.headers().get(ETAG).expect("etag"), "\"abc123\"");
    }

    #[test]
    fn matching_if_none_match_is_not_modified() {
        let head = head_with_if_none_match(Some("\"abc123\""));
        let response =
            not_modified_by_etag(&head, "abc123", Some("public, max-age=60")).expect("304");
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers().get(ETAG).expect("etag"), "\"abc123\"");
        assert_eq!(
            response
                .headers()
                .get(CACHE_CONTROL)
                .expect("cache-control"),
            "public, max-age=60"
        );
    }

    #[test]
    fn weak_and_wildcard_if_none_match_match() {
        assert!(
            not_modified_by_etag(
                &head_with_if_none_match(Some("W/\"abc123\"")),
                "abc123",
                None
            )
            .is_some()
        );
        assert!(
            not_modified_by_etag(&head_with_if_none_match(Some("*")), "abc123", None).is_some()
        );
    }

    #[test]
    fn matching_entry_within_a_list_is_not_modified() {
        let head = head_with_if_none_match(Some("\"other\", \"abc123\""));
        assert!(not_modified_by_etag(&head, "abc123", None).is_some());
    }

    #[test]
    fn different_or_absent_if_none_match_renders() {
        assert!(
            not_modified_by_etag(
                &head_with_if_none_match(Some("\"different\"")),
                "abc123",
                None
            )
            .is_none()
        );
        assert!(not_modified_by_etag(&head_with_if_none_match(None), "abc123", None).is_none());
    }
}
