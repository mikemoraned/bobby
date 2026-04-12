//! Identity of whoever made a manual appraisal.
//!
//! Represented as an enum so new identity providers can be added as
//! variants without breaking existing data. Wire format is
//! `provider:identifier` — a single string column suitable for storage.

/// Identity + provider of whoever made an appraisal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Appraiser {
    GitHub { username: String },
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ParseAppraiserError {
    #[error("appraiser must be of the form `provider:identifier`, got `{0}`")]
    Malformed(String),
    #[error("unknown appraiser provider: `{0}`")]
    UnknownProvider(String),
    #[error("empty identifier for provider `{0}`")]
    EmptyIdentifier(String),
}

impl Appraiser {
    pub fn new_github(username: impl Into<String>) -> Result<Self, ParseAppraiserError> {
        let username = username.into();
        if username.is_empty() {
            return Err(ParseAppraiserError::EmptyIdentifier("github".to_string()));
        }
        Ok(Self::GitHub { username })
    }
}

impl std::fmt::Display for Appraiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHub { username } => write!(f, "github:{username}"),
        }
    }
}

impl std::str::FromStr for Appraiser {
    type Err = ParseAppraiserError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (provider, identifier) = s
            .split_once(':')
            .ok_or_else(|| ParseAppraiserError::Malformed(s.to_string()))?;
        match provider {
            "github" => Self::new_github(identifier),
            other => Err(ParseAppraiserError::UnknownProvider(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_display() {
        let a = Appraiser::new_github("mikemoraned").expect("valid");
        assert_eq!(a.to_string(), "github:mikemoraned");
    }

    #[test]
    fn github_roundtrip() {
        let a = Appraiser::new_github("mikemoraned").expect("valid");
        let parsed: Appraiser = a.to_string().parse().expect("roundtrip");
        assert_eq!(parsed, a);
    }

    #[test]
    fn rejects_malformed_missing_colon() {
        let err = "mikemoraned".parse::<Appraiser>().unwrap_err();
        assert!(matches!(err, ParseAppraiserError::Malformed(_)));
    }

    #[test]
    fn rejects_unknown_provider() {
        let err = "twitter:someone".parse::<Appraiser>().unwrap_err();
        assert!(matches!(err, ParseAppraiserError::UnknownProvider(_)));
    }

    #[test]
    fn rejects_empty_identifier() {
        let err = "github:".parse::<Appraiser>().unwrap_err();
        assert!(matches!(err, ParseAppraiserError::EmptyIdentifier(_)));
    }

    #[test]
    fn new_github_rejects_empty() {
        assert!(Appraiser::new_github("").is_err());
    }
}
