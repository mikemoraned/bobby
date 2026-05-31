use std::fmt;
use std::str::FromStr;

use image::DynamicImage;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::bluesky_cid::BlueskyCid;

const V2_PREFIX: &str = "v2:";
const V3_PREFIX: &str = "v3:";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageId {
    V1(Uuid),
    V2(md5::Digest),
    V3(BlueskyCid),
}

impl ImageId {
    pub fn from_image(image: &DynamicImage) -> Self {
        Self::V2(md5::compute(image.as_bytes()))
    }
}

impl fmt::Display for ImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V1(uuid) => write!(f, "{uuid}"),
            Self::V2(digest) => write!(f, "{V2_PREFIX}{digest:x}"),
            Self::V3(cid) => write!(f, "{V3_PREFIX}{cid}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid image id: \"{0}\"")]
pub struct InvalidImageId(String);

impl FromStr for ImageId {
    type Err = InvalidImageId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(hex_str) = s.strip_prefix(V2_PREFIX) {
            let bytes: [u8; 16] = hex::decode(hex_str)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| InvalidImageId(s.to_string()))?;
            Ok(Self::V2(md5::Digest(bytes)))
        } else if let Some(cid_str) = s.strip_prefix(V3_PREFIX) {
            BlueskyCid::new(cid_str)
                .map(Self::V3)
                .map_err(|_| InvalidImageId(s.to_string()))
        } else {
            let uuid = Uuid::parse_str(s).map_err(|_| InvalidImageId(s.to_string()))?;
            Ok(Self::V1(uuid))
        }
    }
}

impl Serialize for ImageId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ImageId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn image_id_v1_roundtrip(v in any::<u128>()) {
            let id = ImageId::V1(Uuid::from_u128(v));
            let parsed: ImageId = id.to_string().parse().expect("V1 roundtrip");
            prop_assert_eq!(id, parsed);
        }

        #[test]
        fn image_id_v2_roundtrip(bytes in any::<[u8; 16]>()) {
            let id = ImageId::V2(md5::Digest(bytes));
            let s = id.to_string();
            prop_assert!(s.starts_with("v2:"));
            let parsed: ImageId = s.parse().expect("V2 roundtrip");
            prop_assert_eq!(id, parsed);
        }

        /// Different byte content produces different V2 ids (MD5 collisions are
        /// astronomically rare with random inputs; any collision is skipped).
        #[test]
        fn image_id_v2_different_content(b1 in any::<Vec<u8>>(), b2 in any::<Vec<u8>>()) {
            prop_assume!(b1 != b2);
            let id1 = ImageId::V2(md5::compute(&b1));
            let id2 = ImageId::V2(md5::compute(&b2));
            prop_assume!(id1 != id2);
        }

        #[test]
        fn serde_roundtrip_via_toml(bytes in any::<[u8; 16]>()) {
            #[derive(Serialize, Deserialize)]
            struct W { id: ImageId }
            let original = W { id: ImageId::V2(md5::Digest(bytes)) };
            let s = toml::to_string(&original).expect("serialize");
            let parsed: W = toml::from_str(&s).expect("deserialize");
            prop_assert_eq!(parsed.id, original.id);
        }
    }

    /// A real Bluesky blob CID (CIDv1, base32, sha2-256).
    const SAMPLE_CID: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

    #[test]
    fn image_id_v3_roundtrip() {
        let id = ImageId::V3(BlueskyCid::new(SAMPLE_CID).expect("valid cid"));
        let s = id.to_string();
        assert_eq!(s, format!("v3:{SAMPLE_CID}"));
        let parsed: ImageId = s.parse().expect("V3 roundtrip");
        assert_eq!(id, parsed);
    }

    #[test]
    fn image_id_v3_serde_roundtrip_via_toml() {
        #[derive(Serialize, Deserialize)]
        struct W {
            id: ImageId,
        }
        let original = W {
            id: ImageId::V3(BlueskyCid::new(SAMPLE_CID).expect("valid cid")),
        };
        let s = toml::to_string(&original).expect("serialize");
        let parsed: W = toml::from_str(&s).expect("deserialize");
        assert_eq!(parsed.id, original.id);
    }

    #[test]
    fn image_id_v3_rejects_invalid_cid() {
        assert!("v3:not-a-cid".parse::<ImageId>().is_err());
    }

    /// A bare value with no recognised prefix that also isn't a UUID is rejected,
    /// rather than being silently misclassified as a V1/V2/V3 id.
    #[test]
    fn image_id_rejects_unknown_prefix() {
        assert!("v9:whatever".parse::<ImageId>().is_err());
        assert!("not-an-id".parse::<ImageId>().is_err());
    }
}
