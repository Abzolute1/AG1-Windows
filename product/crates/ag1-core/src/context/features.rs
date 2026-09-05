//! Feature resolution consumes captured environment, never process-global state.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureOverrides {
    pub typed_evidence_cursors: Option<bool>,
    pub evidence_leases: Option<bool>,
    pub segmented_artifact_catalog: Option<bool>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Features {
    pub typed_evidence_cursors: bool,
    pub evidence_leases: bool,
    pub segmented_artifact_catalog: bool,
}
impl Features {
    pub fn resolve(explicit: FeatureOverrides, env: &BTreeMap<String, String>) -> Self {
        let read = |key: &str| match env.get(key)?.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        };
        let default = read("AG1_CV_ARCHITECTURE").unwrap_or(true);
        Self {
            typed_evidence_cursors: explicit
                .typed_evidence_cursors
                .or_else(|| read("AG1_CV_TYPED_CURSORS"))
                .unwrap_or(default),
            evidence_leases: explicit
                .evidence_leases
                .or_else(|| read("AG1_CV_EVIDENCE_LEASES"))
                .unwrap_or(default),
            segmented_artifact_catalog: explicit
                .segmented_artifact_catalog
                .or_else(|| read("AG1_CV_SEGMENTED_CATALOG"))
                .unwrap_or(default),
        }
    }
}
impl Default for Features {
    fn default() -> Self {
        Self::resolve(FeatureOverrides::default(), &BTreeMap::new())
    }
}
