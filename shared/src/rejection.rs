use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectionCategory {
    Face,
    Text,
    Metadata,
}

impl std::fmt::Display for RejectionCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Face => write!(f, "Face"),
            Self::Text => write!(f, "Text"),
            Self::Metadata => write!(f, "Metadata"),
        }
    }
}

impl std::str::FromStr for RejectionCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Face" => Ok(Self::Face),
            "Text" => Ok(Self::Text),
            "Metadata" => Ok(Self::Metadata),
            other => Err(format!("unknown rejection category: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rejection {
    FaceTooSmall,
    FaceTooLarge,
    FaceNotInAcceptedZone,
    TooManyFaces,
    TooFewFrontalFaces,
    TooLittleFaceSkin,
    TooMuchSkinOutsideFace,
    TooMuchText,
    BlockedByMetadata,
}

impl Rejection {
    pub const ALL: &'static [Self] = &[
        Self::FaceTooSmall,
        Self::FaceTooLarge,
        Self::FaceNotInAcceptedZone,
        Self::TooManyFaces,
        Self::TooFewFrontalFaces,
        Self::TooLittleFaceSkin,
        Self::TooMuchSkinOutsideFace,
        Self::TooMuchText,
        Self::BlockedByMetadata,
    ];

    pub const fn category(self) -> RejectionCategory {
        match self {
            Self::FaceTooSmall
            | Self::FaceTooLarge
            | Self::FaceNotInAcceptedZone
            | Self::TooManyFaces
            | Self::TooFewFrontalFaces
            | Self::TooLittleFaceSkin
            | Self::TooMuchSkinOutsideFace => RejectionCategory::Face,
            Self::TooMuchText => RejectionCategory::Text,
            Self::BlockedByMetadata => RejectionCategory::Metadata,
        }
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FaceTooSmall => write!(f, "FaceTooSmall"),
            Self::FaceTooLarge => write!(f, "FaceTooLarge"),
            Self::FaceNotInAcceptedZone => write!(f, "FaceNotInAcceptedZone"),
            Self::TooManyFaces => write!(f, "TooManyFaces"),
            Self::TooFewFrontalFaces => write!(f, "TooFewFrontalFaces"),
            Self::TooLittleFaceSkin => write!(f, "TooLittleFaceSkin"),
            Self::TooMuchSkinOutsideFace => write!(f, "TooMuchSkinOutsideFace"),
            Self::TooMuchText => write!(f, "TooMuchText"),
            Self::BlockedByMetadata => write!(f, "BlockedByMetadata"),
        }
    }
}

impl std::str::FromStr for Rejection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "FaceTooSmall" => Ok(Self::FaceTooSmall),
            "FaceTooLarge" => Ok(Self::FaceTooLarge),
            "FaceNotInAcceptedZone" => Ok(Self::FaceNotInAcceptedZone),
            "TooManyFaces" => Ok(Self::TooManyFaces),
            "TooFewFrontalFaces" => Ok(Self::TooFewFrontalFaces),
            "TooLittleFaceSkin" => Ok(Self::TooLittleFaceSkin),
            "TooMuchSkinOutsideFace" => Ok(Self::TooMuchSkinOutsideFace),
            "TooMuchText" => Ok(Self::TooMuchText),
            "BlockedByMetadata" => Ok(Self::BlockedByMetadata),
            other => Err(format!("unknown rejection: {other}")),
        }
    }
}

/// A set of enabled rejection categories.
#[derive(Debug, Clone)]
pub struct RejectionCategories(HashSet<RejectionCategory>);

impl Default for RejectionCategories {
    fn default() -> Self {
        Self(HashSet::from([RejectionCategory::Face, RejectionCategory::Metadata]))
    }
}

impl std::ops::Deref for RejectionCategories {
    type Target = HashSet<RejectionCategory>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<I: IntoIterator<Item = RejectionCategory>> From<I> for RejectionCategories {
    fn from(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_category_display() {
        assert_eq!(RejectionCategory::Face.to_string(), "Face");
        assert_eq!(RejectionCategory::Metadata.to_string(), "Metadata");
    }

    /// Covers all variants — adding a new variant without a `FromStr` arm will fail here.
    #[test]
    fn rejection_roundtrips_through_string() {
        for &r in Rejection::ALL {
            let s = r.to_string();
            let parsed: Rejection = s.parse().expect("roundtrip");
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn rejection_categories() {
        assert_eq!(Rejection::FaceTooSmall.category(), RejectionCategory::Face);
        assert_eq!(Rejection::TooMuchText.category(), RejectionCategory::Text);
        assert_eq!(Rejection::BlockedByMetadata.category(), RejectionCategory::Metadata);
    }
}
