//! Stable identities.
//!
//! Names change; identities do not. A renamed dataset keeps its `DatasetId`, so
//! saved traces, fits and figures keep pointing at it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! identity {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", $prefix, self.0.hyphenated())
            }
        }
    };
}

identity!(DatasetId, "ds-");
identity!(VersionId, "v-");
identity!(FigureId, "fig-");
identity!(FitId, "fit-");
identity!(ComparisonId, "cmp-");
identity!(SeriesId, "s-");
identity!(AnnotationId, "an-");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_unique_and_stable_through_serde() {
        let a = DatasetId::new();
        let b = DatasetId::new();
        assert_ne!(a, b);

        let text = serde_json::to_string(&a).unwrap();
        let back: DatasetId = serde_json::from_str(&text).unwrap();
        assert_eq!(a, back);
    }
}
