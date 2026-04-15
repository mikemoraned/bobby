/// A zone within the image, defined by overlaying a 4x4 grid and taking 2x2
/// blocks at each valid offset (0, 1, 2) in both X and Y, giving 9 zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    CenterCenter,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl std::fmt::Display for Zone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopLeft => write!(f, "TOP_LEFT"),
            Self::TopCenter => write!(f, "TOP_CENTER"),
            Self::TopRight => write!(f, "TOP_RIGHT"),
            Self::CenterLeft => write!(f, "CENTER_LEFT"),
            Self::CenterCenter => write!(f, "CENTER_CENTER"),
            Self::CenterRight => write!(f, "CENTER_RIGHT"),
            Self::BottomLeft => write!(f, "BOTTOM_LEFT"),
            Self::BottomCenter => write!(f, "BOTTOM_CENTER"),
            Self::BottomRight => write!(f, "BOTTOM_RIGHT"),
        }
    }
}

impl std::str::FromStr for Zone {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "TOP_LEFT" => Ok(Self::TopLeft),
            "TOP_CENTER" => Ok(Self::TopCenter),
            "TOP_RIGHT" => Ok(Self::TopRight),
            "CENTER_LEFT" => Ok(Self::CenterLeft),
            "CENTER_CENTER" => Ok(Self::CenterCenter),
            "CENTER_RIGHT" => Ok(Self::CenterRight),
            "BOTTOM_LEFT" => Ok(Self::BottomLeft),
            "BOTTOM_CENTER" => Ok(Self::BottomCenter),
            "BOTTOM_RIGHT" => Ok(Self::BottomRight),
            other => Err(format!("unknown zone: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_roundtrips_through_string() {
        for z in [
            Zone::TopLeft,
            Zone::TopCenter,
            Zone::TopRight,
            Zone::CenterLeft,
            Zone::CenterCenter,
            Zone::CenterRight,
            Zone::BottomLeft,
            Zone::BottomCenter,
            Zone::BottomRight,
        ] {
            let s = z.to_string();
            let parsed: Zone = s.parse().expect("should parse");
            assert_eq!(parsed, z);
        }
    }
}
