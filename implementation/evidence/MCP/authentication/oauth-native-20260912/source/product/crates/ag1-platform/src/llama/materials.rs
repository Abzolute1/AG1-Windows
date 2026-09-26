//! Derive the files a launch consumes from its sealed inventory. Captures may
//! intentionally share a catalog-wide inventory; their audit seal stays intact.
use super::{LaunchPlan, Material};
use ag1_core::DomainError;
use std::{collections::BTreeSet, path::Path};

fn family(path: &str) -> Result<Vec<String>, DomainError> {
    let Some(stem) = path.strip_suffix(".gguf") else {
        return Ok(vec![path.into()]);
    };
    let Some((prefix_index, count)) = stem.rsplit_once("-of-") else {
        return Ok(vec![path.into()]);
    };
    let Some((prefix, index)) = prefix_index.rsplit_once('-') else {
        return Err(DomainError::Conflict);
    };
    if index.len() != 5
        || count.len() != 5
        || !index
            .bytes()
            .chain(count.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(DomainError::Conflict);
    }
    let count: usize = count.parse().map_err(|_| DomainError::Conflict)?;
    if index != "00001" || !(1..=1024).contains(&count) {
        return Err(DomainError::Conflict);
    }
    Ok((1..=count)
        .map(|n| format!("{prefix}-{n:05}-of-{count:05}.gguf"))
        .collect())
}

