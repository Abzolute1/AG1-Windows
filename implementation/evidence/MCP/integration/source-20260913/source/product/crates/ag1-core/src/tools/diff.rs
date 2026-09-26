//! Bounded adaptation of jsdiff's line Myers paths (BSD-3-Clause; licenses/jsdiff-port).
//! Numbered presentation follows AG1's edit-diff.ts (MIT).
use crate::DomainError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Same,
    Added,
    Removed,
}
#[derive(Clone, Copy)]
struct Node {
    kind: Kind,
    count: usize,
    previous: Option<usize>,
}
#[derive(Clone, Copy)]
struct Path {
    old: isize,
    last: Option<usize>,
}
struct Part {
    kind: Kind,
    value: String,
}
fn push(arena: &mut Vec<Node>, path: &mut Path, kind: Kind, count: usize, merge: bool) {
    let mut node = Node {
        kind,
        count,
        previous: path.last,
    };
    if merge
        && let Some(i) = path.last
        && arena[i].kind == kind
    {
        node.count += arena[i].count;
        node.previous = arena[i].previous;
    }
    path.last = Some(arena.len());
    arena.push(node);
}
fn common(
    arena: &mut Vec<Node>,
    path: &mut Path,
    old: &[&str],
    new: &[&str],
    diagonal: isize,
) -> isize {
    let mut n = path.old - diagonal;
    let mut count = 0;
    while path.old + 1 < old.len() as isize
        && n + 1 < new.len() as isize
        && old[(path.old + 1) as usize] == new[(n + 1) as usize]
    {
        path.old += 1;
        n += 1;
        count += 1;
    }
    if count > 0 {
        push(arena, path, Kind::Same, count, false);
    }
    n
}
fn build(arena: &[Node], mut path: Path, old: &[&str], new: &[&str]) -> Vec<Part> {
    let mut nodes = vec![];
    while let Some(i) = path.last {
        nodes.push(arena[i]);
        path.last = arena[i].previous;
    }
    nodes.reverse();
    let (mut o, mut n) = (0, 0);
    nodes
        .into_iter()
        .map(|v| {
            let value = if v.kind == Kind::Removed {
                let text = old[o..o + v.count].join("");
                o += v.count;
                text
            } else {
                let text = new[n..n + v.count].join("");
                n += v.count;
                if v.kind == Kind::Same {
                    o += v.count;
                }
                text
            };
            Part {
                kind: v.kind,
                value,
            }
        })
        .collect()
}
fn parts(old: &str, new: &str) -> Result<Vec<Part>, DomainError> {
    if old.len() > 8 * 1024 * 1024 || new.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("diff input bytes".into()));
    }
    let old: Vec<_> = old.split_inclusive('\n').collect();
    let new: Vec<_> = new.split_inclusive('\n').collect();
    if old.len() + new.len() > 200000 {
        return Err(DomainError::LimitExceeded("diff line count".into()));
    }
    let mut arena = vec![];
    let mut initial = Path {
        old: -1,
        last: None,
    };
    let n = common(&mut arena, &mut initial, &old, &new, 0);
    if initial.old + 1 >= old.len() as isize && n + 1 >= new.len() as isize {
        return Ok(build(&arena, initial, &old, &new));
    }
    let mut best = BTreeMap::from([(0isize, initial)]);
    let (mut min, mut max) = (isize::MIN, isize::MAX);
    let mut work = 0;
    for length in 1..=old.len() + new.len() {
        let length = length as isize;
        let mut diagonal = min.max(-length);
        while diagonal <= max.min(length) {
            work += 1;
            if work > 2_000_000 || arena.len() > 2_000_000 {
                return Err(DomainError::LimitExceeded(
                    "diff work budget; split the change into smaller edits".into(),
                ));
            }
            let remove = best.remove(&(diagonal - 1));
            let add = best.get(&(diagonal + 1)).copied();
            let can_add = add.is_some_and(|p| {
                let n = p.old - diagonal;
                n >= 0 && n < new.len() as isize
            });
            let can_remove = remove.is_some_and(|p| p.old + 1 < old.len() as isize);
            if !can_add && !can_remove {
                best.remove(&diagonal);
                diagonal += 2;
                continue;
            }
            let mut base = if !can_remove || can_add && remove.unwrap().old < add.unwrap().old {
                let mut p = add.unwrap();
                push(&mut arena, &mut p, Kind::Added, 1, true);
                p
            } else {
                let mut p = remove.unwrap();
                p.old += 1;
                push(&mut arena, &mut p, Kind::Removed, 1, true);
                p
            };
            let n = common(&mut arena, &mut base, &old, &new, diagonal);
            if base.old + 1 >= old.len() as isize && n + 1 >= new.len() as isize {
                return Ok(build(&arena, base, &old, &new));
            }
            best.insert(diagonal, base);
            if base.old + 1 >= old.len() as isize {
                max = max.min(diagonal - 1);
            }
            if n + 1 >= new.len() as isize {
                min = min.max(diagonal + 1);
            }
            diagonal += 2;
        }
    }
    Err(DomainError::InvalidInput("unreachable diff path".into()))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub before_lines: usize,
    pub after_lines: usize,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub change_blocks: usize,
    pub diff_bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedDiff {
    pub diff: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_changed_line: Option<usize>,
    pub summary: DiffSummary,
}
fn lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.split('\n').count() - usize::from(s.ends_with('\n'))
    }
}
pub fn generate(old: &str, new: &str, context: usize) -> Result<GeneratedDiff, DomainError> {
    if context > 10000 {
        return Err(DomainError::LimitExceeded("diff context lines".into()));
    }
    let parts = parts(old, new)?;
    let width = old
        .split('\n')
        .count()
        .max(new.split('\n').count())
        .to_string()
        .len();
    let (mut o, mut n) = (1, 1);
    let mut previous_change = false;
    let mut inside = false;
    let mut first = None;
    let mut out = vec![];
    let mut summary = DiffSummary {
        before_lines: lines(old),
        after_lines: lines(new),
        added_lines: 0,
        removed_lines: 0,
        change_blocks: 0,
        diff_bytes: 0,
    };
    for (i, part) in parts.iter().enumerate() {
        let mut raw: Vec<_> = part.value.split('\n').collect();
        if raw.last() == Some(&"") {
            raw.pop();
        }
        if part.kind != Kind::Same {
            if !inside {
                summary.change_blocks += 1;
            }
            inside = true;
            first.get_or_insert(n);
            for line in raw {
                if part.kind == Kind::Added {
                    out.push(format!("+{n:width$} {line}"));
                    n += 1;
                    summary.added_lines += 1;
                } else {
                    out.push(format!("-{o:width$} {line}"));
                    o += 1;
                    summary.removed_lines += 1;
                }
            }
            previous_change = true;
        } else {
            inside = false;
            let following = parts.get(i + 1).is_some_and(|p| p.kind != Kind::Same);
            let mut intervals = vec![];
            if previous_change && following {
                if raw.len() <= context * 2 {
                    intervals.push((0, raw.len()));
                } else {
                    intervals.push((0, context));
                    intervals.push((raw.len() - context, raw.len()));
                }
            } else if previous_change {
                intervals.push((0, context.min(raw.len())));
            } else if following {
                intervals.push((raw.len().saturating_sub(context), raw.len()));
            }
            let mut pos = 0;
            for (start, end) in intervals {
                if start > pos {
                    out.push(format!(" {:width$} ...", ""));
                    o += start - pos;
                    n += start - pos;
                }
                for line in &raw[start..end] {
                    out.push(format!(" {o:width$} {line}"));
                    o += 1;
                    n += 1;
                }
                pos = end;
            }
            if pos < raw.len() {
                if previous_change {
                    out.push(format!(" {:width$} ...", ""));
                }
                o += raw.len() - pos;
                n += raw.len() - pos;
            }
            previous_change = false;
        }
    }
    let diff = out.join("\n");
    summary.diff_bytes = diff.len();
    Ok(GeneratedDiff {
        diff,
        first_changed_line: first,
        summary,
    })
}
pub fn format_summary(s: &DiffSummary) -> String {
    format!(
        "Diff summary: {} change block{}, +{}/-{} lines ({} → {} lines), {} diff bytes.",
        s.change_blocks,
        if s.change_blocks == 1 { "" } else { "s" },
        s.added_lines,
        s.removed_lines,
        s.before_lines,
        s.after_lines,
        s.diff_bytes
    )
}
