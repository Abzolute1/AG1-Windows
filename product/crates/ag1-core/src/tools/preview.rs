//! Exact preview matching rejects ambiguous locations, including overlaps.
//! Hunk locations use actual source offsets (P09 preview correction).
use crate::DomainError;
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub from_line: usize,
    pub to_line: usize,
    pub diff: String,
}
#[derive(Serialize)]
pub struct Preview<'a> {
    pub path: &'a str,
    pub hunks: Vec<Hunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
fn lcs(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    let (m, n) = (old.len(), new.len());
    if m.saturating_mul(n) <= 50000 {
        let mut dp = vec![0usize; (m + 1) * (n + 1)];
        for i in 1..=m {
            for j in 1..=n {
                dp[i * (n + 1) + j] = if old[i - 1] == new[j - 1] {
                    dp[(i - 1) * (n + 1) + j - 1] + 1
                } else {
                    dp[(i - 1) * (n + 1) + j].max(dp[i * (n + 1) + j - 1])
                };
            }
        }
        let (mut i, mut j) = (m, n);
        let mut out = vec![];
        while i > 0 && j > 0 {
            if old[i - 1] == new[j - 1] {
                out.push((i - 1, j - 1));
                i -= 1;
                j -= 1;
            } else if dp[(i - 1) * (n + 1) + j] > dp[i * (n + 1) + j - 1] {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        out.reverse();
        out
    } else {
        let mut indices: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (j, line) in new.iter().enumerate() {
            indices.entry(line).or_default().push(j);
        }
        let mut next = 0;
        let mut out = vec![];
        for (i, line) in old.iter().enumerate() {
            if let Some(js) = indices.get(line) {
                let at = js.partition_point(|j| *j < next);
                if let Some(j) = js.get(at) {
                    out.push((i, *j));
                    next = j + 1;
                }
            }
        }
        out
    }
}
pub fn hunks(old: &str, new: &str) -> Result<Vec<Hunk>, DomainError> {
    if old.len() > 8 * 1024 * 1024 || new.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("preview bytes".into()));
    }
    let old: Vec<_> = old.split('\n').collect();
    let new: Vec<_> = new.split('\n').collect();
    if old.len() + new.len() > 200000 {
        return Err(DomainError::LimitExceeded("preview lines".into()));
    }
    let mut matches = lcs(&old, &new);
    matches.push((old.len(), new.len()));
    let (mut oi, mut ni) = (0usize, 0usize);
    let mut ops: Vec<(char, &str)> = Vec::new();
    for (o, n) in matches {
        ops.extend(old[oi..o].iter().map(|s| ('-', *s)));
        ops.extend(new[ni..n].iter().map(|s| ('+', *s)));
        if o < old.len() {
            ops.push((' ', old[o]));
        }
        oi = o + 1;
        ni = n + 1;
    }
    let mut positions = vec![(0usize, 0usize)];
    let mut ranges: Vec<(usize, usize)> = vec![];
    for (i, (kind, _)) in ops.iter().enumerate() {
        let (o, n) = positions[i];
        positions.push((o + usize::from(*kind != '+'), n + usize::from(*kind != '-')));
        if *kind != ' ' {
            let start = i.saturating_sub(3);
            let end = (i + 4).min(ops.len());
            if let Some(last) = ranges.last_mut().filter(|r| start <= r.1) {
                last.1 = end;
            } else {
                ranges.push((start, end));
            }
        }
    }
    if ranges.len() > 10000 {
        return Err(DomainError::LimitExceeded("preview hunks".into()));
    }
    let mut out = vec![];
    let mut output_bytes = 0;
    for (start, end) in ranges {
        let (os, ns) = positions[start];
        let (oe, ne) = positions[end];
        let lines: Vec<_> = ops[start..end]
            .iter()
            .map(|(kind, line)| format!("{kind}{line}"))
            .collect();
        let diff = format!(
            "@@ -{},{} +{},{} @@\n{}",
            os + 1,
            oe - os,
            ns + 1,
            ne - ns,
            lines.join("\n")
        );
        output_bytes += diff.len();
        if output_bytes > 16 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("preview output".into()));
        }
        out.push(Hunk {
            from_line: os + 1,
            to_line: ns + 1,
            diff,
        });
    }
    Ok(out)
}
pub fn preview<'a>(
    path: &'a str,
    content: Option<&str>,
    old: &str,
    new: &str,
) -> Result<Preview<'a>, DomainError> {
    // The SDK loops forever for empty oldText. Reject before matching (P09 preview correction).
    if old.is_empty() {
        return Err(DomainError::InvalidInput(
            "oldText must not be empty for editPreview".into(),
        ));
    }
    if old.len() + new.len() > 8 * 1024 * 1024 || content.is_some_and(|s| s.len() > 8 * 1024 * 1024)
    {
        return Err(DomainError::LimitExceeded("preview input".into()));
    }
    let mut result = Preview {
        path,
        hunks: vec![],
        warnings: None,
        error: None,
    };
    let Some(content) = content else {
        result.error = Some(format!("file not found: {path}"));
        return Ok(result);
    };
    let count = super::edit::occurrences(content, old);
    if count == 0 {
        result.error = Some("oldText not found in file".into());
        return Ok(result);
    }
    if count > 1 {
        result.error = Some(format!("oldText found {count} times, use more context"));
        return Ok(result);
    }
    let next = content.replacen(old, new, 1);
    result.hunks = hunks(content, &next)?;
    result.warnings = Some(if content.encode_utf16().count() > 50000 {
        vec!["file is large, diff may be slow".into()]
    } else {
        vec![]
    });
    Ok(result)
}
