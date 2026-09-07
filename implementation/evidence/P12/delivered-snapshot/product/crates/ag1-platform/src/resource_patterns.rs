//! Bounded package include/exclude patterns. Pattern evaluation cannot grant a
//! filesystem capability; it only selects from the host's discovered path list.
use ag1_core::DomainError;
use globset::{GlobBuilder, GlobMatcher};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};
fn limit() -> DomainError {
    DomainError::LimitExceeded("resource pattern work".into())
}
struct Work(usize);
impl Work {
    fn step(&mut self) -> Result<(), DomainError> {
        self.0 = self.0.checked_sub(1).ok_or_else(limit)?;
        Ok(())
    }
}
fn split_top(text: &str, separator: char) -> Vec<&str> {
    let mut depth = 0i32;
    let mut escaped = false;
    let mut start = 0;
    let mut out = vec![];
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
        } else if c == separator && depth == 0 {
            out.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&text[start..]);
    out
}
fn braces(text: &str, depth: usize, work: &mut Work) -> Result<Vec<String>, DomainError> {
    work.step()?;
    if depth > 16 {
        return Err(limit());
    }
    let mut start = None;
    let mut nesting = 0;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '{' {
            if nesting == 0 {
                start = Some(i);
            }
            nesting += 1;
        }
        if c == '}' && nesting > 0 {
            nesting -= 1;
            if nesting == 0 {
                let from = start.unwrap();
                let body = &text[from + 1..i];
                let mut options = split_top(body, ',')
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if options.len() == 1 {
                    let range = body.split("..").collect::<Vec<_>>();
                    if (2..=3).contains(&range.len()) {
                        let numeric = range[0]
                            .parse::<i64>()
                            .ok()
                            .zip(range[1].parse::<i64>().ok());
                        let alpha = if range[0].len() == 1
                            && range[1].len() == 1
                            && range[0].as_bytes()[0].is_ascii_alphabetic()
                            && range[1].as_bytes()[0].is_ascii_alphabetic()
                        {
                            Some((
                                i64::from(range[0].as_bytes()[0]),
                                i64::from(range[1].as_bytes()[0]),
                            ))
                        } else {
                            None
                        };
                        if let Some((a, b)) = numeric.or(alpha) {
                            let step = range
                                .get(2)
                                .map(|s| s.parse::<i64>())
                                .transpose()
                                .map_err(|_| {
                                    DomainError::InvalidInput("resource brace step".into())
                                })?
                                .unwrap_or(1)
                                .checked_abs()
                                .filter(|n| *n > 0)
                                .ok_or_else(limit)?;
                            if a.abs_diff(b) / step as u64 > 255 {
                                return Err(limit());
                            }
                            options.clear();
                            let mut value = a;
                            let padded = numeric.is_some()
                                && range[..2].iter().any(|s| {
                                    s.trim_start_matches('-').starts_with('0')
                                        && s.trim_start_matches('-').len() > 1
                                });
                            let width = range[0].len().max(range[1].len());
                            loop {
                                options.push(if numeric.is_some() {
                                    if padded {
                                        format!("{value:0width$}")
                                    } else {
                                        value.to_string()
                                    }
                                } else {
                                    char::from(value as u8).to_string()
                                });
                                if value == b {
                                    break;
                                }
                                let next = if a <= b {
                                    value.checked_add(step)
                                } else {
                                    value.checked_sub(step)
                                }
                                .ok_or_else(limit)?;
                                if (a <= b && next > b) || (a > b && next < b) {
                                    break;
                                }
                                value = next;
                            }
                        }
                    }
                }
                if options.len() > 1 || options.first().is_some_and(|s| s != body) {
                    let mut result = vec![];
                    for option in options {
                        let combined = format!("{}{}{}", &text[..from], option, &text[i + 1..]);
                        result.extend(braces(&combined, depth + 1, work)?);
                        if result.len() > 256 {
                            return Err(limit());
                        }
                    }
                    return Ok(result);
                }
            }
        }
    }
    Ok(vec![text.into()])
}
enum Atom {
    Glob(GlobMatcher),
    Ext(char, Vec<Vec<Atom>>),
}
fn glob(text: &str) -> Result<Atom, DomainError> {
    let parsed = GlobBuilder::new(text)
        .literal_separator(true)
        .backslash_escape(true)
        .empty_alternates(true)
        .build()
        .or_else(|_| GlobBuilder::new(&globset::escape(text)).build())
        .map_err(|_| DomainError::InvalidInput("resource glob".into()))?;
    Ok(Atom::Glob(parsed.compile_matcher()))
}
fn atoms(text: &str, depth: usize) -> Result<Vec<Atom>, DomainError> {
    if depth > 16 {
        return Err(limit());
    }
    let bytes = text.as_bytes();
    let mut out = vec![];
    let mut start = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if b"@+?*!".contains(&bytes[i]) && bytes[i + 1] == b'(' {
            let mut j = i + 2;
            let mut nested = 1;
            let mut separators = vec![j];
            while j < bytes.len() {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'(' {
                    nested += 1;
                } else if bytes[j] == b')' {
                    nested -= 1;
                    if nested == 0 {
                        break;
                    }
                } else if bytes[j] == b'|' && nested == 1 {
                    separators.push(j + 1);
                }
                j += 1;
            }
            if nested == 0 {
                if i > start {
                    out.push(glob(&text[start..i])?);
                }
                let mut choices = vec![];
                for (k, from) in separators.iter().copied().enumerate() {
                    let end = separators.get(k + 1).map(|n| n - 1).unwrap_or(j);
                    choices.push(atoms(&text[from..end], depth + 1)?);
                }
                out.push(Atom::Ext(bytes[i] as char, choices));
                i = j + 1;
                start = i;
                continue;
            }
        }
        i += 1;
    }
    if start < text.len() {
        out.push(glob(&text[start..])?);
    }
    Ok(out)
}
fn ends(
    sequence: &[Atom],
    text: &str,
    start: usize,
    work: &mut Work,
) -> Result<BTreeSet<usize>, DomainError> {
    let mut current = BTreeSet::from([start]);
    for atom in sequence {
        let mut next = BTreeSet::new();
        for from in current {
            work.step()?;
            match atom {
                Atom::Glob(glob) => {
                    for end in (from..=text.len()).filter(|n| text.is_char_boundary(*n)) {
                        work.step()?;
                        if glob.is_match(&text[from..end]) {
                            next.insert(end);
                        }
                    }
                }
                Atom::Ext(mode, choices) => {
                    let mut matches = BTreeSet::new();
                    for choice in choices {
                        matches.extend(ends(choice, text, from, work)?);
                    }
                    match mode {
                        '@' => next.extend(matches),
                        '?' => {
                            next.insert(from);
                            next.extend(matches);
                        }
                        '*' | '+' => {
                            let mut queue = matches.iter().copied().collect::<Vec<_>>();
                            let mut seen = matches;
                            if *mode == '*' {
                                seen.insert(from);
                            }
                            while let Some(offset) = queue.pop() {
                                for choice in choices {
                                    for end in ends(choice, text, offset, work)? {
                                        if seen.insert(end) {
                                            queue.push(end);
                                        }
                                    }
                                }
                            }
                            next.extend(seen);
                        }
                        '!' => {
                            next.extend(
                                (from..=text.len())
                                    .filter(|n| text.is_char_boundary(*n) && !matches.contains(n)),
                            );
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    Ok(current)
}
enum Segment {
    Recursive,
    Match { dot: bool, atoms: Vec<Atom> },
}
struct Pattern {
    negative: bool,
    alternatives: Vec<(bool, Vec<Segment>)>,
}
impl Pattern {
    fn compile(text: &str, work: &mut Work) -> Result<Self, DomainError> {
        if text.len() > 4096 {
            return Err(limit());
        }
        let mut text = text;
        let mut negative = false;
        while let Some(rest) = text.strip_prefix('!') {
            negative = !negative;
            text = rest;
        }
        if text.starts_with('#') {
            return Ok(Self {
                negative: false,
                alternatives: vec![],
            });
        }
        let mut alternatives = vec![];
        for text in braces(text, 0, work)? {
            let mut segments = vec![];
            for segment in text.split('/').filter(|s| !s.is_empty()) {
                segments.push(if segment == "**" {
                    Segment::Recursive
                } else {
                    Segment::Match {
                        dot: segment.starts_with('.') || segment.starts_with("\\."),
                        atoms: atoms(segment, 0)?,
                    }
                });
            }
            // A terminal slash is material for file patterns in the reference.
            if text.ends_with('/') {
                segments.push(Segment::Match {
                    dot: false,
                    atoms: vec![],
                });
            }
            alternatives.push((text.starts_with('/'), segments));
        }
        Ok(Self {
            negative,
            alternatives,
        })
    }
    fn matches(&self, text: &str, work: &mut Work) -> Result<bool, DomainError> {
        let parts = text
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        for (absolute, segments) in &self.alternatives {
            if *absolute != text.starts_with('/') {
                continue;
            }
            let mut offsets = BTreeSet::from([0]);
            for segment in segments {
                let mut next = BTreeSet::new();
                for from in offsets {
                    work.step()?;
                    match segment {
                        Segment::Recursive => {
                            next.insert(from);
                            for (i, part) in parts.iter().enumerate().skip(from) {
                                if part.starts_with('.') {
                                    break;
                                }
                                next.insert(i + 1);
                            }
                        }
                        Segment::Match { dot, atoms } => {
                            if let Some(part) = parts.get(from)
                                && (*dot || !part.starts_with('.'))
                                && ends(atoms, part, 0, work)?.contains(&part.len())
                            {
                                next.insert(from + 1);
                            }
                        }
                    }
                }
                offsets = next;
            }
            if offsets.contains(&parts.len()) {
                return Ok(!self.negative);
            }
        }
        Ok(self.negative)
    }
}
pub fn is_pattern(text: &str) -> bool {
    text.starts_with(['!', '+', '-']) || text.contains(['*', '?'])
}
fn relative(base: &Path, path: &Path) -> String {
    let base = base.components().collect::<Vec<_>>();
    let target = path.components().collect::<Vec<_>>();
    let shared = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut parts = vec!["..".to_string(); base.len() - shared];
    parts.extend(target[shared..].iter().filter_map(|p| match p {
        Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
        _ => None,
    }));
    parts.join("/")
}
fn candidates(path: &str, base: &str, exact: bool) -> Vec<String> {
    let p = Path::new(path);
    let mut out = vec![relative(Path::new(base), p), path.into()];
    if !exact {
        out.push(
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );
    }
    if p.file_name().is_some_and(|n| n == "SKILL.md")
        && let Some(parent) = p.parent()
    {
        out.push(relative(Path::new(base), parent));
        out.push(parent.to_string_lossy().into_owned());
        if !exact {
            out.push(
                parent
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    out
}
pub struct Filters {
    includes: Vec<Pattern>,
    excludes: Vec<Pattern>,
    force_in: Vec<String>,
    force_out: Vec<String>,
}
impl Filters {
    pub fn new(patterns: &[String], overrides_only: bool) -> Result<Self, DomainError> {
        if patterns.len() > 256 {
            return Err(limit());
        }
        let mut work = Work(500_000);
        let mut out = Self {
            includes: vec![],
            excludes: vec![],
            force_in: vec![],
            force_out: vec![],
        };
        for p in patterns {
            if p.len() > 4096 {
                return Err(limit());
            }
            if let Some(p) = p.strip_prefix('+') {
                out.force_in.push(
                    p.strip_prefix("./")
                        .or_else(|| p.strip_prefix(".\\"))
                        .unwrap_or(p)
                        .into(),
                );
            } else if let Some(p) = p.strip_prefix('-') {
                out.force_out.push(
                    p.strip_prefix("./")
                        .or_else(|| p.strip_prefix(".\\"))
                        .unwrap_or(p)
                        .into(),
                );
            } else if let Some(p) = p.strip_prefix('!') {
                out.excludes.push(Pattern::compile(p, &mut work)?);
            } else if !overrides_only {
                out.includes.push(Pattern::compile(p, &mut work)?);
            }
        }
        Ok(out)
    }
    pub fn select(&self, paths: &[String], base: &str) -> Result<Vec<String>, DomainError> {
        if paths.len() > 4096 || paths.iter().any(|p| p.len() > 8192) || base.len() > 8192 {
            return Err(limit());
        }
        let mut work = Work(2_000_000);
        let mut selected = vec![];
        let mut forced = vec![];
        for path in paths {
            let options = candidates(path, base, false);
            let exact = candidates(path, base, true);
            let matches = |patterns: &[Pattern], work: &mut Work| -> Result<bool, DomainError> {
                for pattern in patterns {
                    for option in &options {
                        if pattern.matches(option, work)? {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            };
            let enabled = (self.includes.is_empty() || matches(&self.includes, &mut work)?)
                && !matches(&self.excludes, &mut work)?;
            if exact.iter().any(|p| self.force_out.contains(p)) {
                continue;
            }
            if enabled {
                selected.push(path.clone());
            } else if exact.iter().any(|p| self.force_in.contains(p)) {
                forced.push(path.clone());
            }
        }
        selected.extend(forced);
        let mut seen = BTreeSet::new();
        selected.retain(|p| seen.insert(p.clone()));
        Ok(selected)
    }
}
