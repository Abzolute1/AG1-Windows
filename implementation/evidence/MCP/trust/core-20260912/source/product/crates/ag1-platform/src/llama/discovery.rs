//! Read-only, bounded model/projector discovery. Files are verified again against
//! the immutable launch material manifest before any owned process may start.
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitGguf {
    pub prefix: String,
    pub index: u32,
    pub count: u32,
}
pub fn usable_gguf(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".gguf")
        && !n.contains("mmproj")
        && !n.starts_with("mtp-")
        && !n.starts_with("downloading_")
        && !n.ends_with(".part")
}
pub fn split_gguf(name: &str) -> Option<SplitGguf> {
    if !name.to_lowercase().ends_with(".gguf") {
        return None;
    }
    let stem = &name[..name.len() - 5];
    let separator = stem.to_ascii_lowercase().rfind("-of-")?;
    let (before, count) = (&stem[..separator], &stem[separator + 4..]);
    let (prefix, index) = before.rsplit_once('-')?;
    if prefix.is_empty()
        || index.len() != 5
        || count.len() != 5
        || !index
            .bytes()
            .chain(count.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let index = index.parse().ok()?;
    let count = count.parse().ok()?;
    if count < 2 || index < 1 || index > count {
        return None;
    }
    Some(SplitGguf {
        prefix: prefix.into(),
        index,
        count,
    })
}
fn quant(s: &str) -> bool {
    if ["bf16", "f16", "f32", "fp16", "fp32"].contains(&s) {
        return true;
    }
    let b = s.as_bytes();
    if b.len() >= 4 && b[0..2] == *b"iq" && (b'1'..=b'4').contains(&b[2]) && b[3] == b'_' {
        return b.len() > 4
            && b[4..]
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    }
    if b.len() < 2 || b[0] != b'q' || !(b'2'..=b'8').contains(&b[1]) {
        return false;
    }
    if b.len() == 2 {
        return true;
    }
    if b.len() < 4 || b[2] != b'_' || !b"km".contains(&b[3]) {
        return false;
    }
    b.len() == 4 || (b.len() > 5 && b[4] == b'_' && b[5..].iter().all(|b| b"mxl".contains(b)))
}
pub fn family_key(path: &Path) -> String {
    let mut name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if name.ends_with(".gguf") {
        name.truncate(name.len() - 5);
    }
    for prefix in ["mmproj-", "downloading_"] {
        if let Some(n) = name.strip_prefix(prefix) {
            name = n.into();
        }
    }
    for _ in 0..3 {
        let Some((before, suffix)) = name.rsplit_once('-') else {
            break;
        };
        if !quant(suffix) {
            break;
        }
        name = before.strip_suffix("-ud").unwrap_or(before).into();
    }
    name
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub path: PathBuf,
    pub id: String,
    pub size_bytes: u64,
    pub shards: Vec<PathBuf>,
}
fn score(m: &DiscoveredModel) -> u128 {
    let id = m.id.to_lowercase();
    u128::from(m.size_bytes)
        + if id.contains("a3b") {
            1_000_000_000_000_000
        } else {
            0
        }
        + if id.contains("qwen3.6") || id.contains("qwen36") {
            100_000_000_000_000
        } else {
            0
        }
        + if id.contains("qwen3.8") || id.contains("qwen38") {
            10_000_000_000_000
        } else {
            0
        }
        + if id.contains("q4_k_xl") {
            1_000_000_000_000
        } else {
            0
        }
}
fn files(roots: &[PathBuf]) -> Result<Vec<PathBuf>, DomainError> {
    if roots.len() > 128 {
        return Err(DomainError::LimitExceeded("model search roots".into()));
    }
    let mut stack: Vec<_> = roots.iter().rev().cloned().map(|p| (p, 0)).collect();
    let mut visited = BTreeSet::new();
    let mut out = vec![];
    let mut count = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > 6 {
            continue;
        }
        let Ok(identity) = fs::canonicalize(&dir) else {
            continue;
        };
        if !visited.insert(identity) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut bounded = Vec::new();
        for entry in entries {
            count += 1;
            if count > 100_000 {
                return Err(DomainError::LimitExceeded("model discovery entries".into()));
            }
            if let Ok(entry) = entry {
                bounded.push(entry);
            }
        }
        bounded.sort_by_key(|e| e.file_name());
        let entries = bounded;
        for entry in entries {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push((path, depth + 1));
            } else if meta.is_file() {
                out.push(path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
pub fn discover_models(roots: &[PathBuf]) -> Result<Vec<DiscoveredModel>, DomainError> {
    let mut out = vec![];
    for path in files(roots)? {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !usable_gguf(&name) {
            continue;
        }
        let split = split_gguf(&name);
        if split.as_ref().is_some_and(|s| s.index != 1) {
            continue;
        }
        let shards = if let Some(s) = &split {
            (1..=s.count)
                .map(|i| path.with_file_name(format!("{}-{i:05}-of-{:05}.gguf", s.prefix, s.count)))
                .collect::<Vec<_>>()
        } else {
            vec![path.clone()]
        };
        let mut size = 0u64;
        let mut complete = true;
        for shard in &shards {
            match fs::metadata(shard) {
                Ok(m) if m.is_file() && (split.is_none() || m.len() > 0) => {
                    size = size
                        .checked_add(m.len())
                        .ok_or_else(|| DomainError::LimitExceeded("model size".into()))?;
                }
                _ => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete {
            continue;
        }
        let id = split
            .map(|s| s.prefix)
            .unwrap_or_else(|| name[..name.len() - 5].into());
        out.push(DiscoveredModel {
            path,
            id,
            size_bytes: size,
            shards,
        });
    }
    out.sort_by_key(|m| std::cmp::Reverse(score(m)));
    Ok(out)
}
pub fn discover_projector(
    model: &Path,
    roots: &[PathBuf],
    override_path: Option<&str>,
) -> Result<Option<PathBuf>, DomainError> {
    if let Some(value) = override_path.map(str::trim) {
        if ["0", "off", "false", "none"].contains(&value) {
            return Ok(None);
        }
        if !value.is_empty() && Path::new(value).is_file() {
            return Ok(Some(value.into()));
        }
    }
    let family = family_key(model);
    if family.is_empty() {
        return Ok(None);
    }
    let mut roots = roots.to_vec();
    if let Some(parent) = model.parent() {
        roots.insert(0, parent.into());
    }
    let mut candidates = vec![];
    for path in files(&roots)? {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if !name.ends_with(".gguf")
            || !name.contains("mmproj")
            || name.starts_with("downloading_")
            || family_key(&path) != family
        {
            continue;
        }
        let score = if path.parent() == model.parent() {
            1_000_000_000_000_000_000u128
        } else {
            0
        } + if name.contains("bf16") || name.contains("f16") {
            100_000_000_000_000
        } else {
            0
        } + u128::from(fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
        candidates.push((score, path));
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(candidates.first().map(|v| v.1.clone()))
}
