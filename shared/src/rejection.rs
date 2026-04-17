#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectionCategory {
    Face,
    Metadata,
}

impl std::fmt::Display for RejectionCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Face => write!(f, "Face"),
            Self::Metadata => write!(f, "Metadata"),
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
            "BlockedByMetadata" => Ok(Self::BlockedByMetadata),
            other => Err(format!("unknown rejection: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Rejection::BlockedByMetadata.category(), RejectionCategory::Metadata);
    }
}
