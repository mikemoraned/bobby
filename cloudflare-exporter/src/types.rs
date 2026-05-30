use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountTag(String);

#[derive(Debug, Error)]
pub enum AccountTagError {
    #[error("account tag must be exactly 32 hex chars, got {0} chars")]
    Length(usize),
    #[error("account tag must be hex (0-9, a-f, A-F)")]
    NotHex,
}

impl AccountTag {
    pub fn new(s: impl Into<String>) -> Result<Self, AccountTagError> {
        let s = s.into();
        if s.len() != 32 {
            return Err(AccountTagError::Length(s.len()));
        }
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AccountTagError::NotHex);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for AccountTag {
    type Err = AccountTagError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl std::fmt::Display for AccountTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone)]
pub struct ApiToken(String);

#[derive(Debug, Error)]
pub enum ApiTokenError {
    #[error("api token must be non-empty")]
    Empty,
    #[error("api token must not contain whitespace")]
    Whitespace,
}

impl ApiToken {
    pub fn new(s: impl Into<String>) -> Result<Self, ApiTokenError> {
        let s = s.into();
        if s.is_empty() {
            return Err(ApiTokenError::Empty);
        }
        if s.chars().any(char::is_whitespace) {
            return Err(ApiTokenError::Whitespace);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ApiToken {
    type Err = ApiTokenError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketName(String);

#[derive(Debug, Error)]
pub enum BucketNameError {
    #[error("bucket name must be non-empty")]
    Empty,
    #[error("bucket name contains invalid character {0:?}")]
    InvalidChar(char),
}

impl BucketName {
    pub fn new(s: impl Into<String>) -> Result<Self, BucketNameError> {
        let s = s.into();
        if s.is_empty() {
            return Err(BucketNameError::Empty);
        }
        if let Some(c) = s.chars().find(|c| *c == '/' || c.is_whitespace()) {
            return Err(BucketNameError::InvalidChar(c));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BucketName {
    type Err = BucketNameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl std::fmt::Display for BucketName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_tag_accepts_32_hex_chars() {
        let tag: AccountTag = "3f4c1cd0ba074f81f8ba747e8cdf4e2d".parse().unwrap();
        assert_eq!(tag.as_str(), "3f4c1cd0ba074f81f8ba747e8cdf4e2d");
    }

    #[test]
    fn account_tag_rejects_wrong_length() {
        assert!(matches!(
            AccountTag::new("abcd"),
            Err(AccountTagError::Length(4))
        ));
    }

    #[test]
    fn account_tag_rejects_non_hex() {
        assert!(matches!(
            AccountTag::new("g".repeat(32)),
            Err(AccountTagError::NotHex)
        ));
    }

    #[test]
    fn api_token_rejects_empty_and_whitespace() {
        assert!(matches!(ApiToken::new(""), Err(ApiTokenError::Empty)));
        assert!(matches!(
            ApiToken::new("abc def"),
            Err(ApiTokenError::Whitespace)
        ));
    }

    #[test]
    fn bucket_name_rejects_slash_and_whitespace() {
        assert!(matches!(BucketName::new(""), Err(BucketNameError::Empty)));
        assert!(matches!(
            BucketName::new("foo/bar"),
            Err(BucketNameError::InvalidChar('/'))
        ));
        assert!(matches!(
            BucketName::new("foo bar"),
            Err(BucketNameError::InvalidChar(' '))
        ));
    }

    #[test]
    fn bucket_name_accepts_valid() {
        let name: BucketName = "hom-bobby".parse().unwrap();
        assert_eq!(name.as_str(), "hom-bobby");
    }
}