impl LaunchPlan {
    /// Verify only this launch's weights/projector (all split shards), explicit
    /// auxiliary model arguments, and shared binaries/libraries. Never mutate
    /// the pinned catalog or its fingerprint to obtain this runtime subset.
    pub fn runtime_materials(&self) -> Result<Vec<Material>, DomainError> {
        if self.fingerprint()? != self.plan_sha256
            || self.materials.is_empty()
            || self.materials.len() > 1024
        {
            return Err(DomainError::Conflict);
        }
        let mut required = BTreeSet::new();
        for (path, kind) in std::iter::once((&self.model_path, "weight"))
            .chain(self.projector.iter().map(|p| (p, "projector")))
        {
            for shard in family(path)? {
                required.insert((shard, kind.to_owned()));
            }
        }
        for material in &self.materials {
            if matches!(material.kind.as_str(), "weight" | "projector")
                && self.args.iter().any(|arg| {
                    arg == &material.path
                        || arg
                            .split_once('=')
                            .is_some_and(|(_, value)| value == material.path)
                })
            {
                for shard in family(&material.path)? {
                    required.insert((shard, material.kind.clone()));
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut selected = Vec::new();
        for material in &self.materials {
            if !Path::new(&material.path).is_absolute() || !seen.insert(&material.path) {
                return Err(DomainError::Conflict);
            }
            if required.remove(&(material.path.clone(), material.kind.clone()))
                || !matches!(material.kind.as_str(), "weight" | "projector")
            {
                selected.push(material.clone());
            }
        }
        if !required.is_empty()
            || !selected
                .iter()
                .any(|m| m.path == self.command && m.kind == "executable")
        {
            return Err(DomainError::Unavailable(
                "incomplete selected runtime materials".into(),
            ));
        }
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama::native::MaterialVerifier;
    use std::{collections::BTreeMap, fs};
    fn plan(root: &Path, model: &str, weights: &[&str]) -> LaunchPlan {
        let material = |name: &str, kind: &str| Material {
            path: root.join(name).display().to_string(),
            kind: kind.into(),
            sha256: super::super::hash(b"fixture"),
        };
        let mut materials = vec![
            material("server", "executable"),
            material("library", "library"),
        ];
        for name in ["server", "library"] {
            fs::write(root.join(name), b"fixture").unwrap();
        }
        materials.extend(weights.iter().map(|name| material(name, "weight")));
        let mut p = LaunchPlan {
            command: root.join("server").display().to_string(),
            args: vec![],
            environment: BTreeMap::new(),
            model_id: model.into(),
            model_path: root.join(model).display().to_string(),
            projector: None,
            requested_context: 90000,
            max_tokens: 4096,
            profile_sha256: "0".repeat(64),
            plan_sha256: String::new(),
            sources: BTreeMap::new(),
            materials,
        };
        p.plan_sha256 = p.fingerprint().unwrap();
        p
    }
    #[test]
    fn catalog_wide_manifest_never_reads_unselected_weights_and_warm_checks_reuse_digest() {
        let d = tempfile::tempdir().unwrap();
        let p = plan(
            d.path(),
            "35b.gguf",
            &[
                "35b.gguf",
                "27b.gguf",
                "flash-00001-of-00002.gguf",
                "flash-00002-of-00002.gguf",
            ],
        );
        fs::write(d.path().join("35b.gguf"), b"fixture").unwrap();
        // All unselected files deliberately do not exist. Reading one fails.
        let selected = p.runtime_materials().unwrap();
        assert_eq!(selected.len(), 3);
        let mut verifier = MaterialVerifier::default();
        verifier.verify(&selected).unwrap();
        assert_eq!(verifier.bytes_hashed, 21);
        verifier.verify(&p.runtime_materials().unwrap()).unwrap();
        assert_eq!(verifier.bytes_hashed, 21);
        assert!(verifier.verify(&p.materials).is_err()); // strict capture audit stays strict
        fs::write(d.path().join("35b.gguf"), b"changed").unwrap();
        assert_eq!(verifier.verify(&selected), Err(DomainError::Conflict));
        assert_eq!(p.fingerprint().unwrap(), p.plan_sha256);
    }
    #[test]
    fn split_model_requires_every_shard_and_ignores_neighboring_family() {
        let d = tempfile::tempdir().unwrap();
        let mut p = plan(
            d.path(),
            "flash-00001-of-00002.gguf",
            &[
                "35b.gguf",
                "flash-00001-of-00002.gguf",
                "flash-00002-of-00002.gguf",
                "other-00001-of-00002.gguf",
            ],
        );
        for n in [1, 2] {
            fs::write(
                d.path().join(format!("flash-{n:05}-of-00002.gguf")),
                b"fixture",
            )
            .unwrap();
        }
        let selected = p.runtime_materials().unwrap();
        assert_eq!(selected.len(), 4);
        MaterialVerifier::default().verify(&selected).unwrap();
        fs::remove_file(d.path().join("flash-00002-of-00002.gguf")).unwrap();
        assert!(MaterialVerifier::default().verify(&selected).is_err());
        p.materials
            .retain(|m| !m.path.ends_with("flash-00002-of-00002.gguf"));
        p.plan_sha256 = p.fingerprint().unwrap();
        assert!(p.runtime_materials().is_err());
    }
    #[test]
    fn projector_auxiliary_weights_and_shared_runtime_are_not_skipped() {
        let d = tempfile::tempdir().unwrap();
        let mut p = plan(
            d.path(),
            "35b.gguf",
            &["35b.gguf", "draft.gguf", "unselected.gguf"],
        );
        let projector = d.path().join("projector.gguf").display().to_string();
        p.projector = Some(projector.clone());
        p.materials.push(Material {
            path: projector,
            kind: "projector".into(),
            sha256: super::super::hash(b"fixture"),
        });
        p.args.push(format!(
            "--model-draft={}",
            d.path().join("draft.gguf").display()
        ));
        p.plan_sha256 = p.fingerprint().unwrap();
        let selected = p.runtime_materials().unwrap();
        assert_eq!(selected.len(), 5);
        for name in ["35b.gguf", "draft.gguf", "projector.gguf"] {
            fs::write(d.path().join(name), b"fixture").unwrap();
        }
        MaterialVerifier::default().verify(&selected).unwrap();
        for name in ["library", "projector.gguf", "draft.gguf"] {
            fs::write(d.path().join(name), b"changed").unwrap();
            assert_eq!(
                MaterialVerifier::default().verify(&selected),
                Err(DomainError::Conflict)
            );
            fs::write(d.path().join(name), b"fixture").unwrap();
        }
        p.model_path = d.path().join("unselected.gguf").display().to_string();
        assert_eq!(p.runtime_materials(), Err(DomainError::Conflict));
    }
}
