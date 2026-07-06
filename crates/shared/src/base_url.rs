//! A `Url` wrapper that resolves the WHATWG "cannot-be-a-base" check once.
//!
//! WHATWG treats some URLs as "cannot-be-a-base" (`mailto:`, `data:` — no
//! hierarchical path), so [`Url::path_segments_mut`], `set_port`, etc. all
//! return `Result`/`Option` on *every* call — even for ordinary `http(s)` URLs
//! where the answer can never be `Err`.
//!
//! The key fact: whether a URL can be a base is a fixed property of its scheme +
//! authority, decided at parse time. `http`/`https`/`ws`/`wss`/`ftp`/`file` can
//! always be a base; that never changes from pushing or clearing path segments.
//! It only flips if the scheme or host is changed. So the check belongs *once*
//! at construction, not on every mutation — which is what [`BaseUrl`] does
//! ("parse, don't validate").

use url::{PathSegmentsMut, Url};

#[derive(Debug, thiserror::Error)]
pub enum BaseUrlError {
    #[error("not a valid url: {0}")]
    Unparseable(#[from] url::ParseError),
    #[error("url cannot be a base (no hierarchical path): {0}")]
    CannotBeABase(Url),
}

/// A [`Url`] proven at construction to support path-segment mutation, so
/// [`Self::path_segments_mut`] is infallible thereafter.
///
/// Deliberately does not expose `set_scheme`/`&mut Url`: mutating the scheme or
/// host could invalidate the can-be-a-base invariant, so a caller that must do
/// that re-validates by round-tripping through [`Self::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrl(Url);

impl BaseUrl {
    /// The only place the base check can fail.
    pub fn new(url: Url) -> Result<Self, BaseUrlError> {
        if url.cannot_be_a_base() {
            Err(BaseUrlError::CannotBeABase(url))
        } else {
            Ok(Self(url))
        }
    }

    /// Parse `s` and verify it can be a base in one step.
    pub fn parse(s: &str) -> Result<Self, BaseUrlError> {
        Self::new(Url::parse(s)?)
    }

    /// Infallible: the constructor already verified this URL can be a base.
    fn path_segments_mut(&mut self) -> PathSegmentsMut<'_> {
        #[allow(clippy::expect_used)]
        // BaseUrl invariant: constructor verified this URL can be a base
        self.0
            .path_segments_mut()
            .expect("BaseUrl invariant: URL can be a base")
    }

    /// A copy of this base URL with `segments` appended to its path.
    ///
    /// Infallible by the [`BaseUrl`] invariant, so callers never re-validate.
    pub fn join_path(&self, segments: &[&str]) -> Url {
        let mut url = self.clone();
        url.path_segments_mut().extend(segments.iter().copied());
        url.into_url()
    }

    pub const fn as_url(&self) -> &Url {
        &self.0
    }

    pub fn into_url(self) -> Url {
        self.0
    }
}

impl std::fmt::Display for BaseUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_can_be_a_base_and_appends_segments() {
        let base = BaseUrl::parse("https://bsky.app").expect("valid https base");
        let url = base.join_path(&["profile", "did:plc:abc", "post", "r1"]);
        assert_eq!(url.as_str(), "https://bsky.app/profile/did:plc:abc/post/r1");
    }

    #[test]
    fn mailto_cannot_be_a_base() {
        assert!(matches!(
            BaseUrl::parse("mailto:a@b.com"),
            Err(BaseUrlError::CannotBeABase(_))
        ));
    }

    #[test]
    fn rejects_unparseable() {
        assert!(matches!(
            BaseUrl::parse("not a url"),
            Err(BaseUrlError::Unparseable(_))
        ));
    }
}
