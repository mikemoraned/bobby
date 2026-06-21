/// An opaque per-table version token.
///
/// `tag` is an opaque string, so two tokens compare equal iff the table has not
/// changed between the calls — the comparable key a [`crate::VersionedCache`]
/// gates on. Produced by the [`crate::TableVersions`] port.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Version {
    pub name: String,
    pub tag: String,
}

impl Version {
    pub fn new(name: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tag: tag.into(),
        }
    }
}
