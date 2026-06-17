//! Identity of whoever made a manual appraisal.
//!
//! Represented as an enum so new identity providers can be added as
//! variants without breaking existing data. Wire format is
//! `provider:identifier` — a single string column suitable for storage.

/// Identity + provider of whoever made an appraisal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Appraiser {
    GitHub { username: String },
    LocalAdmin,
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
            Self::LocalAdmin => write!(f, "local:admin"),
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
            "local" if identifier == "admin" => Ok(Self::LocalAdmin),
            "local" => Err(ParseAppraiserError::UnknownProvider(
                format!("local:{identifier}"),
            )),
            other => Err(ParseAppraiserError::UnknownProvider(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn local_admin_display() {
        assert_eq!(Appraiser::LocalAdmin.to_string(), "local:admin");
    }

    #[test]
    fn local_admin_roundtrip() {
        let parsed: Appraiser = "local:admin".parse().expect("roundtrip");
        assert_eq!(parsed, Appraiser::LocalAdmin);
    }

    #[test]
    fn rejects_unknown_local_identifier() {
        assert!("local:other".parse::<Appraiser>().is_err());
    }

    proptest! {
        #[test]
        fn github_roundtrip_arbitrary(username in "[a-zA-Z0-9_-]{1,39}") {
            let a = Appraiser::new_github(&username).expect("non-empty username");
            let parsed: Appraiser = a.to_string().parse().expect("roundtrip");
            prop_assert_eq!(parsed, a);
        }

        /// Strings without a ':' are always `Malformed`.
        #[test]
        fn rejects_malformed_no_colon(s in "[a-zA-Z0-9]{1,30}") {
            let err = s.parse::<Appraiser>().expect_err("no colon → malformed");
            prop_assert!(matches!(err, ParseAppraiserError::Malformed(_)));
        }

        /// Any provider that is neither "github" nor "local" is `UnknownProvider`.
        #[test]
        fn rejects_unknown_provider_arbitrary(
            provider in "[a-z]{2,10}",
            id in "[a-z]{1,20}",
        ) {
            prop_assume!(provider != "github" && provider != "local");
            let s = format!("{provider}:{id}");
            let err = s.parse::<Appraiser>().expect_err("unknown provider");
            prop_assert!(matches!(err, ParseAppraiserError::UnknownProvider(_)));
        }

        /// An empty identifier after a known provider is always rejected.
        #[test]
        fn rejects_empty_identifier(provider in "(github|local)") {
            let s = format!("{provider}:");
            prop_assert!(s.parse::<Appraiser>().is_err());
        }
    }
}
